//! The 76x16 visualiser: a spectrum analyser and an oscilloscope fed with the samples the
//! sound card is playing.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::canvas::{Image, Rect};
use crate::skin::Skin;

/// Each analyser bar is this wide, followed by a one pixel gap.
const BAR_W: i32 = 3;

/// What the visualiser shows. Saved in the settings as `analyzer`, `oscilloscope` or `off`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisMode {
    #[default]
    Analyzer,
    Oscilloscope,
    Off,
}

pub const BARS: usize = 19;
pub const H: i32 = 16;
/// Width of the visualiser, and so the number of oscilloscope points.
pub const W: usize = 76;

/// Samples per analyser FFT: about 21 ms at 48 kHz, 47 Hz per bin.
const FFT_SIZE: usize = 1024;
/// Most recent samples kept (mono, at the device rate).
const HISTORY: usize = 2048;
/// The oscilloscope spreads this many samples over its width, as Winamp did.
const SCOPE_SPAN: usize = 576;
/// The analyser's bars are spaced evenly in pitch between these frequencies.
const MIN_HZ: f32 = 40.0;
const MAX_HZ: f32 = 16_000.0;
/// Bar heights span this many dB below a full-scale sine.
const RANGE_DB: f32 = 60.0;
/// Music has less energy the higher it goes; lift each octave above 1 kHz by this much (and
/// lower each one below) so the bars use their whole height across the range, as Winamp's did.
const TILT_DB_PER_OCTAVE: f32 = 3.0;
/// Audio older than this counts as silence: the bars fall and the trace goes flat.
const STALE: Duration = Duration::from_millis(150);
/// The sample rate assumed until [`Vis::set_sample_rate`] is called.
const DEFAULT_RATE: u32 = 44_100;

/// The FFT and the mapping from its bins to bars.
struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    /// Hann window.
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Bar `i` covers FFT bins `edges[i]..edges[i + 1]`.
    edges: [usize; BARS + 1],
    /// Spectral tilt per bar, in dB.
    tilt: [f32; BARS],
}

impl fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Analyzer")
            .field("edges", &self.edges)
            .finish_non_exhaustive()
    }
}

impl Analyzer {
    fn new(rate: u32) -> Analyzer {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
            })
            .collect();
        let mut a = Analyzer {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            edges: [0; BARS + 1],
            tilt: [0.0; BARS],
        };
        a.set_rate(rate);
        a
    }

    fn set_rate(&mut self, rate: u32) {
        // Sample rates are far below 2^24, so the conversion is exact.
        let rate = rate.max(1) as f32;
        let last_bin = FFT_SIZE / 2;
        let top = MAX_HZ.min(rate / 2.0);
        let hz = |i: usize| MIN_HZ * (top / MIN_HZ).powf(i as f32 / BARS as f32);
        for (i, edge) in self.edges.iter_mut().enumerate() {
            // Non-negative and at most FFT_SIZE / 2, so the cast cannot truncate.
            *edge = ((hz(i) * FFT_SIZE as f32 / rate).round() as usize).clamp(1, last_bin);
        }
        // The lowest bars are narrower than a bin: give each at least one of its own.
        for i in 1..=BARS {
            self.edges[i] = self.edges[i].max(self.edges[i - 1] + 1).min(last_bin + 1);
        }
        for (i, t) in self.tilt.iter_mut().enumerate() {
            let centre = (hz(i) * hz(i + 1)).sqrt();
            *t = TILT_DB_PER_OCTAVE * (centre / 1000.0).log2();
        }
    }

    /// Bar heights in pixels for the `FFT_SIZE` samples of `history` that end just before
    /// index `head` (a ring buffer).
    fn bars(&mut self, history: &[f32], head: usize) -> [f32; BARS] {
        let start = head + history.len() - FFT_SIZE;
        for (i, (x, w)) in self.input.iter_mut().zip(&self.window).enumerate() {
            *x = history[(start + i) % history.len()] * w;
        }
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            return [0.0; BARS];
        }
        // A full-scale sine peaks at N/4 through a Hann window.
        let full_scale = FFT_SIZE as f32 / 4.0;
        std::array::from_fn(|i| {
            let bins = &self.spectrum[self.edges[i]..self.edges[i + 1]];
            let peak = bins.iter().map(|c| c.norm()).fold(0.0f32, f32::max);
            let db = 20.0 * (peak / full_scale).max(1e-9).log10() + self.tilt[i];
            ((db + RANGE_DB) / RANGE_DB * H as f32).clamp(0.0, H as f32)
        })
    }
}

#[derive(Debug)]
pub struct Vis {
    pub mode: VisMode,
    bars: [f32; BARS],
    peaks: [f32; BARS],
    peak_hold: [u8; BARS],
    peak_vel: [f32; BARS],
    last: Option<Instant>,
    /// The most recent samples, a ring written at `head`.
    history: Vec<f32>,
    head: usize,
    /// Samples arrived since the last update.
    fresh: bool,
    /// When samples last arrived.
    heard: Option<Instant>,
    /// The oscilloscope trace, `-1..=1` per pixel column.
    scope: [f32; W],
    analyzer: Analyzer,
}

impl Default for Vis {
    fn default() -> Self {
        Self::new()
    }
}

impl Vis {
    #[must_use]
    pub fn new() -> Vis {
        Vis {
            mode: VisMode::Analyzer,
            bars: [0.0; BARS],
            peaks: [0.0; BARS],
            peak_hold: [0; BARS],
            peak_vel: [0.0; BARS],
            last: None,
            history: vec![0.0; HISTORY],
            head: 0,
            fresh: false,
            heard: None,
            scope: [0.0; W],
            analyzer: Analyzer::new(DEFAULT_RATE),
        }
    }

    pub fn next_mode(&mut self) {
        self.mode = match self.mode {
            VisMode::Analyzer => VisMode::Oscilloscope,
            VisMode::Oscilloscope => VisMode::Off,
            VisMode::Off => VisMode::Analyzer,
        };
    }

    pub fn reset(&mut self) {
        self.bars = [0.0; BARS];
        self.peaks = [0.0; BARS];
        self.peak_hold = [0; BARS];
        self.peak_vel = [0.0; BARS];
        self.scope = [0.0; W];
        self.heard = None;
        self.fresh = false;
    }

    /// The rate of the samples passed to [`Vis::push_samples`].
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.analyzer.set_rate(rate);
    }

    /// Add mono samples, oldest first, as they are played.
    pub fn push_samples(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let recent = &samples[samples.len().saturating_sub(HISTORY)..];
        for &s in recent {
            self.history[self.head] = s;
            self.head = (self.head + 1) % HISTORY;
        }
        self.fresh = true;
    }

    /// Advance one frame: analyse the latest audio while playing, and let the bars and peaks
    /// fall the way Winamp's did.
    pub fn update(&mut self, now: Instant, playing: bool) {
        let dt = self
            .last
            .map(|l| now.saturating_duration_since(l).as_secs_f32())
            .unwrap_or(0.0)
            .min(0.1);
        self.last = Some(now);
        if self.fresh {
            self.heard = Some(now);
            self.fresh = false;
        }
        let live = playing
            && self
                .heard
                .is_some_and(|t| now.saturating_duration_since(t) < STALE);
        let targets = if live {
            self.capture_scope();
            self.analyzer.bars(&self.history, self.head)
        } else {
            self.scope = [0.0; W];
            [0.0; BARS]
        };
        let frames = (dt * 30.0).max(0.0);
        for (i, target) in targets.into_iter().enumerate() {
            // Bars jump up instantly and fall about one pixel per frame.
            self.bars[i] = if target > self.bars[i] {
                target
            } else {
                (self.bars[i] - frames).max(0.0)
            };
            // Peaks hold, then fall with acceleration.
            if self.bars[i] >= self.peaks[i] {
                self.peaks[i] = self.bars[i];
                self.peak_hold[i] = 20;
                self.peak_vel[i] = 0.0;
            } else if self.peak_hold[i] > 0 {
                // A few frames at most, so the cast cannot truncate.
                self.peak_hold[i] = self.peak_hold[i].saturating_sub(frames.round() as u8);
            } else {
                self.peak_vel[i] += 0.15 * frames;
                self.peaks[i] = (self.peaks[i] - self.peak_vel[i]).max(0.0);
            }
        }
    }

    /// Sample the last `SCOPE_SPAN` samples down to one value per pixel column.
    fn capture_scope(&mut self) {
        let start = self.head + HISTORY - SCOPE_SPAN;
        for (x, v) in self.scope.iter_mut().enumerate() {
            *v = self.history[(start + x * SCOPE_SPAN / W) % HISTORY];
        }
    }

    /// Draw into `r` (76x16) using the skin's viscolor palette.
    pub fn draw(&self, c: &mut Image, skin: &Skin, r: Rect) {
        if self.mode == VisMode::Off {
            return;
        }
        let colors = &skin.vis_colors;
        c.fill_rect(r, colors[0]);
        for y in (1..r.h).step_by(2) {
            for x in (0..r.w).step_by(2) {
                c.put(r.x + x, r.y + y, colors[1]);
            }
        }
        match self.mode {
            VisMode::Analyzer => {
                for (i, (bar, peak)) in self.bars.iter().zip(&self.peaks).enumerate() {
                    let x = r.x + i32::try_from(i).unwrap_or(0) * (BAR_W + 1);
                    // Heights are clamped to 0..=16, so the casts cannot truncate.
                    let h = bar.round().clamp(0.0, H as f32) as i32;
                    for row in (H - h)..H {
                        let color = colors[2 + row.unsigned_abs() as usize];
                        c.fill_rect(Rect::new(x, r.y + row, BAR_W, 1), color);
                    }
                    let p = peak.round().clamp(0.0, H as f32) as i32;
                    if p > 0 {
                        c.fill_rect(Rect::new(x, r.y + H - p, BAR_W, 1), colors[23]);
                    }
                }
            }
            VisMode::Oscilloscope => {
                let mut prev: Option<i32> = None;
                for (x, v) in (0..r.w).zip(self.scope) {
                    // Clamped to the 16 rows, so the cast cannot truncate.
                    let y = (8.0 - v * 8.0).round().clamp(0.0, 15.0) as i32;
                    let (y0, y1) = match prev {
                        Some(p) if p < y => (p + 1, y),
                        Some(p) if p > y => (y, p - 1),
                        _ => (y, y),
                    };
                    for yy in y0..=y1 {
                        let d = (yy - 8).unsigned_abs() as usize;
                        c.put(r.x + x, r.y + yy, colors[18 + (d * 5 / 9).min(4)]);
                    }
                    prev = Some(y);
                }
            }
            VisMode::Off => {}
        }
    }
}

/// A stand-in for music when there is no audio device (screenshots): a few tones that swell
/// and fade at their own pace over a little noise. `t` is the time of `out[0]` in seconds.
pub fn demo_signal(t: f64, rate: u32, out: &mut [f32]) {
    // (frequency Hz, amplitude, swell rate)
    const TONES: [(f64, f64, f64); 5] = [
        (55.0, 0.30, 0.7),
        (220.0, 0.20, 1.3),
        (880.0, 0.10, 2.1),
        (3520.0, 0.05, 0.9),
        (9000.0, 0.02, 1.7),
    ];
    let mut noise = 0x2545_F491u32;
    for (i, s) in out.iter_mut().enumerate() {
        let ti = t + i as f64 / f64::from(rate.max(1));
        let mut v = 0.0;
        for (freq, amp, swell) in TONES {
            let env = 0.6 + 0.4 * (ti * swell * std::f64::consts::TAU).sin();
            v += amp * env * (ti * freq * std::f64::consts::TAU).sin();
        }
        // xorshift32
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        v += 0.02 * (f64::from(noise >> 8) / f64::from(1u32 << 24) - 0.5);
        // Within -1..=1, so narrowing loses only precision.
        *s = v as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn loudest_bar(v: &Vis) -> usize {
        v.bars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i)
    }

    #[test]
    fn a_tone_lights_the_bar_for_its_pitch() {
        let now = Instant::now();
        let mut low = Vis::new();
        low.set_sample_rate(48_000);
        low.push_samples(&tone(100.0, 48_000, HISTORY));
        low.update(now, true);
        let mut high = Vis::new();
        high.set_sample_rate(48_000);
        high.push_samples(&tone(8000.0, 48_000, HISTORY));
        high.update(now, true);
        let (lo, hi) = (loudest_bar(&low), loudest_bar(&high));
        assert!(lo < 5, "100 Hz peaked in bar {lo}");
        assert!(hi > 14, "8 kHz peaked in bar {hi}");
        // A full-scale tone nearly fills its bar.
        assert!(low.bars[lo] > 12.0);
    }

    #[test]
    fn silence_and_stale_audio_let_the_bars_fall() {
        let t0 = Instant::now();
        let mut v = Vis::new();
        v.push_samples(&tone(1000.0, 44_100, HISTORY));
        v.update(t0, true);
        let top = v.bars.iter().copied().fold(0.0, f32::max);
        assert!(top > 0.0);
        // No new samples for a while: the analyser stops reading the old ones.
        v.update(t0 + Duration::from_millis(200), true);
        let later = v.bars.iter().copied().fold(0.0, f32::max);
        assert!(later < top);
        assert!(v.scope.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn the_scope_follows_the_waveform() {
        let mut v = Vis::new();
        let mut ramp: Vec<f32> = (0..HISTORY).map(|i| i as f32 / HISTORY as f32).collect();
        v.push_samples(&ramp);
        v.update(Instant::now(), true);
        // A ramp stays a ramp.
        assert!(v.scope.windows(2).all(|w| w[0] < w[1]));
        ramp.clear();
        v.push_samples(&ramp);
        assert!(!v.fresh, "an empty push is not new audio");
    }

    #[test]
    fn bar_edges_increase_and_stay_in_the_spectrum() {
        for rate in [8000, 22_050, 44_100, 48_000, 192_000] {
            let a = Analyzer::new(rate);
            let edges = a.edges;
            assert!(edges.windows(2).all(|w| w[0] < w[1]), "{rate}: {edges:?}");
            assert!(a.edges[BARS] <= FFT_SIZE / 2 + 1);
        }
    }
}
