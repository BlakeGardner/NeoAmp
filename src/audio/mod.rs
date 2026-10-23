//! Audio playback: files are decoded with symphonia and played through cpal.
//!
//! Three threads cooperate, as in Winamp's input plugin -> DSP -> output plugin chain:
//!
//! - The UI thread owns a [`Player`] and sends it commands: open, seek, stop, pause.
//! - A decoder thread (`decode`) reads the file, converts it to stereo `f32` at the device's
//!   sample rate (`resample`) and keeps a lock-free ring buffer about half a second full.
//! - The device callback (`output`) empties the ring, runs the equalizer (`eq`), volume and
//!   balance, and copies what it plays into a second ring for the visualisers.
//!
//! The callback must never block or allocate, so all it shares with the other threads is atomics
//! and ring buffers. Every command the UI sends gets a new *generation* number. Before the
//! decoder queues audio for a generation, it asks the callback to drop whatever is still queued
//! and waits until the callback confirms; that handshake keeps a seek or a new track from ever
//! playing stale samples, and lets the position and "finished" reports name the generation they
//! belong to.
//!
//! Playback is gapless: the UI names the file to play next ([`Player::preload`]), and the
//! decoder starts on it as soon as the current file is decoded, queueing it straight after.
//! A *mark* in a second ring tells the callback where the next track begins, so that its
//! position starts from zero, and its generation is reported, exactly as its first sample
//! plays.
//!
//! The output device is the system's default or one the user chose (`device`). When the
//! device goes away, or another is chosen, a new stream is opened and the decoder carries on
//! into it from where playback was.
//!
//! An internet stream's bytes come from a network thread of its own (`stream`), which feeds
//! the decoder thread through a buffer it reads like a file.
//!
//! [`Scanner`] reads titles and lengths for the playlist on its own thread.

mod decode;
mod device;
mod eq;
mod output;
mod resample;
mod scan;
mod source;
mod stream;

use std::borrow::Cow;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

pub use device::{OutputDevice, output_devices};
pub use eq::{BAND_HZ, BANDS, EqSettings, RANGE_DB as EQ_RANGE_DB};
pub use scan::Scanner;

/// How often to try again to open an output device while none can be.
const RETRY: Duration = Duration::from_secs(2);
/// The rate the decoder works at before any device has been opened.
const FALLBACK_RATE: u32 = 48_000;
/// Preloads remembered while the current track plays. The decoder takes the latest when it
/// reaches the end of the track; the UI sends at most one a frame, so this many cover the
/// eight seconds or so before it is reached, with room to spare.
const MAX_PRELOADS: usize = 256;

/// A kind of file the player decodes, as the open dialog lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// What the open dialog's file type list calls it.
    pub name: &'static str,
    /// File name extensions, lowercase and without the dot.
    pub extensions: &'static [&'static str],
}

/// The formats symphonia is built to decode. Their extensions decide which files a folder adds
/// to the playlist and which files the open dialog shows. Files named explicitly are always
/// tried, whatever their extension: symphonia probes the contents.
pub const FORMATS: [Format; 8] = [
    Format {
        name: "MP3 audio",
        extensions: &["mp3"],
    },
    Format {
        name: "MP1 and MP2 audio",
        extensions: &["mp2", "mp1"],
    },
    Format {
        name: "AAC audio",
        extensions: &["aac"],
    },
    // AAC or Apple Lossless (ALAC) in an MP4 file.
    Format {
        name: "MPEG-4 audio",
        extensions: &["m4a", "m4b", "mp4"],
    },
    Format {
        name: "WAV audio",
        extensions: &["wav", "wave"],
    },
    Format {
        name: "AIFF audio",
        extensions: &["aiff", "aif", "aifc"],
    },
    Format {
        name: "FLAC audio",
        extensions: &["flac"],
    },
    Format {
        name: "Ogg Vorbis audio",
        extensions: &["ogg", "oga"],
    },
];

/// Every extension in [`FORMATS`], in order.
pub fn extensions() -> impl Iterator<Item = &'static str> {
    FORMATS.iter().flat_map(|f| f.extensions.iter().copied())
}

/// Where a playlist entry's audio comes from.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Location {
    /// A file on this computer.
    File(PathBuf),
    /// An internet radio stream, or a file on the web: an `http://` or `https://` URL, as it
    /// was written.
    Url(String),
}

impl Location {
    /// What the user typed or passed on the command line: a URL if it starts with `http://` or
    /// `https://` (in any case), else a file.
    #[must_use]
    pub fn from_arg(text: &str) -> Location {
        if is_web_url(text) {
            Location::Url(text.trim().to_owned())
        } else {
            Location::File(PathBuf::from(text))
        }
    }

    /// The file, for a file.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Location::File(path) => Some(path),
            Location::Url(_) => None,
        }
    }

    /// The URL, for a stream.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        match self {
            Location::File(_) => None,
            Location::Url(url) => Some(url),
        }
    }

    #[must_use]
    pub fn is_url(&self) -> bool {
        matches!(self, Location::Url(_))
    }

    /// A file's name without its folder; for a URL, the last part of its path, or its host
    /// when the path is empty (`http://radio.example:8000/live.mp3?x=1` gives `live.mp3`).
    #[must_use]
    pub fn name(&self) -> Cow<'_, str> {
        match self {
            Location::File(path) => path
                .file_name()
                .map_or(Cow::Borrowed(""), |n| n.to_string_lossy()),
            Location::Url(url) => {
                let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
                let rest = rest.split(['?', '#']).next().unwrap_or(rest);
                let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
                let last = path.rsplit('/').find(|s| !s.is_empty());
                Cow::Borrowed(last.unwrap_or(host))
            }
        }
    }
}

impl std::fmt::Display for Location {
    /// A file's path (with any part that is not Unicode replaced), or the URL.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Location::File(path) => path.display().fmt(f),
            Location::Url(url) => f.write_str(url),
        }
    }
}

/// Whether `text` is an `http://` or `https://` URL (in any case), the kinds NeoAmp streams.
#[must_use]
pub fn is_web_url(text: &str) -> bool {
    let text = text.trim_start().as_bytes();
    [&b"http://"[..], b"https://"].iter().any(|scheme| {
        text.get(..scheme.len())
            .is_some_and(|h| h.eq_ignore_ascii_case(scheme))
    })
}

/// What a file turned out to contain, read when it is opened or scanned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackInfo {
    /// "Artist - Title" from the file's tags, or just the title; `None` without tags.
    pub title: Option<String>,
    /// Length in seconds, when the file states it or it can be estimated.
    pub seconds: Option<f64>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    pub channels: usize,
    /// Average bitrate in kbit/s, from the file size and length.
    pub kbps: Option<u32>,
}

/// A file's tags, as the file info dialog shows them. Tags the file lacks, or leaves empty, are
/// `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The year, or the whole date, as the file writes it.
    pub year: Option<String>,
    /// The track number, with the number of tracks when the file gives it ("7/12").
    pub track: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
}

impl Tags {
    /// "Artist - Title", or just the title, as the playlist names an entry.
    #[must_use]
    pub fn playlist_title(&self) -> Option<String> {
        match (&self.artist, &self.title) {
            (Some(a), Some(t)) => Some(format!("{a} - {t}")),
            (None, Some(t)) => Some(t.clone()),
            _ => None,
        }
    }
}

/// Everything the file info dialog shows about a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileDetails {
    pub tags: Tags,
    /// The kind of file, as symphonia names its reader ("MPEG-4 Part 14", "Free Lossless Audio
    /// Codec Native", ...).
    pub format: String,
    /// The codec, as symphonia names its decoder ("Advanced Audio Coding", ...).
    pub codec: String,
    /// The length, sample rate, channels and average bitrate.
    pub info: TrackInfo,
    /// Bits per sample, for PCM and lossless audio; lossy codecs have none.
    pub bits_per_sample: Option<u32>,
    /// The file's size in bytes.
    pub size: Option<u64>,
}

/// What an internet radio station says about itself in its server's `icy-` headers (or in the
/// playlist that named the stream), and how its audio is coded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Station {
    /// The station's name: `icy-name`, else the title the playlist gave the stream.
    pub name: Option<String>,
    pub genre: Option<String>,
    /// The station's web site (`icy-url`).
    pub homepage: Option<String>,
    pub description: Option<String>,
    /// The bitrate the server announces (`icy-br`), in kbit/s.
    pub kbps: Option<u32>,
    /// The kind of stream, as symphonia names its reader ("MPEG Audio", ...).
    pub format: String,
    /// The codec, as symphonia names its decoder.
    pub codec: String,
}

/// How an internet stream is getting on, for the scrolling title and the "working" light.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamStatus {
    /// Connecting to the server.
    Connecting,
    /// Filling the buffer before playing, or before playing again after it ran dry; this many
    /// percent full.
    Buffering(u8),
    /// The connection dropped and is being made again.
    Reconnecting,
    Playing,
}

/// Something the audio engine reports back to the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The track opened and its audio is on its way to the device.
    Started(TrackInfo),
    /// The last sample of the track has been played.
    Finished,
    /// The track could not be opened, or a stream could not be kept going; the message says
    /// why.
    Failed(String),
    /// A stream's station, once its audio has been recognised.
    Station(Box<Station>),
    /// The song a stream plays now, from its metadata, as the audio that announced it reaches
    /// the speakers. Empty when the station names none.
    StreamTitle(String),
    /// A stream started connecting, buffering or reconnecting, or plays again.
    Status(StreamStatus),
    /// The file named with [`Player::preload`] has started, straight after the track before
    /// it: the current track is now this file. Its own events follow.
    Advanced(PathBuf),
}

/// Instructions for the decoder thread. Each carries the generation it starts.
#[derive(Debug)]
enum Command {
    Open {
        location: Location,
        generation: u64,
    },
    /// `track` is the generation of the track the UI takes for the current one, which the
    /// seek is meant for.
    Seek {
        seconds: f64,
        generation: u64,
        track: u64,
    },
    Stop {
        generation: u64,
    },
    /// The file to play straight after the current track, or none. Its track gets
    /// `generation` once it starts.
    Preload {
        path: Option<PathBuf>,
        generation: u64,
    },
    /// A new device is open: queue into `queue` at `rate` from now on, going on from `seconds`
    /// into track `track` (as for a seek).
    Output {
        queue: output::Queue,
        rate: u32,
        seconds: f64,
        generation: u64,
        track: u64,
    },
}

/// An `f32` stored as its bits, for parameters the device callback reads without locking.
#[derive(Debug, Default)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// Equalizer parameters as the callback sees them.
#[derive(Debug, Default)]
struct EqShared {
    enabled: AtomicBool,
    preamp_db: AtomicF32,
    bands_db: [AtomicF32; BANDS],
    /// Bumped (Release) after the fields above change, so the callback only recomputes its
    /// filters when something moved.
    version: AtomicU32,
}

/// State shared by the UI thread, the decoder thread and the device callback.
#[derive(Debug, Default)]
struct Shared {
    // Set by the UI thread.
    paused: AtomicBool,
    /// Volume slider position, `0..=1`.
    volume: AtomicF32,
    /// Balance, `-1` (left) to `1` (right).
    balance: AtomicF32,
    eq: EqShared,

    // The flush handshake. The decoder stores `flush_position` (device frames), then the new
    // generation in `flush_generation` (Release). The callback drops everything queued, restarts
    // `position` there and confirms by storing the generation in `playing_generation`, then in
    // `flushed` (both Release).
    flush_generation: AtomicU64,
    flush_position: AtomicU64,
    /// The generation being played: the last flush's, or the generation of the last track
    /// boundary the callback has passed since.
    playing_generation: AtomicU64,
    /// The last flush the callback has carried out.
    flushed: AtomicU64,
    /// Device frames played so far in `playing_generation`, counted from its start position.
    position: AtomicU64,
    /// An output stream is running, so the callback will answer a flush.
    output_open: AtomicBool,

    /// The decoder has queued the last sample of this generation (Release).
    ended_generation: AtomicU64,
    /// The callback has played the last sample of this generation (Release).
    finished_generation: AtomicU64,
    /// Bitrate of the audio being decoded, in kbit/s.
    kbps: AtomicU32,
    /// The generation of the last command the UI sent (Release), stored before the command is.
    /// A stream's reader waiting for the network gives up when it moves on, so that the
    /// decoder thread gets to the command.
    requested: AtomicU64,
}

/// The UI thread's handle on audio playback: one output device, one decoder thread.
///
/// Dropping it closes the device stream and lets the decoder thread exit.
pub struct Player {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    /// Decoder reports, tagged with the generation of the track they belong to.
    events: Receiver<(u64, Event)>,
    vis: rtrb::Consumer<f32>,
    /// The device's sample rate, or [`FALLBACK_RATE`] before one has been opened.
    rate: u32,
    /// The last generation handed out.
    counter: u64,
    /// The generation the position and the end of the track are reported in: the last
    /// command that flushed, or the preloaded track played into since.
    generation: u64,
    /// The last command that had the device drop its queue: open, seek, stop or a new device.
    flushed: u64,
    /// The generation of the current track: its `Open`, or its preload. Events of other
    /// tracks are stale.
    track_generation: u64,
    /// Where playback stands once the decoder catches up with the last command, in seconds.
    target: f64,
    /// The last generation whose end has been reported.
    finished_reported: u64,
    /// The last equalizer settings sent, so unchanged ones are not sent again.
    eq: Option<EqSettings>,
    /// What was sent to play next (`None`: nothing), with the generation each would get,
    /// oldest first. The decoder may have started on a file the UI has since replaced.
    preloads: Vec<(u64, Option<PathBuf>)>,
    /// What the decoder plays after the current track, as far as the UI has said: the last
    /// [`Player::preload`] still to come. An unchanged choice is not sent again.
    preloaded: Option<PathBuf>,
    /// Events of preloaded tracks, held until the device reaches them.
    early: Vec<(u64, Event)>,
    /// Events to hand to the UI.
    ready: VecDeque<Event>,
    /// The device the user chose, by [`OutputDevice::id`]; `None` for the system default.
    choice: Option<String>,
    /// `None` while no device can be opened.
    output: Option<output::Output>,
    /// While the chosen device is not connected (and the default plays): looks for it.
    watch: Option<device::Watch>,
    /// When to try again to open a device, while none is open.
    retry: Option<Instant>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("rate", &self.rate)
            .field("generation", &self.generation)
            .field("track_generation", &self.track_generation)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

impl Player {
    /// Open the output device (`choice`, by [`OutputDevice::id`], or the system default) and
    /// start the decoder thread. Without any device, playback waits for one to appear.
    ///
    /// # Errors
    /// Fails if the decoder thread cannot be spawned.
    pub fn new(choice: Option<String>) -> Result<Player> {
        let shared = Arc::new(Shared::default());
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let (_, vis) = rtrb::RingBuffer::new(1);
        let mut player = Player {
            shared: Arc::clone(&shared),
            commands,
            events,
            vis,
            rate: FALLBACK_RATE,
            counter: 0,
            generation: 0,
            flushed: 0,
            track_generation: 0,
            target: 0.0,
            finished_reported: 0,
            eq: None,
            preloads: Vec::new(),
            preloaded: None,
            early: Vec::new(),
            ready: VecDeque::new(),
            choice,
            output: None,
            watch: None,
            retry: None,
        };
        let queue = player.open_device().unwrap_or_else(|| {
            player.retry = Some(Instant::now() + RETRY);
            output::Queue::unplayed(FALLBACK_RATE)
        });
        decode::spawn(
            shared,
            queue,
            player.rate,
            command_rx,
            event_tx,
            stream::Settings::default(),
        )?;
        Ok(player)
    }

    /// The device's sample rate, which is also the rate of the visualiser samples. It changes
    /// when [`Player::maintain`] says the device did.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// The device chosen with [`Player::set_device`], by [`OutputDevice::id`].
    #[must_use]
    pub fn device(&self) -> Option<&str> {
        self.choice.as_deref()
    }

    /// Play through device `choice` (by [`OutputDevice::id`]), or the system default.
    pub fn set_device(&mut self, choice: Option<String>) {
        if choice == self.choice {
            return;
        }
        self.choice = choice;
        self.watch = None;
        self.reopen();
    }

    /// Once a frame: open the device again if it was lost, go back to the chosen device when
    /// it reappears, or try again to open one. Returns true when a new device was opened.
    pub fn maintain(&mut self, now: Instant) -> bool {
        let lost = self
            .output
            .as_ref()
            .is_some_and(|o| o.lost.load(Ordering::Acquire));
        let back = self.watch.as_ref().is_some_and(device::Watch::found);
        let due = self.output.is_none() && self.retry.is_some_and(|t| now >= t);
        if !(lost || back || due) {
            return false;
        }
        if back {
            log::info!("the chosen audio output is back");
        }
        self.reopen();
        self.output.is_some()
    }

    /// Close the device and open the right one again, carrying on from where playback is.
    fn reopen(&mut self) {
        // The position to go on from is the preloaded track's, if the device has reached it.
        self.check_advance();
        let here = self.position();
        // First close the old stream: some devices cannot be opened twice.
        self.output = None;
        self.shared.output_open.store(false, Ordering::Release);
        let Some(queue) = self.open_device() else {
            self.retry = Some(Instant::now() + RETRY);
            return;
        };
        self.retry = None;
        let rate = self.rate;
        let track = self.track_generation;
        self.flush_command(
            |generation| Command::Output {
                queue,
                rate,
                seconds: here,
                generation,
                track,
            },
            here,
        );
    }

    /// Open the chosen device, or the default while the chosen one is not connected, and
    /// return the queue that feeds it.
    fn open_device(&mut self) -> Option<output::Queue> {
        let device = match self.choice.as_deref() {
            Some(id) => match device::find(id) {
                Some(device) => {
                    self.watch = None;
                    Some(device)
                }
                None => {
                    if self.watch.is_none() {
                        log::warn!(
                            "audio output {id} is not connected; playing through the default until it is"
                        );
                    }
                    // A watch that saw the device, gone again by now, looks for it afresh;
                    // left as it was, it would have the device reopened every frame.
                    if self.watch.as_ref().is_none_or(device::Watch::found) {
                        self.watch = device::Watch::spawn(id.to_owned())
                            .map_err(|e| log::error!("{e:#}"))
                            .ok();
                    }
                    device::default_device()
                }
            },
            None => {
                self.watch = None;
                device::default_device()
            }
        };
        let quiet = self.retry.is_some();
        let Some(device) = device else {
            if !quiet {
                log::error!("no audio output device; waiting for one");
            }
            return None;
        };
        match output::open(Arc::clone(&self.shared), &device) {
            Ok(opened) => {
                log::info!(
                    "audio output: {} at {} Hz",
                    opened.output.name,
                    opened.output.rate
                );
                self.rate = opened.output.rate;
                self.vis = opened.vis;
                self.output = Some(opened.output);
                self.shared.output_open.store(true, Ordering::Release);
                Some(opened.queue)
            }
            Err(e) => {
                if !quiet {
                    log::error!("cannot open the audio output: {e:#}");
                }
                None
            }
        }
    }

    fn next_generation(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    fn command(&self, command: Command) {
        if self.commands.send(command).is_err() {
            log::error!("the audio decoder thread has stopped");
        }
    }

    /// Send a command that has the device drop what it has queued; until it has, the position
    /// is `target`.
    fn flush_command(&mut self, make: impl FnOnce(u64) -> Command, target: f64) {
        let generation = self.next_generation();
        self.generation = generation;
        self.flushed = generation;
        self.target = target;
        self.command(make(generation));
    }

    /// A command that leaves the current track behind: what is waiting for it is stale.
    fn forget_track(&mut self) {
        self.preloads.clear();
        self.preloaded = None;
        self.early.clear();
        self.ready.clear();
    }

    /// Have a stream's reader that waits for the network give up, so the decoder thread gets
    /// to the command about to be sent: it does once `requested` is past its generation.
    fn interrupt_stream(&self) {
        self.shared
            .requested
            .store(self.counter + 1, Ordering::Release);
    }

    /// Start playing a file from the beginning, or connect to a stream.
    pub fn open(&mut self, location: &Location) {
        self.shared.paused.store(false, Ordering::Relaxed);
        self.forget_track();
        let location = location.clone();
        self.interrupt_stream();
        self.flush_command(
            |generation| Command::Open {
                location,
                generation,
            },
            0.0,
        );
        self.track_generation = self.generation;
    }

    /// Jump to `seconds` into the current track.
    pub fn seek(&mut self, seconds: f64) {
        let track = self.track_generation;
        self.flush_command(
            |generation| Command::Seek {
                seconds,
                generation,
                track,
            },
            seconds,
        );
    }

    /// Stop and close the current track.
    pub fn stop(&mut self) {
        self.forget_track();
        self.interrupt_stream();
        self.flush_command(|generation| Command::Stop { generation }, 0.0);
    }

    /// Name the file to play straight after the current track, without a gap, or `None` to
    /// stop or move on in the usual way at its end. Called every frame; only a change is sent.
    pub fn preload(&mut self, path: Option<&Path>) {
        if self.preloaded.as_deref() == path {
            return;
        }
        self.preloaded = path.map(Path::to_path_buf);
        let generation = self.next_generation();
        if self.preloads.len() == MAX_PRELOADS {
            self.preloads.remove(0);
        }
        self.preloads.push((generation, self.preloaded.clone()));
        self.command(Command::Preload {
            path: self.preloaded.clone(),
            generation,
        });
    }

    /// Pause or resume. Takes effect at the next device callback.
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    /// Volume slider position, `0..=1`.
    pub fn set_volume(&self, volume: f32) {
        self.shared.volume.store(volume.clamp(0.0, 1.0));
    }

    /// Balance from `-1` (left) to `1` (right).
    pub fn set_balance(&self, balance: f32) {
        self.shared.balance.store(balance.clamp(-1.0, 1.0));
    }

    /// Hand the equalizer settings to the device callback if they changed.
    pub fn set_eq(&mut self, settings: &EqSettings) {
        if self.eq.as_ref() == Some(settings) {
            return;
        }
        let eq = &self.shared.eq;
        eq.enabled.store(settings.enabled, Ordering::Relaxed);
        eq.preamp_db.store(settings.preamp_db);
        for (band, db) in eq.bands_db.iter().zip(settings.bands_db) {
            band.store(db);
        }
        eq.version.fetch_add(1, Ordering::Release);
        self.eq = Some(*settings);
    }

    /// Seconds into the current track that the device is playing. Until the decoder has
    /// caught up with the last command this is where that command will land.
    #[must_use]
    pub fn position(&self) -> f64 {
        if self.shared.playing_generation.load(Ordering::Acquire) != self.generation {
            return self.target;
        }
        // Frame counts stay far below 2^52, so the conversion is exact.
        self.shared.position.load(Ordering::Relaxed) as f64 / f64::from(self.rate)
    }

    /// Bitrate of the audio being decoded, in kbit/s (0 when unknown).
    #[must_use]
    pub fn kbps(&self) -> u32 {
        self.shared.kbps.load(Ordering::Relaxed)
    }

    /// The next thing to tell the UI, if any.
    pub fn poll(&mut self) -> Option<Event> {
        while let Ok((generation, event)) = self.events.try_recv() {
            if generation == self.track_generation {
                self.ready.push_back(event);
            } else if self.preloads.iter().any(|(g, _)| *g == generation) {
                self.early.push((generation, event));
            }
        }
        if self.ready.is_empty() {
            self.check_advance();
        }
        if let Some(event) = self.ready.pop_front() {
            return Some(event);
        }
        let finished = self.shared.finished_generation.load(Ordering::Acquire);
        if finished == self.generation && self.finished_reported != self.generation {
            self.finished_reported = self.generation;
            return Some(Event::Finished);
        }
        None
    }

    /// If the device has reached a preloaded track, make it the current one and queue
    /// [`Event::Advanced`] with the events it sent while waiting.
    fn check_advance(&mut self) {
        // Only once the last flush has been carried out: until then the device may still be
        // passing a boundary that the flush is about to throw away. Read first, so the
        // playing generation read after it is from after the flush.
        if self.shared.flushed.load(Ordering::Acquire) != self.flushed {
            return;
        }
        let playing = self.shared.playing_generation.load(Ordering::Acquire);
        if playing == self.generation {
            return;
        }
        let Some(path) = self
            .preloads
            .iter()
            .find(|(g, _)| *g == playing)
            .and_then(|(_, path)| path.clone())
        else {
            return;
        };
        let generation = playing;
        // The decoder took this file as its next; what it plays after it is whatever was sent
        // since.
        self.preloads.retain(|(g, _)| *g > generation);
        self.preloaded = self.preloads.last().and_then(|(_, path)| path.clone());
        self.generation = generation;
        self.track_generation = generation;
        self.target = 0.0;
        self.ready.push_back(Event::Advanced(path));
        let early = std::mem::take(&mut self.early);
        for (g, event) in early {
            if g == generation {
                self.ready.push_back(event);
            } else if g > generation {
                self.early.push((g, event));
            }
        }
    }

    /// Pass every sample played since the last call to `f`: interleaved stereo, left first, at
    /// [`Player::sample_rate`], after the equalizer but before volume and balance. Each slice
    /// holds whole frames.
    pub fn drain_vis(&mut self, mut f: impl FnMut(&[f32])) {
        let n = self.vis.slots();
        if let Ok(chunk) = self.vis.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            f(a);
            f(b);
            chunk.commit_all();
        }
    }
}

/// Whether `path` has one of the [`extensions`] (case-insensitively).
#[must_use]
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Expand `paths` for the playlist: folders become the audio files inside them (recursively,
/// sorted by name), other paths are kept as given.
#[must_use]
pub fn audio_files(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            collect_dir(&path, &mut out);
        } else {
            out.push(path);
        }
    }
    out
}

fn collect_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("cannot read {}: {e}", dir.display());
            return;
        }
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_cached_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        // `file_type` does not follow symlinks, so a link back up the tree cannot loop.
        match entry.file_type() {
            Ok(t) if t.is_dir() => collect_dir(&path, out),
            Ok(_) if is_audio_file(&path) => out.push(path),
            Ok(_) => {}
            Err(e) => log::warn!("cannot read {}: {e}", path.display()),
        }
    }
}

/// Read a file's tags and length without decoding it.
///
/// # Errors
/// Fails if the file cannot be opened or holds no audio symphonia can decode.
pub fn probe(path: &Path) -> Result<TrackInfo> {
    source::probe(path).with_context(|| format!("reading {}", path.display()))
}

/// Read everything the file info dialog shows about a file, without decoding it.
///
/// # Errors
/// Fails if the file cannot be opened or holds no audio symphonia can decode.
pub fn details(path: &Path) -> Result<FileDetails> {
    source::details(path).with_context(|| format!("reading {}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn audio_extensions_are_matched_case_insensitively() {
        assert!(is_audio_file(Path::new("a/b/Song.MP3")));
        assert!(is_audio_file(Path::new("x.wav")));
        assert!(is_audio_file(Path::new("x.flac")));
        assert!(is_audio_file(Path::new("x.Wave")));
        assert!(is_audio_file(Path::new("x.oga")));
        for name in [
            "x.m4a", "x.M4B", "x.mp4", "x.aac", "x.aiff", "x.aif", "x.mp2", "x.mp1",
        ] {
            assert!(is_audio_file(Path::new(name)), "{name}");
        }
        assert!(!is_audio_file(Path::new("skin.wsz")));
        assert!(!is_audio_file(Path::new("noext")));
    }

    #[test]
    fn folders_expand_to_sorted_audio_files_and_files_pass_through() {
        let dir = std::env::temp_dir().join(format!("neoamp-audio-files-{}", std::process::id()));
        let sub = dir.join("disc 2");
        fs::create_dir_all(&sub).unwrap();
        for name in ["b.mp3", "a.wav", "cover.jpg"] {
            fs::write(dir.join(name), b"").unwrap();
        }
        fs::write(sub.join("c.flac"), b"").unwrap();
        let got = audio_files([dir.clone(), PathBuf::from("/elsewhere/notes.txt")]);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            got,
            vec![
                dir.join("a.wav"),
                dir.join("b.mp3"),
                sub.join("c.flac"),
                PathBuf::from("/elsewhere/notes.txt"),
            ]
        );
    }

    #[test]
    fn atomic_f32_round_trips() {
        let a = AtomicF32::default();
        assert_eq!(a.load(), 0.0);
        a.store(-0.375);
        assert_eq!(a.load(), -0.375);
    }

    /// The decoder thread and the mixer, with a plain thread standing in for the device: it
    /// renders as fast as it can and keeps everything it played.
    struct Rig {
        shared: Arc<Shared>,
        commands: Sender<Command>,
        events: Receiver<(u64, Event)>,
        stop: Arc<AtomicBool>,
        device: std::thread::JoinHandle<Vec<f32>>,
    }

    const RATE: u32 = 48_000;

    impl Rig {
        fn new() -> Rig {
            Rig::with(stream::Settings::default())
        }

        fn with(network: stream::Settings) -> Rig {
            let shared = Arc::new(Shared::default());
            shared.volume.store(1.0);
            shared.output_open.store(true, Ordering::Release);
            let (audio, queue_rx) = rtrb::RingBuffer::new(RATE as usize);
            let (marks, marks_rx) = rtrb::RingBuffer::new(16);
            let (vis_tx, _vis) = rtrb::RingBuffer::new(16);
            let mut mixer =
                output::Mixer::new(Arc::clone(&shared), queue_rx, marks_rx, vis_tx, RATE);
            let (commands, command_rx) = mpsc::channel();
            let (event_tx, events) = mpsc::channel();
            decode::spawn(
                Arc::clone(&shared),
                output::Queue { audio, marks },
                RATE,
                command_rx,
                event_tx,
                network,
            )
            .unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let halt = Arc::clone(&stop);
            let device = std::thread::spawn(move || {
                let mut block = vec![0.0; 1024];
                let mut played = Vec::new();
                while !halt.load(Ordering::Relaxed) {
                    mixer.render(&mut block, 2);
                    played.extend_from_slice(&block);
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
                played
            });
            Rig {
                shared,
                commands,
                events,
                stop,
                device,
            }
        }

        fn event(&self) -> (u64, Event) {
            self.events
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
        }

        fn wait_finished(&self, generation: u64) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while self.shared.finished_generation.load(Ordering::Acquire) != generation {
                assert!(std::time::Instant::now() < deadline, "never finished");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }

        fn position(&self) -> u64 {
            self.shared.position.load(Ordering::Relaxed)
        }

        fn finish(self) -> Vec<f32> {
            self.stop.store(true, Ordering::Relaxed);
            self.device.join().unwrap()
        }
    }

    #[test]
    fn a_file_plays_through_resampled_to_the_device_rate() {
        // One second of a quiet square wave at 22.05 kHz, mono.
        let path = source::tests::write_wav("e2e", 22_050, 1, 22_050, |i, _| {
            if i % 50 < 25 { 3000 } else { -3000 }
        });
        let rig = Rig::new();
        rig.commands
            .send(Command::Open {
                location: Location::File(path.clone()),
                generation: 1,
            })
            .unwrap();
        let (generation, event) = rig.event();
        assert_eq!(generation, 1);
        let Event::Started(info) = event else {
            panic!("expected Started, got {event:?}");
        };
        assert_eq!((info.sample_rate, info.channels), (22_050, 1));
        rig.wait_finished(1);
        // Exactly one second at 48 kHz.
        assert_eq!(rig.position(), u64::from(RATE));
        let played = rig.finish();
        std::fs::remove_file(&path).unwrap();
        let loud = played.chunks_exact(2).filter(|f| f[0].abs() > 0.05).count();
        // Every frame of the square wave is loud apart from a few at the edges.
        assert!((47_000..=48_000).contains(&loud), "{loud} loud frames");
    }

    #[test]
    fn seeking_drops_queued_audio_and_counts_from_the_new_spot() {
        let path = source::tests::write_wav("e2e-seek", RATE, 2, RATE * 2, |_, _| 1000);
        let rig = Rig::new();
        rig.commands
            .send(Command::Open {
                location: Location::File(path.clone()),
                generation: 1,
            })
            .unwrap();
        assert!(matches!(rig.event(), (1, Event::Started(_))));
        rig.commands
            .send(Command::Seek {
                seconds: 1.5,
                generation: 2,
                track: 1,
            })
            .unwrap();
        rig.wait_finished(2);
        assert_eq!(rig.shared.finished_generation.load(Ordering::Acquire), 2);
        assert_eq!(rig.position(), 2 * u64::from(RATE));
        // Stopping, then opening a file that does not exist.
        rig.commands.send(Command::Stop { generation: 3 }).unwrap();
        rig.commands
            .send(Command::Open {
                location: Location::File(PathBuf::from("/nonexistent.mp3")),
                generation: 4,
            })
            .unwrap();
        assert!(matches!(rig.event(), (4, Event::Failed(_))));
        let _ = rig.finish();
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_preloaded_file_follows_without_a_gap() {
        // Half a second at the device rate, then half a second at 44.1 kHz, which is resampled.
        let a = source::tests::write_wav("gapless-a", RATE, 2, RATE / 2, |_, _| 8000);
        let b = source::tests::write_wav("gapless-b", 44_100, 2, 22_050, |_, _| 16000);
        let rig = Rig::new();
        rig.commands
            .send(Command::Preload {
                path: Some(b.clone()),
                generation: 2,
            })
            .unwrap();
        rig.commands
            .send(Command::Open {
                location: Location::File(a.clone()),
                generation: 1,
            })
            .unwrap();
        // Sent at once, so that it is there before the first file has been decoded (as the UI's
        // is, a frame after the track starts). Opening dropped the preload sent before it.
        rig.commands
            .send(Command::Preload {
                path: Some(b.clone()),
                generation: 3,
            })
            .unwrap();
        assert!(matches!(rig.event(), (1, Event::Started(_))));
        let (generation, event) = rig.event();
        assert_eq!(generation, 3);
        assert!(matches!(event, Event::Started(info) if info.sample_rate == 44_100));
        rig.wait_finished(3);
        assert_eq!(rig.shared.playing_generation.load(Ordering::Acquire), 3);
        // The second file's position counts from its own start: exactly half a second.
        assert_eq!(rig.position(), u64::from(RATE / 2));
        let played = rig.finish();
        std::fs::remove_file(&a).unwrap();
        std::fs::remove_file(&b).unwrap();
        // The stand-in device renders far faster than real time and may outrun the decoder
        // now and then: leave out the silence of those underruns. Nothing else is silent: the
        // second file was never opened, only preloaded, so it can only have been queued
        // straight after the first.
        let sound: Vec<f32> = played
            .chunks_exact(2)
            .map(|f| f[0])
            .filter(|&x| x != 0.0)
            .collect();
        assert_eq!(
            sound.len(),
            RATE as usize,
            "both files, each exactly half a second"
        );
        let louder = sound.iter().position(|&x| x > 0.4).unwrap();
        assert!(
            (RATE as usize / 2 - 2..=RATE as usize / 2 + 2).contains(&louder),
            "the second file starts after the first's half second: {louder}"
        );
        assert!(sound[..louder].iter().all(|&x| (0.2..0.3).contains(&x)));
    }

    #[test]
    fn a_seek_before_the_device_reaches_a_preloaded_file_goes_back_to_the_playing_one() {
        // Longer than the queue's half second, so it is still being queued when the preload
        // comes.
        let a = source::tests::write_wav("revert-a", RATE, 2, RATE * 3 / 4, |_, _| 8000);
        let b = source::tests::write_wav("revert-b", RATE, 2, RATE / 4, |_, _| 16000);
        let shared = Arc::new(Shared::default());
        shared.volume.store(1.0);
        shared.output_open.store(true, Ordering::Release);
        let (audio, queue_rx) = rtrb::RingBuffer::new(RATE as usize);
        let (marks, marks_rx) = rtrb::RingBuffer::new(16);
        let (vis_tx, _vis) = rtrb::RingBuffer::new(16);
        let mut mixer = output::Mixer::new(Arc::clone(&shared), queue_rx, marks_rx, vis_tx, RATE);
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let queue = output::Queue { audio, marks };
        decode::spawn(
            Arc::clone(&shared),
            queue,
            RATE,
            command_rx,
            event_tx,
            stream::Settings::default(),
        )
        .unwrap();
        let mut block = vec![0.0; 256];
        // The decoder waits for each flush to be carried out: render until it is.
        let render_until = |mixer: &mut output::Mixer, block: &mut Vec<f32>, generation| {
            while shared.playing_generation.load(Ordering::Acquire) != generation {
                mixer.render(block, 2);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        commands
            .send(Command::Open {
                location: Location::File(a.clone()),
                generation: 1,
            })
            .unwrap();
        render_until(&mut mixer, &mut block, 1);
        let started = events.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(started, (1, Event::Started(_))));
        commands
            .send(Command::Preload {
                path: Some(b.clone()),
                generation: 2,
            })
            .unwrap();
        // Play the first file only until its end is queued and the second started behind it.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(event) = events.try_recv() {
                assert!(matches!(event, (2, Event::Started(_))), "{event:?}");
                break;
            }
            assert!(Instant::now() < deadline, "the second file never started");
            mixer.render(&mut block, 2);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(shared.playing_generation.load(Ordering::Acquire), 1);
        commands
            .send(Command::Seek {
                seconds: 0.125,
                generation: 3,
                track: 1,
            })
            .unwrap();
        render_until(&mut mixer, &mut block, 3);
        assert_eq!(shared.position.load(Ordering::Relaxed), u64::from(RATE / 8));
        let mut played = Vec::new();
        while shared.finished_generation.load(Ordering::Acquire) != 2 {
            mixer.render(&mut block, 2);
            played.extend(block.chunks_exact(2).map(|f| f[0]));
            assert!(Instant::now() < deadline + Duration::from_secs(10));
        }
        // The second file was opened again after the first, in the generation it had.
        let reopened = events.try_recv().unwrap();
        assert!(matches!(reopened, (2, Event::Started(_))), "{reopened:?}");
        drop(commands);
        std::fs::remove_file(&a).unwrap();
        std::fs::remove_file(&b).unwrap();
        // This loop renders far faster than real time and outruns the decoder now and then:
        // leave out the silence of those underruns.
        let sound: Vec<f32> = played.into_iter().filter(|&x| x != 0.0).collect();
        let switch = sound.iter().position(|&x| x > 0.4).unwrap();
        // The rest of the first file from an eighth of a second in, then all of the second.
        assert_eq!(switch, (RATE * 3 / 4 - RATE / 8) as usize);
        assert!(sound[..switch].iter().all(|&x| x < 0.4));
        assert_eq!(sound.len() - switch, (RATE / 4) as usize);
    }

    /// The shared state, the command and event channels, and a mixer to render by hand.
    type ManualRig = (
        Arc<Shared>,
        Sender<Command>,
        Receiver<(u64, Event)>,
        output::Mixer,
    );

    /// The decoder thread with a mixer the test renders itself, at [`RATE`].
    fn manual_rig() -> ManualRig {
        let shared = Arc::new(Shared::default());
        shared.volume.store(1.0);
        shared.output_open.store(true, Ordering::Release);
        let (audio, queue_rx) = rtrb::RingBuffer::new(RATE as usize);
        let (marks, marks_rx) = rtrb::RingBuffer::new(16);
        let (vis_tx, _vis) = rtrb::RingBuffer::new(16);
        let mixer = output::Mixer::new(Arc::clone(&shared), queue_rx, marks_rx, vis_tx, RATE);
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let queue = output::Queue { audio, marks };
        let settings = stream::Settings::default();
        decode::spawn(
            Arc::clone(&shared),
            queue,
            RATE,
            command_rx,
            event_tx,
            settings,
        )
        .unwrap();
        (shared, commands, events, mixer)
    }

    #[test]
    fn a_seek_meant_for_the_track_before_goes_back_to_it_though_the_device_moved_on() {
        let a = source::tests::write_wav("late-a", RATE, 2, RATE * 3 / 4, |_, _| 8000);
        let b = source::tests::write_wav("late-b", RATE, 2, RATE * 3 / 4, |_, _| 16000);
        let (shared, commands, _events, mut mixer) = manual_rig();
        let mut block = vec![0.0; 256];
        let render_until = |mixer: &mut output::Mixer, block: &mut Vec<f32>, generation| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while shared.playing_generation.load(Ordering::Acquire) != generation {
                assert!(Instant::now() < deadline, "never reached {generation}");
                mixer.render(block, 2);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let open = Command::Open {
            location: Location::File(a.clone()),
            generation: 1,
        };
        commands.send(open).unwrap();
        let preload = Command::Preload {
            path: Some(b.clone()),
            generation: 2,
        };
        commands.send(preload).unwrap();
        // The device passes into the second file, but the UI has not seen it: its seek is
        // meant for the first.
        render_until(&mut mixer, &mut block, 2);
        let seek = Command::Seek {
            seconds: 0.5,
            generation: 3,
            track: 1,
        };
        commands.send(seek).unwrap();
        render_until(&mut mixer, &mut block, 3);
        let mut played = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while shared.finished_generation.load(Ordering::Acquire) != 2 {
            assert!(Instant::now() < deadline, "never finished");
            mixer.render(&mut block, 2);
            played.extend(block.chunks_exact(2).map(|f| f[0]));
            std::thread::sleep(Duration::from_micros(100));
        }
        drop(commands);
        std::fs::remove_file(&a).unwrap();
        std::fs::remove_file(&b).unwrap();
        let sound: Vec<f32> = played.into_iter().filter(|&x| x != 0.0).collect();
        let switch = sound.iter().position(|&x| x > 0.4).unwrap();
        // The first file's last quarter second, then the whole of the second.
        assert_eq!(switch, (RATE / 4) as usize);
        assert_eq!(sound.len() - switch, (RATE * 3 / 4) as usize);
    }

    #[test]
    fn a_new_device_at_another_rate_goes_on_from_where_the_old_one_was() {
        // Two seconds at 44.1 kHz, played first at 48 kHz, then at 44.1 kHz from 0.5 s in.
        let path = source::tests::write_wav("new-device", 44_100, 2, 88_200, |_, _| 8000);
        let shared = Arc::new(Shared::default());
        shared.volume.store(1.0);
        shared.output_open.store(true, Ordering::Release);
        let new_mixer = |rate: u32| {
            let (audio, queue_rx) = rtrb::RingBuffer::new(rate as usize);
            let (marks, marks_rx) = rtrb::RingBuffer::new(16);
            let (vis_tx, _vis) = rtrb::RingBuffer::new(16);
            let mixer = output::Mixer::new(Arc::clone(&shared), queue_rx, marks_rx, vis_tx, rate);
            (output::Queue { audio, marks }, mixer)
        };
        let (queue, mut mixer) = new_mixer(RATE);
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        decode::spawn(
            Arc::clone(&shared),
            queue,
            RATE,
            command_rx,
            event_tx,
            stream::Settings::default(),
        )
        .unwrap();
        let mut block = vec![0.0; 256];
        let render_until = |mixer: &mut output::Mixer, block: &mut Vec<f32>, generation| {
            while shared.playing_generation.load(Ordering::Acquire) != generation {
                mixer.render(block, 2);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        commands
            .send(Command::Open {
                location: Location::File(path.clone()),
                generation: 1,
            })
            .unwrap();
        render_until(&mut mixer, &mut block, 1);
        let started = events.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(started, (1, Event::Started(_))));
        // The old device goes; the new one runs at 44.1 kHz.
        drop(mixer);
        let (queue, mut mixer) = new_mixer(44_100);
        commands
            .send(Command::Output {
                queue,
                rate: 44_100,
                seconds: 0.5,
                generation: 2,
                track: 1,
            })
            .unwrap();
        render_until(&mut mixer, &mut block, 2);
        assert_eq!(shared.position.load(Ordering::Relaxed), 22_050);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut played = 0usize;
        while shared.finished_generation.load(Ordering::Acquire) != 2 {
            mixer.render(&mut block, 2);
            played += block.chunks_exact(2).filter(|f| f[0] != 0.0).count();
            assert!(Instant::now() < deadline, "never finished");
        }
        drop(commands);
        std::fs::remove_file(&path).unwrap();
        // The rest, a second and a half, at the new device's rate and not resampled.
        assert_eq!(played, 66_150);
        assert_eq!(shared.position.load(Ordering::Relaxed), 88_200);
    }

    #[test]
    fn a_stream_plays_names_its_songs_reconnects_and_finally_fails() {
        use crate::http::tests::{listen, serve_on};
        use std::time::Duration;
        // One second of a quiet square wave at 22.05 kHz, sent as a SHOUTcast station would,
        // with the song's title after the first 4000 bytes. The server drops the connection
        // after it, and again after a second visit; then nothing answers.
        let wav = source::tests::write_wav("stream", 22_050, 1, 22_050, |i, _| {
            if i % 50 < 25 { 3000 } else { -3000 }
        });
        let bytes = std::fs::read(&wav).unwrap();
        std::fs::remove_file(&wav).unwrap();
        let mut reply = b"ICY 200 OK\r\nicy-name:Radio Llama\r\nicy-metaint:4000\r\n\
                          content-type:audio/wav\r\n\r\n"
            .to_vec();
        reply.extend(stream::tests::with_metadata(
            &bytes,
            4000,
            &[Some("Band - Song")],
        ));
        let (listener, address) = listen();
        // The second visit is answered only once the player says it is reconnecting. Answered
        // at once, it can arrive while the first is still playing out on a slow machine, and
        // then the player goes straight on with no gap to report.
        let mut held = Some((listener.try_clone().unwrap(), reply.clone()));
        serve_on(listener, vec![vec![reply]], Duration::ZERO);
        let rig = Rig::with(stream::Settings {
            retry: stream::Retry {
                attempts: 1,
                delay: Duration::from_millis(50),
            },
            timeouts: crate::http::Timeouts::default(),
        });
        rig.commands
            .send(Command::Open {
                location: Location::Url(format!("{address}/;stream")),
                generation: 1,
            })
            .unwrap();
        let mut events = Vec::new();
        loop {
            let (generation, event) = rig.event();
            assert_eq!(generation, 1);
            if event == Event::Status(StreamStatus::Reconnecting)
                && let Some((listener, reply)) = held.take()
            {
                serve_on(listener, vec![vec![reply]], Duration::ZERO);
            }
            let failed = matches!(event, Event::Failed(_));
            events.push(event);
            if failed {
                break;
            }
        }
        // The device plays out what is queued, both visits' audio: two seconds at 48 kHz.
        let mut played = rig.position();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let now = rig.position();
            if now == played {
                break;
            }
            played = now;
        }
        let _ = rig.finish();
        assert_eq!(events[0], Event::Status(StreamStatus::Connecting));
        let started: Vec<&TrackInfo> = events
            .iter()
            .filter_map(|e| match e {
                Event::Started(info) => Some(info),
                _ => None,
            })
            .collect();
        assert_eq!(started.len(), 2, "{events:?}");
        assert_eq!((started[0].sample_rate, started[0].channels), (22_050, 1));
        let station = events.iter().find_map(|e| match e {
            Event::Station(s) => Some(s),
            _ => None,
        });
        let station = station.unwrap();
        assert_eq!(station.name.as_deref(), Some("Radio Llama"));
        assert!(!station.format.is_empty() && !station.codec.is_empty());
        let titles = events
            .iter()
            .filter(|e| **e == Event::StreamTitle("Band - Song".into()))
            .count();
        assert_eq!(titles, 2);
        assert!(events.contains(&Event::Status(StreamStatus::Playing)));
        assert!(events.contains(&Event::Status(StreamStatus::Reconnecting)));
        assert!(
            matches!(events.last(), Some(Event::Failed(why)) if why.contains("tried")),
            "{events:?}"
        );
        assert!(
            (95_000..=97_000).contains(&played),
            "{played} frames played"
        );
    }
}
