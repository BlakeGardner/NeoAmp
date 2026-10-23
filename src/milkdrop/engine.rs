//! Running MilkDrop frame by frame: time, the sound, the presets (two while one blends into
//! the next), their equations, the warp mesh, and everything drawn, collected in a [`Frame`]
//! for the GPU to draw. The order of work follows `RenderFrame` in MilkDrop's
//! `milkdropfs.cpp`.

use std::time::Instant;

use super::draw::{self, Audio, Canvas, Drawing, Mesh, Scratch, WaveLook};
use super::eel::Globals;
use super::preset::Preset;
use super::sound::Sound;
use super::state::{FrameVars, Moment, Pf, PresetState, Q_COUNT};

/// The warp mesh's cells across and down: MilkDrop's default.
pub const GRID: (usize, usize) = (48, 36);
/// The composite mesh's cells across and down.
pub const COMP_GRID: (usize, usize) = (32, 24);

/// Tells loaded presets apart, so the GPU can keep each one's shaders.
pub type PresetId = u64;

/// What a preset's shaders read, for one pass: MilkDrop's shader constants (`_c0` to
/// `_c13`), the `q` variables, random numbers and rotation matrices, and what the built-in
/// shaders need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderInputs {
    pub c: [[f32; 4]; 14],
    pub q: [f32; Q_COUNT],
    pub rand_frame: [f32; 4],
    pub rand_preset: [f32; 4],
    /// `rot_s1` to `rot_rand4`: each four rows of three, as HLSL's `float4x3`.
    pub rot: [[[f32; 3]; 4]; 24],
    pub decay: f32,
    pub gamma: f32,
    pub echo_zoom: f32,
    pub echo_alpha: f32,
    pub echo_orient: f32,
    pub hue_shader: f32,
    pub brighten: bool,
    pub darken: bool,
    pub solarize: bool,
    pub invert: bool,
    /// Whether the built-in warp repeats the picture past its edges (else clamps).
    pub wrap: bool,
}

/// How a pass combines with what is already drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassBlend {
    Replace,
    /// By the mesh's alpha.
    Alpha,
    /// By one minus the mesh's alpha.
    FlippedAlpha,
}

/// One run of a preset's warp or composite shader over its mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    pub preset: PresetId,
    pub inputs: ShaderInputs,
    pub blend: PassBlend,
}

/// Blur settings for the frame, after MilkDrop's `GetSafeBlurMinMax`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Blur {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub edge_darken: f32,
}

/// Everything the GPU draws for one frame, in order: the motion vectors into the previous
/// frame, the warp passes into the new one, the blur, the drawing over it, and the composite
/// passes to the screen.
#[derive(Debug, Default)]
pub struct Frame {
    pub canvas: Option<Canvas>,
    /// The picture was resized or has just started: clear the previous frame first.
    pub clear: bool,
    pub grid: (usize, usize),
    /// Where each warp mesh vertex samples the previous frame, row by row from the bottom.
    pub warp_uv: Vec<[f32; 2]>,
    /// Each warp mesh vertex's alpha, for blending two presets' warps.
    pub warp_alpha: Vec<f32>,
    pub warp: Vec<Pass>,
    pub motion: Drawing,
    pub blur: Blur,
    pub drawing: Drawing,
    pub comp_grid: (usize, usize),
    /// Each composite mesh vertex's colour (`hue_shader`) and alpha, row by row from the
    /// bottom.
    pub comp_color: Vec<[f32; 4]>,
    pub comp: Vec<Pass>,
}

/// A preset that is running.
#[derive(Debug)]
struct Running {
    id: PresetId,
    state: PresetState,
    /// When it started, in MilkDrop time.
    start: f64,
    /// Its per-frame values this frame.
    vars: Option<FrameVars>,
}

impl Running {
    fn uses_comp_shader(&self) -> bool {
        self.state.preset.comp_ps > 0
    }
}

/// The shape of a blend from one preset to the next: per warp mesh vertex, `a * progress +
/// c` is how far the new preset has come in there (MilkDrop's `m_vertinfo` `a` and `c`).
#[derive(Debug, Default)]
struct Pattern {
    a: Vec<f32>,
    c: Vec<f32>,
}

#[derive(Debug)]
struct Blending {
    start: f64,
    duration: f64,
    progress: f64,
}

/// MilkDrop: presets, time, and the frame being built.
pub struct Engine {
    globals: Globals,
    sound: Sound,
    current: Option<Running>,
    previous: Option<Running>,
    blending: Option<Blending>,
    pattern: Pattern,
    next_id: PresetId,
    /// MilkDrop time: seconds since the first frame.
    time: f64,
    last: Option<Instant>,
    frames: u64,
    fps: f64,
    /// When the current preset will be replaced, for the `progress` variable, if known.
    preset_length: Option<f64>,
    /// Four random numbers chosen at start (MilkDrop's `m_fRandStart`).
    rand_start: [f32; 4],
    frame: Frame,
    scratch: Scratch,
    last_size: Option<(u32, u32)>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("time", &self.time)
            .field("frames", &self.frames)
            .field("blending", &self.blending)
            .finish_non_exhaustive()
    }
}

/// MilkDrop's `CosineInterp`: an S-curve from 0 to 1.
fn cosine_interp(x: f64) -> f64 {
    0.5 - 0.5 * (x * std::f64::consts::PI).cos()
}

impl Engine {
    /// An engine whose random numbers start from `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Engine {
        let mut globals = Globals::new(seed);
        let rand_start = std::array::from_fn(|_| globals.random() as f32);
        Engine {
            globals,
            sound: Sound::new(),
            current: None,
            previous: None,
            blending: None,
            pattern: Pattern::default(),
            next_id: 1,
            time: 0.0,
            last: None,
            frames: 0,
            fps: 30.0,
            preset_length: None,
            rand_start,
            frame: Frame::default(),
            scratch: Scratch::default(),
            last_size: None,
        }
    }

    /// The rate of the samples passed to [`Engine::push_audio`].
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sound.set_rate(rate);
    }

    /// Add interleaved stereo samples as they are played.
    pub fn push_audio(&mut self, samples: &[f32]) {
        self.sound.push(samples);
    }

    /// How long the current preset will run, for presets' `progress` variable.
    pub fn set_preset_length(&mut self, seconds: Option<f64>) {
        self.preset_length = seconds;
    }

    /// The preset running now, if any.
    #[must_use]
    pub fn current(&self) -> Option<(PresetId, &Preset)> {
        self.current.as_ref().map(|r| (r.id, &r.state.preset))
    }

    /// The preset being blended out, if any.
    #[must_use]
    pub fn previous(&self) -> Option<(PresetId, &Preset)> {
        self.previous.as_ref().map(|r| (r.id, &r.state.preset))
    }

    /// Start running `preset`, blending into it over `blend` seconds (0 for a hard cut).
    /// Returns its id.
    pub fn load(&mut self, preset: Preset, blend: f64) -> PresetId {
        let id = self.next_id;
        self.next_id += 1;
        let m = self.moment(0.0);
        let state = PresetState::new(preset, &mut self.globals, &m);
        let running = Running {
            id,
            state,
            start: self.time,
            vars: None,
        };
        let old = self.current.replace(running);
        if blend > 0.0 && old.is_some() {
            self.previous = old;
            self.blending = Some(Blending {
                start: self.time,
                duration: blend,
                progress: 0.0,
            });
            self.make_pattern();
        } else {
            self.previous = None;
            self.blending = None;
        }
        id
    }

    fn moment(&self, progress: f64) -> Moment {
        let size = self.last_size.unwrap_or((640, 480));
        let canvas = Canvas::new(size);
        let levels = self.sound.levels();
        Moment {
            time: self.time,
            frame: self.frames as f64,
            fps: self.fps,
            progress,
            levels,
            mesh: GRID,
            pixels: size,
            inv_aspect: (
                1.0 / f64::from(canvas.aspect.0),
                1.0 / f64::from(canvas.aspect.1),
            ),
        }
    }

    /// Advance to `now` and work out the frame, for a picture `size` pixels across and down.
    pub fn render(&mut self, now: Instant, size: (u32, u32)) -> &Frame {
        let size = (size.0.max(1), size.1.max(1));
        self.advance_time(now);
        let resized = self.last_size != Some(size);
        self.last_size = Some(size);
        let canvas = Canvas::new(size);
        self.sound.analyse(now, self.fps as f32);
        if let Some(b) = &mut self.blending {
            b.progress = (self.time - b.start) / b.duration;
        }
        if self.blending.as_ref().is_some_and(|b| b.progress > 1.0) {
            self.blending = None;
            self.previous = None;
        }

        let frame = &mut self.frame;
        frame.canvas = Some(canvas);
        frame.clear = resized || self.frames == 1;
        frame.grid = GRID;
        frame.comp_grid = COMP_GRID;
        frame.warp.clear();
        frame.comp.clear();
        frame.motion.clear();
        frame.drawing.clear();
        let vertices = (GRID.0 + 1) * (GRID.1 + 1);
        if frame.warp_uv.len() != vertices {
            frame.warp_uv = identity_uv(GRID);
            frame.warp_alpha = vec![1.0; vertices];
        }
        if self.current.is_none() {
            return &self.frame;
        }

        let progress = self.progress();
        let m = self.moment(progress);
        let blend = self.blending.as_ref().map(|b| b.progress);
        let mix = blend.map_or(1.0, cosine_interp);

        // 1. Per-frame equations, both presets while blending.
        let snap = self.snap_point();
        let globals = &mut self.globals;
        for running in [&mut self.current, &mut self.previous]
            .into_iter()
            .flatten()
        {
            running.vars = Some(running.state.run_frame(globals, &m));
        }
        if let (Some(cur), Some(old)) = (&mut self.current, &self.previous)
            && let (Some(v), Some(ov)) = (&mut cur.vars, &old.vars)
        {
            v.blend_from(ov, mix, snap);
        }

        // 2. Motion vectors, into the previous frame, along last frame's mesh.
        if let Some(v) = self.current.as_ref().and_then(|c| c.vars.as_ref()) {
            let mesh = Mesh {
                uv: &self.frame.warp_uv,
                grid: GRID,
            };
            draw::motion_vectors(&mut self.frame.motion, v, &mesh, &canvas);
        }

        // 3. The warp mesh.
        self.warp_mesh(&canvas, blend);

        // 4. Warp passes: the old preset's first, then the new one blended over it.
        self.frame.warp.clear();
        let new_inputs = self.inputs_for(true, &canvas);
        let old_inputs = self.inputs_for(false, &canvas);
        if let (Some(cur), Some(new_inputs)) = (&self.current, new_inputs) {
            match (&self.previous, old_inputs) {
                (Some(old), Some(old_inputs)) => {
                    self.frame.warp.push(Pass {
                        preset: old.id,
                        inputs: old_inputs.clone(),
                        blend: PassBlend::Replace,
                    });
                    self.frame.warp.push(Pass {
                        preset: cur.id,
                        inputs: new_inputs.clone(),
                        blend: PassBlend::Alpha,
                    });
                    // The composite: as MilkDrop's RenderFrame picks for each combination.
                    let (old_comp, new_comp) = (old.uses_comp_shader(), cur.uses_comp_shader());
                    let passes = if old_comp && !new_comp {
                        [
                            (cur.id, new_inputs, PassBlend::Replace),
                            (old.id, old_inputs, PassBlend::FlippedAlpha),
                        ]
                    } else {
                        [
                            (old.id, old_inputs, PassBlend::Replace),
                            (cur.id, new_inputs, PassBlend::Alpha),
                        ]
                    };
                    self.frame
                        .comp
                        .extend(passes.map(|(preset, inputs, blend)| Pass {
                            preset,
                            inputs,
                            blend,
                        }));
                }
                _ => {
                    self.frame.warp.push(Pass {
                        preset: cur.id,
                        inputs: new_inputs.clone(),
                        blend: PassBlend::Replace,
                    });
                    self.frame.comp.push(Pass {
                        preset: cur.id,
                        inputs: new_inputs,
                        blend: PassBlend::Replace,
                    });
                }
            }
        }

        // 5. Blur settings.
        if let Some(v) = self.current.as_ref().and_then(|c| c.vars.as_ref()) {
            self.frame.blur = safe_blur(v);
        }

        // 6. Shapes, waves, the basic wave and the borders.
        self.draw_over(&canvas, &m, mix);

        // 7. The composite mesh's colours.
        self.comp_colors();
        &self.frame
    }

    fn advance_time(&mut self, now: Instant) {
        let dt = self
            .last
            .map(|l| now.saturating_duration_since(l).as_secs_f64())
            .filter(|dt| *dt > 0.0 && *dt < 1.0)
            .unwrap_or(1.0 / 30.0);
        self.last = Some(now);
        self.time += dt;
        self.frames += 1;
        // A smoothed frame rate, quick to follow big changes.
        let fps = 1.0 / dt;
        self.fps = if (fps - self.fps).abs() > 3.0 && self.frames > 120 {
            fps
        } else {
            0.93 * self.fps + 0.07 * fps
        };
    }

    fn progress(&self) -> f64 {
        match (&self.current, self.preset_length) {
            (Some(c), Some(len)) if len > 0.0 => ((self.time - c.start) / len).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// MilkDrop's `m_fSnapPoint`: when flags switch over during a blend, tuned to whether the
    /// presets have composite shaders.
    fn snap_point(&self) -> f64 {
        match (&self.previous, &self.current) {
            (Some(old), Some(new)) => match (old.uses_comp_shader(), new.uses_comp_shader()) {
                (true, false) => -0.01,
                (false, true) => 1.01,
                _ => 0.5,
            },
            _ => 0.5,
        }
    }

    /// A new random blend pattern (as butterchurn's `createBlendPattern`, after MilkDrop's).
    fn make_pattern(&mut self) {
        let (gx, gy) = GRID;
        let n = (gx + 1) * (gy + 1);
        let canvas = Canvas::new(self.last_size.unwrap_or((640, 480)));
        let (ax, ay) = (f64::from(canvas.aspect.0), f64::from(canvas.aspect.1));
        let g = &mut self.globals;
        let p = &mut self.pattern;
        p.a = vec![0.0; n];
        p.c = vec![0.0; n];
        let set = |p: &mut Pattern, band: f64, t: &dyn Fn(usize, usize) -> f64| {
            let inv = 1.0 / band;
            for y in 0..=gy {
                for x in 0..=gx {
                    let i = y * (gx + 1) + x;
                    p.a[i] = (inv * (1.0 + band)) as f32;
                    p.c[i] = (-inv + inv * t(x, y)) as f32;
                }
            }
        };
        match 1 + (g.random() * 3.0) as u32 {
            1 => {
                // A straight wipe at a random angle.
                let ang = g.random() * std::f64::consts::TAU;
                let (vx, vy) = (ang.cos(), ang.sin());
                let band = 0.1 + 0.2 * g.random();
                set(p, band, &|x, y| {
                    let fx = x as f64 / gx as f64 * ax;
                    let fy = y as f64 / gy as f64 * ay;
                    let t = (fx - 0.5) * vx + (fy - 0.5) * vy + 0.5;
                    (t - 0.5) / std::f64::consts::SQRT_2 + 0.5
                });
            }
            2 => {
                // A cloudy (plasma) dissolve.
                let band = 0.12 + 0.13 * g.random();
                let mut c = vec![0.0f64; n];
                c[0] = g.random();
                c[gx] = g.random();
                c[gy * (gx + 1)] = g.random();
                c[gy * (gx + 1) + gx] = g.random();
                plasma(&mut c, g, (gx, gy), (0, gx, 0, gy), 0.25, (ax, ay));
                let (min, max) = c
                    .iter()
                    .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
                let mult = 1.0 / (max - min).max(1e-9);
                set(p, band, &|x, y| (c[y * (gx + 1) + x] - min) * mult);
            }
            _ => {
                // A circle growing from the middle, or closing in on it.
                let band = 0.02 + 0.14 * g.random() + 0.34 * g.random();
                let inward = g.random() < 0.5;
                set(p, band, &|x, y| {
                    let dx = (x as f64 / gx as f64 - 0.5) * ax;
                    let dy = (y as f64 / gy as f64 - 0.5) * ay;
                    let t = (dx * dx + dy * dy).sqrt() * std::f64::consts::SQRT_2;
                    if inward { 1.0 - t } else { t }
                });
            }
        }
    }

    /// MilkDrop's `ComputeGridAlphaValues`: where each vertex of the warp mesh samples the
    /// previous frame, after each preset's per-vertex code, the old preset's mixed in through
    /// the blend pattern.
    fn warp_mesh(&mut self, canvas: &Canvas, blend: Option<f64>) {
        let (gx, gy) = GRID;
        let (ax, ay) = (f64::from(canvas.aspect.0), f64::from(canvas.aspect.1));
        let Some(cur) = &self.current else {
            return;
        };
        let Some(cur_vars) = &cur.vars else {
            return;
        };
        // Warp time and scale come from the new preset, for both.
        let values = &cur.state.preset.values;
        let warp_time = self.time * values.get("warpanimspeed");
        let warp_scale_inv = 1.0 / values.get("warpscale");
        let f = [
            11.68 + 4.0 * (warp_time * 1.413 + 10.0).cos(),
            8.77 + 3.0 * (warp_time * 1.113 + 7.0).cos(),
            10.54 + 3.0 * (warp_time * 1.233 + 3.0).cos(),
            11.49 + 4.0 * (warp_time * 0.933 + 5.0).cos(),
        ];
        let motion_of = |v: &FrameVars| {
            [
                Pf::Zoom,
                Pf::ZoomExp,
                Pf::Rot,
                Pf::Warp,
                Pf::Cx,
                Pf::Cy,
                Pf::Dx,
                Pf::Dy,
                Pf::Sx,
                Pf::Sy,
            ]
            .map(|p| v.get(p))
        };
        let cur_motion = motion_of(cur_vars);
        let old_motion = self
            .previous
            .as_ref()
            .and_then(|o| o.vars.as_ref())
            .map(motion_of);
        let globals = &mut self.globals;
        let frame = &mut self.frame;
        let reps: [(Option<&mut Running>, Option<[f64; 10]>); 2] = [
            (self.current.as_mut(), Some(cur_motion)),
            (self.previous.as_mut(), old_motion),
        ];
        for (rep, (running, motion)) in reps.into_iter().enumerate() {
            let (Some(running), Some(frame_motion)) = (running, motion) else {
                continue;
            };
            let state = &mut running.state;
            let has_code = state.has_vertex_code();
            for y in 0..=gy {
                for x in 0..=gx {
                    let n = y * (gx + 1) + x;
                    // MilkDrop's `m_verts`: -1..1, y up from the bottom row.
                    let vx = x as f64 / gx as f64 * 2.0 - 1.0;
                    let vy = y as f64 / gy as f64 * 2.0 - 1.0;
                    let (rad, ang) = vertex_rad_ang(vx, vy, ax, ay);
                    let mut mo = frame_motion;
                    if has_code {
                        let pos = (vx * 0.5 * ax + 0.5, vy * -0.5 * ay + 0.5);
                        state.run_vertex(globals, pos, rad, ang, &mut mo);
                    }
                    let [zoom, zoom_exp, rot, warp, cx, cy, dx, dy, sx, sy] = mo;
                    let zoom2 = zoom.powf(zoom_exp.powf(rad * 2.0 - 1.0));
                    let zoom2_inv = 1.0 / zoom2;
                    let mut u = vx * ax * 0.5 * zoom2_inv + 0.5;
                    let mut v = -vy * ay * 0.5 * zoom2_inv + 0.5;
                    u = (u - cx) / sx + cx;
                    v = (v - cy) / sy + cy;
                    u += warp
                        * 0.0035
                        * (warp_time * 0.333 + warp_scale_inv * (vx * f[0] - vy * f[3])).sin();
                    v += warp
                        * 0.0035
                        * (warp_time * 0.375 - warp_scale_inv * (vx * f[2] + vy * f[1])).cos();
                    u += warp
                        * 0.0035
                        * (warp_time * 0.753 - warp_scale_inv * (vx * f[1] - vy * f[2])).cos();
                    v += warp
                        * 0.0035
                        * (warp_time * 0.825 + warp_scale_inv * (vx * f[0] + vy * f[3])).sin();
                    let (u2, v2) = (u - cx, v - cy);
                    let (c, s) = (rot.cos(), rot.sin());
                    u = u2 * c - v2 * s + cx;
                    v = u2 * s + v2 * c + cy;
                    u -= dx;
                    v -= dy;
                    u = (u - 0.5) / ax + 0.5;
                    v = (v - 0.5) / ay + 0.5;
                    let (u, v) = (sane(u, 0.5), sane(v, 0.5));
                    if rep == 0 {
                        frame.warp_uv[n] = [u as f32, v as f32];
                        frame.warp_alpha[n] = 1.0;
                    } else {
                        let progress = blend.unwrap_or(1.0) as f32;
                        let mix2 = (self.pattern.a.get(n).copied().unwrap_or(1.0) * progress
                            + self.pattern.c.get(n).copied().unwrap_or(0.0))
                        .clamp(0.0, 1.0);
                        let [nu, nv] = frame.warp_uv[n];
                        frame.warp_uv[n] = [
                            nu * mix2 + u as f32 * (1.0 - mix2),
                            nv * mix2 + v as f32 * (1.0 - mix2),
                        ];
                        // Quantised as the alpha of a Direct3D colour.
                        frame.warp_alpha[n] = f32::from((mix2 * 255.0) as u8) / 255.0;
                    }
                }
            }
        }
    }

    /// The shader inputs for the current (`new`) or previous preset's passes.
    fn inputs_for(&mut self, new: bool, canvas: &Canvas) -> Option<ShaderInputs> {
        let running = if new { &self.current } else { &self.previous };
        let running = running.as_ref()?;
        let vars = running.vars.as_ref()?;
        let state = &running.state;
        let levels = self.sound.levels();
        let (w, h) = (canvas.size.0 as f32, canvas.size.1 as f32);
        let (ax, ay) = canvas.aspect;
        let time = self.time as f32;
        let since_start = (self.time - running.start) as f32;
        let since_start = since_start - (since_start / 10_000.0).trunc() * 10_000.0;
        let blur = safe_blur(vars);
        let roam = |speeds: [f32; 4], phases: [f32; 4], f: fn(f32) -> f32| -> [f32; 4] {
            std::array::from_fn(|i| 0.5 + 0.5 * f(time * speeds[i] + phases[i]))
        };
        let fast = ([0.329, 1.293, 5.070, 20.051], [1.2, 3.9, 2.5, 5.4]);
        let slow = ([0.0050, 0.0085, 0.0133, 0.0217], [2.7, 5.3, 4.5, 3.8]);
        let mip = w.log2();
        let c = [
            [ax, ay, 1.0 / ax, 1.0 / ay],
            [0.0; 4],
            [
                since_start,
                self.fps as f32,
                self.frames as f32,
                self.progress() as f32,
            ],
            // MilkDrop's vol was `0.3333f*(bass, mid, treb)`: a third of the treble.
            [levels.bass, levels.mid, levels.treb, 0.3333 * levels.treb],
            [
                levels.bass_att,
                levels.mid_att,
                levels.treb_att,
                0.3333 * levels.treb_att,
            ],
            [
                blur.max[0] - blur.min[0],
                blur.min[0],
                blur.max[1] - blur.min[1],
                blur.min[1],
            ],
            [
                blur.max[2] - blur.min[2],
                blur.min[2],
                blur.min[0],
                blur.max[0],
            ],
            [w, h, 1.0 / w, 1.0 / h],
            roam(fast.0, fast.1, f32::cos),
            roam(fast.0, fast.1, f32::sin),
            roam(slow.0, slow.1, f32::cos),
            roam(slow.0, slow.1, f32::sin),
            [mip, mip, mip, 0.0],
            [blur.min[1], blur.max[1], blur.min[2], blur.max[2]],
        ];
        let mut rot = [[[0.0; 3]; 4]; 24];
        for (k, r) in state.rotations.iter().enumerate() {
            let a = |i: usize| r.base[i] + r.speed[i] * time;
            rot[k] = rotation_matrix([a(0), a(1), a(2)], r.translate);
        }
        let g = &mut self.globals;
        for m in &mut rot[20..] {
            let mut rnd = || g.random() as f32;
            let angles = [rnd(), rnd(), rnd()].map(|r| r * std::f32::consts::TAU);
            *m = rotation_matrix(angles, [rnd(), rnd(), rnd()]);
        }
        let rand_frame = std::array::from_fn(|_| g.random() as f32);
        let snap = self.snap_point();
        Some(ShaderInputs {
            c,
            q: vars.q.map(|v| v as f32),
            rand_frame,
            rand_preset: state.rand_preset,
            rot,
            decay: draw::d3d(vars.get(Pf::Decay)),
            gamma: vars.get(Pf::Gamma) as f32,
            echo_zoom: vars.get(Pf::EchoZoom) as f32,
            echo_alpha: vars.get(Pf::EchoAlpha) as f32,
            echo_orient: vars.get(Pf::EchoOrient) as f32,
            hue_shader: state.preset.values.get("fshader") as f32,
            brighten: vars.on(Pf::Brighten),
            darken: vars.on(Pf::Darken),
            solarize: vars.on(Pf::Solarize),
            invert: vars.on(Pf::Invert),
            wrap: vars.get(Pf::Wrap) > snap,
        })
    }

    /// Custom shapes and waves (the new preset's, then the old one's, each faded by the
    /// blend), the basic wave and the borders.
    fn draw_over(&mut self, canvas: &Canvas, m: &Moment, mix: f64) {
        let progress = self
            .blending
            .as_ref()
            .map_or(1.0, |b| b.progress.clamp(0.0, 1.0));
        let wave = [self.sound.wave(0), self.sound.wave(1)];
        let spectrum = [self.sound.spectrum(0), self.sound.spectrum(1)];
        let audio = Audio {
            wave,
            spectrum,
            levels: self.sound.levels(),
        };
        let blending = self.previous.is_some();
        let frame = &mut self.frame;
        let globals = &mut self.globals;
        let scratch = &mut self.scratch;
        let reps = [
            (self.current.as_mut(), if blending { progress } else { 1.0 }),
            (self.previous.as_mut(), 1.0 - progress),
        ];
        let mut runs: Vec<(&mut Running, f64)> = reps
            .into_iter()
            .filter_map(|(r, a)| r.map(|r| (r, a)))
            .collect();
        for (r, alpha) in &mut runs {
            let Some(q) = r.vars.as_ref().map(|v| v.q) else {
                continue;
            };
            draw::custom_shapes(
                &mut frame.drawing,
                &mut r.state,
                globals,
                m,
                &q,
                *alpha,
                canvas,
                scratch,
            );
        }
        for (r, alpha) in &mut runs {
            let Some(q) = r.vars.as_ref().map(|v| v.q) else {
                continue;
            };
            draw::custom_waves(
                &mut frame.drawing,
                &mut r.state,
                globals,
                m,
                &q,
                *alpha,
                canvas,
                &audio,
                scratch,
            );
        }
        drop(runs);
        let Some(cur) = self.current.as_ref() else {
            return;
        };
        let Some(vars) = &cur.vars else {
            return;
        };
        let look = WaveLook::of(&cur.state.preset.values);
        let old = self
            .previous
            .as_ref()
            .and_then(|o| o.vars.as_ref().map(|v| (v, v.get(Pf::WaveMode) as i64)));
        draw::basic_wave(
            &mut frame.drawing,
            vars,
            &look,
            old,
            mix,
            canvas,
            &audio,
            self.time,
            scratch,
        );
        draw::sprites(&mut frame.drawing, vars, canvas);
    }

    /// The composite mesh's `hue_shader` colours, and its alpha from the warp mesh while
    /// blending (MilkDrop's `ShowToUser_Shaders`).
    fn comp_colors(&mut self) {
        let t = self.time as f32;
        let rs = self.rand_start;
        let shade: [[f32; 3]; 4] = std::array::from_fn(|i| {
            let i = i as f32;
            let mut c = [
                0.6 + 0.3 * (t * 30.0 * 0.0143 + 3.0 + i * 21.0 + rs[3]).sin(),
                0.6 + 0.3 * (t * 30.0 * 0.0107 + 1.0 + i * 13.0 + rs[1]).sin(),
                0.6 + 0.3 * (t * 30.0 * 0.0129 + 6.0 + i * 9.0 + rs[2]).sin(),
            ];
            let max = c[0].max(c[1]).max(c[2]);
            for v in &mut c {
                *v = 0.5 + 0.5 * (*v / max);
            }
            c
        });
        let (cx, cy) = COMP_GRID;
        let (gx, gy) = GRID;
        let blending = self.blending.is_some();
        let frame = &mut self.frame;
        frame.comp_color.clear();
        for j in 0..=cy {
            for i in 0..=cx {
                let x = i as f32 / cx as f32;
                let y = j as f32 / cy as f32;
                let col: [f32; 3] = std::array::from_fn(|k| {
                    shade[0][k] * x * y
                        + shade[1][k] * (1.0 - x) * y
                        + shade[2][k] * x * (1.0 - y)
                        + shade[3][k] * (1.0 - x) * (1.0 - y)
                });
                let alpha = if blending {
                    let fx = (x * (gx + 1) as f32).clamp(0.0, (gx - 1) as f32);
                    let fy = (y * (gy + 1) as f32).clamp(0.0, (gy - 1) as f32);
                    let (nx, ny) = (fx as usize, fy as usize);
                    let (dx, dy) = (fx - nx as f32, fy - ny as f32);
                    let a = |x: usize, y: usize| frame.warp_alpha[y * (gx + 1) + x];
                    a(nx, ny) * (1.0 - dx) * (1.0 - dy)
                        + a(nx + 1, ny) * dx * (1.0 - dy)
                        + a(nx, ny + 1) * (1.0 - dx) * dy
                        + a(nx + 1, ny + 1) * dx * dy
                } else {
                    1.0
                };
                frame.comp_color.push([col[0], col[1], col[2], alpha]);
            }
        }
    }
}

/// A number from the equations, or `fallback` when they went to infinity or NaN.
fn sane(v: f64, fallback: f64) -> f64 {
    if v.is_finite() { v } else { fallback }
}

/// The warp mesh with nothing moved: each vertex samples where it is.
fn identity_uv(grid: (usize, usize)) -> Vec<[f32; 2]> {
    let (gx, gy) = grid;
    (0..=gy)
        .flat_map(|y| (0..=gx).map(move |x| [x as f32 / gx as f32, 1.0 - y as f32 / gy as f32]))
        .collect()
}

/// A warp mesh vertex's `rad` and `ang` for the per-vertex code (MilkDrop's `m_vertinfo`, as
/// butterchurn computes it): the distance from the centre, allowing for the aspect ratio, and
/// the angle counterclockwise from the right, -π to π.
fn vertex_rad_ang(vx: f64, vy: f64, ax: f64, ay: f64) -> (f64, f64) {
    let (px, py) = (vx * ax, vy * ay);
    ((px * px + py * py).sqrt(), py.atan2(px))
}

/// MilkDrop's `GetSafeBlurMinMax`, faults included: when a range is too narrow it sets both
/// ends to just under the middle.
fn safe_blur(v: &FrameVars) -> Blur {
    let mut min = [Pf::Blur1Min, Pf::Blur2Min, Pf::Blur3Min].map(|p| v.get(p) as f32);
    let mut max = [Pf::Blur1Max, Pf::Blur2Max, Pf::Blur3Max].map(|p| v.get(p) as f32);
    const MIN_DIST: f32 = 0.1;
    for i in 0..3 {
        if i > 0 {
            max[i] = max[i - 1].min(max[i]);
            min[i] = min[i - 1].max(min[i]);
        }
        if max[i] - min[i] < MIN_DIST {
            let avg = (min[i] + max[i]) * 0.5;
            min[i] = avg - MIN_DIST * 0.5;
            max[i] = avg - MIN_DIST * 0.5;
        }
    }
    Blur {
        min,
        max,
        edge_darken: v.get(Pf::Blur1EdgeDarken) as f32,
    }
}

/// MilkDrop's plasma for blend pattern 2: midpoint displacement over the grid.
fn plasma(
    c: &mut [f64],
    g: &mut Globals,
    grid: (usize, usize),
    (x0, x1, y0, y1): (usize, usize, usize, usize),
    dt: f64,
    (ax, ay): (f64, f64),
) {
    let w = grid.0 + 1;
    let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
    let (t00, t01, t10, t11) = (
        c[y0 * w + x0],
        c[y0 * w + x1],
        c[y1 * w + x0],
        c[y1 * w + x1],
    );
    let mut rnd = |scale: f64| (g.random() * 2.0 - 1.0) * dt * scale;
    if y1 - y0 >= 2 {
        if x0 == 0 {
            c[my * w + x0] = 0.5 * (t00 + t10) + rnd(ay);
        }
        c[my * w + x1] = 0.5 * (t01 + t11) + rnd(ay);
    }
    if x1 - x0 >= 2 {
        if y0 == 0 {
            c[y0 * w + mx] = 0.5 * (t00 + t01) + rnd(ax);
        }
        c[y1 * w + mx] = 0.5 * (t10 + t11) + rnd(ax);
    }
    if y1 - y0 >= 2 && x1 - x0 >= 2 {
        let (a, b, cc, d) = (
            c[my * w + x0],
            c[my * w + x1],
            c[y0 * w + mx],
            c[y1 * w + mx],
        );
        c[my * w + mx] = 0.25 * (a + b + cc + d) + rnd(1.0);
        plasma(c, g, grid, (x0, mx, y0, my), dt * 0.5, (ax, ay));
        plasma(c, g, grid, (mx, x1, y0, my), dt * 0.5, (ax, ay));
        plasma(c, g, grid, (x0, mx, my, y1), dt * 0.5, (ax, ay));
        plasma(c, g, grid, (mx, x1, my, y1), dt * 0.5, (ax, ay));
    }
}

/// A rotation and translation as Direct3D's matrices built it in MilkDrop's
/// `ApplyShaderParams` (rotate about x, translate, rotate about z, then y), as four rows of
/// three for HLSL's `float4x3`.
fn rotation_matrix(angles: [f32; 3], translate: [f32; 3]) -> [[f32; 3]; 4] {
    type M = [[f32; 4]; 4];
    let id: M = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mul = |a: &M, b: &M| -> M {
        std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum()))
    };
    let (cx, sx) = (angles[0].cos(), angles[0].sin());
    let (cy, sy) = (angles[1].cos(), angles[1].sin());
    let (cz, sz) = (angles[2].cos(), angles[2].sin());
    // D3DXMatrixRotationX, Y and Z, and D3DXMatrixTranslation (row vectors).
    let mut mx = id;
    mx[1] = [0.0, cx, sx, 0.0];
    mx[2] = [0.0, -sx, cx, 0.0];
    let mut my = id;
    my[0] = [cy, 0.0, -sy, 0.0];
    my[2] = [sy, 0.0, cy, 0.0];
    let mut mz = id;
    mz[0] = [cz, sz, 0.0, 0.0];
    mz[1] = [-sz, cz, 0.0, 0.0];
    let mut t = id;
    t[3] = [translate[0], translate[1], translate[2], 1.0];
    let m = mul(&mul(&mul(&mx, &t), &mz), &my);
    std::array::from_fn(|i| [m[i][0], m[i][1], m[i][2]])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn run(engine: &mut Engine, frames: u32) {
        let t0 = Instant::now();
        for k in 0..frames {
            engine.render(t0 + Duration::from_millis(33 * u64::from(k)), (320, 240));
        }
    }

    #[test]
    fn an_identity_preset_leaves_the_mesh_where_it_is() {
        let mut e = Engine::new(1);
        e.load(Preset::parse("[preset00]\nzoom=1\nwarp=0\nrot=0\n"), 0.0);
        run(&mut e, 2);
        let f = &e.frame;
        let identity = identity_uv(GRID);
        for (a, b) in f.warp_uv.iter().zip(&identity) {
            assert!(
                (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5,
                "{a:?} {b:?}"
            );
        }
        assert_eq!(f.warp.len(), 1);
        assert_eq!(f.comp.len(), 1);
        assert_eq!(f.comp_color.len(), 33 * 25);
    }

    #[test]
    fn zooming_in_samples_closer_to_the_centre() {
        let mut e = Engine::new(1);
        e.load(Preset::parse("[preset00]\nzoom=1.1\nwarp=0\n"), 0.0);
        run(&mut e, 1);
        let corner = e.frame.warp_uv[0];
        assert!(corner[0] > 0.0 && corner[1] < 1.0, "{corner:?}");
    }

    #[test]
    fn per_vertex_code_moves_the_mesh_where_it_runs() {
        let mut e = Engine::new(1);
        e.load(
            Preset::parse("[preset00]\nwarp=0\nper_pixel_1=dx = above(x, 0.5)*0.1;\n"),
            0.0,
        );
        run(&mut e, 1);
        let f = &e.frame;
        let (gx, _) = GRID;
        // The left half samples where it is; the right half samples further left.
        assert!((f.warp_uv[0][0] - 0.0).abs() < 1e-5);
        assert!((f.warp_uv[gx][0] - 0.9).abs() < 1e-5);
    }

    #[test]
    fn blending_draws_both_presets_and_ends() {
        let mut e = Engine::new(5);
        e.load(Preset::parse("[preset00]\nzoom=1.1\n"), 0.0);
        run(&mut e, 2);
        let id = e.load(
            Preset::parse("MILKDROP_PRESET_VERSION=201\n[preset00]\nzoom=0.9\n"),
            1.0,
        );
        // Just after the frames above, so the first step is a short one.
        let t0 = Instant::now();
        for k in 0..=5 {
            e.render(t0 + Duration::from_millis(100 * k), (320, 240));
        }
        assert!(e.previous.is_some());
        assert_eq!(e.frame.warp.len(), 2);
        assert_eq!(e.frame.warp[1].preset, id);
        assert_eq!(e.frame.warp[1].blend, PassBlend::Alpha);
        // Half way through, some of the mesh is the new preset's and some the old's.
        assert!(e.frame.warp_alpha.iter().any(|a| *a < 1.0));
        for k in 6..=12 {
            e.render(t0 + Duration::from_millis(100 * k), (320, 240));
        }
        assert!(e.previous.is_none());
        assert_eq!(e.frame.warp.len(), 1);
    }

    #[test]
    fn every_blend_pattern_starts_old_and_ends_new() {
        let mut e = Engine::new(9);
        for _ in 0..12 {
            e.make_pattern();
            let p = &e.pattern;
            let at = |progress: f32| -> Vec<f32> {
                p.a.iter()
                    .zip(&p.c)
                    .map(|(a, c)| (a * progress + c).clamp(0.0, 1.0))
                    .collect()
            };
            assert!(at(0.0).iter().all(|m| *m == 0.0));
            assert!(at(1.0).iter().all(|m| *m == 1.0));
        }
    }

    #[test]
    fn rotation_matrices_rotate_and_translate() {
        let m = rotation_matrix([0.0, 0.0, std::f32::consts::FRAC_PI_2], [1.0, 2.0, 3.0]);
        // Row vector (1, 0, 0, 0) turns a quarter about z to (0, 1, 0).
        assert!((m[0][1] - 1.0).abs() < 1e-6 && m[0][0].abs() < 1e-6);
        // The translation, turned the same way.
        assert!((m[3][0] + 2.0).abs() < 1e-5 && (m[3][1] - 1.0).abs() < 1e-5);
        assert!((m[3][2] - 3.0).abs() < 1e-5);
    }

    #[test]
    fn blur_ranges_keep_milkdrops_faults() {
        let mut e = Engine::new(1);
        e.load(Preset::parse("[preset00]\nb1n=0.5\nb1x=0.52\n"), 0.0);
        run(&mut e, 1);
        let b = e.frame.blur;
        assert!((b.min[0] - 0.46).abs() < 1e-6 && (b.max[0] - 0.46).abs() < 1e-6);
    }
}
