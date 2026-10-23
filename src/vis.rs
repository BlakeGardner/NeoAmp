//! The 76x16 visualiser: a spectrum analyser and an oscilloscope fed with the samples the
//! sound card is playing, in the styles Winamp's visualization menu offered. A 38x5 version
//! of it shows in the main window's windowshade bar.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use serde::{Deserialize, Deserializer, Serialize};

use crate::canvas::{Canvas, Rect};
use crate::settings::{lenient, or_default};
use crate::skin::Skin;

/// Thick analyser bars are this wide, followed by a one pixel gap.
const THICK_W: i32 = 3;

/// What the visualiser shows. Saved in the settings as `analyzer`, `oscilloscope` or `off`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisMode {
    #[default]
    Analyzer,
    Oscilloscope,
    Off,
}

/// How the analyser colours its bars, from `viscolor.txt`'s colours 2 (top) to 17 (bottom).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalyzerStyle {
    /// Each row has its own colour, so only tall bars reach the top colours.
    #[default]
    Normal,
    /// The colours start at the top of each bar, so every bar has the hottest tip.
    Fire,
    /// Each bar is one colour, that of the row its top reaches.
    Line,
}

/// How many analyser bars there are.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bands {
    /// 19 bars three pixels wide.
    #[default]
    Thick,
    /// 75 bars one pixel wide.
    Thin,
}

impl Bands {
    #[must_use]
    pub const fn count(self) -> usize {
        match self {
            Bands::Thick => 19,
            Bands::Thin => 75,
        }
    }
}

/// The most bars there can be.
const MAX_BARS: usize = Bands::Thin.count();

/// How fast the analyser's bars, or its peaks, fall.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Falloff {
    Slowest,
    Slow,
    #[default]
    Moderate,
    Fast,
    Fastest,
}

impl Falloff {
    pub const ALL: [Falloff; 5] = [
        Falloff::Slowest,
        Falloff::Slow,
        Falloff::Moderate,
        Falloff::Fast,
        Falloff::Fastest,
    ];

    /// Pixels a second a bar drops. Winamp's five speeds were 3, 6, 12, 16 and 32 apart;
    /// Moderate is the speed NeoAmp's bars always fell at.
    fn bar_speed(self) -> f32 {
        match self {
            Falloff::Slowest => 7.5,
            Falloff::Slow => 15.0,
            Falloff::Moderate => 30.0,
            Falloff::Fast => 40.0,
            Falloff::Fastest => 80.0,
        }
    }

    /// A falling peak speeds up by this factor every 60th of a second (Winamp's values).
    fn peak_growth(self) -> f32 {
        match self {
            Falloff::Slowest => 1.05,
            Falloff::Slow => 1.1,
            Falloff::Moderate => 1.2,
            Falloff::Fast => 1.4,
            Falloff::Fastest => 1.6,
        }
    }
}

/// A peak starts falling this many pixels per 60th of a second (Winamp's 3/256).
const PEAK_START: f32 = 3.0 / 256.0;

/// How the oscilloscope draws the waveform.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeStyle {
    /// One dot per column.
    Dots,
    /// Dots joined up.
    #[default]
    Lines,
    /// Filled from the centre line.
    Solid,
}

/// The analyser's options, as Winamp's visualization menu set them. Saved in the settings as
/// the `[analyzer]` table; a value that cannot be used falls back to its default on its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalyzerOptions {
    #[serde(deserialize_with = "or_default")]
    pub style: AnalyzerStyle,
    #[serde(deserialize_with = "or_default")]
    pub bands: Bands,
    pub peaks: bool,
    #[serde(deserialize_with = "or_default")]
    pub falloff: Falloff,
    #[serde(deserialize_with = "slow_or_default")]
    pub peak_falloff: Falloff,
}

impl Default for AnalyzerOptions {
    /// Winamp's own defaults.
    fn default() -> Self {
        AnalyzerOptions {
            style: AnalyzerStyle::Normal,
            bands: Bands::Thick,
            peaks: true,
            falloff: Falloff::Moderate,
            peak_falloff: Falloff::Slow,
        }
    }
}

/// Like [`or_default`], for the peaks' falloff, whose default is slow.
fn slow_or_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Falloff, D::Error> {
    Ok(lenient(deserializer)?.unwrap_or(Falloff::Slow))
}

/// The oscilloscope's options. Saved in the settings as the `[oscilloscope]` table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScopeOptions {
    #[serde(deserialize_with = "or_default")]
    pub style: ScopeStyle,
}

pub const H: i32 = 16;
/// Width of the visualiser, and so the number of oscilloscope points.
pub const W: usize = 76;
/// Height of the small visualiser in the main window's windowshade bar.
const SMALL_H: i32 = 5;
/// The small analyser's colours from `viscolor.txt`, top row to bottom, as Winamp picked them.
const SMALL_ROWS: [usize; SMALL_H as usize] = [4, 8, 11, 14, 17];

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

/// The part of the spectrum one bar shows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Band {
    /// The loudest of FFT bins `bins.0..bins.1`, when that range holds any.
    bins: (usize, usize),
    /// Otherwise the band is narrower than a bin: the spectrum interpolated at this bin
    /// position, the band's centre.
    centre: f32,
    /// Spectral tilt, in dB.
    tilt: f32,
}

/// The FFT and the mapping from its bins to bars.
struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    /// Hann window.
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    rate: u32,
    bands: Vec<Band>,
}

impl fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Analyzer")
            .field("rate", &self.rate)
            .field("bands", &self.bands.len())
            .finish_non_exhaustive()
    }
}

impl Analyzer {
    fn new(rate: u32, count: usize) -> Analyzer {
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
            rate,
            bands: Vec::with_capacity(MAX_BARS),
        };
        a.layout(rate, count);
        a
    }

    /// Spread `count` bars over the spectrum of audio at `rate`.
    fn layout(&mut self, rate: u32, count: usize) {
        self.rate = rate;
        // Sample rates are far below 2^24, so the conversion is exact.
        let rate = rate.max(1) as f32;
        let last_bin = FFT_SIZE / 2;
        let top = MAX_HZ.min(rate / 2.0);
        let hz = |i: usize| MIN_HZ * (top / MIN_HZ).powf(i as f32 / count.max(1) as f32);
        let bin = |f: f32| f * FFT_SIZE as f32 / rate;
        self.bands.clear();
        self.bands.extend((0..count).map(|i| {
            let (lo, hi) = (hz(i), hz(i + 1));
            // Non-negative and at most FFT_SIZE / 2 + 1, so the casts cannot truncate.
            let edge = |f: f32| (bin(f).round() as usize).clamp(1, last_bin + 1);
            let centre = (lo * hi).sqrt();
            Band {
                bins: (edge(lo), edge(hi)),
                centre: bin(centre).min(last_bin as f32),
                tilt: TILT_DB_PER_OCTAVE * (centre / 1000.0).log2(),
            }
        }));
    }

    /// Bar heights in pixels into `out` (one per band) for the `FFT_SIZE` samples of `history`
    /// that end just before index `head` (a ring buffer).
    fn bars(&mut self, history: &[f32], head: usize, out: &mut [f32]) {
        let start = head + history.len() - FFT_SIZE;
        for (i, (x, w)) in self.input.iter_mut().zip(&self.window).enumerate() {
            *x = history[(start + i) % history.len()] * w;
        }
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            out.fill(0.0);
            return;
        }
        // A full-scale sine peaks at N/4 through a Hann window.
        let full_scale = FFT_SIZE as f32 / 4.0;
        let spectrum = &self.spectrum;
        let norm = |i: usize| spectrum.get(i).map_or(0.0, |c| c.norm());
        for (bar, band) in out.iter_mut().zip(&self.bands) {
            let (lo, hi) = band.bins;
            let level = if hi > lo {
                (lo..hi).map(norm).fold(0.0f32, f32::max)
            } else {
                // Non-negative and within the spectrum, so the cast cannot truncate.
                let below = band.centre.floor() as usize;
                let frac = band.centre - band.centre.floor();
                norm(below) * (1.0 - frac) + norm(below + 1) * frac
            };
            let db = 20.0 * (level / full_scale).max(1e-9).log10() + band.tilt;
            *bar = ((db + RANGE_DB) / RANGE_DB * H as f32).clamp(0.0, H as f32);
        }
    }
}

#[derive(Debug)]
pub struct Vis {
    pub mode: VisMode,
    pub analyzer: AnalyzerOptions,
    pub scope_style: ScopeStyle,
    bars: [f32; MAX_BARS],
    peaks: [f32; MAX_BARS],
    /// How fast each peak is falling, in pixels per 60th of a second.
    peak_speed: [f32; MAX_BARS],
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
    spectrum: Analyzer,
}

impl Default for Vis {
    fn default() -> Self {
        Self::new()
    }
}

impl Vis {
    #[must_use]
    pub fn new() -> Vis {
        let analyzer = AnalyzerOptions::default();
        Vis {
            mode: VisMode::Analyzer,
            analyzer,
            scope_style: ScopeStyle::default(),
            bars: [0.0; MAX_BARS],
            peaks: [0.0; MAX_BARS],
            peak_speed: [0.0; MAX_BARS],
            last: None,
            history: vec![0.0; HISTORY],
            head: 0,
            fresh: false,
            heard: None,
            scope: [0.0; W],
            spectrum: Analyzer::new(DEFAULT_RATE, analyzer.bands.count()),
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
        self.bars = [0.0; MAX_BARS];
        self.peaks = [0.0; MAX_BARS];
        self.peak_speed = [0.0; MAX_BARS];
        self.scope = [0.0; W];
        self.heard = None;
        self.fresh = false;
    }

    /// The rate of the samples passed to [`Vis::push_samples`].
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.spectrum.layout(rate, self.analyzer.bands.count());
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

    /// Add interleaved stereo samples, oldest first, as they are played. The analyser and
    /// oscilloscope show their mono mix.
    pub fn push_stereo(&mut self, samples: &[f32]) {
        if samples.len() < 2 {
            return;
        }
        let frames = samples.len() / 2;
        let recent = &samples[2 * frames.saturating_sub(HISTORY)..2 * frames];
        for lr in recent.chunks_exact(2) {
            self.history[self.head] = 0.5 * (lr[0] + lr[1]);
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
        let count = self.analyzer.bands.count();
        if self.spectrum.bands.len() != count {
            // Switched between thick and thin bands: the old heights mean nothing now.
            self.spectrum.layout(self.spectrum.rate, count);
            self.reset();
        }
        let live = playing
            && self
                .heard
                .is_some_and(|t| now.saturating_duration_since(t) < STALE);
        let mut targets = [0.0; MAX_BARS];
        if live {
            self.capture_scope();
            self.spectrum
                .bars(&self.history, self.head, &mut targets[..count]);
        } else {
            self.scope = [0.0; W];
        }
        let fall = self.analyzer.falloff.bar_speed() * dt;
        let ticks = dt * 60.0;
        let growth = self.analyzer.peak_falloff.peak_growth().powf(ticks);
        let bars = self
            .bars
            .iter_mut()
            .zip(&mut self.peaks)
            .zip(&mut self.peak_speed);
        for (((bar, peak), speed), target) in bars.zip(targets).take(count) {
            // Bars jump up at once and fall steadily.
            *bar = if target > *bar {
                target
            } else {
                (*bar - fall).max(0.0)
            };
            // Peaks ride the bars up, then fall ever faster.
            if *bar >= *peak {
                *peak = *bar;
                *speed = PEAK_START;
            } else {
                *peak = (*peak - *speed * ticks).max(0.0);
                *speed *= growth;
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
    pub fn draw(&self, c: &mut Canvas, skin: &Skin, r: Rect) {
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
            VisMode::Analyzer => self.draw_analyzer(c, colors, r),
            VisMode::Oscilloscope => self.draw_scope(c, colors, r),
            VisMode::Off => {}
        }
    }

    fn draw_analyzer(&self, c: &mut Canvas, colors: &[[u8; 4]; 24], r: Rect) {
        let (width, pitch) = match self.analyzer.bands {
            Bands::Thick => (THICK_W, THICK_W + 1),
            Bands::Thin => (1, 1),
        };
        let count = self.analyzer.bands.count();
        for (x, (bar, peak)) in (0..)
            .map(|i| r.x + i * pitch)
            .zip(self.bars.iter().zip(&self.peaks))
            .take(count)
        {
            // Heights are clamped to 0..=16, so the casts cannot truncate.
            let h = bar.round().clamp(0.0, H as f32) as i32;
            let top = H - h;
            for row in top..H {
                let shade = match self.analyzer.style {
                    AnalyzerStyle::Normal => row,
                    AnalyzerStyle::Fire => row - top,
                    AnalyzerStyle::Line => top,
                };
                let color = colors[2 + shade.clamp(0, H - 1).unsigned_abs() as usize];
                c.fill_rect(Rect::new(x, r.y + row, width, 1), color);
            }
            let p = peak.round().clamp(0.0, H as f32) as i32;
            if self.analyzer.peaks && p > 0 {
                c.fill_rect(Rect::new(x, r.y + H - p, width, 1), colors[23]);
            }
        }
    }

    /// Draw the small version for the windowshade bar into `r` (38x5). Each pair of the
    /// analyser's bars makes one, so the whole spectrum still fits; the oscilloscope shows the
    /// start of its trace, unsqueezed, as Winamp's did. Neither draws peaks or a dot grid.
    pub fn draw_small(&self, c: &mut Canvas, skin: &Skin, r: Rect) {
        let colors = &skin.vis_colors;
        match self.mode {
            VisMode::Analyzer => {
                c.fill_rect(r, colors[0]);
                self.draw_small_analyzer(c, colors, r);
            }
            VisMode::Oscilloscope => {
                c.fill_rect(r, colors[0]);
                self.draw_small_scope(c, colors, r);
            }
            VisMode::Off => {}
        }
    }

    fn draw_small_analyzer(&self, c: &mut Canvas, colors: &[[u8; 4]; 24], r: Rect) {
        let (width, pitch) = match self.analyzer.bands {
            Bands::Thick => (THICK_W, THICK_W + 1),
            Bands::Thin => (1, 1),
        };
        let count = self.analyzer.bands.count();
        let pairs = self.bars[..count].chunks(2);
        for (x, pair) in (0..).map(|i| r.x + i * pitch).zip(pairs) {
            let bar = pair.iter().copied().fold(0.0, f32::max);
            // Clamped to 0..=5, so the cast cannot truncate.
            let h = (bar * SMALL_H as f32 / H as f32)
                .round()
                .clamp(0.0, SMALL_H as f32) as i32;
            let top = SMALL_H - h;
            for row in top..SMALL_H {
                let shade = match self.analyzer.style {
                    AnalyzerStyle::Normal => row,
                    AnalyzerStyle::Fire => row - top,
                    AnalyzerStyle::Line => top,
                };
                let color = colors[SMALL_ROWS[shade.clamp(0, SMALL_H - 1).unsigned_abs() as usize]];
                c.fill_rect_clipped(Rect::new(x, r.y + row, width, 1), color, r);
            }
        }
    }

    fn draw_small_scope(&self, c: &mut Canvas, colors: &[[u8; 4]; 24], r: Rect) {
        let mid = SMALL_H / 2;
        let mut prev: Option<i32> = None;
        for (x, v) in (0..r.w).zip(self.scope) {
            // Clamped to the 5 rows, so the cast cannot truncate.
            let y = (mid as f32 - v * 2.5)
                .round()
                .clamp(0.0, (SMALL_H - 1) as f32) as i32;
            let (y0, y1) = match (self.scope_style, prev) {
                (ScopeStyle::Dots, _) => (y, y),
                (ScopeStyle::Solid, _) => (y.min(mid), y.max(mid)),
                (ScopeStyle::Lines, Some(p)) if p < y => (p + 1, y),
                (ScopeStyle::Lines, Some(p)) if p > y => (y, p - 1),
                (ScopeStyle::Lines, _) => (y, y),
            };
            for yy in y0..=y1 {
                c.put(r.x + x, r.y + yy, colors[18]);
            }
            prev = Some(y);
        }
    }

    fn draw_scope(&self, c: &mut Canvas, colors: &[[u8; 4]; 24], r: Rect) {
        let mut prev: Option<i32> = None;
        for (x, v) in (0..r.w).zip(self.scope) {
            // Clamped to the 16 rows, so the cast cannot truncate.
            let y = (8.0 - v * 8.0).round().clamp(0.0, 15.0) as i32;
            let (y0, y1) = match (self.scope_style, prev) {
                (ScopeStyle::Dots, _) => (y, y),
                (ScopeStyle::Solid, _) if y >= 8 => (8, y),
                (ScopeStyle::Solid, _) => (y, 7),
                (ScopeStyle::Lines, Some(p)) if p < y => (p + 1, y),
                (ScopeStyle::Lines, Some(p)) if p > y => (y, p - 1),
                (ScopeStyle::Lines, _) => (y, y),
            };
            for yy in y0..=y1 {
                let d = (yy - 8).unsigned_abs() as usize;
                c.put(r.x + x, r.y + yy, colors[18 + (d * 5 / 9).min(4)]);
            }
            prev = Some(y);
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::canvas::Density;

    fn tone(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn loudest_bar(v: &Vis) -> usize {
        v.bars[..v.analyzer.bands.count()]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i)
    }

    fn listening_to(freq: f32, bands: Bands) -> Vis {
        let mut v = Vis::new();
        v.analyzer.bands = bands;
        v.set_sample_rate(48_000);
        v.push_samples(&tone(freq, 48_000, HISTORY));
        v.update(Instant::now(), true);
        v
    }

    #[test]
    fn a_tone_lights_the_bar_for_its_pitch() {
        let (low, high) = (
            listening_to(100.0, Bands::Thick),
            listening_to(8000.0, Bands::Thick),
        );
        let (lo, hi) = (loudest_bar(&low), loudest_bar(&high));
        assert!(lo < 5, "100 Hz peaked in bar {lo}");
        assert!(hi > 14, "8 kHz peaked in bar {hi}");
        // A full-scale tone nearly fills its bar.
        assert!(low.bars[lo] > 12.0);
    }

    #[test]
    fn thin_bands_place_tones_in_proportion() {
        let (low, high) = (
            listening_to(100.0, Bands::Thin),
            listening_to(8000.0, Bands::Thin),
        );
        let (lo, hi) = (loudest_bar(&low), loudest_bar(&high));
        // 100 Hz is 15% of the way up the pitch range, 8 kHz 88%.
        assert!((8..=16).contains(&lo), "100 Hz peaked in bar {lo}");
        assert!((60..=70).contains(&hi), "8 kHz peaked in bar {hi}");
        assert!(low.bars[lo] > 12.0);
    }

    #[test]
    fn switching_band_width_starts_the_bars_afresh() {
        let mut v = listening_to(1000.0, Bands::Thick);
        assert!(v.bars.iter().any(|b| *b > 0.0));
        v.analyzer.bands = Bands::Thin;
        v.update(Instant::now(), false);
        assert_eq!(v.spectrum.bands.len(), 75);
        assert!(v.bars.iter().all(|b| *b == 0.0));
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
    fn faster_falloffs_drop_bars_and_peaks_sooner() {
        let fallen = |falloff: Falloff, peak_falloff: Falloff| {
            let t0 = Instant::now();
            let mut v = Vis::new();
            v.analyzer.falloff = falloff;
            v.analyzer.peak_falloff = peak_falloff;
            v.bars[0] = 16.0;
            v.peaks[0] = 16.0;
            for k in 0..=15 {
                v.update(t0 + Duration::from_millis(33 * k), false);
            }
            (v.bars[0], v.peaks[0])
        };
        let (slow_bar, slow_peak) = fallen(Falloff::Slowest, Falloff::Slowest);
        let (fast_bar, fast_peak) = fallen(Falloff::Fastest, Falloff::Fastest);
        assert!(slow_bar > fast_bar, "{slow_bar} vs {fast_bar}");
        assert!(slow_peak > fast_peak, "{slow_peak} vs {fast_peak}");
        // 15 frames of 33 ms at 30 pixels a second.
        let (moderate, _) = fallen(Falloff::Moderate, Falloff::Slow);
        assert!(
            (moderate - (16.0 - 30.0 * 0.495)).abs() < 0.01,
            "{moderate}"
        );
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
    fn stereo_is_shown_as_its_mono_mix() {
        let mut v = Vis::new();
        v.push_stereo(&[1.0, 0.0, 0.25, 0.75, -1.0]);
        // Two whole frames; the odd sample at the end is no frame.
        assert_eq!(v.head, 2);
        assert_eq!(&v.history[..2], &[0.5, 0.5]);
        // A long push keeps only the newest frames.
        let long: Vec<f32> = (0..HISTORY * 3).map(|i| (i / 2) as f32).collect();
        v.push_stereo(&long);
        let newest = v.history[(v.head + HISTORY - 1) % HISTORY];
        assert_eq!(newest, (HISTORY * 3 / 2 - 1) as f32);
    }

    #[test]
    fn bands_stay_in_the_spectrum_and_rise_in_pitch() {
        for bands in [Bands::Thick, Bands::Thin] {
            for rate in [8000, 22_050, 44_100, 48_000, 192_000] {
                let a = Analyzer::new(rate, bands.count());
                assert_eq!(a.bands.len(), bands.count());
                assert!(a.bands.windows(2).all(|w| w[0].centre < w[1].centre));
                for b in &a.bands {
                    assert!(b.bins.0 <= b.bins.1 && b.bins.1 <= FFT_SIZE / 2 + 1);
                    assert!(b.centre <= (FFT_SIZE / 2) as f32);
                }
            }
        }
    }

    fn drawn(v: &Vis) -> Canvas {
        let skin = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut c = Canvas::new(W as i32, H, Density::ONE);
        v.draw(&mut c, &skin, Rect::new(0, 0, W as i32, H));
        c
    }

    #[test]
    fn analyzer_styles_colour_bars_by_row_tip_or_height() {
        let skin = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let colors = skin.vis_colors;
        let mut v = Vis::new();
        v.analyzer.peaks = false;
        v.bars[0] = 4.0; // rows 12..16
        let bottom_and_top = |v: &Vis| {
            let c = drawn(v);
            (c.get(0, 15), c.get(0, 12))
        };
        assert_eq!(bottom_and_top(&v), (colors[17], colors[14]), "normal");
        v.analyzer.style = AnalyzerStyle::Fire;
        assert_eq!(bottom_and_top(&v), (colors[5], colors[2]), "fire");
        v.analyzer.style = AnalyzerStyle::Line;
        assert_eq!(bottom_and_top(&v), (colors[14], colors[14]), "line");
    }

    #[test]
    fn thin_bands_are_one_pixel_wide_and_peaks_can_be_hidden() {
        let skin = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let colors = skin.vis_colors;
        let mut v = Vis::new();
        v.bars[0] = 2.0;
        v.peaks[0] = 8.0;
        let c = drawn(&v);
        assert_eq!(c.get(2, 15), colors[17], "thick bars are three wide");
        assert_eq!(c.get(1, 8), colors[23], "the peak");
        v.analyzer.bands = Bands::Thin;
        v.analyzer.peaks = false;
        let c = drawn(&v);
        assert_eq!(c.get(0, 15), colors[17]);
        assert_ne!(c.get(1, 15), colors[17], "thin bars are one wide");
        assert_ne!(c.get(0, 8), colors[23], "no peaks");
    }

    #[test]
    fn the_small_visualiser_fits_the_spectrum_and_the_scope_into_38_by_5() {
        let skin = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let small = |v: &Vis| {
            // Drawn one pixel in, to see that nothing spills out of its rectangle.
            let mut c = Canvas::new(40, 7, Density::ONE);
            v.draw_small(&mut c, &skin, Rect::new(1, 1, 38, 5));
            c
        };
        let colors = &skin.vis_colors;
        let mut v = Vis::new();
        v.bars[0] = H as f32; // full height
        v.bars[1] = 0.0;
        v.bars[18] = H as f32 / 2.0; // the last thick bar, alone in its pair
        let c = small(&v);
        assert_eq!(c.get(0, 0), [0, 0, 0, 0], "outside");
        assert_eq!(c.get(39, 6), [0, 0, 0, 0], "outside");
        // Bar pair 0, three pixels wide: every row lit, the top one in colour 4.
        assert_eq!(c.get(1, 1), colors[4]);
        assert_eq!(c.get(3, 5), colors[17]);
        assert_eq!(c.get(4, 5), colors[0], "the gap");
        // The 10th bar starts at x 36 and is cut at the rectangle's edge.
        assert_eq!(c.get(1 + 36, 5), colors[17]);
        assert_eq!(c.get(1 + 37, 5), colors[17]);
        v.mode = VisMode::Oscilloscope;
        v.scope = [0.0; W];
        let c = small(&v);
        assert_eq!(
            c.get(1, 1 + 2),
            colors[18],
            "a flat trace lies on the middle row"
        );
        assert_eq!(c.get(1, 1), colors[0]);
        v.mode = VisMode::Off;
        assert!(small(&v).image().data.iter().all(|b| *b == 0));
    }

    #[test]
    fn scope_styles_draw_dots_lines_or_solid() {
        let mut v = Vis::new();
        v.mode = VisMode::Oscilloscope;
        // A square wave: high for 10 columns, low for 10.
        for (x, s) in v.scope.iter_mut().enumerate() {
            *s = if (x / 10) % 2 == 0 { 0.9 } else { -0.9 };
        }
        let skin = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let bg = [skin.vis_colors[0], skin.vis_colors[1]];
        let lit = |v: &Vis, x: i32| {
            let c = drawn(v);
            (0..H).filter(|&y| !bg.contains(&c.get(x, y))).count()
        };
        v.scope_style = ScopeStyle::Dots;
        assert_eq!(lit(&v, 10), 1);
        v.scope_style = ScopeStyle::Lines;
        assert!(lit(&v, 10) > 10, "the jump is joined up");
        assert_eq!(lit(&v, 5), 1);
        v.scope_style = ScopeStyle::Solid;
        assert!(lit(&v, 5) > 5, "filled down to the centre");
    }
}
