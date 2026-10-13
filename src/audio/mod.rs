//! Audio playback: files are decoded with symphonia and played through cpal.
//!
//! Three threads cooperate, as in Winamp's input plugin -> DSP -> output plugin chain:
//!
//! - The UI thread owns a [`Player`] and sends it commands: open, seek, stop, pause.
//! - A decoder thread (`decode`) reads the file, converts it to stereo `f32` at the device's
//!   sample rate (`resample`) and keeps a lock-free ring buffer about half a second full.
//! - The device callback (`output`) empties the ring, runs the equalizer (`eq`), volume and
//!   balance, and copies what it plays into a second ring for the visualiser.
//!
//! The callback must never block or allocate, so all it shares with the other threads is atomics
//! and ring buffers. Every command the UI sends gets a new *generation* number. Before the
//! decoder queues audio for a generation, it asks the callback to drop whatever is still queued
//! and waits until the callback confirms; that handshake keeps a seek or a new track from ever
//! playing stale samples, and lets the position and "finished" reports name the generation they
//! belong to.
//!
//! [`Scanner`] reads titles and lengths for the playlist on its own thread.

mod decode;
mod eq;
mod output;
mod resample;
mod scan;
mod source;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{Context, Result};

pub use eq::{BAND_HZ, BANDS, EqSettings, RANGE_DB as EQ_RANGE_DB};
pub use scan::Scanner;

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
pub const FORMATS: [Format; 4] = [
    Format {
        name: "MP3 audio",
        extensions: &["mp3"],
    },
    Format {
        name: "WAV audio",
        extensions: &["wav", "wave"],
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

/// Something the audio engine reports back to the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The track opened and its audio is on its way to the device.
    Started(TrackInfo),
    /// The last sample of the track has been played.
    Finished,
    /// The track could not be opened; the message says why.
    Failed(String),
}

/// Instructions for the decoder thread. Each carries the generation it starts.
#[derive(Debug)]
enum Command {
    Open { path: PathBuf, generation: u64 },
    Seek { seconds: f64, generation: u64 },
    Stop { generation: u64 },
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
    // `position` there and confirms by storing the generation in `playing_generation` (Release).
    flush_generation: AtomicU64,
    flush_position: AtomicU64,
    playing_generation: AtomicU64,
    /// Device frames played so far in `playing_generation`, counted from its start position.
    position: AtomicU64,

    /// The decoder has queued the last sample of this generation (Release).
    ended_generation: AtomicU64,
    /// The callback has played the last sample of this generation (Release).
    finished_generation: AtomicU64,
    /// Bitrate of the audio being decoded, in kbit/s.
    kbps: AtomicU32,
}

/// The UI thread's handle on audio playback: one output device, one decoder thread.
///
/// Dropping it closes the device stream and lets the decoder thread exit.
pub struct Player {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    /// Decoder reports, tagged with the generation of the `Open` they belong to.
    events: Receiver<(u64, Event)>,
    vis: rtrb::Consumer<f32>,
    rate: u32,
    /// The generation of the last command sent.
    generation: u64,
    /// The generation of the last `Open`; older events are stale.
    track_generation: u64,
    /// Where playback stands once the decoder catches up with the last command, in seconds.
    target: f64,
    /// The last generation whose end has been reported.
    finished_reported: u64,
    /// The last equalizer settings sent, so unchanged ones are not sent again.
    eq: Option<EqSettings>,
    /// Keeps the device running; the callback stops when it is dropped.
    _stream: cpal::Stream,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("rate", &self.rate)
            .field("generation", &self.generation)
            .field("track_generation", &self.track_generation)
            .finish_non_exhaustive()
    }
}

impl Player {
    /// Open the default output device and start the decoder thread.
    ///
    /// # Errors
    /// Fails if there is no output device, if it cannot be opened in any supported sample
    /// format, or if the decoder thread cannot be spawned.
    pub fn new() -> Result<Player> {
        let shared = Arc::new(Shared::default());
        let out = output::open(Arc::clone(&shared))?;
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        decode::spawn(
            Arc::clone(&shared),
            out.queue,
            out.rate,
            command_rx,
            event_tx,
        )?;
        log::info!("audio output at {} Hz", out.rate);
        Ok(Player {
            shared,
            commands,
            events,
            vis: out.vis,
            rate: out.rate,
            generation: 0,
            track_generation: 0,
            target: 0.0,
            finished_reported: 0,
            eq: None,
            _stream: out.stream,
        })
    }

    /// The device's sample rate, which is also the rate of the visualiser samples.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn send(&mut self, make: impl FnOnce(u64) -> Command, target: f64) {
        self.generation += 1;
        self.target = target;
        if self.commands.send(make(self.generation)).is_err() {
            log::error!("the audio decoder thread has stopped");
        }
    }

    /// Start playing `path` from the beginning.
    pub fn open(&mut self, path: &Path) {
        self.shared.paused.store(false, Ordering::Relaxed);
        let path = path.to_path_buf();
        self.send(|generation| Command::Open { path, generation }, 0.0);
        self.track_generation = self.generation;
    }

    /// Jump to `seconds` into the current track.
    pub fn seek(&mut self, seconds: f64) {
        self.send(
            |generation| Command::Seek {
                seconds,
                generation,
            },
            seconds,
        );
    }

    /// Stop and close the current track.
    pub fn stop(&mut self) {
        self.send(|generation| Command::Stop { generation }, 0.0);
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
                return Some(event);
            }
        }
        let finished = self.shared.finished_generation.load(Ordering::Acquire);
        if finished == self.generation && self.finished_reported != self.generation {
            self.finished_reported = self.generation;
            return Some(Event::Finished);
        }
        None
    }

    /// Pass every sample played since the last call to `f` (mono, at [`Player::sample_rate`]).
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
        assert!(!is_audio_file(Path::new("skin.wsz")));
        assert!(!is_audio_file(Path::new("noext")));
    }

    #[test]
    fn folders_expand_to_sorted_audio_files_and_files_pass_through() {
        let dir = std::env::temp_dir().join(format!("winamp-audio-files-{}", std::process::id()));
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
            let shared = Arc::new(Shared::default());
            shared.volume.store(1.0);
            let (queue, queue_rx) = rtrb::RingBuffer::new(RATE as usize);
            let (vis_tx, _vis) = rtrb::RingBuffer::new(16);
            let mut mixer = output::Mixer::new(Arc::clone(&shared), queue_rx, vis_tx, RATE);
            let (commands, command_rx) = mpsc::channel();
            let (event_tx, events) = mpsc::channel();
            decode::spawn(Arc::clone(&shared), queue, RATE, command_rx, event_tx).unwrap();
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
                path: path.clone(),
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
                path: path.clone(),
                generation: 1,
            })
            .unwrap();
        assert!(matches!(rig.event(), (1, Event::Started(_))));
        rig.commands
            .send(Command::Seek {
                seconds: 1.5,
                generation: 2,
            })
            .unwrap();
        rig.wait_finished(2);
        assert_eq!(rig.shared.finished_generation.load(Ordering::Acquire), 2);
        assert_eq!(rig.position(), 2 * u64::from(RATE));
        // Stopping, then opening a file that does not exist.
        rig.commands.send(Command::Stop { generation: 3 }).unwrap();
        rig.commands
            .send(Command::Open {
                path: PathBuf::from("/nonexistent.mp3"),
                generation: 4,
            })
            .unwrap();
        assert!(matches!(rig.event(), (4, Event::Failed(_))));
        let _ = rig.finish();
        std::fs::remove_file(&path).unwrap();
    }
}
