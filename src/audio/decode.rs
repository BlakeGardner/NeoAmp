//! The decoder thread: turns [`Command`]s into stereo audio at the device rate and keeps the
//! ring buffer to the device callback topped up.
//!
//! Gapless playback: once a file has been decoded to its end, the file preloaded to follow it
//! is opened and its audio queued straight after, behind a [`Mark`] that tells the callback
//! where it begins. The finished file stays open until the next one starts, so that a seek or
//! a new device meant for it, while the UI has not yet seen the device reach the next file,
//! goes back to it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use symphonia::core::formats::probe::Hint;

use super::output::{Mark, Queue};
use super::resample::Resampler;
use super::source::{Read, Source};
use super::stream::{self, Next, PipeReader, SegmentInfo, StreamInput};
use super::{Command, Event, FileDetails, Location, Shared, Station, StreamStatus};

/// How long to wait for room in a full ring before checking for commands again.
const POLL: Duration = Duration::from_millis(10);
/// How long to wait for the device callback to confirm a flush. It runs every few milliseconds;
/// silence this long means the device has gone away.
const FLUSH_TIMEOUT: Duration = Duration::from_millis(500);

/// Start the decoder thread, filling `queue` at `rate`. It exits when `commands` is
/// disconnected (the player was dropped). Streams connect with `network`'s settings.
pub(super) fn spawn(
    shared: Arc<Shared>,
    queue: Queue,
    rate: u32,
    commands: Receiver<Command>,
    events: Sender<(u64, Event)>,
    network: stream::Settings,
) -> Result<()> {
    let worker = Worker {
        shared,
        queue,
        events,
        rate,
        network,
        generation: 0,
        track: None,
        previous: None,
        upcoming: None,
        upcoming_sent: false,
        written: 0,
        flushed: 0,
        mark_pending: false,
        waiting: false,
        pending: Vec::new(),
        pending_at: 0,
        packet: Vec::new(),
    };
    thread::Builder::new()
        .name("audio decoder".into())
        .spawn(move || worker.run(&commands))
        .context("starting the audio decoder thread")?;
    Ok(())
}

/// The track being decoded.
struct Track {
    /// The generation of the `Open` or preload that started it, which its events carry.
    generation: u64,
    /// The file, for a file.
    path: Option<PathBuf>,
    /// `None` while a stream connects and buffers, until its audio has been recognised, and
    /// between one connection's audio and the next's.
    source: Option<Source>,
    /// `None` when the audio is already at the device rate.
    resampler: Option<Resampler>,
    /// The file has been read to the end (its tail may still be in `pending`).
    ended: bool,
    /// `ended_generation` has been published for this generation.
    end_published: bool,
    /// The network side of an internet stream.
    stream: Option<StreamInput>,
}

impl Track {
    fn new(
        generation: u64,
        source: Option<Source>,
        resampler: Option<Resampler>,
        stream: Option<StreamInput>,
    ) -> Track {
        Track {
            generation,
            path: None,
            source,
            resampler,
            ended: false,
            end_published: false,
            stream,
        }
    }
}

/// The file to play straight after the current track.
struct Upcoming {
    path: PathBuf,
    generation: u64,
}

/// Tell the UI `event` about the track started by `generation`.
fn send(events: &Sender<(u64, Event)>, generation: u64, event: Event) {
    // The receiver only goes away with the player, which also ends this thread.
    let _ = events.send((generation, event));
}

/// A resampler from `from` Hz to the device's `to`, unless they are the same.
fn resampler(from: u32, to: u32) -> Result<Option<Resampler>> {
    (from != to).then(|| Resampler::new(from, to)).transpose()
}

struct Worker {
    shared: Arc<Shared>,
    queue: Queue,
    events: Sender<(u64, Event)>,
    /// The device's sample rate.
    rate: u32,
    network: stream::Settings,
    /// The generation of the last command handled, or of the preloaded track since started.
    generation: u64,
    track: Option<Track>,
    /// The track decoded before `track`, when `track` was preloaded: kept until the next one
    /// starts, in case a seek or a new device is meant for it.
    previous: Option<Track>,
    /// The file to play when `track` ends.
    upcoming: Option<Upcoming>,
    /// A preload has been sent since `track` was started from one.
    upcoming_sent: bool,
    /// Frames pushed into the queue since the last flush, which is what marks count in.
    written: u64,
    /// The generation of the last flush asked of the callback. Until the callback has carried
    /// it out nothing is queued, so nothing queued can be thrown away by it.
    flushed: u64,
    /// A mark has been queued that the callback has not reached yet.
    mark_pending: bool,
    /// A stream is connecting or buffering: check back on it shortly.
    waiting: bool,
    /// Audio at the device rate waiting for room in the ring, read from `pending_at`.
    pending: Vec<f32>,
    pending_at: usize,
    /// Scratch: one decoded packet as stereo at the file's rate.
    packet: Vec<f32>,
}

impl Worker {
    fn run(mut self, commands: &Receiver<Command>) {
        loop {
            let command = if self.is_idle() {
                match commands.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                }
            } else if self.is_blocked() || self.waiting {
                match commands.recv_timeout(POLL) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match commands.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            };
            match command {
                Some(c) => self.handle(c),
                None => self.step(),
            }
        }
    }

    /// Nothing to decode and nothing left to queue.
    fn is_idle(&self) -> bool {
        self.track
            .as_ref()
            .is_none_or(|t| t.ended && t.end_published)
    }

    /// Audio is waiting but the ring has no room for it, or the callback has yet to carry out
    /// the last flush.
    fn is_blocked(&self) -> bool {
        self.pending_at < self.pending.len() && (self.queue.audio.slots() < 2 || !self.flushed())
    }

    /// The callback has carried out the last flush, or there is no device to wait for (the
    /// next device's callback starts from the latest flush).
    fn flushed(&self) -> bool {
        !self.shared.output_open.load(Ordering::Acquire)
            || self.shared.flushed.load(Ordering::Acquire) == self.flushed
    }

    fn send(&self, generation: u64, event: Event) {
        send(&self.events, generation, event);
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Open {
                location,
                generation,
            } => {
                self.generation = generation;
                self.track = None;
                self.previous = None;
                self.upcoming = None;
                self.waiting = false;
                self.flush(0);
                self.shared.kbps.store(0, Ordering::Relaxed);
                let opened = match location {
                    Location::File(path) => self.open(path, generation),
                    Location::Url(url) => StreamInput::start(url, self.network)
                        .map(|input| Track::new(generation, None, None, Some(input))),
                };
                match opened {
                    Ok(track) => {
                        let started = track.source.as_ref().map(|s| s.info.clone());
                        self.track = Some(track);
                        match started {
                            Some(info) => {
                                self.shared
                                    .kbps
                                    .store(info.kbps.unwrap_or(0), Ordering::Relaxed);
                                self.send(generation, Event::Started(info));
                            }
                            None => {
                                self.send(generation, Event::Status(StreamStatus::Connecting));
                            }
                        }
                    }
                    Err(e) => self.send(generation, Event::Failed(format!("{e:#}"))),
                }
            }
            Command::Seek {
                seconds,
                generation,
                track,
            } => {
                self.generation = generation;
                self.settle(track);
                self.seek(seconds);
            }
            Command::Stop { generation } => {
                self.generation = generation;
                self.track = None;
                self.previous = None;
                self.upcoming = None;
                self.waiting = false;
                self.flush(0);
            }
            Command::Preload { path, generation } => {
                self.upcoming = path.map(|path| Upcoming { path, generation });
                self.upcoming_sent = true;
            }
            Command::Output {
                queue,
                rate,
                seconds,
                generation,
                track,
            } => {
                self.generation = generation;
                self.settle(track);
                self.queue = queue;
                self.rate = rate;
                if let Some(track) = self.track.as_mut()
                    && let Some(source) = track.source.as_ref()
                {
                    match resampler(source.rate, rate) {
                        Ok(r) => track.resampler = r,
                        Err(e) => {
                            let generation = track.generation;
                            self.track = None;
                            self.send(generation, Event::Failed(format!("{e:#}")));
                        }
                    }
                }
                self.seek(seconds);
            }
        }
    }

    /// Go on from `seconds` into the current track, dropping what is queued. A stream cannot
    /// seek, and goes on from where it is.
    fn seek(&mut self, seconds: f64) {
        let rate = f64::from(self.rate);
        let Some(track) = self.track.as_mut() else {
            self.flush(0);
            return;
        };
        let Some(source) = track.source.as_mut().filter(|_| track.stream.is_none()) else {
            // Positive and far below 2^63, so the cast cannot truncate.
            self.flush((seconds.max(0.0) * rate).round() as u64);
            return;
        };
        match source.seek(seconds) {
            Ok(at) => {
                if let Some(r) = track.resampler.as_mut() {
                    r.reset();
                }
                track.ended = false;
                track.end_published = false;
                // Positive and far below 2^63, so the cast cannot truncate.
                self.flush((at * rate).round() as u64);
            }
            Err(e) => {
                // Usually a seek past the end. Finish the track from where it is.
                log::warn!("seek to {seconds:.1}s failed: {e:#}");
                track.ended = true;
                track.end_published = false;
                let here = self.shared.position.load(Ordering::Relaxed);
                self.flush(here);
                self.publish_end();
            }
        }
    }

    /// Note when the callback reaches the mark queued before the current track.
    fn note_crossing(&mut self) {
        if self.mark_pending
            && self.track.as_ref().is_some_and(|t| {
                self.shared.playing_generation.load(Ordering::Acquire) == t.generation
            })
        {
            self.mark_pending = false;
        }
    }

    /// Before a seek or a new device, which drop what is queued: go on with `track`, the
    /// track the UI takes for the current one. When the UI has not seen the device reach the
    /// preloaded track, that is the track before it, even if the device has just reached it:
    /// the flush throws that away. The preloaded file is then preloaded again (unless the UI
    /// has named another since).
    fn settle(&mut self, track: u64) {
        let current = self.track.as_ref().map(|t| t.generation);
        if current == Some(track) || self.previous.as_ref().is_none_or(|p| p.generation != track) {
            return;
        }
        let Some(previous) = self.previous.take() else {
            return;
        };
        let ahead = self.track.replace(previous);
        if !self.upcoming_sent
            && let Some(Track {
                path: Some(path),
                generation,
                ..
            }) = ahead
        {
            self.upcoming = Some(Upcoming { path, generation });
        }
    }

    fn open(&self, path: PathBuf, generation: u64) -> Result<Track> {
        let source = Source::open(&path)?;
        let resampler = resampler(source.rate, self.rate)?;
        let mut track = Track::new(generation, Some(source), resampler, None);
        track.path = Some(path);
        Ok(track)
    }

    /// Have the callback drop everything queued and count the new generation's position from
    /// `position` (device frames). Waits a moment for the callback to confirm; nothing is
    /// queued until it has (see [`Worker::flushed`]), so nothing queued afterwards can be
    /// thrown away. A device that does not answer holds up the queue, not this thread.
    fn flush(&mut self, position: u64) {
        self.pending.clear();
        self.pending_at = 0;
        self.written = 0;
        self.mark_pending = false;
        self.flushed = self.generation;
        self.shared
            .flush_position
            .store(position, Ordering::Relaxed);
        self.shared
            .flush_generation
            .store(self.generation, Ordering::Release);
        let deadline = Instant::now() + FLUSH_TIMEOUT;
        while !self.flushed() {
            if Instant::now() >= deadline {
                log::warn!("the audio device stopped responding");
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Queue what is pending, then decode the next packet if the ring has room for more (and,
    /// for a stream, there is enough to decode). At the end of a file, go on into the next
    /// one if there is one.
    fn step(&mut self) {
        self.waiting = false;
        self.note_crossing();
        if self.push_pending() {
            if self.ready_to_decode() {
                self.decode_next();
                self.push_pending();
            } else {
                self.waiting = true;
            }
        }
        if !self.advance() {
            self.publish_end();
        }
    }

    /// Once the current file is decoded and queued to its end, start the preloaded one behind
    /// a mark, and return true. Returns false when there is none, or it cannot be opened (it
    /// is then left to the UI to move on, and to report it).
    fn advance(&mut self) -> bool {
        let Some(track) = self.track.as_ref() else {
            return false;
        };
        let finished = track.ended && !track.end_published && track.stream.is_none();
        // One mark in the queue at a time: a preloaded file shorter than the queue ends the
        // usual way.
        if !finished || !self.pending.is_empty() || self.mark_pending || !self.flushed() {
            return false;
        }
        let Some(Upcoming { path, generation }) = self.upcoming.take() else {
            return false;
        };
        let next = match self.open(path, generation) {
            Ok(next) => next,
            Err(e) => {
                log::debug!("cannot open the next file ahead of time: {e:#}");
                return false;
            }
        };
        let mark = Mark {
            at: self.written,
            generation,
        };
        if self.queue.marks.push(mark).is_err() {
            return false;
        }
        let info = next.source.as_ref().map(|s| s.info.clone());
        self.previous = self.track.replace(next);
        self.generation = generation;
        self.upcoming_sent = false;
        self.mark_pending = true;
        if let Some(info) = info {
            self.send(generation, Event::Started(info));
        }
        true
    }

    /// The device has less than an eighth of its queue left to play.
    fn queue_low(&self) -> bool {
        let capacity = self.queue.audio.buffer().capacity();
        capacity - self.queue.audio.slots() < capacity / 8
    }

    /// Whether to decode a packet now. Files always may; a stream is told how the network is
    /// doing, and its news is passed on to the UI.
    fn ready_to_decode(&mut self) -> bool {
        let queue_low = self.queue_low();
        let Some(track) = self.track.as_mut() else {
            return false;
        };
        let generation = track.generation;
        let Some(input) = track.stream.as_mut() else {
            return true;
        };
        for title in input.due_titles() {
            send(&self.events, generation, Event::StreamTitle(title));
        }
        let next = input.next(track.source.is_some(), queue_low);
        if let Some(status) = input.take_status() {
            send(&self.events, generation, Event::Status(status));
        }
        match next {
            Next::Decode => true,
            Next::Wait => false,
            Next::Open(info) => {
                self.open_segment(info);
                false
            }
            Next::End => {
                track.ended = true;
                false
            }
            Next::Fail(why) => {
                send(&self.events, generation, Event::Failed(why));
                self.track = None;
                false
            }
        }
    }

    /// Recognise the audio of the stream's segment now being read, and start playing it.
    fn open_segment(&mut self, info: SegmentInfo) {
        let Some(track) = self.track.as_ref() else {
            return;
        };
        let Some(input) = track.stream.as_ref() else {
            return;
        };
        let reader = PipeReader::new(
            Arc::clone(input.pipe()),
            Arc::clone(&self.shared),
            track.generation,
        );
        let mut hint = Hint::new();
        if let Some(mime) = &info.mime {
            hint.mime_type(mime);
        }
        if let Some(ext) = &info.extension {
            hint.with_extension(ext);
        }
        let opened = Source::open_stream(Box::new(reader), &hint);
        self.start_source(opened, info.station);
    }

    /// Play a stream's audio, recognised afresh after connecting (again), or after a chained
    /// stream started its next song. Tell the UI what it is, or why it cannot be played.
    fn start_source(&mut self, opened: Result<(Source, FileDetails)>, mut station: Station) {
        let opened = opened.and_then(|(source, details)| {
            let resampler = resampler(source.rate, self.rate)?;
            Ok((source, details, resampler))
        });
        let Some(track) = self.track.as_mut() else {
            return;
        };
        let generation = track.generation;
        let (source, details, resampler) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                send(&self.events, generation, Event::Failed(format!("{e:#}")));
                self.track = None;
                return;
            }
        };
        station.format = details.format;
        station.codec = details.codec;
        let mut info = source.info.clone();
        info.kbps = station.kbps.or(info.kbps);
        self.shared
            .kbps
            .store(info.kbps.unwrap_or(0), Ordering::Relaxed);
        track.source = Some(source);
        track.resampler = resampler;
        send(&self.events, generation, Event::Started(info));
        send(
            &self.events,
            generation,
            Event::Station(Box::new(station.clone())),
        );
        // A chained Ogg stream names each song in its own tags.
        if let Some(title) = details.tags.playlist_title() {
            send(&self.events, generation, Event::StreamTitle(title));
        }
        if let Some(input) = track.stream.as_mut() {
            input.playing(station);
            if let Some(status) = input.take_status() {
                send(&self.events, generation, Event::Status(status));
            }
        }
    }

    fn decode_next(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if track.ended {
            return;
        }
        let Some(source) = track.source.as_mut() else {
            return;
        };
        self.packet.clear();
        let read = match source.read(&mut self.packet) {
            Ok(read) => read,
            Err(e) => {
                // A broken file mid-way: play what was decoded and move on.
                log::warn!("decoding stopped: {e:#}");
                Read::End
            }
        };
        let more = read == Read::Audio;
        let converted = match track.resampler.as_mut() {
            Some(r) => {
                let result = r.process(&self.packet, &mut self.pending);
                if more {
                    result
                } else {
                    result.and_then(|()| r.finish(&mut self.pending))
                }
            }
            None => {
                self.pending.extend_from_slice(&self.packet);
                Ok(())
            }
        };
        if let Err(e) = converted {
            log::warn!("resampling failed: {e:#}");
            track.ended = true;
        }
        if let Some(kbps) = source.take_kbps() {
            self.shared.kbps.store(kbps, Ordering::Relaxed);
        }
        if more {
            return;
        }
        if track.stream.is_none() {
            track.ended = true;
            return;
        }
        // A stream: this connection's audio is over. The next is recognised afresh once it
        // has buffered; a chained stream's next song is decoded at once.
        track.resampler = None;
        let Some(source) = track.source.take() else {
            return;
        };
        if read == Read::NewStream {
            let station = track
                .stream
                .as_ref()
                .map(|s| s.station.clone())
                .unwrap_or_default();
            let next = source.next_stream();
            self.start_source(next, station);
        }
    }

    /// Move as much pending audio into the ring as fits. Returns true once nothing is pending.
    fn push_pending(&mut self) -> bool {
        if !self.flushed() {
            return self.pending_at >= self.pending.len();
        }
        let waiting = &self.pending[self.pending_at..];
        // Whole stereo frames only, so the callback never sees half a frame.
        let n = self.queue.audio.slots().min(waiting.len()) & !1;
        if n > 0 {
            let (pushed, _) = self.queue.audio.push_partial_slice(&waiting[..n]);
            self.pending_at += pushed.len();
            // usize always fits in u64 on supported targets.
            self.written += pushed.len() as u64 / 2;
        }
        if self.pending_at < self.pending.len() {
            return false;
        }
        self.pending.clear();
        self.pending_at = 0;
        true
    }

    /// Once the whole file is queued, tell the callback this generation has no more audio.
    fn publish_end(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if track.ended && !track.end_published && self.pending.is_empty() {
            track.end_published = true;
            self.shared
                .ended_generation
                .store(self.generation, Ordering::Release);
        }
    }
}
