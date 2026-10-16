//! The device side: a cpal output stream whose callback runs the [`Mixer`].
//!
//! The callback runs on the audio system's real-time thread. Nothing here may block, lock or
//! allocate once the stream is running: buffers are sized up front and everything shared is an
//! atomic or a ring buffer.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{
    ErrorKind, FromSample, I24, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig, U24,
};

use super::Shared;
use super::eq::Equalizer;

/// The most frames the mixer handles per call; device buffers are split to fit.
const MAX_FRAMES: usize = 4096;
/// Mono samples kept for the visualiser between UI frames (a quarter second at 48 kHz). If the
/// UI stalls, newer samples are dropped until it catches up.
const VIS_CAPACITY: usize = 12_000;
/// Track boundaries the queue can hold at once. Its half second of audio rarely spans more
/// than two tracks.
const MARKS: usize = 16;

/// Where one track ends and the next begins in the queue, for gapless playback: after `at`
/// frames (counted from the last flush), the audio belongs to `generation`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Mark {
    pub at: u64,
    pub generation: u64,
}

/// The decoder's end of the queue to the device.
#[derive(Debug)]
pub(super) struct Queue {
    /// Interleaved stereo at the device's rate.
    pub audio: rtrb::Producer<f32>,
    /// Where the tracks in `audio` begin.
    pub marks: rtrb::Producer<Mark>,
}

impl Queue {
    /// A queue that nothing plays, for the decoder to fill while no device is open.
    pub(super) fn unplayed(rate: u32) -> Queue {
        let (audio, _) = rtrb::RingBuffer::new(capacity(rate));
        let (marks, _) = rtrb::RingBuffer::new(MARKS);
        Queue { audio, marks }
    }
}

/// Half a second of stereo at `rate`: enough to ride out a slow disk, little enough to stay
/// small.
fn capacity(rate: u32) -> usize {
    usize::try_from(rate).unwrap_or(48_000) & !1
}

/// A running output stream.
pub(super) struct Output {
    /// Keeps the device running; the callback stops when it is dropped.
    _stream: cpal::Stream,
    pub rate: u32,
    /// The device's name, for the log.
    pub name: String,
    /// Set when the device has gone away, or the stream can no longer play and has to be
    /// opened again.
    pub lost: Arc<AtomicBool>,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("rate", &self.rate)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A newly opened output, and the ring buffers that feed and tap it.
pub(super) struct Opened {
    pub output: Output,
    pub queue: Queue,
    /// Mono copies of what is played, for the visualiser.
    pub vis: rtrb::Consumer<f32>,
}

/// Open `device` in its preferred format and start it.
pub(super) fn open(shared: Arc<Shared>, device: &cpal::Device) -> Result<Opened> {
    let name = device
        .description()
        .map_or_else(|_| device.to_string(), |d| d.name().to_owned());
    let (format, config) = choose_config(device)?;
    let rate = config.sample_rate;
    let channels = usize::from(config.channels);
    log::debug!("audio output {name}: {format} x {channels} channels at {rate} Hz");

    let (audio, audio_rx) = rtrb::RingBuffer::new(capacity(rate));
    let (marks, marks_rx) = rtrb::RingBuffer::new(MARKS);
    let (vis_tx, vis) = rtrb::RingBuffer::new(VIS_CAPACITY);
    let mixer = Mixer::new(shared, audio_rx, marks_rx, vis_tx, rate);
    let lost = Arc::new(AtomicBool::new(false));
    let on_error = error_handler(Arc::clone(&lost));

    let stream = match format {
        SampleFormat::F32 => build::<f32>(device, config, mixer, channels, on_error),
        SampleFormat::F64 => build::<f64>(device, config, mixer, channels, on_error),
        SampleFormat::I16 => build::<i16>(device, config, mixer, channels, on_error),
        SampleFormat::I24 => build::<I24>(device, config, mixer, channels, on_error),
        SampleFormat::I32 => build::<i32>(device, config, mixer, channels, on_error),
        SampleFormat::I64 => build::<i64>(device, config, mixer, channels, on_error),
        SampleFormat::I8 => build::<i8>(device, config, mixer, channels, on_error),
        SampleFormat::U16 => build::<u16>(device, config, mixer, channels, on_error),
        SampleFormat::U24 => build::<U24>(device, config, mixer, channels, on_error),
        SampleFormat::U32 => build::<u32>(device, config, mixer, channels, on_error),
        SampleFormat::U64 => build::<u64>(device, config, mixer, channels, on_error),
        SampleFormat::U8 => build::<u8>(device, config, mixer, channels, on_error),
        other => bail!("unsupported output sample format {other}"),
    }?;
    stream.play().context("starting the audio output")?;
    Ok(Opened {
        output: Output {
            _stream: stream,
            rate,
            name,
            lost,
        },
        queue: Queue { audio, marks },
        vis,
    })
}

/// The device's preferred format, except that 8- and 16-bit integers give way to 32-bit float
/// at the same rate when the device takes it (a sound server's outputs usually prefer 16 bits),
/// so the equalizer's and volume's output is not rounded to 16 bits.
fn choose_config(device: &cpal::Device) -> Result<(SampleFormat, StreamConfig)> {
    let supported = device
        .default_output_config()
        .context("querying the audio output device")?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let narrow = (format.is_int() || format.is_uint()) && format.sample_size() < 3;
    let float = narrow
        && device.supported_output_configs().is_ok_and(|mut ranges| {
            ranges.any(|r| {
                r.sample_format() == SampleFormat::F32
                    && r.channels() == config.channels
                    && r.contains_rate(config.sample_rate)
            })
        });
    Ok(if float {
        (SampleFormat::F32, config)
    } else {
        (format, config)
    })
}

/// What to do when the stream reports trouble. Runs on one of cpal's threads, sometimes the
/// real-time one, so it only logs what is rare.
fn error_handler(lost: Arc<AtomicBool>) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e: cpal::Error| match e.kind() {
        ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated => {
            log::warn!("audio output: {e}");
            lost.store(true, Ordering::Release);
        }
        // The system moved the stream to its new default device by itself.
        ErrorKind::DeviceChanged => log::info!("audio output moved to the new default device"),
        // A late callback: the click has been heard already, and logging would allocate.
        ErrorKind::Xrun => {}
        _ => log::error!("audio output: {e}"),
    }
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut mixer: Mixer,
    channels: usize,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    // Allocated here, before the stream starts, never in the callback.
    let mut scratch = vec![0.0f32; MAX_FRAMES * channels.max(1)];
    let block = scratch.len();
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &OutputCallbackInfo| {
                for chunk in data.chunks_mut(block) {
                    let buf = &mut scratch[..chunk.len()];
                    mixer.render(buf, channels);
                    for (out, s) in chunk.iter_mut().zip(buf.iter()) {
                        *out = T::from_sample(*s);
                    }
                }
            },
            on_error,
            None,
        )
        .context("opening the audio output stream")?;
    Ok(stream)
}

/// Gains for the left and right channels. The slider is squared so its travel feels even:
/// half way is about -12 dB rather than -6 dB. Balance only ever turns one side down.
fn gains(volume: f32, balance: f32) -> (f32, f32) {
    let v = volume * volume;
    (v * (1.0 - balance.max(0.0)), v * (1.0 + balance.min(0.0)))
}

/// Pulls stereo audio from the decoder's ring and writes it to the device.
pub(super) struct Mixer {
    shared: Arc<Shared>,
    queue: rtrb::Consumer<f32>,
    marks: rtrb::Consumer<Mark>,
    vis: rtrb::Producer<f32>,
    /// The last flush carried out, or asked for before this mixer started.
    flushed: u64,
    /// The generation whose audio is being played: the last flush's, or since then the
    /// generation of the last track boundary passed.
    generation: u64,
    /// Frames taken from the queue since the last flush, which is what marks count in.
    read: u64,
    rate: u32,
    eq: Equalizer,
    eq_version: u32,
    /// The left and right gains the last block ended on. A change of volume or balance glides
    /// to the new gains over the next block rather than jumping, which would click (and step
    /// audibly through a fade-out). `None` before the first block.
    gains: Option<(f32, f32)>,
    /// One block of stereo from the ring.
    stereo: Vec<f32>,
    /// The same block mixed to mono for the visualiser.
    mono: Vec<f32>,
}

impl Mixer {
    pub(super) fn new(
        shared: Arc<Shared>,
        queue: rtrb::Consumer<f32>,
        marks: rtrb::Consumer<Mark>,
        vis: rtrb::Producer<f32>,
        rate: u32,
    ) -> Mixer {
        // A new device's mixer starts where the last one was, with a queue of its own: there
        // is nothing to drop, and the decoder flushes again for the new device.
        let flushed = shared.flush_generation.load(Ordering::Acquire);
        let generation = shared.playing_generation.load(Ordering::Acquire);
        Mixer {
            shared,
            queue,
            marks,
            vis,
            flushed,
            generation,
            read: 0,
            rate,
            eq: Equalizer::default(),
            // Differs from the shared version's start (0), so the first call reads the settings.
            eq_version: u32::MAX,
            gains: None,
            stereo: vec![0.0; MAX_FRAMES * 2],
            mono: vec![0.0; MAX_FRAMES],
        }
    }

    /// Fill `out` (interleaved, `channels` samples per frame) with the next audio.
    pub(super) fn render(&mut self, out: &mut [f32], channels: usize) {
        self.follow_flush();
        self.follow_eq();
        if channels == 0 {
            return;
        }
        if self.shared.paused.load(Ordering::Relaxed) {
            out.fill(0.0);
            return;
        }
        let frames = (out.len() / channels).min(MAX_FRAMES);
        let got = self
            .queue
            .pop_partial_slice(&mut self.stereo[..frames * 2])
            .0
            .len()
            / 2;

        let to = gains(self.shared.volume.load(), self.shared.balance.load());
        let from = self.gains.replace(to).unwrap_or(to);
        // A block is a few thousand frames at most, so the conversions are exact.
        let step = 1.0 / got.max(1) as f32;
        let frames_out = out.chunks_exact_mut(channels);
        let input = self.stereo.chunks_exact(2).take(got);
        for (i, ((frame, lr), mono)) in frames_out.zip(input).zip(self.mono.iter_mut()).enumerate()
        {
            let k = (i + 1) as f32 * step;
            let gl = from.0 + (to.0 - from.0) * k;
            let gr = from.1 + (to.1 - from.1) * k;
            let (l, r) = self.eq.process(lr[0], lr[1]);
            *mono = 0.5 * (l + r);
            let (l, r) = ((l * gl).clamp(-1.0, 1.0), (r * gr).clamp(-1.0, 1.0));
            if let [only] = frame {
                *only = 0.5 * (l + r);
            } else {
                frame[0] = l;
                frame[1] = r;
                frame[2..].fill(0.0);
            }
        }
        out[got * channels..].fill(0.0);
        // If the visualiser is not keeping up, its oldest view of the music is simply missed.
        let _ = self.vis.push_partial_slice(&self.mono[..got]);
        self.count(got);

        if got < frames
            && self.queue.is_empty()
            && self.shared.ended_generation.load(Ordering::Acquire) == self.generation
        {
            self.shared
                .finished_generation
                .store(self.generation, Ordering::Release);
        }
    }

    /// Count `got` frames just played towards the position, moving on to the next track at
    /// each boundary passed: its position starts again from the frames after the boundary.
    fn count(&mut self, got: usize) {
        let start = self.read;
        // usize always fits in u64 on supported targets.
        self.read += got as u64;
        let mut counted = start;
        while let Ok(mark) = self.marks.peek().copied() {
            if mark.at > self.read {
                break;
            }
            let _ = self.marks.pop();
            let at = mark.at.max(counted);
            self.shared
                .position
                .fetch_add(at - counted, Ordering::Relaxed);
            counted = at;
            self.generation = mark.generation;
            self.shared.position.store(0, Ordering::Relaxed);
            self.shared
                .playing_generation
                .store(mark.generation, Ordering::Release);
        }
        self.shared
            .position
            .fetch_add(self.read - counted, Ordering::Relaxed);
    }

    /// Answer the decoder's flush request, if there is a new one.
    fn follow_flush(&mut self) {
        let wanted = self.shared.flush_generation.load(Ordering::Acquire);
        if wanted == self.flushed {
            return;
        }
        let queued = self.queue.slots();
        if let Ok(chunk) = self.queue.read_chunk(queued) {
            chunk.commit_all();
        }
        while self.marks.pop().is_ok() {}
        self.read = 0;
        let start = self.shared.flush_position.load(Ordering::Relaxed);
        self.shared.position.store(start, Ordering::Relaxed);
        self.flushed = wanted;
        self.generation = wanted;
        self.shared
            .playing_generation
            .store(wanted, Ordering::Release);
        self.shared.flushed.store(wanted, Ordering::Release);
    }

    /// Recompute the equalizer filters if the settings changed.
    fn follow_eq(&mut self) {
        let shared = &self.shared.eq;
        let version = shared.version.load(Ordering::Acquire);
        if version == self.eq_version {
            return;
        }
        self.eq_version = version;
        let bands = std::array::from_fn(|i| shared.bands_db[i].load());
        self.eq.configure(
            self.rate,
            shared.enabled.load(Ordering::Relaxed),
            shared.preamp_db.load(),
            &bands,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    struct Rig {
        shared: Arc<Shared>,
        queue: rtrb::Producer<f32>,
        marks: rtrb::Producer<Mark>,
        vis: rtrb::Consumer<f32>,
        mixer: Mixer,
    }

    fn rig() -> Rig {
        let shared = Arc::new(Shared::default());
        shared.volume.store(1.0);
        let (queue, queue_rx) = rtrb::RingBuffer::new(64);
        let (marks, marks_rx) = rtrb::RingBuffer::new(4);
        let (vis_tx, vis) = rtrb::RingBuffer::new(64);
        let mixer = Mixer::new(Arc::clone(&shared), queue_rx, marks_rx, vis_tx, 48_000);
        Rig {
            shared,
            queue,
            marks,
            vis,
            mixer,
        }
    }

    fn mark(rig: &mut Rig, at: u64, generation: u64) {
        rig.marks.push(Mark { at, generation }).unwrap();
    }

    fn playing(rig: &Rig) -> (u64, u64) {
        (
            rig.shared.playing_generation.load(Ordering::Acquire),
            rig.shared.position.load(Ordering::Relaxed),
        )
    }

    fn queue(rig: &mut Rig, samples: &[f32]) {
        rig.queue.push_entire_slice(samples).unwrap();
    }

    fn flush(rig: &mut Rig, generation: u64, position: u64) {
        rig.shared.flush_position.store(position, Ordering::Relaxed);
        rig.shared
            .flush_generation
            .store(generation, Ordering::Release);
    }

    #[test]
    fn plays_queued_stereo_then_silence() {
        let mut rig = rig();
        queue(&mut rig, &[0.5, -0.5, 0.25, -0.25]);
        let mut out = [9.0f32; 8];
        rig.mixer.render(&mut out, 2);
        assert_eq!(out, [0.5, -0.5, 0.25, -0.25, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(rig.shared.position.load(Ordering::Relaxed), 2);
        // The visualiser gets the mono mix.
        assert_eq!(rig.vis.slots(), 2);
        assert_eq!(rig.vis.pop().unwrap(), 0.0);
    }

    #[test]
    fn volume_balance_and_mono_and_surround_devices() {
        let mut rig = rig();
        rig.shared.volume.store(0.5);
        rig.shared.balance.store(0.5);
        queue(&mut rig, &[1.0, 1.0, 1.0, 1.0]);
        let mut out = [0.0f32; 3];
        rig.mixer.render(&mut out, 3);
        // Volume 0.5 squares to 0.25; balance right halves the left side.
        assert_eq!(out, [0.125, 0.25, 0.0]);
        let mut mono = [0.0f32; 1];
        rig.mixer.render(&mut mono, 1);
        assert_eq!(mono, [0.1875]);
    }

    #[test]
    fn a_volume_change_glides_over_the_next_block() {
        let mut rig = rig();
        queue(&mut rig, &[1.0; 2]);
        let mut out = [0.0f32; 2];
        rig.mixer.render(&mut out, 2);
        assert_eq!(out, [1.0, 1.0], "the first block starts at the volume set");
        rig.shared.volume.store(0.0);
        queue(&mut rig, &[1.0; 8]);
        let mut out = [0.0f32; 8];
        rig.mixer.render(&mut out, 2);
        assert_eq!(out, [0.75, 0.75, 0.5, 0.5, 0.25, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn pausing_holds_the_queue() {
        let mut rig = rig();
        queue(&mut rig, &[0.5, 0.5]);
        rig.shared.paused.store(true, Ordering::Relaxed);
        let mut out = [1.0f32; 2];
        rig.mixer.render(&mut out, 2);
        assert_eq!(out, [0.0, 0.0]);
        assert_eq!(rig.queue.slots(), 62, "nothing was consumed");
        rig.shared.paused.store(false, Ordering::Relaxed);
        rig.mixer.render(&mut out, 2);
        assert_eq!(out, [0.5, 0.5]);
    }

    #[test]
    fn a_flush_drops_queued_audio_and_restarts_the_count() {
        let mut rig = rig();
        queue(&mut rig, &[0.5; 16]);
        flush(&mut rig, 7, 4800);
        let mut out = [1.0f32; 4];
        rig.mixer.render(&mut out, 2);
        assert_eq!(out, [0.0; 4], "the stale audio is gone");
        assert_eq!(rig.shared.playing_generation.load(Ordering::Acquire), 7);
        assert_eq!(rig.shared.position.load(Ordering::Relaxed), 4800);
    }

    #[test]
    fn finishing_needs_the_end_of_this_generation_and_an_empty_queue() {
        let mut rig = rig();
        flush(&mut rig, 3, 0);
        let mut out = [0.0f32; 2];
        // The decoder queues audio only once the callback has acknowledged the flush.
        rig.mixer.render(&mut out, 2);
        queue(&mut rig, &[0.1; 4]);
        rig.shared.ended_generation.store(3, Ordering::Release);
        rig.mixer.render(&mut out, 2);
        rig.mixer.render(&mut out, 2);
        assert_eq!(
            rig.shared.finished_generation.load(Ordering::Acquire),
            0,
            "the last frame filled a whole block, so nothing has run dry yet"
        );
        rig.mixer.render(&mut out, 2);
        assert_eq!(rig.shared.finished_generation.load(Ordering::Acquire), 3);
    }

    #[test]
    fn a_track_boundary_inside_a_block_starts_the_next_track_counting_from_zero() {
        let mut rig = rig();
        flush(&mut rig, 3, 100);
        let mut out = [0.0f32; 2];
        rig.mixer.render(&mut out, 2);
        // Three frames of this track, then the next one's.
        queue(&mut rig, &[0.25; 6]);
        mark(&mut rig, 3, 4);
        queue(&mut rig, &[0.5; 4]);
        let mut out = [0.0f32; 8];
        rig.mixer.render(&mut out, 2);
        assert_eq!(
            playing(&rig),
            (4, 1),
            "one frame of the next track has played"
        );
        assert_eq!(out, [0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.5, 0.5]);
        rig.mixer.render(&mut out, 2);
        assert_eq!(playing(&rig), (4, 2));
    }

    #[test]
    fn a_boundary_at_the_end_of_a_block_is_passed_with_it() {
        let mut rig = rig();
        flush(&mut rig, 3, 0);
        let mut out = [0.0f32; 4];
        rig.mixer.render(&mut out, 2);
        queue(&mut rig, &[0.25; 4]);
        mark(&mut rig, 2, 4);
        rig.mixer.render(&mut out, 2);
        assert_eq!(playing(&rig), (4, 0));
    }

    #[test]
    fn the_next_tracks_end_finishes_it_not_the_one_before() {
        let mut rig = rig();
        flush(&mut rig, 3, 0);
        let mut out = [0.0f32; 4];
        rig.mixer.render(&mut out, 2);
        queue(&mut rig, &[0.25; 2]);
        mark(&mut rig, 1, 4);
        queue(&mut rig, &[0.5; 2]);
        rig.shared.ended_generation.store(4, Ordering::Release);
        rig.mixer.render(&mut out, 2);
        assert_eq!(rig.shared.finished_generation.load(Ordering::Acquire), 0);
        rig.mixer.render(&mut out, 2);
        assert_eq!(rig.shared.finished_generation.load(Ordering::Acquire), 4);
    }

    #[test]
    fn a_flush_drops_boundaries_still_queued() {
        let mut rig = rig();
        queue(&mut rig, &[0.5; 8]);
        mark(&mut rig, 2, 9);
        flush(&mut rig, 7, 0);
        let mut out = [0.0f32; 8];
        rig.mixer.render(&mut out, 2);
        queue(&mut rig, &[0.5; 8]);
        rig.mixer.render(&mut out, 2);
        assert_eq!(playing(&rig), (7, 4));
        assert_eq!(rig.shared.flushed.load(Ordering::Acquire), 7);
    }

    #[test]
    fn a_new_devices_mixer_starts_where_the_last_one_was() {
        let mut rig = rig();
        flush(&mut rig, 5, 480);
        let mut out = [0.0f32; 2];
        rig.mixer.render(&mut out, 2);
        rig.shared.position.store(9_000, Ordering::Relaxed);
        // A flush the old device never carried out, as when it went away.
        flush(&mut rig, 6, 100);
        let (_, queue_rx) = rtrb::RingBuffer::new(8);
        let (_, marks_rx) = rtrb::RingBuffer::new(2);
        let (vis_tx, _) = rtrb::RingBuffer::new(8);
        let mut fresh = Mixer::new(Arc::clone(&rig.shared), queue_rx, marks_rx, vis_tx, 44_100);
        fresh.render(&mut out, 2);
        assert_eq!(playing(&rig), (5, 9_000), "nothing reset");
        flush(&mut rig, 7, 200);
        fresh.render(&mut out, 2);
        assert_eq!(playing(&rig), (7, 200));
    }

    #[test]
    fn gains_follow_the_slider_curve() {
        assert_eq!(gains(1.0, 0.0), (1.0, 1.0));
        assert_eq!(gains(0.0, 0.0), (0.0, 0.0));
        assert_eq!(gains(1.0, -1.0), (1.0, 0.0));
        assert_eq!(gains(1.0, 1.0), (0.0, 1.0));
    }
}
