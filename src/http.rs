//! A small HTTP/1.1 client for internet radio: `GET` with redirects, over plain TCP or TLS
//! ([rustls](https://crates.io/crates/rustls), with Mozilla's root certificates built in, so it
//! works the same on every platform).
//!
//! It exists because radio servers are not quite web servers. SHOUTcast 1.x answers
//! `ICY 200 OK` instead of `HTTP/1.0 200 OK`, which general-purpose clients reject, and a
//! stream's body never ends: it runs until the server closes the connection. Bodies sized by
//! `Content-Length` or sent in chunks are read too, for playlists and files on the web.
//!
//! Every request asks for SHOUTcast/Icecast metadata (`Icy-MetaData: 1`); the server then says
//! how often it is mixed into the audio (`icy-metaint`), and [`crate::audio`] takes it out.
//!
//! Reads never block for long: the socket times out every [`TICK`], so that whoever reads can
//! notice it should stop. While waiting for the response, [`get`] checks its `cancel` closure on
//! each tick; reading the body, the caller sees the ticks as `WouldBlock` or `TimedOut` errors
//! (see [`is_tick`]) and may simply read again.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::playlist_file::decode_text;

/// How long a read waits before giving the caller a chance to stop.
pub const TICK: Duration = Duration::from_millis(250);
/// Redirects followed before giving up.
const MAX_REDIRECTS: usize = 5;
/// The most a response's status line and headers may take.
const MAX_HEAD: usize = 16 * 1024;
/// The longest line of a chunked body's framing.
const MAX_LINE: usize = 4096;
/// Bytes read from the socket at a time.
const BUFFER: usize = 16 * 1024;

/// How long to wait for the network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timeouts {
    /// For a connection to each of the server's addresses.
    pub connect: Duration,
    /// For the server to answer, and between reads of the response. A stream that sends nothing
    /// for this long has stalled.
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: Duration::from_secs(10),
            idle: Duration::from_secs(15),
        }
    }
}

/// Why a request failed.
#[derive(Debug)]
pub enum Error {
    /// The URL is not one this client can fetch.
    InvalidUrl(String),
    /// Connecting, sending or receiving failed.
    Io(io::Error),
    /// The secure connection could not be made: often a certificate that does not check out.
    Tls(rustls::Error),
    /// The server answered with an error.
    Status {
        code: u16,
        reason: String,
    },
    /// The server's answer is not HTTP.
    BadResponse(String),
    TooManyRedirects,
    /// The body is longer than the caller allows, in bytes.
    TooLarge(usize),
    /// The server said nothing for [`Timeouts::idle`].
    TimedOut,
    /// The caller asked to stop.
    Cancelled,
}

impl Error {
    /// Whether trying again later might work: a network problem or a server in trouble, rather
    /// than an address that is wrong or refused.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Io(_) | Error::TimedOut | Error::BadResponse(_) => true,
            // Overloaded or failing servers, and SHOUTcast's "server full".
            Error::Status { code, .. } => matches!(code, 408 | 429 | 500..=599),
            Error::InvalidUrl(_)
            | Error::Tls(_)
            | Error::TooManyRedirects
            | Error::TooLarge(_)
            | Error::Cancelled => false,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidUrl(why) => write!(f, "invalid URL: {why}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Tls(e) => write!(f, "secure connection failed: {e}"),
            Error::Status { code, reason } => write!(f, "the server answered {code} {reason}"),
            Error::BadResponse(why) => write!(f, "the server's answer is not HTTP: {why}"),
            Error::TooManyRedirects => write!(f, "too many redirects"),
            Error::TooLarge(limit) => write!(f, "the answer is longer than {limit} bytes"),
            Error::TimedOut => write!(f, "the server stopped answering"),
            Error::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Tls(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        // rustls reports a failed handshake as an I/O error wrapping its own.
        match e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        {
            Some(tls) => Error::Tls(tls.clone()),
            None => Error::Io(e),
        }
    }
}

/// Whether `e` is only a read timing out after [`TICK`], so that the read can be tried again.
#[must_use]
pub fn is_tick(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// An `http://` or `https://` URL, taken apart.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Url {
    pub https: bool,
    /// The host name in lowercase, or an IP address (an IPv6 one without its brackets).
    pub host: String,
    pub port: u16,
    /// The path and query, starting with `/`, with spaces and other characters a request line
    /// cannot hold percent-encoded.
    pub target: String,
}

impl Url {
    /// Parse a URL as a user would type or paste it: surrounding spaces, a `user:password@`
    /// (which is ignored) and a `#fragment` are allowed, and so are spaces and non-ASCII
    /// characters in the path. Host names must be ASCII.
    ///
    /// # Errors
    /// Fails for anything but an `http` or `https` URL with a host.
    pub fn parse(text: &str) -> Result<Url, Error> {
        let invalid = |why: &str| Error::InvalidUrl(format!("{why}: {}", text.trim()));
        let text = text.trim();
        let (scheme, rest) = text
            .split_once("://")
            .ok_or_else(|| invalid("no http:// or https://"))?;
        let https = if scheme.eq_ignore_ascii_case("http") {
            false
        } else if scheme.eq_ignore_ascii_case("https") {
            true
        } else {
            return Err(invalid("only http and https can be played"));
        };
        let rest = rest.split('#').next().unwrap_or(rest);
        let (authority, target) = rest
            .find(['/', '?'])
            .map_or((rest, ""), |i| rest.split_at(i));
        // user:password@ is not used by radio servers; leave it out of the Host header.
        let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
        let (host, port) = match authority.strip_prefix('[') {
            Some(v6) => {
                let (host, after) = v6
                    .split_once(']')
                    .ok_or_else(|| invalid("unclosed [ in the host"))?;
                (host, after.strip_prefix(':'))
            }
            None => match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            },
        };
        if host.is_empty() {
            return Err(invalid("no host"));
        }
        if !host.is_ascii() {
            return Err(invalid("international domain names are not supported"));
        }
        let default_port = if https { 443 } else { 80 };
        let port = match port {
            None | Some("") => default_port,
            Some(p) => p.parse().map_err(|_| invalid("bad port"))?,
        };
        Ok(Url {
            https,
            host: host.to_ascii_lowercase(),
            port,
            target: request_target(target),
        })
    }

    /// Where a redirect's `Location` leads from this URL: a whole URL, one without its scheme
    /// (`//host/path`), a path on this server, or a path relative to this one's folder.
    ///
    /// # Errors
    /// Fails if the result is not a URL this client can fetch.
    pub fn join(&self, location: &str) -> Result<Url, Error> {
        let location = location.trim();
        if location.contains("://") {
            return Url::parse(location);
        }
        let scheme = if self.https { "https" } else { "http" };
        if let Some(rest) = location.strip_prefix("//") {
            return Url::parse(&format!("{scheme}://{rest}"));
        }
        let target = if location.starts_with('/') {
            location.to_owned()
        } else {
            let path = self.path();
            let folder = &path[..path.rfind('/').map_or(0, |i| i + 1)];
            format!("{folder}{location}")
        };
        Ok(Url {
            target: request_target(&target),
            ..self.clone()
        })
    }

    /// The path, without the query.
    #[must_use]
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or(&self.target)
    }

    /// The host, with the port unless it is the scheme's own, as the `Host` header gives them.
    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default_port = if self.https { 443 } else { 80 };
        if self.port == default_port {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.https { "https" } else { "http" };
        write!(f, "{scheme}://{}{}", self.authority(), self.target)
    }
}

/// `target` as a request line can carry it: starting with `/`, with spaces, controls and
/// non-ASCII bytes percent-encoded (`%` itself is left alone, being most likely an escape
/// already).
fn request_target(target: &str) -> String {
    let mut out = String::with_capacity(target.len() + 1);
    if !target.starts_with('/') {
        out.push('/');
    }
    for b in target.bytes() {
        if b.is_ascii_graphic() {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The connection itself, plain or encrypted.
enum Stream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Stream {
    fn socket(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            Stream::Tls(s) => &s.sock,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => match s.read(buf) {
                // Radio servers often close the connection without TLS's goodbye.
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                other => other,
            },
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

/// A connection with a read buffer, which the response's head may have read past.
struct Conn {
    stream: Stream,
    buf: Vec<u8>,
    /// The unread bytes are `buf[pos..len]`.
    pos: usize,
    len: usize,
}

impl Conn {
    fn new(stream: Stream) -> Conn {
        Conn {
            stream,
            buf: vec![0; BUFFER],
            pos: 0,
            len: 0,
        }
    }

    fn buffered(&self) -> &[u8] {
        &self.buf[self.pos..self.len]
    }

    fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.len);
    }

    /// Read more from the socket after what is buffered. Returns how much came; 0 at the end of
    /// the connection, or when the buffer is full.
    fn fill(&mut self) -> io::Result<usize> {
        if self.pos == self.len {
            self.pos = 0;
            self.len = 0;
        } else if self.len == self.buf.len() && self.pos > 0 {
            self.buf.copy_within(self.pos..self.len, 0);
            self.len -= self.pos;
            self.pos = 0;
        }
        let n = self.stream.read(&mut self.buf[self.len..])?;
        self.len += n;
        Ok(n)
    }

    /// Read into `out`: what is buffered first, then from the socket.
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.len && self.fill()? == 0 {
            return Ok(0);
        }
        let n = self.buffered().len().min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.consume(n);
        Ok(n)
    }

    /// Add the next line's bytes to `line`, up to and without its `\n` (and `\r`). Returns
    /// `Ok(true)` once the line is whole, `Ok(false)` at the end of the connection; a tick may
    /// interrupt it, and `line` keeps what was read for the next call.
    fn read_line(&mut self, line: &mut Vec<u8>) -> io::Result<bool> {
        loop {
            if let Some(i) = self.buffered().iter().position(|&b| b == b'\n') {
                line.extend_from_slice(&self.buf[self.pos..self.pos + i]);
                self.consume(i + 1);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(true);
            }
            line.extend_from_slice(self.buffered());
            self.consume(self.buffered().len());
            if line.len() > MAX_LINE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a line of the response is too long",
                ));
            }
            if self.fill()? == 0 {
                return Ok(false);
            }
        }
    }
}

/// Where a chunked body is.
#[derive(Debug)]
enum Chunked {
    /// Reading a chunk's size line, so far.
    Size(Vec<u8>),
    /// In a chunk, with this much of it left.
    Data(u64),
    /// Reading the line break after a chunk.
    DataEnd(Vec<u8>),
    /// Reading the trailer lines after the last chunk.
    Trailer(Vec<u8>),
    Done,
}

/// How the body's end is known.
#[derive(Debug)]
enum Framing {
    /// `Content-Length`: this many bytes are still to come.
    Length(u64),
    Chunked(Chunked),
    /// It ends when the server closes the connection, as a stream's does.
    Close,
}

/// A response whose status and headers have been read. Reading it reads the body.
pub struct Response {
    /// The URL that answered, after redirects.
    pub url: Url,
    /// The status line was SHOUTcast's `ICY 200 OK` rather than HTTP's.
    pub icy: bool,
    /// Header names in lowercase, with their values.
    headers: Vec<(String, String)>,
    framing: Framing,
    conn: Conn,
    idle: Duration,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("url", &self.url)
            .field("icy", &self.icy)
            .field("headers", &self.headers)
            .field("framing", &self.framing)
            .finish_non_exhaustive()
    }
}

impl Response {
    /// A header's value; names are matched ignoring case. Repeated headers give the first.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every header: its name in lowercase, and its value.
    pub fn headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.headers.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    /// The media type, in lowercase and without parameters (`audio/mpeg`).
    #[must_use]
    pub fn content_type(&self) -> Option<String> {
        let value = self.header("content-type")?;
        let mime = value.split(';').next().unwrap_or(value).trim();
        (!mime.is_empty()).then(|| mime.to_ascii_lowercase())
    }

    /// Whether the body has a known end (a length, or chunks), rather than running until the
    /// server closes the connection.
    #[must_use]
    pub fn has_known_end(&self) -> bool {
        !matches!(self.framing, Framing::Close)
    }

    /// Whether the body has been read to its known end. A body that runs until the
    /// connection closes never is.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        matches!(
            self.framing,
            Framing::Length(0) | Framing::Chunked(Chunked::Done)
        )
    }

    /// Read the whole body, if it is no longer than `limit`, waiting up to the idle timeout
    /// between reads and giving up when `cancel` says so.
    ///
    /// # Errors
    /// Fails if the body is longer than `limit`, the connection fails or stalls, or `cancel`
    /// asks to stop.
    pub fn read_to_end_limited(
        &mut self,
        limit: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        let mut chunk = [0; 4096];
        let mut last = Instant::now();
        loop {
            match self.read(&mut chunk) {
                Ok(0) => return Ok(out),
                Ok(n) => {
                    out.extend_from_slice(&chunk[..n]);
                    if out.len() > limit {
                        return Err(Error::TooLarge(limit));
                    }
                    last = Instant::now();
                }
                Err(e) if is_tick(&e) => {
                    if cancel() {
                        return Err(Error::Cancelled);
                    }
                    if last.elapsed() >= self.idle {
                        return Err(Error::TimedOut);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn read_chunked(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Framing::Chunked(state) = &mut self.framing else {
            return Ok(0);
        };
        loop {
            match state {
                Chunked::Size(line) => {
                    if !self.conn.read_line(line)? {
                        return Ok(0);
                    }
                    let text = String::from_utf8_lossy(line);
                    let hex = text.split(';').next().unwrap_or("").trim();
                    let size = u64::from_str_radix(hex, 16).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "bad chunk size")
                    })?;
                    *state = if size == 0 {
                        Chunked::Trailer(Vec::new())
                    } else {
                        Chunked::Data(size)
                    };
                }
                Chunked::Data(left) => {
                    let max = usize::try_from(*left).unwrap_or(usize::MAX).min(out.len());
                    let n = self.conn.read(&mut out[..max])?;
                    if n == 0 {
                        return Ok(0);
                    }
                    // usize always fits in u64 on supported targets.
                    *left -= n as u64;
                    if *left == 0 {
                        *state = Chunked::DataEnd(Vec::new());
                    }
                    return Ok(n);
                }
                Chunked::DataEnd(line) => {
                    if !self.conn.read_line(line)? {
                        return Ok(0);
                    }
                    *state = Chunked::Size(Vec::new());
                }
                Chunked::Trailer(line) => {
                    if !self.conn.read_line(line)? {
                        return Ok(0);
                    }
                    if line.is_empty() {
                        *state = Chunked::Done;
                    } else {
                        line.clear();
                    }
                }
                Chunked::Done => return Ok(0),
            }
        }
    }
}

impl Read for Response {
    /// Read the body. Returns 0 at its end, or where the connection closed before it; see
    /// [`Response::is_complete`]. A read that times out after [`TICK`] fails with an error
    /// [`is_tick`] accepts, and may be tried again.
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        match &mut self.framing {
            Framing::Length(0) => Ok(0),
            Framing::Length(left) => {
                let max = usize::try_from(*left).unwrap_or(usize::MAX).min(out.len());
                let n = self.conn.read(&mut out[..max])?;
                // usize always fits in u64 on supported targets.
                *left -= n as u64;
                Ok(n)
            }
            Framing::Close => self.conn.read(out),
            Framing::Chunked(_) => self.read_chunked(out),
        }
    }
}

/// The TLS settings, made once: ring's cryptography and Mozilla's root certificates.
fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    Arc::clone(CONFIG.get_or_init(|| {
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("BUG: ring supports rustls's default TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    }))
}

/// Connect to `url`'s server, trying each of its addresses in turn.
fn connect(url: &Url, timeouts: Timeouts) -> Result<Stream, Error> {
    let addresses = (url.host.as_str(), url.port).to_socket_addrs()?;
    let mut last = None;
    let mut socket = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeouts.connect) {
            Ok(s) => {
                socket = Some(s);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let socket = match (socket, last) {
        (Some(s), _) => s,
        (None, Some(e)) => return Err(e.into()),
        (None, None) => {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} has no address", url.host),
            )));
        }
    };
    socket.set_nodelay(true)?;
    // Generous until the request is out, which includes the TLS handshake.
    socket.set_read_timeout(Some(timeouts.idle))?;
    socket.set_write_timeout(Some(timeouts.idle))?;
    if !url.https {
        return Ok(Stream::Plain(socket));
    }
    let name = ServerName::try_from(url.host.clone())
        .map_err(|_| Error::InvalidUrl(format!("bad host name: {}", url.host)))?;
    let conn = ClientConnection::new(tls_config(), name).map_err(Error::Tls)?;
    Ok(Stream::Tls(Box::new(StreamOwned::new(conn, socket))))
}

/// Send the request for `url`.
fn send_request(stream: &mut Stream, url: &Url) -> Result<(), Error> {
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: NeoAmp/{}\r\nAccept: */*\r\n\
         Icy-MetaData: 1\r\nConnection: close\r\n\r\n",
        url.target,
        url.authority(),
        env!("CARGO_PKG_VERSION"),
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    Ok(())
}

/// Where the head ends in `bytes`: the length of the head and its blank line.
fn head_end(bytes: &[u8]) -> Option<usize> {
    let crlf = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4);
    let lf = bytes.windows(2).position(|w| w == b"\n\n").map(|i| i + 2);
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// A response's status line and headers.
#[derive(Debug, PartialEq, Eq)]
struct Head {
    code: u16,
    reason: String,
    icy: bool,
    headers: Vec<(String, String)>,
}

fn parse_head(bytes: &[u8]) -> Result<Head, Error> {
    let text = decode_text(bytes);
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    let status = lines.next().unwrap_or("");
    let mut parts = status.splitn(3, ' ');
    let protocol = parts.next().unwrap_or("");
    let icy = protocol.eq_ignore_ascii_case("ICY");
    if !icy && !protocol.to_ascii_uppercase().starts_with("HTTP/") {
        let shown: String = status.chars().take(40).collect();
        return Err(Error::BadResponse(format!("it starts {shown:?}")));
    }
    let code = parts
        .next()
        .and_then(|c| c.trim().parse().ok())
        .ok_or_else(|| Error::BadResponse(format!("no status code in {status:?}")))?;
    let reason = parts.next().unwrap_or("").trim().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    Ok(Head {
        code,
        reason,
        icy,
        headers,
    })
}

/// Read the response's head from `conn`, waiting up to `idle` and checking `cancel` on each
/// tick.
fn read_head(conn: &mut Conn, idle: Duration, cancel: &dyn Fn() -> bool) -> Result<Head, Error> {
    let mut last = Instant::now();
    loop {
        if let Some(end) = head_end(conn.buffered()) {
            let head = parse_head(&conn.buffered()[..end]);
            conn.consume(end);
            return head;
        }
        if conn.buffered().len() >= MAX_HEAD {
            return Err(Error::BadResponse("its headers are too long".into()));
        }
        match conn.fill() {
            Ok(0) => {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the server closed the connection without answering",
                )));
            }
            Ok(_) => last = Instant::now(),
            Err(e) if is_tick(&e) => {
                if cancel() {
                    return Err(Error::Cancelled);
                }
                if last.elapsed() >= idle {
                    return Err(Error::TimedOut);
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// `GET` `url`, following redirects, and return the response once its headers are in.
/// `cancel` is asked on every tick while waiting for the server.
///
/// # Errors
/// Fails if the server cannot be reached, answers with an error status or not in HTTP, sends
/// too many redirects, stays silent for the idle timeout, or `cancel` asks to stop.
pub fn get(url: &Url, timeouts: Timeouts, cancel: &dyn Fn() -> bool) -> Result<Response, Error> {
    let mut url = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        if cancel() {
            return Err(Error::Cancelled);
        }
        let mut stream = connect(&url, timeouts)?;
        send_request(&mut stream, &url)?;
        stream.socket().set_read_timeout(Some(TICK))?;
        let mut conn = Conn::new(stream);
        let head = read_head(&mut conn, timeouts.idle, cancel)?;
        let header = |name: &str| {
            head.headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        if matches!(head.code, 301 | 302 | 303 | 307 | 308)
            && let Some(location) = header("location")
        {
            log::debug!("{url} redirects to {location}");
            url = url.join(location)?;
            continue;
        }
        if !(200..300).contains(&head.code) {
            return Err(Error::Status {
                code: head.code,
                reason: head.reason,
            });
        }
        let chunked =
            header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
        let length = header("content-length").and_then(|v| v.trim().parse::<u64>().ok());
        let framing = match (chunked, length) {
            (true, _) => Framing::Chunked(Chunked::Size(Vec::new())),
            (false, Some(n)) => Framing::Length(n),
            (false, None) => Framing::Close,
        };
        return Ok(Response {
            url,
            icy: head.icy,
            headers: head.headers,
            framing,
            conn,
            idle: timeouts.idle,
        });
    }
    Err(Error::TooManyRedirects)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// A server on a free local port that answers each connection with the next of `replies`,
    /// written in pieces with a pause between them, then closes it. Returns its address
    /// (`http://127.0.0.1:port`) and a handle that gives the requests it read.
    pub(crate) fn serve(
        replies: Vec<Vec<Vec<u8>>>,
        pause: Duration,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let (listener, address) = listen();
        (address, serve_on(listener, replies, pause))
    }

    /// A listener on a free local port, and its address (`http://127.0.0.1:port`).
    pub(crate) fn listen() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        (listener, address)
    }

    /// [`serve`] on a listener made beforehand, so that the replies can name its address.
    pub(crate) fn serve_on(
        listener: TcpListener,
        replies: Vec<Vec<Vec<u8>>>,
        pause: Duration,
    ) -> thread::JoinHandle<Vec<String>> {
        thread::spawn(move || {
            let mut requests = Vec::new();
            for pieces in replies {
                let Ok((mut socket, _)) = listener.accept() else {
                    break;
                };
                let mut request = Vec::new();
                let mut byte = [0];
                // A TLS handshake (record type 0x16) has no blank line to wait for.
                while !request.ends_with(b"\r\n\r\n")
                    && request.first() != Some(&0x16)
                    && socket.read(&mut byte).unwrap_or(0) == 1
                {
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                for piece in pieces {
                    if socket.write_all(&piece).is_err() {
                        break;
                    }
                    thread::sleep(pause);
                }
            }
            requests
        })
    }

    fn never() -> bool {
        false
    }

    fn fetch(address: &str, path: &str) -> Result<Response, Error> {
        let url = Url::parse(&format!("{address}{path}")).unwrap();
        get(&url, Timeouts::default(), &never)
    }

    fn body(r: &mut Response) -> Vec<u8> {
        r.read_to_end_limited(1 << 20, &never).unwrap()
    }

    #[test]
    fn urls_are_taken_apart_leniently() {
        let u = Url::parse("  HTTP://Radio.Example:8000/live stream.mp3?id=ü#top ").unwrap();
        assert_eq!(
            u,
            Url {
                https: false,
                host: "radio.example".into(),
                port: 8000,
                target: "/live%20stream.mp3?id=%C3%BC".into(),
            }
        );
        assert_eq!(u.path(), "/live%20stream.mp3");
        assert_eq!(
            u.to_string(),
            "http://radio.example:8000/live%20stream.mp3?id=%C3%BC"
        );
        let u = Url::parse("https://user:pw@radio.example").unwrap();
        assert_eq!((u.https, u.port, u.target.as_str()), (true, 443, "/"));
        assert_eq!(u.to_string(), "https://radio.example/");
        let u = Url::parse("http://[::1]:8080?x").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 8080));
        assert_eq!(u.to_string(), "http://[::1]:8080/?x");
        for bad in [
            "radio.example/live",
            "ftp://radio.example/x",
            "http://",
            "http://:80/",
            "http://radio.example:99999/",
            "http://bücher.example/",
        ] {
            assert!(
                matches!(Url::parse(bad), Err(Error::InvalidUrl(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn redirects_lead_to_whole_urls_and_paths() {
        let base = Url::parse("https://radio.example/lists/a.pls").unwrap();
        let join = |l| base.join(l).unwrap().to_string();
        assert_eq!(join("http://other.example/x"), "http://other.example/x");
        assert_eq!(join("//cdn.example/y"), "https://cdn.example/y");
        assert_eq!(join("/live"), "https://radio.example/live");
        assert_eq!(join("b.m3u"), "https://radio.example/lists/b.m3u");
    }

    #[test]
    fn shoutcast_icy_replies_are_read_until_the_connection_closes() {
        let (address, server) = serve(
            vec![vec![
                b"ICY 200 OK\r\nicy-name:Radio Caf\xe9\r\nicy-metaint:8192\r\ncontent-type:audio/mpeg\r\n\r\nab"
                    .to_vec(),
                b"cd".to_vec(),
            ]],
            Duration::from_millis(20),
        );
        let mut r = fetch(&address, "/;stream.mp3").unwrap();
        assert!(r.icy);
        assert_eq!(r.header("ICY-NAME"), Some("Radio Café"));
        assert_eq!(r.header("icy-metaint"), Some("8192"));
        assert_eq!(r.content_type().as_deref(), Some("audio/mpeg"));
        assert!(!r.has_known_end());
        assert_eq!(body(&mut r), b"abcd");
        assert!(!r.is_complete());
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /;stream.mp3 HTTP/1.1\r\n"));
        assert!(requests[0].contains("\r\nIcy-MetaData: 1\r\n"));
        assert!(requests[0].contains(&format!(
            "\r\nHost: {}\r\n",
            address.trim_start_matches("http://")
        )));
    }

    #[test]
    fn lengths_say_whether_the_body_came_whole() {
        let (address, _server) = serve(
            vec![
                vec![b"HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nhello, and more".to_vec()],
                vec![b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhal".to_vec()],
            ],
            Duration::ZERO,
        );
        let mut r = fetch(&address, "/a").unwrap();
        assert_eq!(body(&mut r), b"hello");
        assert!(r.has_known_end() && r.is_complete());
        let mut r = fetch(&address, "/b").unwrap();
        assert_eq!(body(&mut r), b"hal");
        assert!(!r.is_complete(), "the server closed early");
    }

    #[test]
    fn chunked_bodies_are_put_back_together() {
        let (address, _server) = serve(
            vec![vec![
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;ext=1\r\nWiki".to_vec(),
                b"\r\n5\r\npedia\r".to_vec(),
                b"\n0\r\nTrailer: x\r\n\r\n".to_vec(),
            ]],
            Duration::from_millis(300),
        );
        let mut r = fetch(&address, "/").unwrap();
        assert_eq!(body(&mut r), b"Wikipedia");
        assert!(r.is_complete());
    }

    #[test]
    fn redirects_are_followed_and_errors_reported() {
        let (address, server) = serve(
            vec![
                vec![
                    b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\n\r\n".to_vec(),
                ],
                vec![b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()],
                vec![b"HTTP/1.1 404 Not Found\r\n\r\n".to_vec()],
                vec![b"ICY 503 Service Unavailable\r\n\r\n".to_vec()],
                vec![b"<html><body>nope</body></html>\n\n".to_vec()],
            ],
            Duration::ZERO,
        );
        let mut r = fetch(&address, "/first").unwrap();
        assert_eq!(body(&mut r), b"ok");
        assert_eq!(r.url.path(), "/next");
        let e = fetch(&address, "/missing").unwrap_err();
        assert!(matches!(e, Error::Status { code: 404, .. }), "{e}");
        assert!(!e.is_retryable());
        assert_eq!(e.to_string(), "the server answered 404 Not Found");
        let e = fetch(&address, "/full").unwrap_err();
        assert!(e.is_retryable(), "{e}");
        let e = fetch(&address, "/page").unwrap_err();
        assert!(matches!(e, Error::BadResponse(_)), "{e}");
        let requests = server.join().unwrap();
        assert!(requests[1].starts_with("GET /next "));
    }

    #[test]
    fn a_body_longer_than_allowed_is_refused() {
        let (address, server) = serve(
            vec![vec![
                b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789".to_vec(),
            ]],
            Duration::ZERO,
        );
        let e = fetch(&address, "/big")
            .unwrap()
            .read_to_end_limited(9, &never)
            .unwrap_err();
        assert!(matches!(e, Error::TooLarge(9)), "{e}");
        assert!(!e.is_retryable());
        assert_eq!(e.to_string(), "the answer is longer than 9 bytes");
        server.join().unwrap();
    }

    #[test]
    fn a_silent_server_times_out_or_is_cancelled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/", listener.local_addr().unwrap());
        let url = Url::parse(&address).unwrap();
        let quick = Timeouts {
            connect: Duration::from_secs(5),
            idle: Duration::from_millis(600),
        };
        let started = Instant::now();
        assert!(matches!(get(&url, quick, &never), Err(Error::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(3));
        let started = Instant::now();
        let stop = || started.elapsed() > Duration::from_millis(300);
        assert!(matches!(
            get(&url, Timeouts::default(), &stop),
            Err(Error::Cancelled)
        ));
        drop(listener);
    }

    #[test]
    fn https_to_a_plain_server_fails_cleanly() {
        let (address, _server) = serve(
            vec![vec![b"HTTP/1.1 200 OK\r\n\r\n".to_vec()]],
            Duration::ZERO,
        );
        let url = Url::parse(&address.replace("http://", "https://")).unwrap();
        let e = get(&url, Timeouts::default(), &never).unwrap_err();
        assert!(!matches!(e, Error::Cancelled), "{e}");
    }

    #[test]
    fn heads_end_at_the_first_blank_line_of_either_kind() {
        assert_eq!(head_end(b"ICY 200 OK\r\na:b\r\n\r\nbody\n\n"), Some(19));
        assert_eq!(head_end(b"HTTP/1.0 200 OK\na:b\n\nbody"), Some(21));
        assert_eq!(head_end(b"HTTP/1.0 200 OK\r\n"), None);
        let head = parse_head(b"HTTP/1.1 200 OK\r\nX-Thing:  spaced  \r\nbroken line\r\n\r\n");
        assert_eq!(
            head.unwrap(),
            Head {
                code: 200,
                reason: "OK".into(),
                icy: false,
                headers: vec![("x-thing".into(), "spaced".into())],
            }
        );
    }
}
