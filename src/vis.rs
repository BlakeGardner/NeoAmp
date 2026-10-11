//! The little 76x16 visualiser: a fake spectrum analyser and oscilloscope.

use std::time::Instant;

use crate::canvas::{Image, Rect};
use crate::skin::Skin;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisMode {
    Analyzer,
    Oscilloscope,
    Off,
}

pub const BARS: usize = 19;
pub const H: i32 = 16;

pub struct Vis {
    pub mode: VisMode,
    bars: [f32; BARS],
    peaks: [f32; BARS],
    peak_hold: [u8; BARS],
    peak_vel: [f32; BARS],
    last: Option<Instant>,
    rng: u32,
}

impl Vis {
    pub fn new() -> Vis {
        Vis {
            mode: VisMode::Analyzer,
            bars: [0.0; BARS],
            peaks: [0.0; BARS],
            peak_hold: [0; BARS],
            peak_vel: [0.0; BARS],
            last: None,
            rng: 0x2545_F491,
        }
    }

    pub fn next_mode(&mut self) {
        self.mode = match self.mode {
            VisMode::Analyzer => VisMode::Oscilloscope,
            VisMode::Oscilloscope => VisMode::Off,
            VisMode::Off => VisMode::Analyzer,
        };
    }

    fn rand(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn reset(&mut self) {
        self.bars = [0.0; BARS];
        self.peaks = [0.0; BARS];
        self.peak_hold = [0; BARS];
        self.peak_vel = [0.0; BARS];
    }

    /// Advance the fake analyser by one frame while playing.
    pub fn update(&mut self, now: Instant, playing: bool) {
        let dt = self.last.map(|l| (now - l).as_secs_f32()).unwrap_or(0.0).min(0.1);
        self.last = Some(now);
        if !playing {
            return;
        }
        let frames = (dt * 30.0).max(0.0);
        let t = now.elapsed().as_secs_f32();
        for i in 0..BARS {
            // A slowly moving "music-like" envelope: heavier in the bass, with random hits.
            let env = 1.0 - (i as f32 / BARS as f32) * 0.55;
            let wobble = 0.5 + 0.5 * ((t * (1.3 + i as f32 * 0.37)) + i as f32).sin();
            let hit = if self.rand() < 0.12 { self.rand() } else { 0.0 };
            let target = (H as f32) * env * (0.25 + 0.55 * wobble * self.rand() + hit).min(1.0);
            // Bars jump up instantly and fall about one pixel per frame.
            self.bars[i] = if target > self.bars[i] { target } else { (self.bars[i] - 1.0 * frames).max(0.0) };
            // Peaks hold, then fall with acceleration.
            if self.bars[i] >= self.peaks[i] {
                self.peaks[i] = self.bars[i];
                self.peak_hold[i] = 20;
                self.peak_vel[i] = 0.0;
            } else if self.peak_hold[i] > 0 {
                self.peak_hold[i] = self.peak_hold[i].saturating_sub(frames.round() as u8);
            } else {
                self.peak_vel[i] += 0.15 * frames;
                self.peaks[i] = (self.peaks[i] - self.peak_vel[i]).max(0.0);
            }
        }
    }

    /// Draw into `r` (76x16) using the skin's viscolor palette.
    pub fn draw(&self, c: &mut Image, skin: &Skin, r: Rect, now: Instant) {
        if self.mode == VisMode::Off {
            return;
        }
        let colors = &skin.vis_colors;
        c.fill_rect(r.x, r.y, r.w, r.h, colors[0]);
        for y in (1..r.h).step_by(2) {
            for x in (0..r.w).step_by(2) {
                c.put(r.x + x, r.y + y, colors[1]);
            }
        }
        match self.mode {
            VisMode::Analyzer => {
                for i in 0..BARS {
                    let x = r.x + i as i32 * 4;
                    let h = self.bars[i].round().clamp(0.0, H as f32) as i32;
                    for row in (H - h)..H {
                        let color = colors[2 + row as usize];
                        c.fill_rect(x, r.y + row, 3, 1, color);
                    }
                    let p = self.peaks[i].round().clamp(0.0, H as f32) as i32;
                    if p > 0 {
                        c.fill_rect(x, r.y + H - p, 3, 1, colors[23]);
                    }
                }
            }
            VisMode::Oscilloscope => {
                let t = now.elapsed().as_secs_f32();
                let mut prev: Option<i32> = None;
                for x in 0..r.w {
                    let fx = x as f32;
                    let v = (fx * 0.35 + t * 9.0).sin() * 0.55
                        + (fx * 0.9 - t * 23.0).sin() * 0.25
                        + (fx * 2.1 + t * 41.0).sin() * 0.12;
                    let y = (8.0 - v * 7.0).round().clamp(0.0, 15.0) as i32;
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
