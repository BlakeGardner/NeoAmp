//! What MilkDrop hears: the latest stereo samples, turned each frame into the waveforms the
//! waves are drawn from, the spectra, and the bass, mid and treble levels presets react to.
//!
//! The numbers follow butterchurn's (`audioProcessor.js`, `audioLevels.js`), which follow
//! MilkDrop's: samples are scaled to the signed bytes Winamp handed its visualization plug-ins
//! (-128 to 127), and the levels are relative to their own long-term average, so 1 is "as loud
//! as lately" whatever the volume.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// Samples in each waveform, and bands in each spectrum.
pub const SAMPLES: usize = 512;
/// Stereo frames analysed each time: about 21 ms at 48 kHz.
const WINDOW: usize = 2 * SAMPLES;
/// Audio older than this counts as silence, as in the classic visualiser.
const STALE: Duration = Duration::from_millis(150);
/// The sample rate assumed until [`Sound::set_rate`] is called.
const DEFAULT_RATE: u32 = 44_100;
/// Where bass, mid and treble start and stop, in Hz.
const EDGES: [f32; 4] = [20.0, 320.0, 2800.0, 11_025.0];

/// Bass, mid and treble, each relative to its own average of the last few seconds; the `_att`
/// values are smoothed ("attenuated") versions that change more slowly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub bass: f32,
    pub mid: f32,
    pub treb: f32,
    pub bass_att: f32,
    pub mid_att: f32,
    pub treb_att: f32,
}

impl Default for Levels {
    /// What a preset hears before any audio: everything at its average.
    fn default() -> Self {
        Levels {
            bass: 1.0,
            mid: 1.0,
            treb: 1.0,
            bass_att: 1.0,
            mid_att: 1.0,
            treb_att: 1.0,
        }
    }
}

/// Recent audio and its analysis.
pub struct Sound {
    /// The last [`WINDOW`] frames of each channel, rings written at `head`.
    left: Box<[f32]>,
    right: Box<[f32]>,
    head: usize,
    /// Samples arrived since the last analysis, and when samples last arrived.
    fresh: bool,
    heard: Option<Instant>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Lifts the higher bands, as MilkDrop's FFT did.
    equalize: Box<[f32]>,
    /// Each channel's waveform: smoothed, and every other sample.
    wave: [Box<[f32]>; 2],
    /// Each channel's spectrum.
    spectrum: [Box<[f32]>; 2],
    /// The spectrum of the two channels mixed, which the levels are measured on.
    mixed: Box<[f32]>,
    /// The spectrum bands bass, mid and treble cover.
    bands: [(usize, usize); 3],
    /// Per level: this frame's loudness, its short and long averages.
    now: [f32; 3],
    avg: [f32; 3],
    long_avg: [f32; 3],
    levels: Levels,
    /// Analyses run, for the long average's quicker start.
    frames: u64,
}

impl fmt::Debug for Sound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sound")
            .field("bands", &self.bands)
            .field("levels", &self.levels)
            .finish_non_exhaustive()
    }
}

impl Default for Sound {
    fn default() -> Self {
        Self::new()
    }
}

impl Sound {
    #[must_use]
    pub fn new() -> Sound {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        // MilkDrop's equalisation: -0.02 ln((512 - i) / 512), nothing at the bottom and about
        // 0.12 at the top.
        let equalize = (0..SAMPLES)
            .map(|i| -0.02 * ((SAMPLES - i) as f32 / SAMPLES as f32).ln())
            .collect();
        let zeros = |n: usize| vec![0.0; n].into_boxed_slice();
        Sound {
            left: zeros(WINDOW),
            right: zeros(WINDOW),
            head: 0,
            fresh: false,
            heard: None,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            equalize,
            wave: [zeros(SAMPLES), zeros(SAMPLES)],
            spectrum: [zeros(SAMPLES), zeros(SAMPLES)],
            mixed: zeros(SAMPLES),
            bands: bands(DEFAULT_RATE),
            now: [0.0; 3],
            avg: [1.0; 3],
            long_avg: [1.0; 3],
            levels: Levels::default(),
            frames: 0,
        }
    }

    /// The rate of the samples passed to [`Sound::push`].
    pub fn set_rate(&mut self, rate: u32) {
        self.bands = bands(rate);
    }

    /// Add interleaved stereo samples, oldest first, as they are played.
    pub fn push(&mut self, samples: &[f32]) {
        let frames = samples.len() / 2;
        if frames == 0 {
            return;
        }
        let recent = &samples[2 * frames.saturating_sub(WINDOW)..2 * frames];
        for lr in recent.chunks_exact(2) {
            // Winamp gave visualisations signed bytes.
            self.left[self.head] = (lr[0] * 128.0).clamp(-128.0, 127.0);
            self.right[self.head] = (lr[1] * 128.0).clamp(-128.0, 127.0);
            self.head = (self.head + 1) % WINDOW;
        }
        self.fresh = true;
    }

    /// Analyse the latest audio for a frame drawn at `fps` frames a second. Audio that has
    /// stopped coming counts as silence.
    pub fn analyse(&mut self, now: Instant, fps: f32) {
        if self.fresh {
            self.heard = Some(now);
            self.fresh = false;
        } else if self
            .heard
            .is_none_or(|t| now.saturating_duration_since(t) >= STALE)
        {
            self.left.fill(0.0);
            self.right.fill(0.0);
        }

        // The waveforms: each sample averaged with the one before, then every other one kept.
        for (channel, wave) in [&self.left, &self.right].into_iter().zip(&mut self.wave) {
            let at = |i: usize| channel[(self.head + i) % WINDOW];
            for (j, w) in wave.iter_mut().enumerate() {
                let i = 2 * j;
                *w = 0.5 * (at(i) + at(i.saturating_sub(1)));
            }
        }

        let (left, right, head) = (&self.left, &self.right, self.head);
        let ordered = |ring: &[f32], i: usize| ring[(head + i) % WINDOW];
        for (ch, ring) in [left, right].into_iter().enumerate() {
            for (i, x) in self.input.iter_mut().enumerate() {
                *x = ordered(ring, i);
            }
            magnitudes(
                &*self.fft,
                &mut self.input,
                &mut self.output,
                &mut self.scratch,
                &self.equalize,
                &mut self.spectrum[ch],
            );
        }
        for (i, x) in self.input.iter_mut().enumerate() {
            // The mixed signal was bytes too, rounded towards zero.
            *x = (0.5 * (ordered(left, i) + ordered(right, i))).trunc();
        }
        magnitudes(
            &*self.fft,
            &mut self.input,
            &mut self.output,
            &mut self.scratch,
            &self.equalize,
            &mut self.mixed,
        );
        self.measure(fps);
    }

    /// Bass, mid and treble from the mixed spectrum.
    fn measure(&mut self, fps: f32) {
        // Rates below are per frame at 30 fps; at other speeds they are scaled to match.
        let fps = if fps.is_finite() {
            fps.clamp(15.0, 144.0)
        } else {
            15.0
        };
        let per_frame = |rate: f32| rate.powf(30.0 / fps);
        let mut out = [(0.0, 0.0); 3];
        for (i, &(start, stop)) in self.bands.iter().enumerate() {
            let now = self.mixed[start..stop].iter().sum::<f32>();
            self.now[i] = now;
            let rate = per_frame(if now > self.avg[i] { 0.2 } else { 0.5 });
            self.avg[i] = self.avg[i] * rate + now * (1.0 - rate);
            let rate = per_frame(if self.frames < 50 { 0.9 } else { 0.992 });
            self.long_avg[i] = self.long_avg[i] * rate + now * (1.0 - rate);
            out[i] = if self.long_avg[i] < 0.001 {
                (1.0, 1.0)
            } else {
                (now / self.long_avg[i], self.avg[i] / self.long_avg[i])
            };
        }
        self.frames += 1;
        self.levels = Levels {
            bass: out[0].0,
            mid: out[1].0,
            treb: out[2].0,
            bass_att: out[0].1,
            mid_att: out[1].1,
            treb_att: out[2].1,
        };
    }

    #[must_use]
    pub fn levels(&self) -> Levels {
        self.levels
    }

    /// Channel `ch`'s waveform (0 left, 1 right): [`SAMPLES`] values from -128 to 127,
    /// oldest first.
    #[must_use]
    pub fn wave(&self, ch: usize) -> &[f32] {
        &self.wave[ch.min(1)]
    }

    /// Channel `ch`'s spectrum (0 left, 1 right): [`SAMPLES`] bands from 0 Hz to half the
    /// sample rate.
    #[must_use]
    pub fn spectrum(&self, ch: usize) -> &[f32] {
        &self.spectrum[ch.min(1)]
    }
}

/// The spectrum bands bass, mid and treble cover at `rate`, as MilkDrop chose them.
fn bands(rate: u32) -> [(usize, usize); 3] {
    // Sample rates are far below 2^24, so the conversion is exact.
    let bucket_hz = rate.max(1) as f32 / WINDOW as f32;
    // Rounded and clamped to the spectrum, so the cast cannot truncate.
    let edge = |hz: f32| ((hz / bucket_hz).round() - 1.0).clamp(0.0, (SAMPLES - 1) as f32) as usize;
    let [a, b, c, d] = EDGES.map(edge);
    [(a, b), (b, c), (c, d)]
}

/// The equalised magnitudes of the first [`SAMPLES`] bins of `input`'s spectrum, into `out`.
fn magnitudes(
    fft: &dyn RealToComplex<f32>,
    input: &mut [f32],
    output: &mut [Complex<f32>],
    scratch: &mut [Complex<f32>],
    equalize: &[f32],
    out: &mut [f32],
) {
    if fft.process_with_scratch(input, output, scratch).is_err() {
        out.fill(0.0);
        return;
    }
    for ((o, c), e) in out.iter_mut().zip(output.iter()).zip(equalize) {
        *o = e * c.norm();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `amp` on the left, and `right` times it on the right.
    fn tone(freq: f32, rate: u32, frames: usize, amp: f32, right: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let s = amp * (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin();
                [s, right * s]
            })
            .collect()
    }

    #[test]
    fn bands_are_milkdrops_at_44_1_and_48_khz() {
        assert_eq!(bands(44_100), [(0, 6), (6, 64), (64, 255)]);
        assert_eq!(bands(48_000), [(0, 6), (6, 59), (59, 234)]);
        // At a low rate treble ends at the top of the spectrum.
        assert_eq!(bands(8000)[2].1, 511);
    }

    #[test]
    fn a_tone_lands_in_its_band_and_channel_signs_survive() {
        let mut s = Sound::new();
        s.set_rate(48_000);
        s.push(&tone(1500.0, 48_000, WINDOW, 0.5, -1.0));
        s.analyse(Instant::now(), 30.0);
        // 1500 Hz is bin 32 at 46.875 Hz per bin.
        let loudest = (0..SAMPLES)
            .max_by(|&a, &b| s.spectrum(0)[a].total_cmp(&s.spectrum(0)[b]))
            .unwrap_or(0);
        assert_eq!(loudest, 32);
        // The right channel is the left upside down.
        let (l, r) = (s.wave(0), s.wave(1));
        assert!(l.iter().zip(r).all(|(a, b)| (a + b).abs() < 1e-4));
        assert!(l.iter().any(|v| *v > 50.0));
        assert!(l.iter().all(|v| (-128.0..=127.0).contains(v)));
    }

    #[test]
    fn levels_rise_above_one_on_a_louder_beat_and_go_flat_in_silence() {
        let mut s = Sound::new();
        s.set_rate(48_000);
        let t0 = Instant::now();
        let quiet = tone(100.0, 48_000, WINDOW, 0.05, 1.0);
        for k in 0..120 {
            s.push(&quiet);
            s.analyse(t0 + Duration::from_millis(33 * k), 30.0);
        }
        let settled = s.levels();
        assert!((settled.bass - 1.0).abs() < 0.2, "{settled:?}");
        s.push(&tone(100.0, 48_000, WINDOW, 0.5, 1.0));
        s.analyse(t0 + Duration::from_millis(33 * 120), 30.0);
        let beat = s.levels();
        assert!(beat.bass > 3.0, "{beat:?}");
        assert!(beat.bass_att > 1.0 && beat.bass_att < beat.bass, "{beat:?}");
        // No audio for a while: the waveforms go flat.
        s.analyse(t0 + Duration::from_secs(10), 30.0);
        assert!(s.wave(0).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn an_odd_sample_is_no_frame() {
        let mut s = Sound::new();
        s.push(&[0.5]);
        assert!(!s.fresh);
        s.push(&[0.5, 0.25, 1.0]);
        assert_eq!(s.head, 1);
        assert_eq!((s.left[0], s.right[0]), (64.0, 32.0));
    }
}
