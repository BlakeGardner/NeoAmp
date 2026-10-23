//! Reading MilkDrop presets (`.milk` files): the starting values, the equations of the
//! preset and of its custom waves and shapes, and the warp and composite shaders.
//!
//! A `.milk` file is `key=value` lines under a `[preset00]` heading. Code comes a line at a
//! time (`per_frame_1=...`, `per_frame_2=...`), which MilkDrop joined with nothing between
//! them, and shader lines start with a backquote. The
//! keys of the starting values are MilkDrop's internal names (`fDecay`, `nWaveMode`); they are
//! stored under the names the equations use (`decay`, `wave_mode`), the mapping
//! milkdrop-preset-utils (butterchurn's converter) uses.

use std::collections::HashMap;

use super::eel::Dialect;

/// Custom waves and shapes a preset can have.
pub const CUSTOM: usize = 4;

/// A preset's starting values that it does not set itself, as MilkDrop chose them.
const BASE_DEFAULTS: &[(&str, f64)] = &[
    ("decay", 0.98),
    ("gammaadj", 2.0),
    ("echo_zoom", 2.0),
    ("echo_alpha", 0.0),
    ("echo_orient", 0.0),
    ("red_blue", 0.0),
    ("brighten", 0.0),
    ("darken", 0.0),
    ("wrap", 1.0),
    ("darken_center", 0.0),
    ("solarize", 0.0),
    ("invert", 0.0),
    ("bmotionvectorson", 1.0),
    ("fshader", 0.0),
    ("b1n", 0.0),
    ("b2n", 0.0),
    ("b3n", 0.0),
    ("b1x", 1.0),
    ("b2x", 1.0),
    ("b3x", 1.0),
    ("b1ed", 0.25),
    ("wave_mode", 0.0),
    ("additivewave", 0.0),
    ("wave_dots", 0.0),
    ("wave_thick", 0.0),
    ("wave_a", 0.8),
    ("wave_scale", 1.0),
    ("wave_smoothing", 0.75),
    ("wave_mystery", 0.0),
    ("modwavealphabyvolume", 0.0),
    ("modwavealphastart", 0.75),
    ("modwavealphaend", 0.95),
    ("wave_r", 1.0),
    ("wave_g", 1.0),
    ("wave_b", 1.0),
    ("wave_x", 0.5),
    ("wave_y", 0.5),
    ("wave_brighten", 1.0),
    ("mv_x", 12.0),
    ("mv_y", 9.0),
    ("mv_dx", 0.0),
    ("mv_dy", 0.0),
    ("mv_l", 0.9),
    ("mv_r", 1.0),
    ("mv_g", 1.0),
    ("mv_b", 1.0),
    ("mv_a", 1.0),
    ("warpanimspeed", 1.0),
    ("warpscale", 1.0),
    ("zoomexp", 1.0),
    ("zoom", 1.0),
    ("rot", 0.0),
    ("cx", 0.5),
    ("cy", 0.5),
    ("dx", 0.0),
    ("dy", 0.0),
    ("warp", 1.0),
    ("sx", 1.0),
    ("sy", 1.0),
    ("ob_size", 0.01),
    ("ob_r", 0.0),
    ("ob_g", 0.0),
    ("ob_b", 0.0),
    ("ob_a", 0.0),
    ("ib_size", 0.01),
    ("ib_r", 0.25),
    ("ib_g", 0.25),
    ("ib_b", 0.25),
    ("ib_a", 0.0),
];

/// A custom wave's starting values that it does not set itself.
const WAVE_DEFAULTS: &[(&str, f64)] = &[
    ("enabled", 0.0),
    ("samples", 512.0),
    ("sep", 0.0),
    ("scaling", 1.0),
    ("smoothing", 0.5),
    ("r", 1.0),
    ("g", 1.0),
    ("b", 1.0),
    ("a", 1.0),
    ("spectrum", 0.0),
    ("usedots", 0.0),
    ("thick", 0.0),
    ("additive", 0.0),
];

/// A custom shape's starting values that it does not set itself.
const SHAPE_DEFAULTS: &[(&str, f64)] = &[
    ("enabled", 0.0),
    ("sides", 4.0),
    ("additive", 0.0),
    ("thickoutline", 0.0),
    ("textured", 0.0),
    ("num_inst", 1.0),
    ("tex_zoom", 1.0),
    ("tex_ang", 0.0),
    ("x", 0.5),
    ("y", 0.5),
    ("rad", 0.1),
    ("ang", 0.0),
    ("r", 1.0),
    ("g", 0.0),
    ("b", 0.0),
    ("a", 1.0),
    ("r2", 0.0),
    ("g2", 1.0),
    ("b2", 0.0),
    ("a2", 0.0),
    ("border_r", 1.0),
    ("border_g", 1.0),
    ("border_b", 1.0),
    ("border_a", 0.1),
];

/// The name the equations use for a key of the file, where it differs.
fn equation_name(key: &str) -> &str {
    match key {
        "frating" => "rating",
        "fgammaadj" => "gammaadj",
        "fdecay" => "decay",
        "fvideoechozoom" => "echo_zoom",
        "fvideoechoalpha" => "echo_alpha",
        "nvideoechoorientation" => "echo_orient",
        "nwavemode" => "wave_mode",
        "badditivewaves" => "additivewave",
        "bwavedots" => "wave_dots",
        "bwavethick" => "wave_thick",
        "bmodwavealphabyvolume" => "modwavealphabyvolume",
        "bmaximizewavecolor" => "wave_brighten",
        "btexwrap" => "wrap",
        "bdarkencenter" => "darken_center",
        "bredbluestereo" => "red_blue",
        "bbrighten" => "brighten",
        "bdarken" => "darken",
        "bsolarize" => "solarize",
        "binvert" => "invert",
        "fwavealpha" => "wave_a",
        "fwavescale" => "wave_scale",
        "fwavesmoothing" => "wave_smoothing",
        "fwaveparam" => "wave_mystery",
        "fmodwavealphastart" => "modwavealphastart",
        "fmodwavealphaend" => "modwavealphaend",
        "fwarpanimspeed" => "warpanimspeed",
        "fwarpscale" => "warpscale",
        "fzoomexponent" => "zoomexp",
        "nmotionvectorsx" => "mv_x",
        "nmotionvectorsy" => "mv_y",
        "thick" | "bdrawthick" => "thick",
        "instances" | "num_instances" => "num_inst",
        "badditive" => "additive",
        "busedots" => "usedots",
        "bspectrum" => "spectrum",
        other => other,
    }
}

/// Starting values, under the names the equations use, falling back to MilkDrop's defaults.
#[derive(Clone, Debug, Default)]
pub struct Values {
    set: HashMap<String, f64>,
    defaults: &'static [(&'static str, f64)],
}

impl Values {
    fn new(defaults: &'static [(&'static str, f64)]) -> Values {
        Values {
            set: HashMap::new(),
            defaults,
        }
    }

    /// The value of `name`, the preset's own or else the default; 0 for names MilkDrop
    /// gives no default.
    #[must_use]
    pub fn get(&self, name: &str) -> f64 {
        self.set.get(name).copied().unwrap_or_else(|| {
            self.defaults
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(0.0, |(_, v)| *v)
        })
    }

    /// Every value: the defaults, then the preset's own, which may name more.
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> {
        let defaults = self
            .defaults
            .iter()
            .filter(|(n, _)| !self.set.contains_key(*n))
            .map(|(n, v)| (*n, *v));
        defaults.chain(self.set.iter().map(|(n, v)| (n.as_str(), *v)))
    }

    fn insert(&mut self, key: &str, value: f64) {
        self.set.insert(equation_name(key).to_owned(), value);
    }
}

/// A custom wave: a line or dots drawn from the sound, point by point.
#[derive(Clone, Debug)]
pub struct Wave {
    pub values: Values,
    pub init: String,
    pub per_frame: String,
    pub per_point: String,
}

/// A custom shape: a polygon drawn, perhaps many times, each frame.
#[derive(Clone, Debug)]
pub struct Shape {
    pub values: Values,
    pub init: String,
    pub per_frame: String,
}

/// A parsed preset.
#[derive(Clone, Debug)]
pub struct Preset {
    /// `MILKDROP_PRESET_VERSION`: 201 for MilkDrop 2 presets, 100 when missing (MilkDrop 1).
    pub version: u32,
    pub values: Values,
    pub init: String,
    pub per_frame: String,
    pub per_pixel: String,
    pub waves: [Wave; CUSTOM],
    pub shapes: [Shape; CUSTOM],
    /// The warp and composite pixel shaders (HLSL), when the preset has them.
    pub warp: Option<String>,
    pub comp: Option<String>,
}

impl Preset {
    /// The equations' language: MilkDrop 1 presets used the first version.
    #[must_use]
    pub fn dialect(&self) -> Dialect {
        if self.version >= 200 {
            Dialect::Eel2
        } else {
            Dialect::Eel1
        }
    }

    /// Read a `.milk` file's text. Anything that is not understood is skipped, as MilkDrop
    /// skipped it.
    #[must_use]
    pub fn parse(text: &str) -> Preset {
        let mut p = Preset {
            version: 100,
            values: Values::new(BASE_DEFAULTS),
            init: String::new(),
            per_frame: String::new(),
            per_pixel: String::new(),
            waves: std::array::from_fn(|_| Wave {
                values: Values::new(WAVE_DEFAULTS),
                init: String::new(),
                per_frame: String::new(),
                per_point: String::new(),
            }),
            shapes: std::array::from_fn(|_| Shape {
                values: Values::new(SHAPE_DEFAULTS),
                init: String::new(),
                per_frame: String::new(),
            }),
            warp: None,
            comp: None,
        };
        let mut warp = String::new();
        let mut comp = String::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            if key == "milkdrop_preset_version" {
                p.version = value.trim().parse().unwrap_or(100);
                continue;
            }
            // Shader lines are kept as written; a backquote starts each one.
            if numbered(&key, "warp_").is_some() {
                push_line(&mut warp, value.trim_start_matches('`'));
                continue;
            }
            if numbered(&key, "comp_").is_some() {
                push_line(&mut comp, value.trim_start_matches('`'));
                continue;
            }
            if numbered(&key, "per_frame_init_").is_some() {
                push_code(&mut p.init, value);
            } else if numbered(&key, "per_frame_").is_some() {
                push_code(&mut p.per_frame, value);
            } else if numbered(&key, "per_pixel_").is_some() {
                push_code(&mut p.per_pixel, value);
            } else if let Some((i, rest)) = indexed(&key, "wavecode_") {
                p.waves[i].values.insert(rest, number(value));
            } else if let Some((i, rest)) = indexed(&key, "shapecode_") {
                p.shapes[i].values.insert(rest, number(value));
            } else if let Some((i, rest)) = indexed(&key, "wave_") {
                let wave = &mut p.waves[i];
                if numbered(rest, "init").is_some() {
                    push_code(&mut wave.init, value);
                } else if numbered(rest, "per_frame").is_some() {
                    push_code(&mut wave.per_frame, value);
                } else if numbered(rest, "per_point").is_some() {
                    push_code(&mut wave.per_point, value);
                }
            } else if let Some((i, rest)) = indexed(&key, "shape_") {
                let shape = &mut p.shapes[i];
                if numbered(rest, "init").is_some() {
                    push_code(&mut shape.init, value);
                } else if numbered(rest, "per_frame").is_some() {
                    push_code(&mut shape.per_frame, value);
                }
            } else if !key.starts_with('[') && !key.is_empty() {
                p.values.insert(&key, number(value));
            }
        }
        p.warp = (!warp.trim().is_empty()).then_some(warp);
        p.comp = (!comp.trim().is_empty()).then_some(comp);
        p
    }
}

/// The number after `prefix` in `key` (`per_frame_12` gives 12), if that is all there is.
fn numbered(key: &str, prefix: &str) -> Option<u32> {
    key.strip_prefix(prefix)?.parse().ok()
}

/// For keys like `wavecode_2_samples`: the index (below [`CUSTOM`]) and what follows it.
fn indexed<'a>(key: &'a str, prefix: &str) -> Option<(usize, &'a str)> {
    let rest = key.strip_prefix(prefix)?;
    let (i, rest) = rest.split_once('_')?;
    let i: usize = i.parse().ok()?;
    (i < CUSTOM).then_some((i, rest))
}

fn push_line(code: &mut String, line: &str) {
    code.push_str(line);
    code.push('\n');
}

/// Add a line of equations as MilkDrop did: without its `//` or `\\` comment, and joined
/// straight onto the line before, so that a name split across two lines is one name.
fn push_code(code: &mut String, line: &str) {
    let end = [line.find("//"), line.find("\\\\")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(line.len());
    code.push_str(&line[..end]);
}

/// A value as C's `atof` reads it: the longest number at the start, 0 if there is none.
fn number(text: &str) -> f64 {
    let t = text.trim();
    let end = t
        .char_indices()
        .take_while(|(i, c)| {
            c.is_ascii_digit()
                || matches!(c, '.' | 'e' | 'E')
                || (matches!(c, '+' | '-') && {
                    // A sign only at the start or right after an exponent's e.
                    *i == 0 || matches!(t.as_bytes()[i - 1], b'e' | b'E')
                })
        })
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    (1..=end)
        .rev()
        .find_map(|n| t[..n].parse().ok())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "MILKDROP_PRESET_VERSION=201\r\n\
        PSVERSION=2\r\n\
        [preset00]\r\n\
        fRating=4.000000\r\n\
        fDecay=0.500\r\n\
        nWaveMode=7\r\n\
        zoom=1.01\r\n\
        nMotionVectorsX=64.000\r\n\
        wavecode_0_enabled=1\r\n\
        wavecode_0_bSpectrum=1\r\n\
        wavecode_0_scaling=2.5\r\n\
        wave_0_per_frame1=r = 1;\r\n\
        wave_0_per_point1=x = sample;\r\n\
        wave_0_per_point2=y = value1;\r\n\
        shapecode_3_enabled=1\r\n\
        shapecode_3_num_inst=12\r\n\
        shape_3_init1=t1 = 2;\r\n\
        per_frame_init_1=q1 = 0;\r\n\
        per_frame_1=zoom = zoom + 0.1*bass;\r\n\
        per_frame_2=rot = 0.01;\r\n\
        per_pixel_1=zoom = zoom + rad*0.1;\r\n\
        warp_1=`shader_body\r\n\
        warp_2=`{\r\n\
        warp_3=`  ret = tex2D(sampler_main, uv).xyz;\r\n\
        warp_4=`}\r\n";

    #[test]
    fn a_milkdrop_2_preset_is_read_whole() {
        let p = Preset::parse(SAMPLE);
        assert_eq!(p.version, 201);
        assert_eq!(p.dialect(), Dialect::Eel2);
        assert_eq!(p.values.get("decay"), 0.5);
        assert_eq!(p.values.get("wave_mode"), 7.0);
        assert_eq!(p.values.get("zoom"), 1.01);
        assert_eq!(p.values.get("mv_x"), 64.0);
        assert_eq!(p.values.get("rating"), 4.0);
        // Not in the file: the default.
        assert_eq!(p.values.get("gammaadj"), 2.0);
        assert_eq!(p.init, "q1 = 0;");
        assert_eq!(p.per_frame, "zoom = zoom + 0.1*bass;rot = 0.01;");
        assert_eq!(p.per_pixel, "zoom = zoom + rad*0.1;");
        let w = &p.waves[0];
        assert_eq!(w.values.get("enabled"), 1.0);
        assert_eq!(w.values.get("spectrum"), 1.0);
        assert_eq!(w.values.get("scaling"), 2.5);
        assert_eq!(w.values.get("samples"), 512.0);
        assert_eq!(w.per_frame, "r = 1;");
        assert_eq!(w.per_point, "x = sample;y = value1;");
        assert_eq!(p.waves[1].values.get("enabled"), 0.0);
        let s = &p.shapes[3];
        assert_eq!(s.values.get("num_inst"), 12.0);
        assert_eq!(s.values.get("sides"), 4.0);
        assert_eq!(s.init, "t1 = 2;");
        assert_eq!(
            p.warp.as_deref(),
            Some("shader_body\n{\n  ret = tex2D(sampler_main, uv).xyz;\n}\n")
        );
        assert_eq!(p.comp, None);
    }

    #[test]
    fn code_lines_join_without_their_comments() {
        let p = Preset::parse(
            "per_frame_1=k = is_\nper_frame_2=beat; // the beat\nper_frame_3=x = 1; \\\\ old\n",
        );
        assert_eq!(p.per_frame, "k = is_beat; x = 1; ");
    }

    #[test]
    fn a_milkdrop_1_preset_has_no_version_and_old_rand() {
        let p = Preset::parse("[preset00]\nfDecay=0.9\nper_frame_1=x = rand(5);\n");
        assert_eq!(p.version, 100);
        assert_eq!(p.dialect(), Dialect::Eel1);
        assert_eq!(p.values.get("decay"), 0.9);
    }

    #[test]
    fn values_read_like_atof_and_odd_lines_are_skipped() {
        assert_eq!(number(" 1.5e2xyz"), 150.0);
        assert_eq!(number("-0.25"), -0.25);
        assert_eq!(number(".5"), 0.5);
        assert_eq!(number("abc"), 0.0);
        assert_eq!(number("1e"), 1.0);
        let p = Preset::parse("junk\n=\nwavecode_9_enabled=1\nwave_x=0.25\n");
        assert_eq!(p.values.get("wave_x"), 0.25);
        assert!(p.waves.iter().all(|w| w.values.get("enabled") == 0.0));
    }

    #[test]
    fn values_iterate_defaults_and_the_presets_own() {
        let p = Preset::parse("zoom=2\nmy_thing=3\n");
        let all: HashMap<&str, f64> = p.values.iter().collect();
        assert_eq!(all["zoom"], 2.0);
        assert_eq!(all["my_thing"], 3.0);
        assert_eq!(all["decay"], 0.98);
        assert_eq!(all.len(), BASE_DEFAULTS.len() + 1);
    }

    /// Every `.milk` file under the folder named by `NEOAMP_MILK_DIR`, recursively.
    fn corpus() -> Vec<(std::path::PathBuf, Preset)> {
        let Some(dir) = std::env::var_os("NEOAMP_MILK_DIR") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("milk"))
                    && let Ok(bytes) = std::fs::read(&path)
                {
                    let preset = Preset::parse(&String::from_utf8_lossy(&bytes));
                    out.push((path, preset));
                }
            }
        }
        out
    }

    /// Run by hand with `NEOAMP_MILK_DIR` set to a folder of presets, such as projectM's
    /// presets-cream-of-the-crop: the equations must compile, but for a few presets whose code
    /// is broken (6 of 225,285 blocks in that pack, such as `0.99l` or a line ending in `=`).
    #[test]
    #[ignore = "needs NEOAMP_MILK_DIR"]
    fn every_presets_equations_compile() {
        use super::super::eel::{Program, Vars};
        let presets = corpus();
        let mut failures = Vec::new();
        let mut blocks = 0;
        for (path, p) in &presets {
            let mut code: Vec<&str> = vec![&p.init, &p.per_frame, &p.per_pixel];
            for w in &p.waves {
                code.extend([w.init.as_str(), &w.per_frame, &w.per_point]);
            }
            for s in &p.shapes {
                code.extend([s.init.as_str(), &s.per_frame]);
            }
            for src in code {
                blocks += 1;
                if let Err(e) = Program::compile(src, &mut Vars::new(), p.dialect()) {
                    let from = src.floor_char_boundary(e.at.saturating_sub(60));
                    let to = src.ceil_char_boundary((e.at + 20).min(src.len()));
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    failures.push(format!("{name}: {e}: {:?}", &src[from..to]));
                }
            }
        }
        for f in failures.iter().take(40) {
            eprintln!("{f}");
        }
        eprintln!(
            "{} presets, {blocks} blocks of code, {} failed to compile",
            presets.len(),
            failures.len()
        );
        assert!(failures.len() * 1000 <= blocks, "more than 0.1% failed");
    }
}
