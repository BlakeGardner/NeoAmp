//! Internet radio. A stream's bytes come in on a thread of their own, the *network thread*,
//! and wait in a [`Pipe`] until the decoder thread reads them, through symphonia, as it would
//! a file that cannot seek.
//!
//! - The network thread connects ([`crate::http`]), follows a playlist (`.pls`, `.m3u`) to
//!   the stream it names, takes the SHOUTcast/Icecast song titles out of the audio ([`Icy`])
//!   and fills the pipe. When the connection drops or stalls it connects again, a few times
//!   with growing pauses, before giving up.
//! - Each connection's bytes are a *segment* of the pipe. The decoder reads one segment as
//!   one stream; when it ends, the next connection's bytes are recognised afresh, since a new
//!   connection does not pick up where the old one stopped.
//! - The decoder waits for a couple of seconds of audio before it starts, and again if the
//!   pipe runs dry, and tells the UI how full the buffer is ([`StreamInput::next`]).

use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use symphonia::core::io::MediaSource;

use super::{Shared, Station, StreamStatus};
use crate::http::{self, Response, Timeouts, Url};
use crate::playlist_file::{self, decode_text};
use crate::{m3u, pls};

/// How much the pipe holds before the network thread waits for the decoder: about half a
/// minute at 128 kbit/s. While playback is paused the pipe fills up, and then the server,
/// left waiting, may drop the connection.
const CAPACITY: usize = 512 * 1024;
/// How much audio to buffer before playing, and before playing again after running dry.
const PREBUFFER_SECS: usize = 2;
/// Bytes per second of a stream that does not say its bitrate: 128 kbit/s.
const DEFAULT_RATE: usize = 16_000;
/// The least to buffer, whatever the bitrate.
const MIN_TARGET: usize = 8 * 1024;
/// The decoder only reads from the pipe while it holds at least this much (or all the segment
/// will ever have): more than any MP3 or AAC frame, so a packet is seldom left waiting for the
/// network halfway through.
const MIN_DECODE: usize = 4 * 1024;
/// How long a reader waiting for bytes sleeps before checking whether to give up.
const WAIT: Duration = Duration::from_millis(20);
/// The most a playlist leading to a stream may hold.
const PLAYLIST_LIMIT: usize = 256 * 1024;
/// How many playlists may lead to one another before the stream.
const MAX_DEPTH: usize = 3;
/// A connection that lasted this long was a good one: the reconnection count starts afresh.
const HEALTHY: Duration = Duration::from_secs(30);

/// How hard the network thread tries before giving up on a stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Retry {
    /// Tries after the first.
    pub attempts: u32,
    /// The pause before the first of them; each pause is twice the one before.
    pub delay: Duration,
}

/// The network thread's settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Settings {
    pub retry: Retry,
    pub timeouts: Timeouts,
}

impl Default for Settings {
    /// Three more tries, after 1, 2 and 4 seconds.
    fn default() -> Self {
        Settings {
            retry: Retry {
                attempts: 3,
                delay: Duration::from_secs(1),
            },
            timeouts: Timeouts::default(),
        }
    }
}

/// Takes SHOUTcast/Icecast metadata out of a stream's bytes. After every `interval` bytes of
/// audio comes a byte giving the length of a metadata block in units of 16 bytes (usually
/// 0: nothing new), then the block, which reads like `StreamTitle='Artist - Song';`.
#[derive(Debug)]
pub(super) struct Icy {
    interval: usize,
    phase: Phase,
    block: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// This many bytes of audio to go before the next block.
    Audio(usize),
    /// The next byte is a block's length.
    Length,
    /// This many bytes of the block to go.
    Meta(usize),
}

impl Icy {
    pub(super) fn new(interval: usize) -> Icy {
        Icy {
            interval,
            phase: Phase::Audio(interval),
            block: Vec::new(),
        }
    }

    /// Split `input`: its audio goes on the end of `audio`, and each song title in it on the
    /// end of `titles`, with the length `audio` had where the title came, which is where in
    /// the audio it starts.
    pub(super) fn feed(
        &mut self,
        mut input: &[u8],
        audio: &mut Vec<u8>,
        titles: &mut Vec<(usize, String)>,
    ) {
        while let Some(&first) = input.first() {
            match self.phase {
                Phase::Audio(left) => {
                    let n = left.min(input.len());
                    audio.extend_from_slice(&input[..n]);
                    input = &input[n..];
                    self.phase = if n == left {
                        Phase::Length
                    } else {
                        Phase::Audio(left - n)
                    };
                }
                Phase::Length => {
                    input = &input[1..];
                    let len = usize::from(first) * 16;
                    self.block.clear();
                    self.phase = if len == 0 {
                        Phase::Audio(self.interval)
                    } else {
                        Phase::Meta(len)
                    };
                }
                Phase::Meta(left) => {
                    let n = left.min(input.len());
                    self.block.extend_from_slice(&input[..n]);
                    input = &input[n..];
                    if n == left {
                        if let Some(title) = stream_title(&self.block) {
                            titles.push((audio.len(), title));
                        }
                        self.phase = Phase::Audio(self.interval);
                    } else {
                        self.phase = Phase::Meta(left - n);
                    }
                }
            }
        }
    }
}

/// The song title in a metadata block (`StreamTitle='...';`), which may be empty; `None` if
/// the block has none. Titles may hold apostrophes, so the one that ends a title is the one
/// before `;`. Text that is not UTF-8 is read as Windows-1252, as stations often send it.
fn stream_title(block: &[u8]) -> Option<String> {
    const KEY: &str = "streamtitle='";
    let used = block.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let text = decode_text(&block[..used]);
    // ASCII lowercasing keeps every byte where it was.
    let start = text.to_ascii_lowercase().find(KEY)? + KEY.len();
    let rest = &text[start..];
    let end = rest
        .find("';")
        .or_else(|| rest.rfind('\''))
        .unwrap_or(rest.len());
    Some(rest[..end].trim().to_owned())
}

/// The state of the connection, as the network thread last set it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Net {
    /// Connecting, and not yet through once.
    Connecting,
    /// Connected, filling the last segment.
    Streaming,
    /// The connection dropped; connecting again.
    Reconnecting,
    /// The stream ended where it should: a file on the web, read to its end.
    Complete,
    /// Gave up, for this reason.
    Failed(String),
}

/// What a connection said about what it sends, for recognising its audio.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SegmentInfo {
    pub station: Station,
    /// The media type the server gave (`audio/mpeg`).
    pub mime: Option<String>,
    /// The extension the URL's path ends in (`mp3`).
    pub extension: Option<String>,
}

/// One connection's bytes.
#[derive(Debug)]
struct Segment {
    info: SegmentInfo,
    data: VecDeque<u8>,
    /// The connection has ended: no more bytes will come.
    ended: bool,
    read: u64,
    written: u64,
    /// Song titles, each with the byte it starts at.
    titles: VecDeque<(u64, String)>,
}

#[derive(Debug)]
struct PipeState {
    segments: VecDeque<Segment>,
    /// Bytes in all the segments.
    total: usize,
    net: Net,
    /// The decoder has let go of the stream: the network thread should stop.
    closed: bool,
}

/// How full the pipe is, for the decoder.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Level {
    /// There is a segment to read.
    pub segment: bool,
    /// Bytes waiting in the segment being read.
    pub front: usize,
    /// The segment being read has ended: what it holds is all it will have.
    pub ended: bool,
    pub net: Net,
}

/// Bytes on their way from the network thread to the decoder.
#[derive(Debug)]
pub(super) struct Pipe {
    state: Mutex<PipeState>,
    changed: Condvar,
    capacity: usize,
}

impl Pipe {
    pub(super) fn new(capacity: usize) -> Arc<Pipe> {
        Arc::new(Pipe {
            state: Mutex::new(PipeState {
                segments: VecDeque::new(),
                total: 0,
                net: Net::Connecting,
                closed: false,
            }),
            changed: Condvar::new(),
            capacity,
        })
    }

    fn lock(&self) -> MutexGuard<'_, PipeState> {
        // Nothing done under the lock can leave the state half changed.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait_on<'a>(
        &self,
        state: MutexGuard<'a, PipeState>,
        timeout: Duration,
    ) -> MutexGuard<'a, PipeState> {
        self.changed
            .wait_timeout(state, timeout)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    // ----- the network thread's side

    pub(super) fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// A connection's bytes start coming. False once the decoder has let go.
    pub(super) fn begin(&self, info: SegmentInfo) -> bool {
        let mut state = self.lock();
        if state.closed {
            return false;
        }
        state.segments.push_back(Segment {
            info,
            data: VecDeque::new(),
            ended: false,
            read: 0,
            written: 0,
            titles: VecDeque::new(),
        });
        state.net = Net::Streaming;
        self.changed.notify_all();
        true
    }

    /// Add `bytes` to the last segment, waiting for room as the decoder reads. False once the
    /// decoder has let go.
    pub(super) fn write(&self, mut bytes: &[u8]) -> bool {
        let mut state = self.lock();
        while !bytes.is_empty() {
            if state.closed {
                return false;
            }
            let room = self.capacity.saturating_sub(state.total);
            if room == 0 {
                state = self.wait_on(state, WAIT);
                continue;
            }
            let n = room.min(bytes.len());
            let Some(segment) = state.segments.back_mut() else {
                debug_assert!(false, "BUG: writing before a segment began");
                return false;
            };
            segment.data.extend(&bytes[..n]);
            // usize always fits in u64 on supported targets.
            segment.written += n as u64;
            state.total += n;
            bytes = &bytes[n..];
            self.changed.notify_all();
        }
        true
    }

    /// The song changes to `title` where the last segment has got to.
    pub(super) fn title(&self, title: String) {
        let mut state = self.lock();
        if let Some(segment) = state.segments.back_mut() {
            let at = segment.written;
            segment.titles.push_back((at, title));
        }
    }

    /// The last segment's connection has ended, and the connection is now `net`.
    pub(super) fn end_segment(&self, net: Net) {
        let mut state = self.lock();
        if let Some(segment) = state.segments.back_mut() {
            segment.ended = true;
        }
        state.net = net;
        self.changed.notify_all();
    }

    pub(super) fn set_net(&self, net: Net) {
        self.lock().net = net;
    }

    /// Pause for `time`, or until the decoder lets go, in which case this returns false.
    pub(super) fn wait(&self, time: Duration) -> bool {
        let until = Instant::now() + time;
        let mut state = self.lock();
        loop {
            if state.closed {
                return false;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            state = self.wait_on(state, left.min(WAIT * 5));
        }
    }

    // ----- the decoder's side

    /// Let go of the stream: the network thread stops.
    pub(super) fn close(&self) {
        self.lock().closed = true;
        self.changed.notify_all();
    }

    pub(super) fn level(&self) -> Level {
        let state = self.lock();
        let front = state.segments.front();
        Level {
            segment: front.is_some(),
            front: front.map_or(0, |s| s.data.len()),
            ended: front.is_some_and(|s| s.ended),
            net: state.net.clone(),
        }
    }

    /// What the connection of the segment being read said about itself.
    pub(super) fn info(&self) -> Option<SegmentInfo> {
        self.lock().segments.front().map(|s| s.info.clone())
    }

    /// Move on from a segment read to its end to the next, if there is one.
    pub(super) fn advance(&self) -> bool {
        let mut state = self.lock();
        let done = state
            .segments
            .front()
            .is_some_and(|s| s.ended && s.data.is_empty());
        if done && state.segments.len() > 1 {
            state.segments.pop_front();
            true
        } else {
            false
        }
    }

    /// The song titles the reading has reached, oldest first.
    pub(super) fn due_titles(&self) -> Vec<String> {
        let mut state = self.lock();
        let Some(segment) = state.segments.front_mut() else {
            return Vec::new();
        };
        let mut due = Vec::new();
        while let Some((at, _)) = segment.titles.front() {
            if *at > segment.read {
                break;
            }
            if let Some((_, title)) = segment.titles.pop_front() {
                due.push(title);
            }
        }
        due
    }
}

/// The segment being read, as symphonia sees it: a file that cannot seek and ends where the
/// connection did. A read waits for the network, but gives up (with an error) as soon as the
/// UI sends the decoder another command, so that the decoder can get to it. The UI sends no
/// seeks to streams, so a command always means this stream is finished with.
pub(super) struct PipeReader {
    pipe: Arc<Pipe>,
    shared: Arc<Shared>,
    /// The generation of the `Open` that started the stream.
    generation: u64,
}

impl PipeReader {
    pub(super) fn new(pipe: Arc<Pipe>, shared: Arc<Shared>, generation: u64) -> PipeReader {
        PipeReader {
            pipe,
            shared,
            generation,
        }
    }
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = self.pipe.lock();
        loop {
            let state_ref = &mut *state;
            if let Some(segment) = state_ref.segments.front_mut() {
                if !segment.data.is_empty() {
                    let n = segment.data.read(buf)?;
                    // usize always fits in u64 on supported targets.
                    segment.read += n as u64;
                    state_ref.total -= n;
                    self.pipe.changed.notify_all();
                    return Ok(n);
                }
                if segment.ended {
                    return Ok(0);
                }
            } else if matches!(state_ref.net, Net::Complete | Net::Failed(_)) {
                return Ok(0);
            }
            // Not `Interrupted`, which symphonia would retry.
            if self.shared.requested.load(Ordering::Acquire) != self.generation {
                return Err(io::Error::other("the stream was stopped"));
            }
            state = self.pipe.wait_on(state, WAIT);
        }
    }
}

impl Seek for PipeReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        match to {
            SeekFrom::Current(0) => Ok(self.pipe.lock().segments.front().map_or(0, |s| s.read)),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a stream cannot seek",
            )),
        }
    }
}

impl MediaSource for PipeReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// What the decoder should do next with a stream.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Next {
    /// Decode a packet.
    Decode,
    /// Wait: too little is buffered, or the connection is being made.
    Wait,
    /// Recognise the audio of the segment now being read, which has enough buffered.
    Open(SegmentInfo),
    /// The stream ended where it should.
    End,
    /// The stream could not be kept going, for this reason.
    Fail(String),
}

/// The decoder's hold on a stream. Dropping it lets go of the pipe, and the network thread
/// stops.
#[derive(Debug)]
pub(super) struct StreamInput {
    pipe: Arc<Pipe>,
    /// The status last told to the UI, and one still to tell.
    told: StreamStatus,
    news: Option<StreamStatus>,
    /// Waiting for the buffer to fill before decoding again.
    rebuffering: bool,
    /// What the segment being played said about itself, for when a chained stream starts its
    /// next song in it.
    pub station: Station,
}

impl Drop for StreamInput {
    fn drop(&mut self) {
        self.pipe.close();
    }
}

impl StreamInput {
    /// Connect to `url` on a network thread of its own. The UI has been told it is connecting.
    ///
    /// # Errors
    /// Fails if the thread cannot be started.
    pub(super) fn start(url: String, settings: Settings) -> Result<StreamInput> {
        let pipe = Pipe::new(CAPACITY);
        let theirs = Arc::clone(&pipe);
        thread::Builder::new()
            .name("stream".into())
            .spawn(move || run(&url, &theirs, settings))
            .context("starting the stream's network thread")?;
        Ok(StreamInput::with_pipe(pipe))
    }

    fn with_pipe(pipe: Arc<Pipe>) -> StreamInput {
        StreamInput {
            pipe,
            told: StreamStatus::Connecting,
            news: None,
            rebuffering: false,
            station: Station::default(),
        }
    }

    pub(super) fn pipe(&self) -> &Arc<Pipe> {
        &self.pipe
    }

    fn report(&mut self, status: StreamStatus) {
        if status != self.told {
            self.told = status;
            self.news = Some(status);
        }
    }

    /// A status the UI has not been told yet.
    pub(super) fn take_status(&mut self) -> Option<StreamStatus> {
        self.news.take()
    }

    /// The segment being read is playing.
    pub(super) fn playing(&mut self, station: Station) {
        self.station = station;
        self.rebuffering = false;
        self.report(StreamStatus::Playing);
    }

    /// How much to buffer before playing: a couple of seconds at the stream's bitrate.
    fn target(&self) -> usize {
        let bitrate = self
            .pipe
            .info()
            .and_then(|i| i.station.kbps)
            .and_then(|k| usize::try_from(k).ok())
            .map_or(DEFAULT_RATE, |k| k.saturating_mul(125));
        bitrate
            .saturating_mul(PREBUFFER_SECS)
            .clamp(MIN_TARGET, CAPACITY / 2)
    }

    fn buffering(&mut self, front: usize) {
        // A percentage, so the cast cannot truncate.
        let percent = (front.saturating_mul(100) / self.target().max(1)).min(99) as u8;
        self.report(StreamStatus::Buffering(percent));
    }

    /// What to do next, given whether the segment being read has a source decoding it, and
    /// whether the device's queue is about to run dry.
    pub(super) fn next(&mut self, decoding: bool, queue_low: bool) -> Next {
        let mut level = self.pipe.level();
        if !decoding {
            // Done with a segment: move on to the next one, if its connection has been made.
            while level.ended && level.front == 0 {
                if !self.pipe.advance() {
                    return match level.net {
                        Net::Complete => Next::End,
                        Net::Failed(why) => Next::Fail(why),
                        Net::Connecting | Net::Streaming | Net::Reconnecting => {
                            self.report(StreamStatus::Reconnecting);
                            Next::Wait
                        }
                    };
                }
                level = self.pipe.level();
            }
            if !level.segment {
                return match level.net {
                    Net::Failed(why) => Next::Fail(why),
                    Net::Complete => Next::End,
                    Net::Reconnecting => {
                        self.report(StreamStatus::Reconnecting);
                        Next::Wait
                    }
                    Net::Connecting | Net::Streaming => {
                        self.report(StreamStatus::Connecting);
                        Next::Wait
                    }
                };
            }
            if level.front >= self.target() || level.ended {
                return self.pipe.info().map_or(Next::Wait, Next::Open);
            }
            self.buffering(level.front);
            return Next::Wait;
        }
        if self.rebuffering {
            if level.front >= self.target() || level.ended {
                self.rebuffering = false;
                self.report(StreamStatus::Playing);
                return Next::Decode;
            }
            self.buffering(level.front);
            return Next::Wait;
        }
        if level.front < MIN_DECODE && !level.ended {
            if queue_low {
                // The device is about to play silence: wait for a good buffer this time.
                self.rebuffering = true;
                self.buffering(level.front);
            }
            return Next::Wait;
        }
        Next::Decode
    }

    /// The song titles the decoding has reached.
    pub(super) fn due_titles(&self) -> Vec<String> {
        self.pipe.due_titles()
    }
}

/// Why connecting failed, and whether trying again might help.
#[derive(Debug)]
struct Failure {
    message: String,
    retryable: bool,
}

impl Failure {
    fn fatal(message: impl Into<String>) -> Failure {
        Failure {
            message: message.into(),
            retryable: false,
        }
    }
}

impl From<http::Error> for Failure {
    fn from(e: http::Error) -> Self {
        Failure {
            retryable: e.is_retryable(),
            message: e.to_string(),
        }
    }
}

/// A connection with audio coming.
struct Opened {
    response: Response,
    info: SegmentInfo,
    /// How often the audio carries metadata (`icy-metaint`).
    metaint: Option<usize>,
    /// A radio station's server (it answered `ICY`, or with `icy-` headers): if the
    /// connection closes, it dropped rather than ended.
    radio: bool,
}

/// What a response holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Audio,
    /// A playlist leading to the stream.
    Playlist(playlist_file::Format),
    /// A web page.
    Page,
}

/// The extension `path` ends in, in lowercase (SHOUTcast's `/;stream.mp3` gives `mp3`).
fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 5 && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_lowercase())
}

fn kind(mime: Option<&str>, path: &str) -> Kind {
    use playlist_file::Format;
    match mime {
        Some("audio/x-scpls" | "audio/scpls" | "application/pls+xml") => {
            Kind::Playlist(Format::Pls)
        }
        Some(
            "audio/x-mpegurl"
            | "audio/mpegurl"
            | "application/x-mpegurl"
            | "application/mpegurl"
            | "application/vnd.apple.mpegurl",
        ) => Kind::Playlist(Format::M3u),
        Some("text/html" | "application/xhtml+xml") => Kind::Page,
        None | Some("text/plain" | "application/octet-stream") => {
            match extension(path).as_deref() {
                Some("pls") => Kind::Playlist(Format::Pls),
                Some("m3u" | "m3u8") => Kind::Playlist(Format::M3u),
                _ => Kind::Audio,
            }
        }
        Some(_) => Kind::Audio,
    }
}

/// The streams a playlist names, in order, with the titles it gives them.
fn playlist_streams(
    format: playlist_file::Format,
    bytes: &[u8],
) -> Result<Vec<(String, Option<String>)>, Failure> {
    let tracks = match format {
        playlist_file::Format::Pls => pls::parse(bytes, Path::new("")),
        playlist_file::Format::M3u => {
            if bytes.windows(7).any(|w| w.eq_ignore_ascii_case(b"#EXT-X-")) {
                return Err(Failure::fatal(
                    "HLS streams (playlists of segments) are not supported",
                ));
            }
            m3u::parse(bytes, Path::new(""))
        }
    };
    Ok(tracks
        .into_iter()
        .filter_map(|t| {
            let title = (!t.is_untitled_stream()).then(|| t.title.clone());
            Some((t.location.url()?.to_owned(), title))
        })
        .collect())
}

/// What a station's server says about it in its `icy-` headers.
fn station(response: &Response) -> Station {
    let text = |name| {
        response
            .header(name)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let kbps = response.header("icy-br").and_then(|v| {
        let digits = v.trim().split(|c: char| !c.is_ascii_digit()).next()?;
        digits.parse().ok().filter(|&k| k > 0)
    });
    Station {
        name: text("icy-name"),
        genre: text("icy-genre"),
        homepage: text("icy-url"),
        description: text("icy-description"),
        kbps,
        format: String::new(),
        codec: String::new(),
    }
}

/// Connect to `url`, following playlists (`depth` deep so far) to the stream they name: each
/// of a playlist's streams is tried in turn, as SHOUTcast's lists name several servers.
fn open(url: &str, pipe: &Pipe, timeouts: Timeouts, depth: usize) -> Result<Opened, Failure> {
    let parsed = Url::parse(url)?;
    let cancel = || pipe.is_closed();
    let mut response = http::get(&parsed, timeouts, &cancel)?;
    let mime = response.content_type();
    match kind(mime.as_deref(), response.url.path()) {
        Kind::Page => Err(Failure::fatal("the address is a web page, not a stream")),
        Kind::Playlist(format) => {
            if depth >= MAX_DEPTH {
                return Err(Failure::fatal(
                    "playlists lead to playlists, never a stream",
                ));
            }
            let bytes = response.read_to_end_limited(PLAYLIST_LIMIT, &cancel)?;
            drop(response);
            let mut last = Failure::fatal("the playlist names no streams");
            for (entry, title) in playlist_streams(format, &bytes)? {
                log::info!("{url} leads to {entry}");
                match open(&entry, pipe, timeouts, depth + 1) {
                    Ok(mut opened) => {
                        if opened.info.station.name.is_none() {
                            opened.info.station.name = title;
                        }
                        return Ok(opened);
                    }
                    Err(e) if pipe.is_closed() => return Err(e),
                    Err(e) => {
                        log::warn!("cannot play {entry}: {}", e.message);
                        last = e;
                    }
                }
            }
            Err(last)
        }
        Kind::Audio => {
            let radio = response.icy
                || response
                    .headers()
                    .any(|(name, _)| name.starts_with("icy-") || name.starts_with("ice-"));
            let metaint = response
                .header("icy-metaint")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|&n| n > 0);
            let info = SegmentInfo {
                station: station(&response),
                mime,
                extension: extension(response.url.path()),
            };
            Ok(Opened {
                response,
                info,
                metaint,
                radio,
            })
        }
    }
}

/// How a connection ended.
#[derive(Debug, PartialEq, Eq)]
enum Pumped {
    /// All of a stream with an end (a file on the web) came.
    Complete,
    /// The decoder let go.
    Closed,
    /// A radio stream's connection dropped or stalled, for this reason: connect again.
    Dropped(String),
    /// A stream with an end stopped short of it, for this reason.
    Cut(String),
}

/// Move a connection's audio into the pipe until it ends, taking out the song titles.
fn pump(mut opened: Opened, pipe: &Pipe, idle: Duration) -> Pumped {
    if !pipe.begin(opened.info.clone()) {
        return Pumped::Closed;
    }
    let mut icy = opened.metaint.map(Icy::new);
    let mut buf = vec![0; 16 * 1024];
    let mut audio = Vec::with_capacity(buf.len());
    let mut titles = Vec::new();
    let mut last = Instant::now();
    let response = &mut opened.response;
    loop {
        if pipe.is_closed() {
            return Pumped::Closed;
        }
        match response.read(&mut buf) {
            Ok(0) if response.is_complete() => return Pumped::Complete,
            Ok(0) if response.has_known_end() => {
                return Pumped::Cut("the connection closed before the end".into());
            }
            Ok(0) if opened.radio => {
                return Pumped::Dropped("the server closed the connection".into());
            }
            // A file on the web whose length was not given.
            Ok(0) => return Pumped::Complete,
            Ok(n) => {
                last = Instant::now();
                let Some(icy) = icy.as_mut() else {
                    if !pipe.write(&buf[..n]) {
                        return Pumped::Closed;
                    }
                    continue;
                };
                audio.clear();
                icy.feed(&buf[..n], &mut audio, &mut titles);
                let mut from = 0;
                for (at, title) in titles.drain(..) {
                    if !pipe.write(&audio[from..at]) {
                        return Pumped::Closed;
                    }
                    log::debug!("stream title: {title}");
                    pipe.title(title);
                    from = at;
                }
                if !pipe.write(&audio[from..]) {
                    return Pumped::Closed;
                }
            }
            Err(e) if http::is_tick(&e) => {
                if last.elapsed() >= idle {
                    return Pumped::Dropped(format!("nothing came for {} seconds", idle.as_secs()));
                }
            }
            Err(e) if response.has_known_end() => return Pumped::Cut(e.to_string()),
            Err(e) => return Pumped::Dropped(e.to_string()),
        }
    }
}

/// The network thread: connect to `url` and keep the pipe filled, connecting again when a
/// radio stream drops, until the decoder lets go or the stream cannot be kept going.
fn run(url: &str, pipe: &Pipe, settings: Settings) {
    let mut failures = 0u32;
    let mut connected = false;
    loop {
        let started = Instant::now();
        let failure = match open(url, pipe, settings.timeouts, 0) {
            Ok(opened) => {
                log::info!(
                    "streaming {} ({})",
                    opened.response.url,
                    opened.info.mime.as_deref().unwrap_or("no media type")
                );
                connected = true;
                match pump(opened, pipe, settings.timeouts.idle) {
                    Pumped::Complete => {
                        pipe.end_segment(Net::Complete);
                        return;
                    }
                    Pumped::Closed => return,
                    Pumped::Cut(why) => {
                        pipe.end_segment(Net::Failed(why));
                        return;
                    }
                    Pumped::Dropped(why) => {
                        if started.elapsed() >= HEALTHY {
                            failures = 0;
                        }
                        pipe.end_segment(Net::Reconnecting);
                        Failure {
                            message: why,
                            retryable: true,
                        }
                    }
                }
            }
            Err(failure) => failure,
        };
        if pipe.is_closed() {
            return;
        }
        failures += 1;
        if !failure.retryable || failures > settings.retry.attempts {
            let message = if failure.retryable {
                format!("{} (tried {failures} times)", failure.message)
            } else {
                failure.message
            };
            log::warn!("giving up on {url}: {message}");
            pipe.end_segment(Net::Failed(message));
            return;
        }
        let delay = settings
            .retry
            .delay
            .saturating_mul(1 << (failures - 1).min(6));
        log::warn!(
            "{url}: {}; trying again in {:.1} s",
            failure.message,
            delay.as_secs_f64()
        );
        pipe.set_net(if connected {
            Net::Reconnecting
        } else {
            Net::Connecting
        });
        if !pipe.wait(delay) {
            return;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(super) mod tests {
    use super::*;
    use crate::http::tests::{listen, serve, serve_on};

    /// Audio bytes with metadata blocks mixed in every `interval` bytes: `titles[i]` after the
    /// `i`th stretch of audio, or an empty block for `None`.
    pub(crate) fn with_metadata(audio: &[u8], interval: usize, titles: &[Option<&str>]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, chunk) in audio.chunks(interval).enumerate() {
            out.extend_from_slice(chunk);
            if chunk.len() < interval {
                break;
            }
            match titles.get(i).copied().flatten() {
                Some(title) => {
                    let mut block = format!("StreamTitle='{title}';StreamUrl='';").into_bytes();
                    block.resize(block.len().div_ceil(16) * 16, 0);
                    out.push(u8::try_from(block.len() / 16).unwrap());
                    out.extend_from_slice(&block);
                }
                None => out.push(0),
            }
        }
        out
    }

    #[test]
    fn metadata_is_taken_out_wherever_the_pieces_break() {
        let audio: Vec<u8> = (0..100u8).collect();
        let mixed = with_metadata(&audio, 16, &[None, Some("Band - Song"), None, Some("Next")]);
        for piece in [1, 3, 7, 1000] {
            let mut icy = Icy::new(16);
            let (mut got, mut titles) = (Vec::new(), Vec::new());
            for chunk in mixed.chunks(piece) {
                icy.feed(chunk, &mut got, &mut titles);
            }
            assert_eq!(got, audio, "pieces of {piece}");
            assert_eq!(
                titles,
                [(32, "Band - Song".to_owned()), (64, "Next".to_owned())]
            );
        }
    }

    #[test]
    fn titles_keep_their_apostrophes_and_accents() {
        assert_eq!(
            stream_title(b"StreamTitle='Guns N' Roses - Don't Cry';StreamUrl='x';\0\0"),
            Some("Guns N' Roses - Don't Cry".into())
        );
        assert_eq!(
            stream_title(b"streamtitle='Caf\xe9 del Mar';"),
            Some("Café del Mar".into())
        );
        assert_eq!(stream_title(b"StreamTitle='';"), Some(String::new()));
        assert_eq!(stream_title(b"StreamTitle='No end"), Some("No end".into()));
        assert_eq!(stream_title(b"StreamUrl='http://x';"), None);
    }

    fn info(kbps: Option<u32>) -> SegmentInfo {
        SegmentInfo {
            station: Station {
                kbps,
                ..Station::default()
            },
            mime: Some("audio/mpeg".into()),
            extension: None,
        }
    }

    fn reader(pipe: &Arc<Pipe>) -> PipeReader {
        PipeReader::new(Arc::clone(pipe), Arc::new(Shared::default()), 0)
    }

    #[test]
    fn segments_end_where_their_connections_did() {
        let pipe = Pipe::new(1024);
        let mut r = reader(&pipe);
        assert!(pipe.begin(info(None)));
        assert!(pipe.write(b"hello"));
        pipe.title("Song".into());
        assert!(pipe.write(b" world"));
        pipe.end_segment(Net::Reconnecting);
        assert!(pipe.begin(info(Some(64))));
        assert!(pipe.write(b"again"));
        let mut buf = [0; 7];
        assert_eq!(r.read(&mut buf).unwrap(), 7);
        assert_eq!(&buf, b"hello w");
        assert_eq!(pipe.due_titles(), ["Song"]);
        assert!(pipe.due_titles().is_empty());
        assert_eq!(r.read(&mut buf).unwrap(), 4);
        assert_eq!(r.read(&mut buf).unwrap(), 0, "the first segment ends");
        assert!(pipe.advance());
        assert!(!pipe.advance(), "the second has not ended");
        assert_eq!(pipe.info().unwrap().station.kbps, Some(64));
        assert_eq!(r.read(&mut buf).unwrap(), 5);
        assert_eq!(pipe.level().net, Net::Streaming);
    }

    #[test]
    fn a_full_pipe_holds_the_writer_until_read_or_closed() {
        let pipe = Pipe::new(4);
        assert!(pipe.begin(info(None)));
        let writer = {
            let pipe = Arc::clone(&pipe);
            thread::spawn(move || pipe.write(b"0123456789"))
        };
        let mut r = reader(&pipe);
        let mut got = Vec::new();
        let mut buf = [0; 3];
        while got.len() < 10 {
            let n = r.read(&mut buf).unwrap();
            got.extend_from_slice(&buf[..n]);
        }
        assert!(writer.join().unwrap());
        assert_eq!(got, b"0123456789");
        let writer = {
            let pipe = Arc::clone(&pipe);
            thread::spawn(move || pipe.write(b"0123456789"))
        };
        thread::sleep(Duration::from_millis(50));
        pipe.close();
        assert!(!writer.join().unwrap(), "closing lets the writer go");
        assert!(!pipe.wait(Duration::from_secs(5)));
    }

    #[test]
    fn a_waiting_reader_gives_up_when_the_ui_moves_on() {
        let pipe = Pipe::new(64);
        assert!(pipe.begin(info(None)));
        let shared = Arc::new(Shared::default());
        let mut r = PipeReader::new(Arc::clone(&pipe), Arc::clone(&shared), 0);
        let started = Instant::now();
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            shared.requested.store(1, Ordering::Release);
        });
        let e = r.read(&mut [0; 8]).unwrap_err();
        assert_ne!(e.kind(), io::ErrorKind::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(2));
        stopper.join().unwrap();
        assert!(r.seek(SeekFrom::Start(0)).is_err());
        assert!(!r.is_seekable());
    }

    #[test]
    fn the_decoder_buffers_before_playing_and_after_running_dry() {
        let pipe = Pipe::new(CAPACITY);
        let mut input = StreamInput::with_pipe(Arc::clone(&pipe));
        assert_eq!(input.next(false, true), Next::Wait);
        assert_eq!(input.take_status(), None, "it started out connecting");
        assert!(pipe.begin(info(Some(64))));
        // 64 kbit/s is 8000 bytes a second: two seconds is 16000 bytes.
        assert!(pipe.write(&[0; 4000]));
        assert_eq!(input.next(false, true), Next::Wait);
        assert_eq!(input.take_status(), Some(StreamStatus::Buffering(25)));
        assert!(pipe.write(&[0; 12_000]));
        assert_eq!(input.next(false, true), Next::Open(info(Some(64))));
        input.playing(Station::default());
        assert_eq!(input.take_status(), Some(StreamStatus::Playing));
        assert_eq!(input.next(true, true), Next::Decode);
        // Read it nearly dry.
        let mut r = reader(&pipe);
        let mut buf = vec![0; 15_000];
        let mut got = 0;
        while got < 15_000 {
            got += r.read(&mut buf[got..]).unwrap();
        }
        assert_eq!(input.next(true, false), Next::Wait, "the device has plenty");
        assert_eq!(input.take_status(), None);
        assert_eq!(input.next(true, true), Next::Wait, "the device runs dry");
        assert_eq!(input.take_status(), Some(StreamStatus::Buffering(6)));
        assert!(pipe.write(&[0; 15_000]));
        assert_eq!(input.next(true, true), Next::Decode);
        assert_eq!(input.take_status(), Some(StreamStatus::Playing));
    }

    #[test]
    fn the_decoder_follows_reconnections_to_the_end() {
        let pipe = Pipe::new(CAPACITY);
        let mut input = StreamInput::with_pipe(Arc::clone(&pipe));
        assert!(pipe.begin(info(None)));
        assert!(pipe.write(b"tiny"));
        pipe.end_segment(Net::Reconnecting);
        assert_eq!(
            input.next(false, true),
            Next::Open(info(None)),
            "an ended segment opens with what it has"
        );
        let _ = reader(&pipe).read(&mut [0; 16]).unwrap();
        assert_eq!(input.next(false, true), Next::Wait);
        assert_eq!(input.take_status(), Some(StreamStatus::Reconnecting));
        assert!(pipe.begin(info(None)));
        assert_eq!(input.next(false, true), Next::Wait);
        assert_eq!(input.take_status(), Some(StreamStatus::Buffering(0)));
        assert!(pipe.write(b"more"));
        pipe.end_segment(Net::Complete);
        assert_eq!(input.next(false, true), Next::Open(info(None)));
        let _ = reader(&pipe).read(&mut [0; 16]).unwrap();
        assert_eq!(input.next(false, true), Next::End);
        let failing = Pipe::new(CAPACITY);
        let mut input = StreamInput::with_pipe(Arc::clone(&failing));
        failing.end_segment(Net::Failed("no".into()));
        assert_eq!(input.next(false, true), Next::Fail("no".into()));
    }

    #[test]
    fn playlists_and_pages_are_told_from_audio() {
        use playlist_file::Format;
        assert_eq!(kind(Some("audio/mpeg"), "/live.pls"), Kind::Audio);
        assert_eq!(
            kind(Some("audio/x-scpls"), "/x"),
            Kind::Playlist(Format::Pls)
        );
        assert_eq!(
            kind(Some("application/vnd.apple.mpegurl"), "/x"),
            Kind::Playlist(Format::M3u)
        );
        assert_eq!(
            kind(Some("text/plain"), "/radio.M3U"),
            Kind::Playlist(Format::M3u)
        );
        assert_eq!(kind(None, "/listen.pls"), Kind::Playlist(Format::Pls));
        assert_eq!(kind(None, "/;stream.mp3"), Kind::Audio);
        assert_eq!(kind(Some("text/html"), "/"), Kind::Page);
        assert_eq!(extension("/;stream.MP3"), Some("mp3".into()));
        assert_eq!(extension("/live"), None);
        assert_eq!(extension("/a.b/c"), None);
    }

    #[test]
    fn playlists_give_their_streams_and_titles() {
        let pls = b"[playlist]\nFile1=http://a.example/\nTitle1=Radio A\nFile2=http://b.example/\n\
                    File3=local.mp3\n";
        assert_eq!(
            playlist_streams(playlist_file::Format::Pls, pls).unwrap(),
            [
                ("http://a.example/".to_owned(), Some("Radio A".to_owned())),
                ("http://b.example/".to_owned(), None),
            ]
        );
        let hls = b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-STREAM-INF:BANDWIDTH=1\nhttp://a/x.m3u8\n";
        let e = playlist_streams(playlist_file::Format::M3u, hls).unwrap_err();
        assert!(!e.retryable && e.message.contains("HLS"), "{e:?}");
    }

    fn quick() -> Settings {
        Settings {
            retry: Retry {
                attempts: 2,
                delay: Duration::from_millis(50),
            },
            timeouts: Timeouts {
                connect: Duration::from_secs(5),
                idle: Duration::from_secs(5),
            },
        }
    }

    /// Everything the pipe received, by segment, once the network thread is done.
    fn drain(pipe: &Arc<Pipe>) -> (Vec<Vec<u8>>, Vec<String>, Net) {
        let mut r = reader(pipe);
        let mut segments = vec![Vec::new()];
        let mut titles = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(
                Instant::now() < deadline,
                "the network thread never finished"
            );
            let mut buf = [0; 4096];
            let n = r.read(&mut buf).unwrap();
            titles.extend(pipe.due_titles());
            if let Some(last) = segments.last_mut() {
                last.extend_from_slice(&buf[..n]);
            }
            if n > 0 {
                continue;
            }
            if pipe.advance() {
                segments.push(Vec::new());
                continue;
            }
            let net = pipe.level().net;
            if matches!(net, Net::Complete | Net::Failed(_)) {
                return (segments, titles, net);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_shoutcast_station_reached_through_a_playlist_is_streamed_and_reconnected() {
        let audio: Vec<u8> = (0..=255u8).cycle().take(3000).collect();
        let mixed = with_metadata(&audio, 1000, &[Some("Band - Song"), None]);
        let station = |body: &[u8]| {
            let mut reply = b"ICY 200 OK\r\nicy-name:Radio Llama\r\nicy-br:128\r\n\
                              icy-metaint:1000\r\ncontent-type:audio/mpeg\r\n\r\n"
                .to_vec();
            reply.extend_from_slice(body);
            vec![reply]
        };
        let (listener, address) = listen();
        // The first server in the list refuses; the second is this one.
        let playlist = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls\r\n\r\n[playlist]\r\n\
             File1=http://127.0.0.1:1/\r\nFile2={address}/live\r\nTitle2=Llama FM\r\n"
        );
        let server = serve_on(
            listener,
            vec![
                vec![playlist.clone().into_bytes()],
                station(&mixed),
                // After the drop, the playlist again and a shorter stay; then nothing answers.
                vec![playlist.into_bytes()],
                station(&mixed[..500]),
            ],
            Duration::ZERO,
        );
        let pipe = Pipe::new(CAPACITY);
        let theirs = Arc::clone(&pipe);
        let url = format!("{address}/listen.pls");
        let runner = thread::spawn(move || run(&url, &theirs, quick()));
        let (segments, titles, net) = drain(&pipe);
        runner.join().unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /listen.pls "));
        assert!(requests[1].starts_with("GET /live "));
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0], audio, "the metadata is taken out");
        assert_eq!(segments[1], audio[..500]);
        assert_eq!(titles, ["Band - Song"]);
        assert!(
            matches!(&net, Net::Failed(why) if why.contains("3 times")),
            "{net:?}"
        );
        assert_eq!(
            pipe.info().unwrap().station.name.as_deref(),
            Some("Radio Llama")
        );
    }

    #[test]
    fn web_pages_and_missing_streams_fail_at_once() {
        let (address, _server) = serve(
            vec![
                vec![b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html>".to_vec()],
                vec![b"HTTP/1.1 404 Not Found\r\n\r\n".to_vec()],
            ],
            Duration::ZERO,
        );
        for (path, expected) in [("/", "web page"), ("/gone", "404")] {
            let pipe = Pipe::new(CAPACITY);
            run(&format!("{address}{path}"), &pipe, quick());
            let net = pipe.level().net;
            assert!(
                matches!(&net, Net::Failed(why) if why.contains(expected)),
                "{net:?}"
            );
        }
    }

    #[test]
    fn a_file_on_the_web_ends_where_its_length_says() {
        let (address, _server) = serve(
            vec![vec![
                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabcdef".to_vec(),
            ]],
            Duration::ZERO,
        );
        let pipe = Pipe::new(CAPACITY);
        run(&format!("{address}/song.mp3"), &pipe, quick());
        let (segments, titles, net) = drain(&pipe);
        assert_eq!(segments, [b"abcdef".to_vec()]);
        assert!(titles.is_empty());
        assert_eq!(net, Net::Complete);
        assert_eq!(pipe.info().unwrap().extension.as_deref(), Some("mp3"));
    }
}
