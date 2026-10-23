//! What MilkDrop draws on top of each warped frame, as lists of vertices: custom shapes and
//! waves, the basic waveform, the darkened centre and the borders (`DrawCustomShapes`,
//! `DrawCustomWaves`, `DrawWave` and `DrawSprites` in MilkDrop's `milkdropfs.cpp`), and the
//! motion vectors drawn into the previous frame before it is warped (`DrawMotionVectors`).
//!
//! Positions are in MilkDrop's 2D space: x from -1 (left) to 1, y from -1 (top) to 1 (bottom).
//! The GPU turns y over. Colours go through [`d3d`], which keeps MilkDrop's habit of wrapping
//! values outside 0..1 rather than clamping them.

use std::ops::Range;

use super::eel::Globals;
use super::preset::Values;
use super::sound::{Levels, SAMPLES};
use super::state::{FrameVars, Moment, Pf, PresetState, Q_COUNT};

/// A vertex of the drawing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub color: [f32; 4],
    /// Where a textured shape samples the previous frame.
    pub uv: [f32; 2],
}

/// How a draw call blends with what is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Alpha,
    Add,
}

/// What a draw call draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One pixel wide lines, two vertices each.
    Lines,
    /// Triangles, three vertices each.
    Triangles,
    /// Triangles that sample the previous frame and multiply it by their colour.
    Textured,
}

/// A run of vertices drawn the same way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub kind: Kind,
    pub blend: Blend,
    pub range: Range<u32>,
}

/// Vertices and the calls that draw them, in order.
#[derive(Clone, Debug, Default)]
pub struct Drawing {
    pub vertices: Vec<Vertex>,
    pub calls: Vec<Call>,
}

impl Drawing {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.calls.clear();
    }

    /// Add vertices to draw as `kind`, joining the last call when it draws the same way.
    fn push(&mut self, kind: Kind, blend: Blend, vertices: impl IntoIterator<Item = Vertex>) {
        let start = self.len();
        self.vertices.extend(vertices);
        let end = self.len();
        if start == end {
            return;
        }
        match self.calls.last_mut() {
            Some(c) if c.kind == kind && c.blend == blend && c.range.end == start => {
                c.range.end = end;
            }
            _ => self.calls.push(Call {
                kind,
                blend,
                range: start..end,
            }),
        }
    }

    fn len(&self) -> u32 {
        // Far fewer than 2^32 vertices are ever drawn in a frame.
        u32::try_from(self.vertices.len()).unwrap_or(u32::MAX)
    }

    /// A line through `points` (a line strip), as pairs of vertices.
    fn line_strip(&mut self, blend: Blend, points: &[Vertex], offset: [f32; 2]) {
        let moved = |v: &Vertex| Vertex {
            pos: [v.pos[0] + offset[0], v.pos[1] + offset[1]],
            ..*v
        };
        self.push(
            Kind::Lines,
            blend,
            points.windows(2).flat_map(|w| [moved(&w[0]), moved(&w[1])]),
        );
    }

    /// A triangle fan around `points[0]`, as triangles.
    fn triangle_fan(&mut self, kind: Kind, blend: Blend, points: &[Vertex]) {
        let Some((centre, rim)) = points.split_first() else {
            return;
        };
        self.push(
            kind,
            blend,
            rim.windows(2).flat_map(|w| [*centre, w[0], w[1]]),
        );
    }

    /// Square dots `size` pixels across at each point, as Direct3D drew point lists.
    fn dots(&mut self, blend: Blend, points: &[Vertex], size: f32, canvas: &Canvas) {
        let (hx, hy) = (size / canvas.size.0 as f32, size / canvas.size.1 as f32);
        self.push(
            Kind::Triangles,
            blend,
            points.iter().flat_map(|v| {
                let [x, y] = v.pos;
                let at = |dx: f32, dy: f32| Vertex {
                    pos: [x + dx, y + dy],
                    ..*v
                };
                let (a, b, c, d) = (at(-hx, -hy), at(hx, -hy), at(hx, hy), at(-hx, hy));
                [a, b, c, a, c, d]
            }),
        );
    }
}

/// The size and shape of the picture MilkDrop draws into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    /// Pixels across and down.
    pub size: (u32, u32),
    /// MilkDrop's `m_fAspectX` and `m_fAspectY`: 1, or less along the longer side.
    pub aspect: (f32, f32),
}

impl Canvas {
    #[must_use]
    pub fn new(size: (u32, u32)) -> Canvas {
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        Canvas {
            size,
            aspect: (
                if h > w { w / h } else { 1.0 },
                if w > h { h / w } else { 1.0 },
            ),
        }
    }

    /// Two pixels across and down, in MilkDrop's 2D space: how far thick lines are drawn
    /// apart.
    fn two_pixels(&self) -> (f32, f32) {
        (
            2.0 / self.size.0.max(1) as f32,
            2.0 / self.size.1.max(1) as f32,
        )
    }
}

/// A colour component as MilkDrop packed it into a Direct3D colour: truncated to a byte, which
/// wraps rather than clamps.
#[must_use]
pub fn d3d(v: f64) -> f32 {
    // A saturating cast; the low byte is all Direct3D kept.
    f32::from(((v * 255.0) as i64 & 0xFF) as u8) / 255.0
}

fn rgba(c: [f64; 4], alpha_mult: f64) -> [f32; 4] {
    [d3d(c[0]), d3d(c[1]), d3d(c[2]), d3d(c[3] * alpha_mult)]
}

/// MilkDrop's `SmoothWave`: a better-than-linear smooth that roughly doubles the points.
fn smooth_wave(input: &[Vertex], out: &mut Vec<Vertex>) {
    const C1: f32 = -0.15;
    const C2: f32 = 1.15;
    const C3: f32 = 1.15;
    const C4: f32 = -0.15;
    const INV_SUM: f32 = 1.0 / (C1 + C2 + C3 + C4);
    out.clear();
    let n = input.len();
    if n == 0 {
        return;
    }
    let mut below = 0;
    let mut above2 = 1.min(n - 1);
    for i in 0..n - 1 {
        let above = above2;
        above2 = (i + 2).min(n - 1);
        out.push(input[i]);
        let mix = |k: usize| {
            (C1 * input[below].pos[k]
                + C2 * input[i].pos[k]
                + C3 * input[above].pos[k]
                + C4 * input[above2].pos[k])
                * INV_SUM
        };
        out.push(Vertex {
            pos: [mix(0), mix(1)],
            ..input[i]
        });
        below = i;
    }
    out.push(input[n - 1]);
}

/// The sound data the waves are drawn from: each channel's waveform and spectrum, [`SAMPLES`]
/// values each.
#[derive(Clone, Copy, Debug)]
pub struct Audio<'a> {
    pub wave: [&'a [f32]; 2],
    pub spectrum: [&'a [f32]; 2],
    pub levels: Levels,
}

/// Scratch space reused from frame to frame.
#[derive(Debug, Default)]
pub struct Scratch {
    points: Vec<Vertex>,
    smoothed: Vec<Vertex>,
    old_points: Vec<Vertex>,
    data: [Vec<f32>; 2],
}

/// Draw `state`'s custom shapes. `alpha_mult` fades them while presets blend.
#[allow(clippy::too_many_arguments)]
pub fn custom_shapes(
    d: &mut Drawing,
    state: &mut PresetState,
    globals: &mut Globals,
    m: &Moment,
    q: &[f64; Q_COUNT],
    alpha_mult: f64,
    canvas: &Canvas,
    scratch: &mut Scratch,
) {
    let (x_inc, y_inc) = canvas.two_pixels();
    let aspect_y = f64::from(canvas.aspect.1);
    for shape in state.shapes.iter_mut().flatten() {
        for instance in 0..shape.settings.instances {
            let s = shape.run_instance(globals, m, q, instance);
            // MilkDrop read the sides as an int.
            let sides = (s.sides as i64).clamp(3, 100) as usize;
            let blend = if s.additive { Blend::Add } else { Blend::Alpha };
            let cx = s.x * 2.0 - 1.0;
            let cy = s.y * -2.0 + 1.0;
            let centre = Vertex {
                pos: [cx as f32, cy as f32],
                color: rgba(s.color, alpha_mult),
                uv: [0.5, 0.5],
            };
            let rim_color = rgba(s.color2, alpha_mult);
            let points = &mut scratch.points;
            points.clear();
            points.push(centre);
            for j in 1..=sides {
                let t = (j - 1) as f64 / sides as f64;
                let a = t * std::f64::consts::TAU + s.ang + std::f64::consts::FRAC_PI_4;
                let ta = t * std::f64::consts::TAU + s.tex_ang + std::f64::consts::FRAC_PI_4;
                points.push(Vertex {
                    pos: [
                        (cx + s.rad * a.cos() * aspect_y) as f32,
                        (cy + s.rad * a.sin()) as f32,
                    ],
                    color: rim_color,
                    uv: [
                        (0.5 + 0.5 * ta.cos() / s.tex_zoom * aspect_y) as f32,
                        (0.5 + 0.5 * ta.sin() / s.tex_zoom) as f32,
                    ],
                });
            }
            points.push(points[1]);
            let kind = if s.textured {
                Kind::Textured
            } else {
                Kind::Triangles
            };
            d.triangle_fan(kind, blend, points);

            if s.border[3] > 0.0 {
                let border = rgba(s.border, alpha_mult);
                for p in points.iter_mut() {
                    p.color = border;
                }
                let rim = &points[1..];
                let its = if s.thick { 4 } else { 1 };
                for offset in thick_offsets(its, x_inc, y_inc) {
                    d.line_strip(blend, rim, offset);
                }
            }
        }
    }
}

/// Where MilkDrop drew the copies of a thick line or dot: to the right, down-right and down.
fn thick_offsets(its: usize, x_inc: f32, y_inc: f32) -> impl Iterator<Item = [f32; 2]> {
    [[0.0, 0.0], [x_inc, 0.0], [x_inc, y_inc], [0.0, y_inc]]
        .into_iter()
        .take(its)
}

/// Draw `state`'s custom waves. `alpha_mult` fades them while presets blend; `wave_scale` is
/// the preset's `fWaveScale`.
#[allow(clippy::too_many_arguments)]
pub fn custom_waves(
    d: &mut Drawing,
    state: &mut PresetState,
    globals: &mut Globals,
    m: &Moment,
    q: &[f64; Q_COUNT],
    alpha_mult: f64,
    canvas: &Canvas,
    audio: &Audio<'_>,
    scratch: &mut Scratch,
) {
    let wave_scale = state.preset.values.get("wave_scale");
    let (x_inc, y_inc) = canvas.two_pixels();
    let (inv_ax, inv_ay) = (
        1.0 / f64::from(canvas.aspect.0),
        1.0 / f64::from(canvas.aspect.1),
    );
    for wave in state.waves.iter_mut().flatten() {
        let f = wave.run_frame(globals, m, q);
        let s = wave.settings;
        // MilkDrop read the sample count as an int.
        let n = (f.samples as i64).min(SAMPLES as i64);
        if !(n >= 2 || (s.dots && n >= 1)) {
            continue;
        }
        let n = n as usize;
        let max = SAMPLES as i32;
        let mult = if s.spectrum { 0.15 } else { 0.004 } * s.scaling * wave_scale;
        let sources = if s.spectrum {
            audio.spectrum
        } else {
            audio.wave
        };
        let (j0, j1) = if s.spectrum {
            (0, 0)
        } else {
            let mid = (max - n as i32) / 2;
            (mid - s.sep / 2, mid + s.sep / 2)
        };
        let t = if s.spectrum {
            f64::from(max - s.sep) / n as f64
        } else {
            1.0
        };
        let mix1 = (s.smoothing * 0.98).max(0.0).sqrt();
        let mix2 = 1.0 - mix1;
        let read = |ch: usize, j: usize, start: i32| -> f64 {
            // In range for any sensible sep; out of range reads as silence.
            let i = (j as f64 * t) as i64 + i64::from(start);
            usize::try_from(i)
                .ok()
                .and_then(|i| sources[ch].get(i))
                .map_or(0.0, |v| f64::from(*v))
        };
        let [d0, d1] = &mut scratch.data;
        for (ch, (data, start)) in [(&mut *d0, j0), (&mut *d1, j1)].into_iter().enumerate() {
            data.clear();
            data.push(read(ch, 0, start) as f32);
            for j in 1..n {
                let prev = f64::from(data[j - 1]);
                data.push((read(ch, j, start) * mix2 + prev * mix1) as f32);
            }
            // Smoothed again backwards, which fixes the start and end not matching.
            for j in (0..n.saturating_sub(1)).rev() {
                data[j] = (f64::from(data[j]) * mix2 + f64::from(data[j + 1]) * mix1) as f32;
            }
            for v in data.iter_mut() {
                *v = (f64::from(*v) * mult) as f32;
            }
        }
        let points = &mut scratch.points;
        points.clear();
        let step = if n > 1 { 1.0 / (n - 1) as f64 } else { 0.0 };
        for j in 0..n {
            let values = (f64::from(scratch.data[0][j]), f64::from(scratch.data[1][j]));
            let p = wave.run_point(globals, j as f64 * step, values, f.color);
            points.push(Vertex {
                pos: [
                    ((p.x * 2.0 - 1.0) * inv_ax) as f32,
                    ((p.y * -2.0 + 1.0) * inv_ay) as f32,
                ],
                color: rgba(p.color, alpha_mult),
                uv: [0.0; 2],
            });
        }
        let blend = if s.additive { Blend::Add } else { Blend::Alpha };
        if s.dots {
            let size =
                if canvas.size.0 >= 1024 { 2.0 } else { 1.0 } + if s.thick { 1.0 } else { 0.0 };
            d.dots(blend, points, size, canvas);
        } else {
            smooth_wave(points, &mut scratch.smoothed);
            let its = if s.thick { 4 } else { 1 };
            for offset in thick_offsets(its, x_inc, y_inc) {
                d.line_strip(blend, &scratch.smoothed, offset);
            }
        }
    }
}

/// The basic waveform's settings that come from the preset file rather than its code.
#[derive(Clone, Copy, Debug)]
pub struct WaveLook {
    pub scale: f64,
    pub smoothing: f64,
    pub mod_alpha_by_volume: bool,
    pub mod_alpha_start: f64,
    pub mod_alpha_end: f64,
}

impl WaveLook {
    #[must_use]
    pub fn of(values: &Values) -> WaveLook {
        WaveLook {
            scale: values.get("wave_scale"),
            smoothing: values.get("wave_smoothing"),
            mod_alpha_by_volume: values.get("modwavealphabyvolume") != 0.0,
            mod_alpha_start: values.get("modwavealphastart"),
            mod_alpha_end: values.get("modwavealphaend"),
        }
    }
}

/// One wave mode's points, alpha, and where its line breaks in two (mode 7's two lines).
struct WaveShape {
    alpha: f64,
    brk: Option<usize>,
}

/// Points for one of MilkDrop's eight basic wave modes (`DrawWave`), into `v`.
#[allow(clippy::too_many_arguments)]
fn wave_mode(
    mode: i64,
    v: &mut Vec<Vertex>,
    f: &FrameVars,
    look: &WaveLook,
    l: &[f32],
    r: &[f32],
    levels: &Levels,
    canvas: &Canvas,
    time: f64,
    blending: bool,
) -> WaveShape {
    v.clear();
    let n_all = l.len();
    let at = |data: &[f32], i: usize| f64::from(data[i % n_all.max(1)]);
    let (ax, ay) = (f64::from(canvas.aspect.0), f64::from(canvas.aspect.1));
    let pos_x = f.get(Pf::WaveX) * 2.0 - 1.0;
    let pos_y = f.get(Pf::WaveY) * 2.0 - 1.0;
    let mut param = f.get(Pf::WaveMystery);
    if matches!(mode, 0 | 1 | 4) && !(-1.0..=1.0).contains(&param) {
        param = param * 0.5 + 0.5;
        param -= param.floor();
        param = param.abs() * 2.0 - 1.0;
    }
    let vol = f64::from(levels.bass + levels.mid + levels.treb) / 3.0;
    let mut alpha = f.get(Pf::WaveA);
    let tex = canvas.size.0;
    let sized = |a: [f64; 4]| match tex {
        0..512 => a[0],
        512..1024 => a[1],
        1024..2048 => a[2],
        _ => a[3],
    };
    match mode {
        1 => alpha *= 1.25,
        2 | 5 => alpha *= sized([0.07, 0.09, 0.11, 0.13]),
        3 => {
            alpha = sized([0.075, 0.15, 0.22, 0.33]) * 1.3 * f64::from(levels.treb).powi(2);
        }
        _ => {}
    }
    if look.mod_alpha_by_volume {
        alpha *= (vol - look.mod_alpha_start) / (look.mod_alpha_end - look.mod_alpha_start);
    }
    let alpha = alpha.clamp(0.0, 1.0);
    let point = |x: f64, y: f64| Vertex {
        pos: [x as f32, y as f32],
        ..Vertex::default()
    };
    let mut brk = None;
    match mode {
        0 => {
            let n = n_all / 2 + 1;
            let offset = (n_all - n) / 2;
            let inv = 1.0 / (n - 1) as f64;
            for i in 0..n - 1 {
                let mut rad = 0.5 + 0.4 * at(r, i + offset) + param;
                let ang = i as f64 * inv * std::f64::consts::TAU + time * 0.2;
                if i < n / 10 {
                    let mix = i as f64 / (n as f64 * 0.1);
                    let mix = 0.5 - 0.5 * (mix * std::f64::consts::PI).cos();
                    let rad2 = 0.5 + 0.4 * at(r, i + n + offset) + param;
                    rad = rad2 * (1.0 - mix) + rad * mix;
                }
                v.push(point(
                    rad * ang.cos() * ay + pos_x,
                    rad * ang.sin() * ax + pos_y,
                ));
            }
            // The loop is closed, except while blending (MilkDrop's choice).
            if !blending && let Some(&first) = v.first() {
                v.push(first);
            }
        }
        1 => {
            for i in 0..n_all / 2 {
                let rad = 0.53 + 0.43 * at(r, i) + param;
                let ang = at(l, i + 32) * 0.5 * std::f64::consts::PI + time * 2.3;
                v.push(point(
                    rad * ang.cos() * ay + pos_x,
                    rad * ang.sin() * ax + pos_y,
                ));
            }
        }
        2 | 3 => {
            for i in 0..n_all {
                v.push(point(at(r, i) * ay + pos_x, at(l, i + 32) * ax + pos_y));
            }
        }
        4 => {
            let n = n_all.min(tex as usize / 3).max(2);
            let offset = (n_all - n.min(n_all)) / 2;
            let inv = 1.0 / n as f64;
            let w1 = 0.45 + 0.5 * (param * 0.5 + 0.5);
            let w2 = 1.0 - w1;
            for i in 0..n {
                let mut x = -1.0 + 2.0 * i as f64 * inv + pos_x + at(r, i + 25 + offset) * 0.44;
                let mut y = at(l, i + offset) * 0.47 + pos_y;
                if i > 1 {
                    let (p1, p2) = (v[i - 1].pos, v[i - 2].pos);
                    x = x * w2 + w1 * (f64::from(p1[0]) * 2.0 - f64::from(p2[0]));
                    y = y * w2 + w1 * (f64::from(p1[1]) * 2.0 - f64::from(p2[1]));
                }
                v.push(point(x, y));
            }
        }
        5 => {
            let (c, s) = ((time * 0.3).cos(), (time * 0.3).sin());
            for i in 0..n_all {
                let x0 = at(r, i) * at(l, i + 32) + at(l, i) * at(r, i + 32);
                let y0 = at(r, i) * at(r, i) - at(l, i + 32) * at(l, i + 32);
                v.push(point(
                    (x0 * c - y0 * s) * ay + pos_x,
                    (x0 * s + y0 * c) * ax + pos_y,
                ));
            }
        }
        6 | 7 => {
            let n = (n_all / 2).min(tex as usize / 3).max(2);
            let offset = (n_all - n) / 2;
            let ang = 1.57 * param;
            let (dx, dy) = (ang.cos(), ang.sin());
            let mut ex = [
                pos_x * (ang + 1.57).cos() - dx * 3.0,
                pos_x * (ang + 1.57).cos() + dx * 3.0,
            ];
            let mut ey = [
                pos_x * (ang + 1.57).sin() - dy * 3.0,
                pos_x * (ang + 1.57).sin() + dy * 3.0,
            ];
            for i in 0..2 {
                for j in 0..4 {
                    let t = match j {
                        0 if ex[i] > 1.1 => Some((1.1 - ex[1 - i]) / (ex[i] - ex[1 - i])),
                        1 if ex[i] < -1.1 => Some((-1.1 - ex[1 - i]) / (ex[i] - ex[1 - i])),
                        2 if ey[i] > 1.1 => Some((1.1 - ey[1 - i]) / (ey[i] - ey[1 - i])),
                        3 if ey[i] < -1.1 => Some((-1.1 - ey[1 - i]) / (ey[i] - ey[1 - i])),
                        _ => None,
                    };
                    if let Some(t) = t {
                        let (ddx, ddy) = (ex[i] - ex[1 - i], ey[i] - ey[1 - i]);
                        ex[i] = ex[1 - i] + ddx * t;
                        ey[i] = ey[1 - i] + ddy * t;
                    }
                }
            }
            let dx = (ex[1] - ex[0]) / n as f64;
            let dy = (ey[1] - ey[0]) / n as f64;
            let ang2 = dy.atan2(dx);
            let (px, py) = ((ang2 + 1.57).cos(), (ang2 + 1.57).sin());
            if mode == 6 {
                for i in 0..n {
                    let s = 0.25 * at(l, i + offset);
                    v.push(point(
                        ex[0] + dx * i as f64 + px * s,
                        ey[0] + dy * i as f64 + py * s,
                    ));
                }
            } else {
                let sep = (pos_y * 0.5 + 0.5).powi(2);
                for (data, sign) in [(l, 1.0), (r, -1.0)] {
                    for i in 0..n {
                        let s = 0.25 * at(data, i + offset) + sign * sep;
                        v.push(point(
                            ex[0] + dx * i as f64 + px * s,
                            ey[0] + dy * i as f64 + py * s,
                        ));
                    }
                }
                brk = Some(n);
            }
        }
        _ => {}
    }
    WaveShape { alpha, brk }
}

/// The basic waveform (`DrawWave`). While blending, `old` is the outgoing preset's frame and
/// wave mode, and `mix` how far the new one has come in.
#[allow(clippy::too_many_arguments)]
pub fn basic_wave(
    d: &mut Drawing,
    f: &FrameVars,
    look: &WaveLook,
    old: Option<(&FrameVars, i64)>,
    mix: f64,
    canvas: &Canvas,
    audio: &Audio<'_>,
    time: f64,
    scratch: &mut Scratch,
) {
    // Smoothed and scaled for display, as MilkDrop prepared `mysound.fWaveform`.
    let scale = look.scale / 128.0;
    let smooth = look.smoothing;
    let mix1 = scale * (1.0 - smooth);
    for (data, src) in scratch.data.iter_mut().zip(audio.wave) {
        data.clear();
        let mut prev = 0.0;
        for (i, s) in src.iter().enumerate() {
            let v = if i == 0 {
                f64::from(*s) * scale
            } else {
                f64::from(*s) * mix1 + prev * smooth
            };
            data.push(v as f32);
            prev = v;
        }
    }
    if scratch.data[0].is_empty() {
        return;
    }
    let mode = (f.get(Pf::WaveMode) as i64) % 8;
    let blending = old.is_some();
    let (l, r) = (&scratch.data[0], &scratch.data[1]);
    let mut points = std::mem::take(&mut scratch.points);
    let new = wave_mode(
        mode,
        &mut points,
        f,
        look,
        l,
        r,
        &audio.levels,
        canvas,
        time,
        blending,
    );
    let mut alpha = new.alpha;
    let mut brk = new.brk;
    if let Some((old_f, old_mode)) = old.filter(|(_, m)| *m != mode) {
        let mut old_points = std::mem::take(&mut scratch.old_points);
        let old_shape = wave_mode(
            old_mode % 8,
            &mut old_points,
            old_f,
            look,
            l,
            r,
            &audio.levels,
            canvas,
            time,
            true,
        );
        if !old_points.is_empty() && !points.is_empty() {
            let mix2 = 1.0 - mix;
            let m = (old_points.len() - 1) as f64 / points.len() as f64;
            for (i, p) in points.iter_mut().enumerate() {
                let fi = i as f64 * m;
                let ni = fi as usize;
                let t = (fi - ni as f64) as f32;
                let a = old_points[ni.min(old_points.len() - 1)].pos;
                let (x, y) = if Some(ni + 1) == old_shape.brk {
                    brk = Some(i + 1);
                    (a[0], a[1])
                } else {
                    let b = old_points[(ni + 1).min(old_points.len() - 1)].pos;
                    (a[0] * (1.0 - t) + b[0] * t, a[1] * (1.0 - t) + b[1] * t)
                };
                p.pos[0] = (f64::from(p.pos[0]) * mix + f64::from(x) * mix2) as f32;
                p.pos[1] = (f64::from(p.pos[1]) * mix + f64::from(y) * mix2) as f32;
            }
            alpha = alpha * mix + old_shape.alpha * (1.0 - mix);
        }
        scratch.old_points = old_points;
    }

    let mut cr = f.get(Pf::WaveR).clamp(0.0, 1.0);
    let mut cg = f.get(Pf::WaveG).clamp(0.0, 1.0);
    let mut cb = f.get(Pf::WaveB).clamp(0.0, 1.0);
    if f.on(Pf::WaveBrighten) {
        let max = cr.max(cg).max(cb);
        if max > 0.01 {
            cr /= max;
            cg /= max;
            cb /= max;
        }
    }
    if alpha >= 0.004 && !points.is_empty() {
        let color = [d3d(cr), d3d(cg), d3d(cb), d3d(alpha)];
        for p in &mut points {
            p.color = color;
            // MilkDrop turned the basic wave upside down, to match its first versions.
            p.pos[1] = -p.pos[1];
        }
        let blend = if f.on(Pf::WaveAdditive) {
            Blend::Add
        } else {
            Blend::Alpha
        };
        let dots = f.on(Pf::WaveUseDots);
        let its = if (f.on(Pf::WaveThick) || dots) && canvas.size.0 >= 512 {
            4
        } else {
            1
        };
        let (x_inc, y_inc) = canvas.two_pixels();
        let parts: [&[Vertex]; 2] = match brk {
            Some(b) if b < points.len() => [&points[..b], &points[b..]],
            _ => [&points[..], &[]],
        };
        for part in parts {
            if part.is_empty() {
                continue;
            }
            smooth_wave(part, &mut scratch.smoothed);
            for offset in thick_offsets(its, x_inc, y_inc) {
                if dots {
                    let moved: Vec<Vertex> = scratch
                        .smoothed
                        .iter()
                        .map(|v| Vertex {
                            pos: [v.pos[0] + offset[0], v.pos[1] + offset[1]],
                            ..*v
                        })
                        .collect();
                    d.dots(blend, &moved, 1.0, canvas);
                } else {
                    d.line_strip(blend, &scratch.smoothed, offset);
                }
            }
        }
    }
    scratch.points = points;
}

/// The darkened centre and the outer and inner borders (`DrawSprites`).
pub fn sprites(d: &mut Drawing, f: &FrameVars, canvas: &Canvas) {
    let ay = canvas.aspect.1;
    if f.on(Pf::DarkenCenter) {
        let centre = Vertex {
            color: [0.0, 0.0, 0.0, 3.0 / 32.0],
            ..Vertex::default()
        };
        let rim = |x: f32, y: f32| Vertex {
            pos: [x, y],
            color: [0.0; 4],
            ..Vertex::default()
        };
        let h = 0.05;
        let fan = [
            centre,
            rim(-h * ay, 0.0),
            rim(0.0, -h),
            rim(h * ay, 0.0),
            rim(0.0, h),
            rim(-h * ay, 0.0),
        ];
        d.triangle_fan(Kind::Triangles, Blend::Alpha, &fan);
    }
    let outer = f.get(Pf::ObSize) as f32;
    let inner = f.get(Pf::IbSize) as f32;
    let borders = [
        ([Pf::ObR, Pf::ObG, Pf::ObB, Pf::ObA], 1.0 - outer, 1.0),
        (
            [Pf::IbR, Pf::IbG, Pf::IbB, Pf::IbA],
            1.0 - outer - inner,
            1.0 - outer,
        ),
    ];
    for (vars, inner_rad, outer_rad) in borders {
        let c = vars.map(|v| f.get(v));
        if c[3] <= 0.001 {
            continue;
        }
        let color = [d3d(c[0]), d3d(c[1]), d3d(c[2]), d3d(c[3])];
        let mut quad = [
            [inner_rad, inner_rad],
            [outer_rad, outer_rad],
            [outer_rad, -outer_rad],
            [inner_rad, -inner_rad],
        ];
        for _ in 0..4 {
            let v = quad.map(|pos| Vertex {
                pos,
                color,
                uv: [0.0; 2],
            });
            d.triangle_fan(Kind::Triangles, Blend::Alpha, &v);
            // A quarter turn for the next side.
            for p in &mut quad {
                *p = [-p[1], p[0]];
            }
        }
    }
}

/// The warp mesh's texture coordinates, for the motion vectors to follow.
#[derive(Clone, Copy, Debug)]
pub struct Mesh<'a> {
    /// `(grid_x + 1) * (grid_y + 1)` coordinates, row by row.
    pub uv: &'a [[f32; 2]],
    pub grid: (usize, usize),
}

impl Mesh<'_> {
    /// MilkDrop's `ReversePropagatePoint`: where the last warp took the point from.
    fn reverse(&self, fx: f32, fy: f32) -> Option<(f32, f32)> {
        let (gx, gy) = self.grid;
        let y0 = (fy * gy as f32) as i64;
        let x0 = (fx * gx as f32) as i64;
        let (dy, dx) = (fy * gy as f32 - y0 as f32, fx * gx as f32 - x0 as f32);
        if x0 < 0 || y0 < 0 || x0 + 1 > gx as i64 || y0 + 1 > gy as i64 {
            return None;
        }
        let (x0, y0) = (x0 as usize, y0 as usize);
        let at = |x: usize, y: usize| self.uv.get(y * (gx + 1) + x).copied().unwrap_or([0.0; 2]);
        let mut tu = 0.0;
        let mut tv = 0.0;
        for (x, y, w) in [
            (x0, y0, (1.0 - dx) * (1.0 - dy)),
            (x0 + 1, y0, dx * (1.0 - dy)),
            (x0, y0 + 1, (1.0 - dx) * dy),
            (x0 + 1, y0 + 1, dx * dy),
        ] {
            let [u, v] = at(x, y);
            tu += u * w;
            tv += v * w;
        }
        Some((tu, 1.0 - tv))
    }
}

/// The motion vectors (`DrawMotionVectors`): little lines showing how the picture moves,
/// drawn into the previous frame before it is warped.
pub fn motion_vectors(d: &mut Drawing, f: &FrameVars, mesh: &Mesh<'_>, canvas: &Canvas) {
    if f.get(Pf::MvA) < 0.001 {
        return;
    }
    // MilkDrop read the counts as ints, at most 64 across and 48 down.
    let mut nx = f.get(Pf::MvX) as i64;
    let mut ny = f.get(Pf::MvY) as i64;
    let mut dx = (f.get(Pf::MvX) - nx as f64) as f32;
    let mut dy = (f.get(Pf::MvY) - ny as f64) as f32;
    if nx > 64 {
        nx = 64;
        dx = 0.0;
    }
    if ny > 48 {
        ny = 48;
        dy = 0.0;
    }
    if nx <= 0 || ny <= 0 {
        return;
    }
    let (dx, dy) = (dx.clamp(0.0, 1.0), dy.clamp(0.0, 1.0));
    let dx2 = f.get(Pf::MvDx) as f32;
    let dy2 = f.get(Pf::MvDy) as f32;
    let len_mult = f.get(Pf::MvL) as f32;
    let min_len = 1.0 / canvas.size.0.max(1) as f32;
    let c = [Pf::MvR, Pf::MvG, Pf::MvB, Pf::MvA].map(|v| d3d(f.get(v)));
    let vertex = |x: f32, y: f32| Vertex {
        pos: [x * 2.0 - 1.0, y * 2.0 - 1.0],
        color: c,
        uv: [0.0; 2],
    };
    let mut lines = Vec::new();
    for y in 0..ny {
        let fy = (y as f32 + 0.25) / (ny as f32 + dy + 0.25 - 1.0) - dy2;
        if !(0.0001..0.9999).contains(&fy) {
            continue;
        }
        for x in 0..nx {
            let fx = (x as f32 + 0.25) / (nx as f32 + dx + 0.25 - 1.0) + dx2;
            if !(0.0001..0.9999).contains(&fx) {
                continue;
            }
            let Some((fx2, fy2)) = mesh.reverse(fx, fy) else {
                continue;
            };
            let (mut ddx, mut ddy) = ((fx2 - fx) * len_mult, (fy2 - fy) * len_mult);
            let len = (ddx * ddx + ddy * ddy).sqrt();
            if len <= min_len {
                if len > 0.000_000_01 {
                    let k = min_len / len;
                    ddx *= k;
                    ddy *= k;
                } else {
                    ddx = min_len;
                    ddy = min_len;
                }
            }
            lines.push(vertex(fx, fy));
            lines.push(vertex(fx + ddx, fy + ddy));
        }
    }
    d.push(Kind::Lines, Blend::Alpha, lines);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::milkdrop::eel::Globals;
    use crate::milkdrop::preset::Preset;

    fn moment() -> Moment {
        Moment {
            time: 1.0,
            frame: 30.0,
            fps: 30.0,
            progress: 0.0,
            levels: Levels::default(),
            mesh: (48, 36),
            pixels: (640, 480),
            inv_aspect: (1.0, 4.0 / 3.0),
        }
    }

    fn sine() -> Vec<f32> {
        (0..SAMPLES)
            .map(|i| 60.0 * (i as f32 * 0.1).sin())
            .collect()
    }

    #[test]
    fn colours_wrap_like_direct3d() {
        assert_eq!(d3d(1.0), 1.0);
        assert_eq!(d3d(0.0), 0.0);
        assert_eq!(d3d(0.5), 127.0 / 255.0);
        // 1.5 * 255 = 382, and 382 & 255 = 126.
        assert_eq!(d3d(1.5), 126.0 / 255.0);
        assert_eq!(d3d(-0.1), 231.0 / 255.0);
    }

    #[test]
    fn calls_merge_when_they_draw_the_same_way() {
        let mut d = Drawing::default();
        let v = Vertex::default();
        d.line_strip(Blend::Alpha, &[v, v, v], [0.0; 2]);
        d.line_strip(Blend::Alpha, &[v, v], [0.0; 2]);
        d.line_strip(Blend::Add, &[v, v], [0.0; 2]);
        d.triangle_fan(Kind::Triangles, Blend::Add, &[v, v, v, v]);
        assert_eq!(d.vertices.len(), 4 + 2 + 2 + 6);
        let kinds: Vec<_> = d
            .calls
            .iter()
            .map(|c| (c.kind, c.blend, c.range.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                (Kind::Lines, Blend::Alpha, 0..6),
                (Kind::Lines, Blend::Add, 6..8),
                (Kind::Triangles, Blend::Add, 8..14),
            ]
        );
    }

    #[test]
    fn smoothing_doubles_the_points_and_keeps_the_ends() {
        let pts: Vec<Vertex> = (0..4)
            .map(|i| Vertex {
                pos: [i as f32, 0.0],
                ..Vertex::default()
            })
            .collect();
        let mut out = Vec::new();
        smooth_wave(&pts, &mut out);
        assert_eq!(out.len(), 7);
        assert_eq!(out[0].pos, [0.0, 0.0]);
        assert_eq!(out[6].pos, [3.0, 0.0]);
        // A straight line stays straight: the inserted point is half way.
        assert!((out[3].pos[0] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn a_shape_is_a_fan_with_a_border() {
        let p = Preset::parse(
            "[preset00]\nshapecode_0_enabled=1\nshapecode_0_sides=5\nshapecode_0_rad=0.5\n\
             shapecode_0_border_a=1\nshapecode_0_thickOutline=1\nshapecode_0_num_inst=2\n",
        );
        let mut g = Globals::new(3);
        let m = moment();
        let mut s = PresetState::new(p, &mut g, &m);
        let f = s.run_frame(&mut g, &m);
        let mut d = Drawing::default();
        let canvas = Canvas::new((640, 480));
        custom_shapes(
            &mut d,
            &mut s,
            &mut g,
            &m,
            &f.q,
            1.0,
            &canvas,
            &mut Scratch::default(),
        );
        // Per instance: 5 triangles, then 4 thick copies of a 5-segment outline.
        assert_eq!(d.vertices.len(), 2 * (5 * 3 + 4 * 5 * 2));
        assert_eq!(d.calls.len(), 4);
        // The shape sits at (0.5, 0.5): the centre of MilkDrop's space.
        assert_eq!(d.vertices[0].pos, [0.0, 0.0]);
        // Default colours: red inside fading to transparent green.
        assert_eq!(d.vertices[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(d.vertices[1].color, [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_custom_wave_follows_its_per_point_code() {
        let p = Preset::parse(
            "[preset00]\nwavecode_0_enabled=1\nwavecode_0_samples=10\n\
             wave_0_per_point1=x = sample; y = 0.25;\n",
        );
        let mut g = Globals::new(3);
        let m = moment();
        let mut s = PresetState::new(p, &mut g, &m);
        let f = s.run_frame(&mut g, &m);
        let wave = sine();
        let audio = Audio {
            wave: [&wave, &wave],
            spectrum: [&wave, &wave],
            levels: Levels::default(),
        };
        let mut d = Drawing::default();
        let canvas = Canvas::new((480, 480));
        custom_waves(
            &mut d,
            &mut s,
            &mut g,
            &m,
            &f.q,
            1.0,
            &canvas,
            &audio,
            &mut Scratch::default(),
        );
        // 10 points smoothed to 19, drawn as 18 segments.
        assert_eq!(d.vertices.len(), 36);
        assert_eq!(d.vertices[0].pos, [-1.0, 0.5]);
        let last = d.vertices.last().unwrap();
        assert!((last.pos[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn every_basic_wave_mode_draws_something_inside_the_picture() {
        let wave = sine();
        let audio = Audio {
            wave: [&wave, &wave],
            spectrum: [&wave, &wave],
            levels: Levels::default(),
        };
        let canvas = Canvas::new((800, 600));
        let mut g = Globals::new(3);
        for mode in 0..8 {
            let p = Preset::parse(&format!(
                "[preset00]\nnWaveMode={mode}\nfWaveAlpha=1\nwave_r=1\nwave_x=0.5\nwave_y=0.5\n"
            ));
            let m = moment();
            let mut s = PresetState::new(p, &mut g, &m);
            let f = s.run_frame(&mut g, &m);
            let look = WaveLook::of(&s.preset.values);
            let mut d = Drawing::default();
            basic_wave(
                &mut d,
                &f,
                &look,
                None,
                1.0,
                &canvas,
                &audio,
                1.0,
                &mut Scratch::default(),
            );
            if mode == 3 {
                // Its alpha follows the treble, which is silent here.
                continue;
            }
            assert!(!d.vertices.is_empty(), "mode {mode}");
            assert!(
                d.vertices
                    .iter()
                    .all(|v| v.pos.iter().all(|c| c.is_finite() && c.abs() < 3.0)),
                "mode {mode}"
            );
        }
    }

    #[test]
    fn borders_frame_the_picture() {
        let p = Preset::parse("[preset00]\nob_size=0.1\nob_a=1\nib_a=0\nbDarkenCenter=1\n");
        let mut g = Globals::new(3);
        let m = moment();
        let mut s = PresetState::new(p, &mut g, &m);
        let f = s.run_frame(&mut g, &m);
        let mut d = Drawing::default();
        sprites(&mut d, &f, &Canvas::new((640, 480)));
        // The centre's fan of 4 triangles, then 4 sides of 2 triangles.
        assert_eq!(d.vertices.len(), 12 + 24);
        let xs: Vec<f32> = d.vertices[12..].iter().map(|v| v.pos[0].abs()).collect();
        assert!(xs.iter().all(|x| *x <= 1.0 + 1e-6));
    }

    #[test]
    fn motion_vectors_are_a_grid_of_short_lines() {
        let p = Preset::parse("[preset00]\nmv_a=1\nnMotionVectorsX=4\nnMotionVectorsY=3\n");
        let mut g = Globals::new(3);
        let m = moment();
        let mut s = PresetState::new(p, &mut g, &m);
        let f = s.run_frame(&mut g, &m);
        let grid = (8, 6);
        // An identity warp: every vector is the minimum length.
        let uv: Vec<[f32; 2]> = (0..=grid.1)
            .flat_map(|j| {
                (0..=grid.0)
                    .map(move |i| [i as f32 / grid.0 as f32, 1.0 - j as f32 / grid.1 as f32])
            })
            .collect();
        let mut d = Drawing::default();
        motion_vectors(
            &mut d,
            &f,
            &Mesh { uv: &uv, grid },
            &Canvas::new((100, 100)),
        );
        // MilkDrop's spacing puts the last row and column at exactly 1, which it skips: 3 by 2.
        assert_eq!(d.vertices.len(), 3 * 2 * 2);
        let a = d.vertices[0].pos;
        let b = d.vertices[1].pos;
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        assert!(len < 0.05, "{len}");
    }
}
