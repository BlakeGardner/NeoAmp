//! Sample rate conversion of the decoded stereo stream to the device's rate.
//!
//! Most music is 44.1 kHz while sound servers usually run at 48 kHz. On Linux PipeWire would
//! convert silently, but Windows' and macOS's shared mixers cannot be relied on to, so the
//! decoder converts everything itself and every platform plays the same samples.

use anyhow::{Context, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

/// Input frames per FFT block.
const CHUNK: usize = 1024;
const CHANNELS: usize = 2;

/// A fixed-ratio stereo resampler that hides rubato's block size and delay: whatever goes in
/// comes out at the new rate, time-aligned and exactly as long.
pub(super) struct Resampler {
    fft: Fft<f32>,
    /// Interleaved input not yet processed (less than one block after each call).
    input: Vec<f32>,
    /// One block of output.
    output: Vec<f32>,
    /// Output frames still to drop to cancel the filter delay.
    delay: usize,
    ratio: f64,
    frames_in: u64,
    frames_out: u64,
}

impl Resampler {
    pub(super) fn new(from: u32, to: u32) -> Result<Resampler> {
        let fft = Fft::<f32>::new(
            usize::try_from(from)?,
            usize::try_from(to)?,
            CHUNK,
            CHANNELS,
            FixedSync::Input,
        )
        .with_context(|| format!("resampling {from} Hz to {to} Hz"))?;
        let block_in = fft.input_frames_max();
        let block_out = fft.output_frames_max();
        let delay = fft.output_delay();
        Ok(Resampler {
            fft,
            input: Vec::with_capacity(block_in * CHANNELS * 4),
            output: vec![0.0; block_out * CHANNELS],
            delay,
            ratio: f64::from(to) / f64::from(from),
            frames_in: 0,
            frames_out: 0,
        })
    }

    /// Forget everything buffered, for a seek.
    pub(super) fn reset(&mut self) {
        self.fft.reset();
        self.input.clear();
        self.delay = self.fft.output_delay();
        self.frames_in = 0;
        self.frames_out = 0;
    }

    /// Convert interleaved stereo `samples`, appending whatever is ready to `out`.
    pub(super) fn process(&mut self, samples: &[f32], out: &mut Vec<f32>) -> Result<()> {
        self.input.extend_from_slice(samples);
        // usize always fits in u64 on supported targets.
        self.frames_in += (samples.len() / CHANNELS) as u64;
        let available = self.input.len() / CHANNELS;
        let mut consumed = 0;
        while available - consumed >= self.fft.input_frames_next() {
            let indexing = Indexing::new().input_offset(consumed);
            let (used, produced) = self.run(available, &indexing)?;
            consumed += used;
            self.emit(produced, out, u64::MAX);
        }
        self.input.drain(..consumed * CHANNELS);
        Ok(())
    }

    /// Flush the end of the stream: the last partial block, then silence until the output is as
    /// long as the input implied.
    pub(super) fn finish(&mut self, out: &mut Vec<f32>) -> Result<()> {
        // Positive and far below 2^52, so the conversions are exact.
        let expected = (self.frames_in as f64 * self.ratio).round() as u64;
        let mut partial = self.input.len() / CHANNELS;
        let block = self.fft.input_frames_next();
        self.input.resize(block * CHANNELS, 0.0);
        while self.frames_out < expected {
            let indexing = Indexing::new().partial_len(partial);
            let (_, produced) = self.run(block, &indexing)?;
            partial = 0;
            if produced == 0 {
                break;
            }
            self.emit(produced, out, expected);
        }
        self.input.clear();
        Ok(())
    }

    /// Run one block; returns the input frames used and the frames written to `self.output`.
    fn run(&mut self, input_frames: usize, indexing: &Indexing) -> Result<(usize, usize)> {
        let input = InterleavedSlice::new(&self.input[..], CHANNELS, input_frames)?;
        let out_frames = self.output.len() / CHANNELS;
        let mut output = InterleavedSlice::new_mut(&mut self.output[..], CHANNELS, out_frames)?;
        Ok(self
            .fft
            .process_into_buffer(&input, &mut output, Some(indexing))?)
    }

    /// Append the first `produced` output frames to `out`, minus any remaining filter delay and
    /// without going past `limit` frames in total.
    fn emit(&mut self, produced: usize, out: &mut Vec<f32>, limit: u64) {
        let skip = self.delay.min(produced);
        self.delay -= skip;
        let room = usize::try_from(limit.saturating_sub(self.frames_out)).unwrap_or(usize::MAX);
        let take = (produced - skip).min(room);
        out.extend_from_slice(&self.output[skip * CHANNELS..(skip + take) * CHANNELS]);
        self.frames_out += take as u64;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 0.5;
                [v, -v]
            })
            .collect()
    }

    #[test]
    fn output_is_exactly_as_long_as_the_ratio_implies() {
        let mut r = Resampler::new(44_100, 48_000).unwrap();
        let input = sine(441.0, 44_100, 44_100);
        let mut out = Vec::new();
        // Feed in uneven packets, like a decoder would.
        for chunk in input.chunks(1152 * 2) {
            r.process(chunk, &mut out).unwrap();
        }
        r.finish(&mut out).unwrap();
        assert_eq!(out.len(), 48_000 * 2);
    }

    #[test]
    fn a_tone_keeps_its_phase_and_level() {
        let mut r = Resampler::new(44_100, 48_000).unwrap();
        let mut out = Vec::new();
        r.process(&sine(480.0, 44_100, 44_100), &mut out).unwrap();
        r.finish(&mut out).unwrap();
        // Compare against the ideal 480 Hz tone at 48 kHz away from the edges: the delay has
        // been removed, so the two line up sample for sample.
        let ideal = sine(480.0, 48_000, 48_000);
        let worst = out[2000..90_000]
            .iter()
            .zip(&ideal[2000..90_000])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "worst error {worst}");
    }

    #[test]
    fn reset_starts_over() {
        let mut r = Resampler::new(32_000, 48_000).unwrap();
        let mut out = Vec::new();
        r.process(&sine(100.0, 32_000, 5000), &mut out).unwrap();
        r.reset();
        out.clear();
        r.process(&sine(100.0, 32_000, 3200), &mut out).unwrap();
        r.finish(&mut out).unwrap();
        assert_eq!(out.len(), 4800 * 2);
    }
}
