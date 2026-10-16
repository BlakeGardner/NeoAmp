//! One NeoAmp at a time: files opened while NeoAmp runs (from the file manager, or with
//! `neoamp song.mp3` in a terminal) go to the running copy instead of starting a second one.
//!
//! The first copy listens on a local socket: a Unix domain socket in the user's runtime folder
//! on Linux (`$XDG_RUNTIME_DIR`), in the user's own temporary folder on macOS, and a named
//! pipe on Windows. Its name comes from the config folder, so copies kept apart with
//! `NEOAMP_CONFIG_DIR` stay apart. A later copy sends the running one its command line, waits
//! for it to say it has it, and exits.
//!
//! A request is `MAGIC`, a flags byte, the number of items (`u32`, little-endian), and each
//! item as a kind byte, its length in bytes (`u32`) and its bytes. Paths keep every byte of
//! the platform's file names, which need not be Unicode: raw bytes on Unix, UTF-16 code units
//! (little-endian) on Windows. The answer is one byte, `ACK`.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ListenerOptions, Name};

use crate::audio::Location;

/// What every request starts with, the protocol's version included.
const MAGIC: &[u8; 7] = b"NEOAMP\x01";
/// The running copy's answer once it has the request.
const ACK: u8 = 0x06;
/// Flag: add the files to the playlist rather than replace it.
const ENQUEUE: u8 = 1;
/// More items than this, or a longer item, is not a request from NeoAmp.
const MAX_ITEMS: u32 = 1 << 20;
const MAX_ITEM: u32 = 1 << 16;
/// How long a later copy waits for the running one before starting on its own.
const TIMEOUT: Duration = Duration::from_secs(5);

/// What a later copy asks of the running one: its command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// Audio files, folders, playlist files and streams, with paths already made absolute.
    pub files: Vec<Location>,
    /// A skin to show.
    pub skin: Option<PathBuf>,
    /// Add the files to the playlist instead of replacing it.
    pub enqueue: bool,
}

/// How this copy of NeoAmp stands.
#[derive(Debug)]
pub enum Claim {
    /// The first copy: it listens for later ones.
    First(Server),
    /// Another copy runs and has taken the request: this one has nothing more to do.
    Forwarded,
    /// The running copy could not be reached, nor a listener made: run on alone.
    Alone,
}

/// The running copy's end: requests from later copies.
#[derive(Debug)]
pub struct Server {
    requests: Receiver<Request>,
    /// The socket file, removed when the server goes (Unix only; named pipes vanish by
    /// themselves).
    #[cfg(unix)]
    path: PathBuf,
}

impl Server {
    /// The next request from a later copy, if one has come.
    pub fn poll(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }
}

#[cfg(unix)]
impl Drop for Server {
    fn drop(&mut self) {
        // The listener's thread never ends, so it would not remove the file itself.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Hand `request` to the NeoAmp already running for `config_dir`, or become the one that runs.
#[must_use]
pub fn claim(config_dir: &Path, request: &Request) -> Claim {
    let address = Address::for_config(config_dir);
    let message = encode(request);
    // A copy starting at the same moment may take the name between the two steps: try again.
    for _ in 0..3 {
        match forward(&address, message.clone()) {
            Ok(()) => return Claim::Forwarded,
            Err(e) if is_absent(&e) => {}
            Err(e) => {
                log::warn!("NeoAmp is running but does not answer ({e}); starting another");
                return Claim::Alone;
            }
        }
        match listen(&address) {
            Ok(server) => return Claim::First(server),
            // Another copy took the name in the meantime. On Windows a second listener on a
            // pipe is refused as access denied.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
                ) => {}
            Err(e) => {
                log::warn!("files opened later will start another NeoAmp: {e}");
                return Claim::Alone;
            }
        }
    }
    Claim::Alone
}

/// No copy listens there: the name does not exist, or is a socket file left by one that
/// crashed.
fn is_absent(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

/// Where the running copy listens.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Address {
    /// The socket file on Unix, the pipe's name on Windows.
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(not(unix))]
    pipe: String,
}

impl Address {
    fn for_config(config_dir: &Path) -> Address {
        let file = format!("neoamp-{:016x}.sock", fnv1a(&config_key(config_dir)));
        #[cfg(target_os = "macos")]
        let path = std::env::temp_dir().join(file);
        #[cfg(all(unix, not(target_os = "macos")))]
        let path = dirs::runtime_dir()
            .unwrap_or_else(|| config_dir.to_path_buf())
            .join(file);
        #[cfg(unix)]
        return Address { path };
        #[cfg(not(unix))]
        Address { pipe: file }
    }

    fn name(&self) -> io::Result<Name<'_>> {
        #[cfg(unix)]
        {
            use interprocess::local_socket::{GenericFilePath, ToFsName};
            self.path.as_path().to_fs_name::<GenericFilePath>()
        }
        #[cfg(not(unix))]
        {
            use interprocess::local_socket::{GenericNamespaced, ToNsName};
            self.pipe.as_str().to_ns_name::<GenericNamespaced>()
        }
    }
}

/// What tells two config folders apart. Windows and macOS file systems ignore case.
fn config_key(dir: &Path) -> Vec<u8> {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        dir.to_string_lossy().to_lowercase().into_bytes()
    } else {
        dir.into_os_string().into_encoded_bytes()
    }
}

/// The 64-bit FNV-1a hash: short, and the same in every build, unlike the standard
/// library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Send `message` to the copy at `address` and wait for its answer, for [`TIMEOUT`] at most.
fn forward(address: &Address, message: Vec<u8>) -> io::Result<()> {
    let address = address.clone();
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("hand over to NeoAmp".into())
        .spawn(move || {
            let _ = tx.send(exchange(&address, &message));
        })?;
    rx.recv_timeout(TIMEOUT)
        .unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::TimedOut)))
}

fn exchange(address: &Address, message: &[u8]) -> io::Result<()> {
    let mut stream = LocalSocketStream::connect(address.name()?)?;
    stream.write_all(message)?;
    stream.flush()?;
    let mut answer = [0u8];
    stream.read_exact(&mut answer)?;
    if answer[0] == ACK {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an unexpected answer",
        ))
    }
}

/// Listen at `address` for later copies, on a thread of its own.
fn listen(address: &Address) -> io::Result<Server> {
    let create = || ListenerOptions::new().name(address.name()?).create_sync();
    #[cfg(unix)]
    let listener = match create() {
        // A socket file nothing answers on is left over from a copy that crashed.
        Err(e) if e.kind() == io::ErrorKind::AddrInUse && stale(address) => {
            let _ = std::fs::remove_file(&address.path);
            create()
        }
        other => other,
    }?;
    #[cfg(not(unix))]
    let listener = create()?;
    let (tx, requests) = mpsc::channel();
    thread::Builder::new()
        .name("single instance".into())
        .spawn(move || serve(&listener, &tx))?;
    Ok(Server {
        requests,
        #[cfg(unix)]
        path: address.path.clone(),
    })
}

/// Whether the socket file at `address` is one nothing listens on.
#[cfg(unix)]
fn stale(address: &Address) -> bool {
    address
        .name()
        .and_then(LocalSocketStream::connect)
        .is_err_and(|e| e.kind() == io::ErrorKind::ConnectionRefused)
}

/// Take requests, each connection on a thread of its own so that one that never finishes
/// holds up no other.
fn serve(listener: &LocalSocketListener, requests: &Sender<Request>) {
    for connection in listener.incoming() {
        let mut connection = match connection {
            Ok(c) => c,
            Err(e) => {
                log::warn!("a later NeoAmp could not connect: {e}");
                continue;
            }
        };
        let requests = requests.clone();
        let spawned = thread::Builder::new()
            .name("single instance request".into())
            .spawn(move || match decode(&mut connection) {
                Ok(request) => {
                    // Gone only as NeoAmp closes.
                    if requests.send(request).is_ok() {
                        let _ = connection.write_all(&[ACK]);
                    }
                }
                Err(e) => log::warn!("a request from a later NeoAmp was unreadable: {e}"),
            });
        if let Err(e) = spawned {
            log::warn!("cannot take a request from a later NeoAmp: {e}");
        }
    }
}

/// Kinds of item in a request.
const FILE: u8 = 0;
const URL: u8 = 1;
const SKIN: u8 = 2;

fn encode(request: &Request) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.push(if request.enqueue { ENQUEUE } else { 0 });
    let items: Vec<(u8, Vec<u8>)> = request
        .files
        .iter()
        .map(|location| match location {
            Location::File(path) => (FILE, os_bytes(path.as_os_str())),
            Location::Url(url) => (URL, url.as_bytes().to_vec()),
        })
        .chain(
            request
                .skin
                .iter()
                .map(|skin| (SKIN, os_bytes(skin.as_os_str()))),
        )
        .collect();
    put_u32(&mut out, items.len());
    for (kind, bytes) in items {
        out.push(kind);
        put_u32(&mut out, bytes.len());
        out.extend(bytes);
    }
    out
}

fn put_u32(out: &mut Vec<u8>, n: usize) {
    out.extend(u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
}

fn decode(input: &mut impl Read) -> io::Result<Request> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let mut magic = [0u8; MAGIC.len()];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(bad("not a NeoAmp request"));
    }
    let mut request = Request {
        enqueue: read_u8(input)? & ENQUEUE != 0,
        ..Request::default()
    };
    let count = read_u32(input)?;
    if count > MAX_ITEMS {
        return Err(bad("too many items"));
    }
    for _ in 0..count {
        let kind = read_u8(input)?;
        let len = read_u32(input)?;
        if len > MAX_ITEM {
            return Err(bad("an item too long"));
        }
        let mut bytes = vec![0; usize::try_from(len).map_err(|_| bad("an item too long"))?];
        input.read_exact(&mut bytes)?;
        match kind {
            FILE => request.files.push(Location::File(
                os_string(bytes).ok_or_else(|| bad("a bad path"))?.into(),
            )),
            URL => request.files.push(Location::Url(
                String::from_utf8(bytes).map_err(|_| bad("a bad URL"))?,
            )),
            SKIN => request.skin = Some(os_string(bytes).ok_or_else(|| bad("a bad path"))?.into()),
            _ => return Err(bad("an unknown item")),
        }
    }
    Ok(request)
}

fn read_u8(input: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8];
    input.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    input.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// A file name's bytes, as the platform holds them.
#[cfg(unix)]
fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(unix)]
fn os_string(bytes: Vec<u8>) -> Option<OsString> {
    use std::os::unix::ffi::OsStringExt;
    Some(OsString::from_vec(bytes))
}

/// A file name's UTF-16 code units, little-endian.
#[cfg(windows)]
fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().flat_map(u16::to_le_bytes).collect()
}

#[cfg(windows)]
fn os_string(bytes: Vec<u8>) -> Option<OsString> {
    use std::os::windows::ffi::OsStringExt;
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Some(OsString::from_wide(&wide))
}

/// Elsewhere file names are Unicode.
#[cfg(not(any(unix, windows)))]
fn os_bytes(s: &OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

#[cfg(not(any(unix, windows)))]
fn os_string(bytes: Vec<u8>) -> Option<OsString> {
    String::from_utf8(bytes).ok().map(OsString::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            files: vec![
                Location::File(PathBuf::from("/music/Band - Song.mp3")),
                Location::Url("https://radio.example/stream".into()),
            ],
            skin: Some(PathBuf::from("/skins/Classic.wsz")),
            enqueue: true,
        }
    }

    #[test]
    fn requests_survive_the_trip() {
        let bytes = encode(&request());
        assert_eq!(decode(&mut bytes.as_slice()).unwrap(), request());
        let empty = Request::default();
        assert_eq!(decode(&mut encode(&empty).as_slice()).unwrap(), empty);
    }

    #[cfg(unix)]
    #[test]
    fn file_names_that_are_not_unicode_arrive_intact() {
        use std::os::unix::ffi::OsStrExt;
        let name = OsStr::from_bytes(b"/music/caf\xe9.mp3");
        let sent = Request {
            files: vec![Location::File(PathBuf::from(name))],
            ..Request::default()
        };
        assert_eq!(decode(&mut encode(&sent).as_slice()).unwrap(), sent);
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(decode(&mut &b"GET / HTTP/1.1\r\n"[..]).is_err());
        let mut long = MAGIC.to_vec();
        long.push(0);
        long.extend(u32::MAX.to_le_bytes());
        assert!(decode(&mut long.as_slice()).is_err());
        let mut cut = encode(&request());
        cut.truncate(cut.len() - 3);
        assert!(decode(&mut cut.as_slice()).is_err());
    }

    #[test]
    fn each_config_folder_has_its_own_address() {
        let a = Address::for_config(Path::new("/home/a/.config/neoamp"));
        let b = Address::for_config(Path::new("/home/b/.config/neoamp"));
        assert_ne!(a, b);
        assert_eq!(a, Address::for_config(Path::new("/home/a/.config/neoamp")));
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn a_second_copy_hands_its_request_to_the_first() {
        let dir = std::env::temp_dir().join(format!("neoamp-instance-{}", std::process::id()));
        let other = dir.join("other");
        // Without $XDG_RUNTIME_DIR the socket goes in the config folder, so it must exist.
        std::fs::create_dir_all(&other).unwrap();
        let Claim::First(server) = claim(&dir, &Request::default()) else {
            panic!("nothing else runs with this folder");
        };
        assert!(matches!(claim(&dir, &request()), Claim::Forwarded));
        assert_eq!(server.poll(), Some(request()));
        assert_eq!(server.poll(), None);
        let Claim::First(other_server) = claim(&other, &Request::default()) else {
            panic!("another folder is another NeoAmp");
        };
        drop((server, other_server));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_left_by_a_crash_is_taken_over() {
        let dir = std::env::temp_dir().join(format!("neoamp-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let address = Address::for_config(&dir);
        // A socket file with nobody listening, as a crash leaves it.
        drop(std::os::unix::net::UnixListener::bind(&address.path).unwrap());
        assert!(address.path.exists());
        let Claim::First(server) = claim(&dir, &Request::default()) else {
            panic!("the stale socket should have been replaced");
        };
        assert!(matches!(claim(&dir, &request()), Claim::Forwarded));
        assert_eq!(server.poll(), Some(request()));
        drop(server);
        assert!(!address.path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
