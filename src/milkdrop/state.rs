//! A preset while it runs: its compiled code, its tables of variables, and the values its
//! per-frame code leaves each frame. This follows MilkDrop 2's `CState` (`state.cpp`) and the
//! per-frame half of `RenderFrame` (`milkdropfs.cpp`): which variables each kind of code sees,
//! what is reset each frame, and how `q1`-`q32` and `t1`-`t8` pass from one table to the next.

use std::fmt;

use super::eel::{Dialect, Globals, Program, Slot, Vars};
use super::preset::{CUSTOM, Preset, Values};
use super::sound::Levels;

/// `q1` to `q32`: per-frame code hands these to every other kind of code.
pub const Q_COUNT: usize = 32;
/// `t1` to `t8`: a custom wave's or shape's init code hands these to its per-frame code.
pub const T_COUNT: usize = 8;

/// Declares the built-in variables of one kind of table: an enum to index them by, and their
/// names in the same order.
macro_rules! builtins {
    ($(#[$meta:meta])* $name:ident { $($var:ident = $text:literal,)* }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum $name { $($var,)* }

        impl $name {
            /// Every variable's name, in the enum's order.
            pub const NAMES: &'static [&'static str] = &[$($text,)*];
        }
    };
}

builtins! {
    /// The per-frame (and per-frame init) code's variables.
    Pf {
        Zoom = "zoom", ZoomExp = "zoomexp", Rot = "rot", Warp = "warp", Cx = "cx", Cy = "cy",
        Dx = "dx", Dy = "dy", Sx = "sx", Sy = "sy",
        Time = "time", Fps = "fps", Bass = "bass", Mid = "mid", Treb = "treb",
        BassAtt = "bass_att", MidAtt = "mid_att", TrebAtt = "treb_att", Frame = "frame",
        Progress = "progress", MeshX = "meshx", MeshY = "meshy", PixelsX = "pixelsx",
        PixelsY = "pixelsy", AspectX = "aspectx", AspectY = "aspecty",
        Decay = "decay", WaveA = "wave_a", WaveR = "wave_r", WaveG = "wave_g", WaveB = "wave_b",
        WaveX = "wave_x", WaveY = "wave_y", WaveMystery = "wave_mystery", WaveMode = "wave_mode",
        ObSize = "ob_size", ObR = "ob_r", ObG = "ob_g", ObB = "ob_b", ObA = "ob_a",
        IbSize = "ib_size", IbR = "ib_r", IbG = "ib_g", IbB = "ib_b", IbA = "ib_a",
        MvX = "mv_x", MvY = "mv_y", MvDx = "mv_dx", MvDy = "mv_dy", MvL = "mv_l",
        MvR = "mv_r", MvG = "mv_g", MvB = "mv_b", MvA = "mv_a",
        EchoZoom = "echo_zoom", EchoAlpha = "echo_alpha", EchoOrient = "echo_orient",
        WaveUseDots = "wave_usedots", WaveThick = "wave_thick", WaveAdditive = "wave_additive",
        WaveBrighten = "wave_brighten", DarkenCenter = "darken_center", Gamma = "gamma",
        Wrap = "wrap", Invert = "invert", Brighten = "brighten", Darken = "darken",
        Solarize = "solarize",
        Blur1Min = "blur1_min", Blur2Min = "blur2_min", Blur3Min = "blur3_min",
        Blur1Max = "blur1_max", Blur2Max = "blur2_max", Blur3Max = "blur3_max",
        Blur1EdgeDarken = "blur1_edge_darken", Monitor = "monitor",
    }
}

builtins! {
    /// The per-vertex (`per_pixel_`) code's variables.
    Pv {
        Zoom = "zoom", ZoomExp = "zoomexp", Rot = "rot", Warp = "warp", Cx = "cx", Cy = "cy",
        Dx = "dx", Dy = "dy", Sx = "sx", Sy = "sy",
        Time = "time", Fps = "fps", Bass = "bass", Mid = "mid", Treb = "treb",
        BassAtt = "bass_att", MidAtt = "mid_att", TrebAtt = "treb_att", Frame = "frame",
        Progress = "progress", MeshX = "meshx", MeshY = "meshy", PixelsX = "pixelsx",
        PixelsY = "pixelsy", AspectX = "aspectx", AspectY = "aspecty",
        X = "x", Y = "y", Rad = "rad", Ang = "ang",
    }
}

builtins! {
    /// A custom wave's per-frame (and init) code's variables.
    Wf {
        Time = "time", Fps = "fps", Frame = "frame", Progress = "progress",
        Bass = "bass", Mid = "mid", Treb = "treb",
        BassAtt = "bass_att", MidAtt = "mid_att", TrebAtt = "treb_att",
        R = "r", G = "g", B = "b", A = "a", Samples = "samples",
    }
}

builtins! {
    /// A custom wave's per-point code's variables.
    Wp {
        Time = "time", Fps = "fps", Frame = "frame", Progress = "progress",
        Bass = "bass", Mid = "mid", Treb = "treb",
        BassAtt = "bass_att", MidAtt = "mid_att", TrebAtt = "treb_att",
        Sample = "sample", Value1 = "value1", Value2 = "value2", X = "x", Y = "y",
        R = "r", G = "g", B = "b", A = "a",
    }
}

builtins! {
    /// A custom shape's per-frame (and init) code's variables.
    Sf {
        Time = "time", Fps = "fps", Frame = "frame", Progress = "progress",
        Bass = "bass", Mid = "mid", Treb = "treb",
        BassAtt = "bass_att", MidAtt = "mid_att", TrebAtt = "treb_att",
        X = "x", Y = "y", Rad = "rad", Ang = "ang", TexAng = "tex_ang", TexZoom = "tex_zoom",
        Sides = "sides", Textured = "textured", Instance = "instance", NumInst = "num_inst",
        Additive = "additive", Thick = "thick",
        R = "r", G = "g", B = "b", A = "a", R2 = "r2", G2 = "g2", B2 = "b2", A2 = "a2",
        BorderR = "border_r", BorderG = "border_g", BorderB = "border_b", BorderA = "border_a",
    }
}

/// The per-frame variables MilkDrop sets from the preset's starting values at the start of
/// every frame, so that per-frame code changes them for that frame only.
const PF_FROM_PRESET: &[Pf] = &[
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
    Pf::Decay,
    Pf::WaveA,
    Pf::WaveR,
    Pf::WaveG,
    Pf::WaveB,
    Pf::WaveX,
    Pf::WaveY,
    Pf::WaveMystery,
    Pf::WaveMode,
    Pf::ObSize,
    Pf::ObR,
    Pf::ObG,
    Pf::ObB,
    Pf::ObA,
    Pf::IbSize,
    Pf::IbR,
    Pf::IbG,
    Pf::IbB,
    Pf::IbA,
    Pf::MvX,
    Pf::MvY,
    Pf::MvDx,
    Pf::MvDy,
    Pf::MvL,
    Pf::MvR,
    Pf::MvG,
    Pf::MvB,
    Pf::MvA,
    Pf::EchoZoom,
    Pf::EchoAlpha,
    Pf::EchoOrient,
    Pf::WaveUseDots,
    Pf::WaveThick,
    Pf::WaveAdditive,
    Pf::WaveBrighten,
    Pf::DarkenCenter,
    Pf::Gamma,
    Pf::Wrap,
    Pf::Invert,
    Pf::Brighten,
    Pf::Darken,
    Pf::Solarize,
    Pf::Blur1Min,
    Pf::Blur2Min,
    Pf::Blur3Min,
    Pf::Blur1Max,
    Pf::Blur2Max,
    Pf::Blur3Max,
    Pf::Blur1EdgeDarken,
];

/// What every kind of code can read about the moment: the same for both presets while one
/// blends into the other.
#[derive(Clone, Copy, Debug, Default)]
pub struct Moment {
    /// Seconds since MilkDrop started.
    pub time: f64,
    pub frame: f64,
    pub fps: f64,
    /// How far through its time the preset is, 0 to 1.
    pub progress: f64,
    pub levels: Levels,
    pub mesh: (usize, usize),
    pub pixels: (u32, u32),
    /// The equations' `aspectx` and `aspecty`: 1, or more along the shorter side.
    pub inv_aspect: (f64, f64),
}

/// One of the values every kind of code reads from the [`Moment`].
#[derive(Clone, Copy, Debug)]
enum Input {
    Time,
    Fps,
    Frame,
    Progress,
    Bass,
    Mid,
    Treb,
    BassAtt,
    MidAtt,
    TrebAtt,
    MeshX,
    MeshY,
    PixelsX,
    PixelsY,
    AspectX,
    AspectY,
}

impl Input {
    fn named(name: &str) -> Option<Input> {
        Some(match name {
            "time" => Input::Time,
            "fps" => Input::Fps,
            "frame" => Input::Frame,
            "progress" => Input::Progress,
            "bass" => Input::Bass,
            "mid" => Input::Mid,
            "treb" => Input::Treb,
            "bass_att" => Input::BassAtt,
            "mid_att" => Input::MidAtt,
            "treb_att" => Input::TrebAtt,
            "meshx" => Input::MeshX,
            "meshy" => Input::MeshY,
            "pixelsx" => Input::PixelsX,
            "pixelsy" => Input::PixelsY,
            "aspectx" => Input::AspectX,
            "aspecty" => Input::AspectY,
            _ => return None,
        })
    }

    fn value(self, m: &Moment) -> f64 {
        let l = m.levels;
        match self {
            Input::Time => m.time,
            Input::Fps => m.fps,
            Input::Frame => m.frame,
            Input::Progress => m.progress,
            Input::Bass => f64::from(l.bass),
            Input::Mid => f64::from(l.mid),
            Input::Treb => f64::from(l.treb),
            Input::BassAtt => f64::from(l.bass_att),
            Input::MidAtt => f64::from(l.mid_att),
            Input::TrebAtt => f64::from(l.treb_att),
            Input::MeshX => m.mesh.0 as f64,
            Input::MeshY => m.mesh.1 as f64,
            Input::PixelsX => f64::from(m.pixels.0),
            Input::PixelsY => f64::from(m.pixels.1),
            Input::AspectX => m.inv_aspect.0,
            Input::AspectY => m.inv_aspect.1,
        }
    }
}

/// A table of variables with its built-in names, `q1`-`q32` and `t1`-`t8` resolved.
#[derive(Debug)]
struct Table {
    vars: Vars,
    slots: Box<[Slot]>,
    q: [Slot; Q_COUNT],
    t: [Slot; T_COUNT],
    /// The built-in variables that come from the [`Moment`].
    inputs: Box<[(Slot, Input)]>,
}

impl Table {
    fn new(names: &[&str]) -> Table {
        let mut vars = Vars::new();
        let slots: Box<[Slot]> = names.iter().map(|n| vars.slot(n)).collect();
        let q = std::array::from_fn(|i| vars.slot(&format!("q{}", i + 1)));
        let t = std::array::from_fn(|i| vars.slot(&format!("t{}", i + 1)));
        let inputs = names
            .iter()
            .zip(&slots)
            .filter_map(|(n, s)| Input::named(n).map(|i| (*s, i)))
            .collect();
        Table {
            vars,
            slots,
            q,
            t,
            inputs,
        }
    }

    fn get(&self, i: usize) -> f64 {
        self.vars.get(self.slots[i])
    }

    fn set(&mut self, i: usize, v: f64) {
        self.vars.set(self.slots[i], v);
    }

    fn qs(&self) -> [f64; Q_COUNT] {
        self.q.map(|s| self.vars.get(s))
    }

    fn set_qs(&mut self, q: &[f64; Q_COUNT]) {
        for (s, v) in self.q.iter().zip(q) {
            self.vars.set(*s, *v);
        }
    }

    fn ts(&self) -> [f64; T_COUNT] {
        self.t.map(|s| self.vars.get(s))
    }

    fn set_ts(&mut self, t: &[f64; T_COUNT]) {
        for (s, v) in self.t.iter().zip(t) {
            self.vars.set(*s, *v);
        }
    }

    /// Set the variables that come from the moment.
    fn set_moment(&mut self, m: &Moment) {
        for &(s, input) in &*self.inputs {
            self.vars.set(s, input.value(m));
        }
    }
}

/// Compile one block of a preset's code, logging (and dropping) code that does not compile,
/// as MilkDrop reported it and ran the preset without it.
fn compile(src: &str, table: &mut Table, dialect: Dialect, what: &str) -> Option<Program> {
    if src.trim().is_empty() {
        return None;
    }
    match Program::compile(src, &mut table.vars, dialect) {
        Ok(p) if p.is_empty() => None,
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("milkdrop: error in {what} code: {e}");
            None
        }
    }
}

/// The values of a frame's per-frame variables once its code has run, and its `q`s.
#[derive(Clone, Debug)]
pub struct FrameVars {
    values: Box<[f64]>,
    pub q: [f64; Q_COUNT],
}

impl FrameVars {
    #[must_use]
    pub fn get(&self, v: Pf) -> f64 {
        self.values[v as usize]
    }

    pub fn set(&mut self, v: Pf, value: f64) {
        self.values[v as usize] = value;
    }

    /// Whether a flag-like variable is on.
    #[must_use]
    pub fn on(&self, v: Pf) -> bool {
        self.get(v) != 0.0
    }

    /// Blend in the other preset's values for everything that does not move pixels, as
    /// MilkDrop did while one preset blends into the next: numbers mix, and flags switch at
    /// `snap`. `mix` is how far this preset has come in (0 to 1).
    pub fn blend_from(&mut self, old: &FrameVars, mix: f64, snap: f64) {
        const MIXED: &[Pf] = &[
            Pf::Decay,
            Pf::WaveA,
            Pf::WaveR,
            Pf::WaveG,
            Pf::WaveB,
            Pf::WaveX,
            Pf::WaveY,
            Pf::WaveMystery,
            Pf::ObSize,
            Pf::ObR,
            Pf::ObG,
            Pf::ObB,
            Pf::ObA,
            Pf::IbSize,
            Pf::IbR,
            Pf::IbG,
            Pf::IbB,
            Pf::IbA,
            Pf::MvX,
            Pf::MvY,
            Pf::MvDx,
            Pf::MvDy,
            Pf::MvL,
            Pf::MvR,
            Pf::MvG,
            Pf::MvB,
            Pf::MvA,
            Pf::EchoZoom,
            Pf::EchoAlpha,
            Pf::Gamma,
            Pf::Blur1Min,
            Pf::Blur2Min,
            Pf::Blur3Min,
            Pf::Blur1Max,
            Pf::Blur2Max,
            Pf::Blur3Max,
            Pf::Blur1EdgeDarken,
        ];
        const SNAPPED: &[Pf] = &[
            Pf::EchoOrient,
            Pf::WaveUseDots,
            Pf::WaveThick,
            Pf::WaveAdditive,
            Pf::WaveBrighten,
            Pf::DarkenCenter,
            Pf::Wrap,
            Pf::Invert,
            Pf::Brighten,
            Pf::Darken,
            Pf::Solarize,
        ];
        for &v in MIXED {
            self.set(v, mix * self.get(v) + (1.0 - mix) * old.get(v));
        }
        if mix < snap {
            for &v in SNAPPED {
                self.set(v, old.get(v));
            }
        }
    }
}

/// A custom wave's settings from the file, which its code cannot change.
#[derive(Clone, Copy, Debug)]
pub struct WaveSettings {
    pub samples: f64,
    pub sep: i32,
    pub spectrum: bool,
    pub dots: bool,
    pub thick: bool,
    pub additive: bool,
    pub scaling: f64,
    pub smoothing: f64,
    pub color: [f64; 4],
}

/// A custom wave while it runs.
#[derive(Debug)]
pub struct WaveState {
    pub settings: WaveSettings,
    frame_table: Table,
    point_table: Table,
    per_frame: Option<Program>,
    per_point: Option<Program>,
    t_after_init: [f64; T_COUNT],
}

/// What a custom wave's per-frame code decided.
#[derive(Clone, Copy, Debug)]
pub struct WaveFrame {
    pub samples: f64,
    pub color: [f64; 4],
}

/// One point of a custom wave, after its per-point code.
#[derive(Clone, Copy, Debug)]
pub struct WavePoint {
    pub x: f64,
    pub y: f64,
    pub color: [f64; 4],
}

impl WaveState {
    /// Run the per-frame code for this frame (MilkDrop's `LoadCustomWavePerFrameEvallibVars`
    /// then the code), and pass `q` and `t` on to the per-point code.
    pub fn run_frame(
        &mut self,
        globals: &mut Globals,
        m: &Moment,
        q: &[f64; Q_COUNT],
    ) -> WaveFrame {
        let f = &mut self.frame_table;
        f.set_moment(m);
        f.set_qs(q);
        f.set_ts(&self.t_after_init);
        let s = &self.settings;
        for (v, c) in [Wf::R, Wf::G, Wf::B, Wf::A].into_iter().zip(s.color) {
            f.set(v as usize, c);
        }
        f.set(Wf::Samples as usize, s.samples);
        self.point_table.set_moment(m);
        if let Some(code) = &self.per_frame {
            code.run(&mut f.vars, globals);
        }
        let (qs, ts) = (f.qs(), f.ts());
        self.point_table.set_qs(&qs);
        self.point_table.set_ts(&ts);
        WaveFrame {
            samples: f.get(Wf::Samples as usize),
            color: [Wf::R, Wf::G, Wf::B, Wf::A].map(|v| f.get(v as usize)),
        }
    }

    /// Run the per-point code for one point.
    pub fn run_point(
        &mut self,
        globals: &mut Globals,
        sample: f64,
        values: (f64, f64),
        color: [f64; 4],
    ) -> WavePoint {
        let p = &mut self.point_table;
        p.set(Wp::Sample as usize, sample);
        p.set(Wp::Value1 as usize, values.0);
        p.set(Wp::Value2 as usize, values.1);
        p.set(Wp::X as usize, 0.5 + values.0);
        p.set(Wp::Y as usize, 0.5 + values.1);
        for (v, c) in [Wp::R, Wp::G, Wp::B, Wp::A].into_iter().zip(color) {
            p.set(v as usize, c);
        }
        if let Some(code) = &self.per_point {
            code.run(&mut p.vars, globals);
        }
        WavePoint {
            x: p.get(Wp::X as usize),
            y: p.get(Wp::Y as usize),
            color: [Wp::R, Wp::G, Wp::B, Wp::A].map(|v| p.get(v as usize)),
        }
    }
}

/// A custom shape's starting values, which its per-frame code starts from for each instance.
#[derive(Clone, Debug)]
pub struct ShapeSettings {
    pub instances: usize,
    /// Values for [`Sf`]'s variables from `x` on, in that order.
    start: Box<[f64]>,
}

/// A custom shape while it runs.
#[derive(Debug)]
pub struct ShapeState {
    pub settings: ShapeSettings,
    table: Table,
    per_frame: Option<Program>,
    t_after_init: [f64; T_COUNT],
}

/// One instance of a custom shape, as its per-frame code left it.
#[derive(Clone, Copy, Debug)]
pub struct ShapeInstance {
    pub x: f64,
    pub y: f64,
    pub rad: f64,
    pub ang: f64,
    pub tex_ang: f64,
    pub tex_zoom: f64,
    pub sides: f64,
    pub textured: bool,
    pub additive: bool,
    pub thick: bool,
    pub color: [f64; 4],
    pub color2: [f64; 4],
    pub border: [f64; 4],
}

impl ShapeState {
    /// Run the per-frame code for one instance (MilkDrop's
    /// `LoadCustomShapePerFrameEvallibVars` then the code).
    pub fn run_instance(
        &mut self,
        globals: &mut Globals,
        m: &Moment,
        q: &[f64; Q_COUNT],
        instance: usize,
    ) -> ShapeInstance {
        let t = &mut self.table;
        t.set_moment(m);
        t.set_qs(q);
        t.set_ts(&self.t_after_init);
        for (i, v) in self.settings.start.iter().enumerate() {
            t.set(Sf::X as usize + i, *v);
        }
        t.set(Sf::Instance as usize, instance as f64);
        if let Some(code) = &self.per_frame {
            code.run(&mut t.vars, globals);
        }
        let g = |v: Sf| t.get(v as usize);
        let rgba = |r, gg, b, a| [g(r), g(gg), g(b), g(a)];
        ShapeInstance {
            x: g(Sf::X),
            y: g(Sf::Y),
            rad: g(Sf::Rad),
            ang: g(Sf::Ang),
            tex_ang: g(Sf::TexAng),
            tex_zoom: g(Sf::TexZoom),
            sides: g(Sf::Sides),
            // MilkDrop read these flags with (int) casts.
            textured: g(Sf::Textured).trunc() != 0.0,
            additive: g(Sf::Additive).trunc() != 0.0,
            thick: g(Sf::Thick).trunc() != 0.0,
            color: rgba(Sf::R, Sf::G, Sf::B, Sf::A),
            color2: rgba(Sf::R2, Sf::G2, Sf::B2, Sf::A2),
            border: rgba(Sf::BorderR, Sf::BorderG, Sf::BorderB, Sf::BorderA),
        }
    }
}

/// The shape variables a shape starts each instance from, in [`Sf`]'s order from `x`.
const SHAPE_START: &[&str] = &[
    "x", "y", "rad", "ang", "tex_ang", "tex_zoom", "sides", "textured", "instance", "num_inst",
    "additive", "thick", "r", "g", "b", "a", "r2", "g2", "b2", "a2", "border_r", "border_g",
    "border_b", "border_a",
];

/// A loaded preset, ready to run frame after frame.
pub struct PresetState {
    pub preset: Preset,
    frame_table: Table,
    vertex_table: Table,
    per_frame: Option<Program>,
    per_vertex: Option<Program>,
    q_after_init: [f64; Q_COUNT],
    monitor_after_init: f64,
    /// The variables per-frame code starts each frame from (see [`PF_FROM_PRESET`]).
    start: Box<[(Pf, f64)]>,
    pub waves: Vec<Option<WaveState>>,
    pub shapes: Vec<Option<ShapeState>>,
    /// Four random numbers chosen when the preset loads, for its shaders.
    pub rand_preset: [f32; 4],
    /// The random turns behind its shaders' `rot_s1` to `rot_uf4` matrices.
    pub rotations: [Rotation; 20],
}

/// One of a preset's slowly turning rotations (MilkDrop's `RandomizePresetVars`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Rotation {
    pub base: [f32; 3],
    pub speed: [f32; 3],
    pub translate: [f32; 3],
}

impl Rotation {
    /// The `k`th of a preset's 20, from `random` numbers in 0..1: later ones turn faster.
    fn random(k: usize, random: &mut impl FnMut() -> f32) -> Rotation {
        let speed_mult = 0.9 * (k as f32 / 8.0).powf(3.2);
        let mut three = |f: &mut dyn FnMut(f32) -> f32| [f(random()), f(random()), f(random())];
        let translate = three(&mut |r| r * 2.0 - 1.0);
        let base = three(&mut |r| r * std::f32::consts::TAU);
        let speed = three(&mut |r| (r * 2.0 - 1.0) * speed_mult);
        Rotation {
            base,
            speed,
            translate,
        }
    }
}

impl fmt::Debug for PresetState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PresetState")
            .field("version", &self.preset.version)
            .finish_non_exhaustive()
    }
}

impl PresetState {
    /// Compile `preset`'s code and run its init code, as MilkDrop's `RecompileExpressions`.
    #[must_use]
    pub fn new(preset: Preset, globals: &mut Globals, m: &Moment) -> PresetState {
        let dialect = preset.dialect();
        let mut frame_table = Table::new(Pf::NAMES);
        let mut vertex_table = Table::new(Pv::NAMES);
        let init = compile(&preset.init, &mut frame_table, dialect, "per-frame init");
        let per_frame = compile(&preset.per_frame, &mut frame_table, dialect, "per-frame");
        let per_vertex = compile(&preset.per_pixel, &mut vertex_table, dialect, "per-vertex");
        let start = PF_FROM_PRESET
            .iter()
            .map(|&v| (v, preset.values.get(Pf::NAMES[v as usize])))
            .collect();
        let rand_preset = std::array::from_fn(|_| globals.random() as f32);
        let mut random = || globals.random() as f32;
        let rotations = std::array::from_fn(|k| Rotation::random(k, &mut random));
        let mut state = PresetState {
            frame_table,
            vertex_table,
            per_frame,
            per_vertex,
            q_after_init: [0.0; Q_COUNT],
            monitor_after_init: 0.0,
            start,
            waves: Vec::new(),
            shapes: Vec::new(),
            rand_preset,
            rotations,
            preset,
        };
        if let Some(init) = init {
            state.load_frame_vars(m);
            init.run(&mut state.frame_table.vars, globals);
            state.q_after_init = state.frame_table.qs();
            state.monitor_after_init = state.frame_table.get(Pf::Monitor as usize);
        }
        state.waves = (0..CUSTOM)
            .map(|i| state.load_wave(i, globals, m, dialect))
            .collect();
        state.shapes = (0..CUSTOM)
            .map(|i| state.load_shape(i, globals, m, dialect))
            .collect();
        state
    }

    fn load_wave(
        &self,
        i: usize,
        globals: &mut Globals,
        m: &Moment,
        dialect: Dialect,
    ) -> Option<WaveState> {
        let w = &self.preset.waves[i];
        let v = &w.values;
        if v.get("enabled") == 0.0 {
            return None;
        }
        let what = |part: &str| format!("wave {i} {part}");
        let mut frame_table = Table::new(Wf::NAMES);
        let mut point_table = Table::new(Wp::NAMES);
        let init = compile(&w.init, &mut frame_table, dialect, &what("init"));
        let per_frame = compile(&w.per_frame, &mut frame_table, dialect, &what("per-frame"));
        let per_point = compile(&w.per_point, &mut point_table, dialect, &what("per-point"));
        let settings = WaveSettings {
            samples: v.get("samples"),
            // MilkDrop read these as ints.
            sep: v.get("sep").trunc() as i32,
            spectrum: v.get("spectrum").trunc() != 0.0,
            dots: v.get("usedots").trunc() != 0.0,
            thick: v.get("thick").trunc() != 0.0,
            additive: v.get("additive").trunc() != 0.0,
            scaling: v.get("scaling"),
            smoothing: v.get("smoothing"),
            color: ["r", "g", "b", "a"].map(|c| v.get(c)),
        };
        let mut wave = WaveState {
            settings,
            frame_table,
            point_table,
            per_frame,
            per_point,
            t_after_init: [0.0; T_COUNT],
        };
        if let Some(init) = init {
            let f = &mut wave.frame_table;
            f.set_moment(m);
            f.set_qs(&self.q_after_init);
            for (c, val) in [Wf::R, Wf::G, Wf::B, Wf::A].into_iter().zip(settings.color) {
                f.set(c as usize, val);
            }
            f.set(Wf::Samples as usize, settings.samples);
            init.run(&mut f.vars, globals);
            wave.t_after_init = f.ts();
        }
        Some(wave)
    }

    fn load_shape(
        &self,
        i: usize,
        globals: &mut Globals,
        m: &Moment,
        dialect: Dialect,
    ) -> Option<ShapeState> {
        let s = &self.preset.shapes[i];
        let v: &Values = &s.values;
        if v.get("enabled") == 0.0 {
            return None;
        }
        let what = |part: &str| format!("shape {i} {part}");
        let mut table = Table::new(Sf::NAMES);
        let init = compile(&s.init, &mut table, dialect, &what("init"));
        let per_frame = compile(&s.per_frame, &mut table, dialect, &what("per-frame"));
        // MilkDrop read the instance count as an int; it drew at least one.
        let instances = v.get("num_inst").trunc().clamp(1.0, 1024.0) as usize;
        let start = SHAPE_START
            .iter()
            .map(|n| match *n {
                "instance" => 0.0,
                "num_inst" => instances as f64,
                n => v.get(n),
            })
            .collect();
        let mut shape = ShapeState {
            settings: ShapeSettings { instances, start },
            table,
            per_frame,
            t_after_init: [0.0; T_COUNT],
        };
        if let Some(init) = init {
            let t = &mut shape.table;
            t.set_moment(m);
            t.set_qs(&self.q_after_init);
            for (k, val) in shape.settings.start.iter().enumerate() {
                t.set(Sf::X as usize + k, *val);
            }
            init.run(&mut t.vars, globals);
            shape.t_after_init = t.ts();
        }
        Some(shape)
    }

    /// MilkDrop's `LoadPerFrameEvallibVars`: everything per-frame code starts a frame from.
    fn load_frame_vars(&mut self, m: &Moment) {
        let t = &mut self.frame_table;
        for &(v, value) in &*self.start {
            t.set(v as usize, value);
        }
        t.set_moment(m);
        t.set_qs(&self.q_after_init);
        t.set(Pf::Monitor as usize, self.monitor_after_init);
    }

    /// Run the per-frame code for a frame and return what it left.
    pub fn run_frame(&mut self, globals: &mut Globals, m: &Moment) -> FrameVars {
        self.load_frame_vars(m);
        if let Some(code) = &self.per_frame {
            code.run(&mut self.frame_table.vars, globals);
        }
        self.monitor_after_init = self.frame_table.get(Pf::Monitor as usize);
        let t = &self.frame_table;
        let mut out = FrameVars {
            values: (0..Pf::NAMES.len()).map(|i| t.get(i)).collect(),
            q: t.qs(),
        };
        // MilkDrop's range checks.
        out.set(Pf::Gamma, out.get(Pf::Gamma).clamp(0.0, 8.0));
        out.set(Pf::EchoZoom, out.get(Pf::EchoZoom).clamp(0.001, 1000.0));
        // The per-vertex code sees the same moment and q values.
        self.vertex_table.set_moment(m);
        self.vertex_table.set_qs(&out.q);
        out
    }

    /// Whether the preset has per-vertex code.
    #[must_use]
    pub fn has_vertex_code(&self) -> bool {
        self.per_vertex.is_some()
    }

    /// Run the per-vertex code for one vertex of the warp mesh. `motion` holds the frame's
    /// zoom, zoomexp, rot, warp, cx, cy, dx, dy, sx and sy, in that order; it comes back as the
    /// code changed it for this vertex.
    pub fn run_vertex(
        &mut self,
        globals: &mut Globals,
        pos: (f64, f64),
        rad: f64,
        ang: f64,
        motion: &mut [f64; 10],
    ) {
        let Some(code) = &self.per_vertex else {
            return;
        };
        let t = &mut self.vertex_table;
        t.set(Pv::X as usize, pos.0);
        t.set(Pv::Y as usize, pos.1);
        t.set(Pv::Rad as usize, rad);
        t.set(Pv::Ang as usize, ang);
        for (i, v) in motion.iter().enumerate() {
            t.set(Pv::Zoom as usize + i, *v);
        }
        code.run(&mut t.vars, globals);
        for (i, v) in motion.iter_mut().enumerate() {
            *v = t.get(Pv::Zoom as usize + i);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn moment(frame: f64) -> Moment {
        Moment {
            time: frame / 30.0,
            frame,
            fps: 30.0,
            progress: 0.0,
            levels: Levels::default(),
            mesh: (48, 36),
            pixels: (640, 480),
            inv_aspect: (1.0, 4.0 / 3.0),
        }
    }

    #[test]
    fn names_match_their_enums() {
        assert_eq!(Pf::NAMES[Pf::Monitor as usize], "monitor");
        assert_eq!(Pf::NAMES[Pf::Blur3Max as usize], "blur3_max");
        assert_eq!(Pv::NAMES[Pv::Ang as usize], "ang");
        assert_eq!(Wp::NAMES[Wp::A as usize], "a");
        assert_eq!(Sf::NAMES[Sf::BorderA as usize], "border_a");
        assert_eq!(&Sf::NAMES[Sf::X as usize..], SHAPE_START);
        assert_eq!(&Pv::NAMES[..10], &Pf::NAMES[..10]);
    }

    #[test]
    fn per_frame_code_starts_each_frame_from_the_preset_and_keeps_its_own_variables() {
        let p = Preset::parse(
            "MILKDROP_PRESET_VERSION=201\n[preset00]\nzoom=1.1\nfDecay=0.9\n\
             per_frame_init_1=q1 = 5; counter = 100;\n\
             per_frame_1=zoom = zoom + 0.1; counter = counter + 1; q2 = q1 + 1; q1 = 0;\n",
        );
        let mut g = Globals::new(1);
        let mut s = PresetState::new(p, &mut g, &moment(0.0));
        let f1 = s.run_frame(&mut g, &moment(1.0));
        let f2 = s.run_frame(&mut g, &moment(2.0));
        // zoom is reset each frame; the user's counter carries on.
        assert!((f1.get(Pf::Zoom) - 1.2).abs() < 1e-9);
        assert!((f2.get(Pf::Zoom) - 1.2).abs() < 1e-9);
        assert_eq!(f2.get(Pf::Decay), 0.9);
        let counter = s.frame_table.vars.find("counter").unwrap();
        assert_eq!(s.frame_table.vars.get(counter), 102.0);
        // q1 starts every frame at its value after the init code.
        assert_eq!(f2.q[1], 6.0);
        assert_eq!(f2.q[0], 0.0);
        assert_eq!(f2.get(Pf::Time), 2.0 / 30.0);
    }

    #[test]
    fn per_vertex_code_sees_q_and_changes_motion_per_vertex() {
        let p = Preset::parse(
            "[preset00]\nper_frame_1=q3 = 2;\nper_pixel_1=zoom = zoom * q3 + x; rot = rad;\n",
        );
        let mut g = Globals::new(1);
        let mut s = PresetState::new(p, &mut g, &moment(0.0));
        let f = s.run_frame(&mut g, &moment(1.0));
        let mut motion = [f.get(Pf::Zoom), 1.0, 0.0, 1.0, 0.5, 0.5, 0.0, 0.0, 1.0, 1.0];
        s.run_vertex(&mut g, (0.25, 0.5), 0.7, 0.0, &mut motion);
        assert_eq!(motion[0], 2.25);
        assert_eq!(motion[2], 0.7);
    }

    #[test]
    fn waves_and_shapes_get_q_from_the_frame_and_t_from_their_init() {
        let p = Preset::parse(
            "[preset00]\n\
             wavecode_0_enabled=1\nwavecode_0_samples=100\nwavecode_0_r=0.5\n\
             wave_0_init1=t1 = 7;\n\
             wave_0_per_frame1=samples = 50; g = q1;\n\
             wave_0_per_point1=x = sample + t1; a = r;\n\
             shapecode_1_enabled=1\nshapecode_1_num_inst=3\nshapecode_1_rad=0.2\n\
             shape_1_init1=t2 = 1;\n\
             shape_1_per_frame1=rad = rad + instance*t2; n = n + 1;\n\
             per_frame_1=q1 = 0.25;\n",
        );
        let mut g = Globals::new(1);
        let m = moment(1.0);
        let mut s = PresetState::new(p, &mut g, &m);
        let f = s.run_frame(&mut g, &m);
        let wave = s.waves[0].as_mut().unwrap();
        let wf = wave.run_frame(&mut g, &m, &f.q);
        assert_eq!(wf.samples, 50.0);
        assert_eq!(wf.color, [0.5, 0.25, 1.0, 1.0]);
        let pt = wave.run_point(&mut g, 0.5, (0.1, 0.2), wf.color);
        assert_eq!(pt.x, 7.5);
        assert_eq!(pt.color[3], 0.5);
        assert!(s.waves[1].is_none());
        let shape = s.shapes[1].as_mut().unwrap();
        assert_eq!(shape.settings.instances, 3);
        let radii: Vec<f64> = (0..3)
            .map(|i| shape.run_instance(&mut g, &m, &f.q, i).rad)
            .collect();
        assert_eq!(radii, [0.2, 1.2, 2.2]);
        let n = shape.table.vars.find("n").unwrap();
        assert_eq!(shape.table.vars.get(n), 3.0);
    }

    #[test]
    fn blending_mixes_numbers_and_snaps_flags() {
        let mut new = FrameVars {
            values: vec![0.0; Pf::NAMES.len()].into_boxed_slice(),
            q: [0.0; Q_COUNT],
        };
        let mut old = new.clone();
        new.set(Pf::Decay, 1.0);
        old.set(Pf::Decay, 0.5);
        old.set(Pf::Invert, 1.0);
        new.set(Pf::Zoom, 2.0);
        old.set(Pf::Zoom, 1.0);
        let mut early = new.clone();
        early.blend_from(&old, 0.25, 0.5);
        assert_eq!(early.get(Pf::Decay), 0.625);
        assert!(early.on(Pf::Invert));
        // Motion is blended through the meshes instead.
        assert_eq!(early.get(Pf::Zoom), 2.0);
        new.blend_from(&old, 0.75, 0.5);
        assert!(!new.on(Pf::Invert));
    }

    #[test]
    fn broken_code_is_dropped_and_the_rest_runs() {
        let p = Preset::parse("[preset00]\nper_frame_1=zoom = ;\nper_pixel_1=rot = 1;\n");
        let mut g = Globals::new(1);
        let s = PresetState::new(p, &mut g, &moment(0.0));
        assert!(s.per_frame.is_none());
        assert!(s.has_vertex_code());
    }
}
