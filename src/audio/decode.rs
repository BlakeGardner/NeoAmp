//! The decoder thread: turns [`Command`]s into stereo audio at the device rate and keeps the
//! ring buffer to the device callback topped up.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::resample::Resampler;
use super::source::Source;
use super::{Command, Event, Shared};

/// How long to wait for room in a full ring before checking for commands again.
const POLL: Duration = Duration::from_millis(10);
/// How long to wait for the device callback to confirm a flush. It runs every few milliseconds;
/// silence this long means the device has gone away.
const FLUSH_TIMEOUT: Duration = Duration::from_millis(500);

/// Start the decoder thread. It exits when `commands` is disconnected (the player was dropped).
pub(super) fn spawn(
    shared: Arc<Shared>,
    queue: rtrb::Producer<f32>,
    rate: u32,
    commands: Receiver<Command>,
    events: Sender<(u64, Event)>,
) -> Result<()> {
    let worker = Worker {
        shared,
        queue,
        events,
        rate,
        generation: 0,
        track: None,
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
    source: Source,
    /// `None` when the file is already at the device rate.
    resampler: Option<Resampler>,
    /// The file has been read to the end (its tail may still be in `pending`).
    ended: bool,
    /// `ended_generation` has been published for this generation.
    end_published: bool,
}

struct Worker {
    shared: Arc<Shared>,
    queue: rtrb::Producer<f32>,
    events: Sender<(u64, Event)>,
    /// The device's sample rate.
    rate: u32,
    /// The generation of the last command handled.
    generation: u64,
    track: Option<Track>,
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
            } else if self.is_blocked() {
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

    /// Audio is waiting but the ring has no room for it.
    fn is_blocked(&self) -> bool {
        self.pending_at < self.pending.len() && self.queue.slots() < 2
    }

    fn send(&self, generation: u64, event: Event) {
        // The receiver only goes away with the player, which also ends this thread.
        let _ = self.events.send((generation, event));
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Open { path, generation } => {
                self.generation = generation;
                self.track = None;
                self.flush(0);
                match self.open(&path) {
                    Ok(track) => {
                        let info = track.source.info.clone();
                        self.shared
                            .kbps
                            .store(info.kbps.unwrap_or(0), Ordering::Relaxed);
                        self.track = Some(track);
                        self.send(generation, Event::Started(info));
                    }
                    Err(e) => self.send(generation, Event::Failed(format!("{e:#}"))),
                }
            }
            Command::Seek {
                seconds,
                generation,
            } => {
                self.generation = generation;
                let Some(track) = self.track.as_mut() else {
                    self.flush(0);
                    return;
                };
                match track.source.seek(seconds) {
                    Ok(at) => {
                        if let Some(r) = track.resampler.as_mut() {
                            r.reset();
                        }
                        track.ended = false;
                        track.end_published = false;
                        // Positive and far below 2^63, so the cast cannot truncate.
                        self.flush((at * f64::from(self.rate)).round() as u64);
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
            Command::Stop { generation } => {
                self.generation = generation;
                self.track = None;
                self.flush(0);
            }
        }
    }

    fn open(&self, path: &Path) -> Result<Track> {
        let source = Source::open(path)?;
        let resampler = (source.rate != self.rate)
            .then(|| Resampler::new(source.rate, self.rate))
            .transpose()?;
        Ok(Track {
            source,
            resampler,
            ended: false,
            end_published: false,
        })
    }

    /// Have the callback drop everything queued and count the new generation's position from
    /// `position` (device frames). Returns once the callback has confirmed, so nothing queued
    /// afterwards can be thrown away.
    fn flush(&mut self, position: u64) {
        self.pending.clear();
        self.pending_at = 0;
        self.shared
            .flush_position
            .store(position, Ordering::Relaxed);
        self.shared
            .flush_generation
            .store(self.generation, Ordering::Release);
        let deadline = Instant::now() + FLUSH_TIMEOUT;
        while self.shared.playing_generation.load(Ordering::Acquire) != self.generation {
            if Instant::now() >= deadline {
                log::warn!("the audio device stopped responding");
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Queue what is pending, then decode the next packet if the ring has room for more.
    fn step(&mut self) {
        if self.push_pending() {
            self.decode_next();
            self.push_pending();
        }
        self.publish_end();
    }

    fn decode_next(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if track.ended {
            return;
        }
        self.packet.clear();
        let more = match track.source.read(&mut self.packet) {
            Ok(more) => more,
            Err(e) => {
                // A broken file mid-way: play what was decoded and move on.
                log::warn!("decoding stopped: {e:#}");
                false
            }
        };
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
        if !more {
            track.ended = true;
        }
        if let Some(kbps) = track.source.take_kbps() {
            self.shared.kbps.store(kbps, Ordering::Relaxed);
        }
    }

    /// Move as much pending audio into the ring as fits. Returns true once nothing is pending.
    fn push_pending(&mut self) -> bool {
        let waiting = &self.pending[self.pending_at..];
        // Whole stereo frames only, so the callback never sees half a frame.
        let n = self.queue.slots().min(waiting.len()) & !1;
        if n > 0 {
            let (pushed, _) = self.queue.push_partial_slice(&waiting[..n]);
            self.pending_at += pushed.len();
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
