//! MilkDrop 2's pixel shaders, which presets write in Direct3D 9's HLSL, translated to WGSL
//! for wgpu: the C preprocessor ([`pre`]), the parser ([`syntax`]), and type checking and
//! writing WGSL (`emit`).
//!
//! A preset's shader is a header of declarations and functions, then `shader_body { ... }`,
//! which sets `ret` (the colour) from `uv` and the rest. MilkDrop compiled it after its
//! `include.fx`, whose definitions ([`prelude`]) are given here the same way, so presets'
//! `GetBlur1(uv)`, `q1` or `time` mean what they meant to MilkDrop.
//!
//! The WGSL reads its inputs from one uniform block (see [`INPUTS_SIZE`] for its layout) and
//! its textures from fixed bindings (see [`binding`]); the textures a preset brings are bound
//! in group 1, in the order of [`Translated::user_textures`].

mod emit;
mod pre;
mod syntax;
mod types;

use std::fmt::{self, Write as _};

pub use emit::{Filter, Texture};

/// Which of a preset's two shaders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Draws the previous frame, warped, into the new one.
    Warp,
    /// Draws the new frame to the screen.
    Comp,
}

/// Why a shader could not be translated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// The line of the preset's shader, from 1 (0 for MilkDrop's own definitions).
    pub line: u32,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for Error {}

/// A shader translated to WGSL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translated {
    pub wgsl: String,
    /// The textures it samples that come from the presets' textures folder, by name in
    /// lower case: `t_user0`, `t_user1` and so on in group 1.
    pub user_textures: Vec<String>,
    /// The highest of `sampler_blur1` to `sampler_blur3` it uses, 0 for none.
    pub blur: u8,
}

/// The size of the uniform block, in bytes. It holds, in order: `c`, MilkDrop's `_c0` to
/// `_c13` (14 vec4s); `q`, `q1` to `q32` (8 vec4s); `d`, the built-in shaders' settings (3
/// vec4s); `rand_frame` and `rand_preset` (vec4s); and `rot`, `rot_s1` to `rot_rand4` (24
/// `mat4x3`s, each four columns of three padded to four).
pub const INPUTS_SIZE: usize = (14 + 8 + 3 + 2) * 16 + 24 * 64;

/// Group 0's bindings: the uniform block, then the built-in textures, then the samplers.
pub const INPUTS_BINDING: u32 = 0;

/// The binding (in group 0) of a built-in texture; user textures are in group 1.
#[must_use]
pub fn binding(t: &Texture) -> Option<u32> {
    Some(match t {
        Texture::Main => 1,
        Texture::Blur(n) => 1 + u32::from(*n),
        Texture::NoiseLq => 5,
        Texture::NoiseLqLite => 6,
        Texture::NoiseMq => 7,
        Texture::NoiseHq => 8,
        Texture::NoiseVolLq => 9,
        Texture::NoiseVolHq => 10,
        Texture::User(_) => return None,
    })
}

/// The binding (in group 0) of a sampler.
#[must_use]
pub fn sampler_binding(f: Filter) -> u32 {
    match f {
        Filter::LinearWrap => 11,
        Filter::LinearClamp => 12,
        Filter::PointWrap => 13,
        Filter::PointClamp => 14,
    }
}

/// The most textures a preset can bring.
pub const MAX_USER_TEXTURES: usize = 6;

/// MilkDrop's `include.fx`, as HLSL: its constants, the uniforms it declares, and its
/// `#define`s. The built-in shaders' settings `_d0` to `_d2` are NeoAmp's.
fn prelude() -> String {
    let mut p = String::from(
        "#define M_PI 3.14159265359\n\
         #define M_PI_2 6.28318530718\n\
         #define M_INV_PI_2 0.159154943091895\n\
         uniform float4 rand_frame;\n\
         uniform float4 rand_preset;\n",
    );
    for i in 0..14 {
        let _ = writeln!(p, "uniform float4 _c{i};");
    }
    for c in "abcdefgh".chars() {
        let _ = writeln!(p, "uniform float4 _q{c};");
    }
    for i in 0..3 {
        let _ = writeln!(p, "uniform float4 _d{i};");
    }
    for group in ["s", "d", "f", "vf", "uf", "rand"] {
        for i in 1..=4 {
            let _ = writeln!(p, "uniform float4x3 rot_{group}{i};");
        }
    }
    let defines = [
        ("time", "_c2.x"),
        ("fps", "_c2.y"),
        ("frame", "_c2.z"),
        ("progress", "_c2.w"),
        ("bass", "_c3.x"),
        ("mid", "_c3.y"),
        ("treb", "_c3.z"),
        ("vol", "_c3.w"),
        ("bass_att", "_c4.x"),
        ("mid_att", "_c4.y"),
        ("treb_att", "_c4.z"),
        ("vol_att", "_c4.w"),
        ("aspect", "_c0"),
        ("texsize", "_c7"),
        ("roam_cos", "_c8"),
        ("roam_sin", "_c9"),
        ("slow_roam_cos", "_c10"),
        ("slow_roam_sin", "_c11"),
        ("mip_x", "_c12.x"),
        ("mip_y", "_c12.y"),
        ("mip_xy", "_c12.xy"),
        ("mip_avg", "_c12.z"),
        ("blur1_min", "_c6.z"),
        ("blur1_max", "_c6.w"),
        ("blur2_min", "_c13.x"),
        ("blur2_max", "_c13.y"),
        ("blur3_min", "_c13.z"),
        ("blur3_max", "_c13.w"),
        ("sampler_FC_main", "sampler_fc_main"),
        ("sampler_PC_main", "sampler_pc_main"),
        ("sampler_FW_main", "sampler_fw_main"),
        ("sampler_PW_main", "sampler_pw_main"),
        ("GetMain(uv)", "(tex2D(sampler_main,uv).xyz)"),
        ("GetPixel(uv)", "(tex2D(sampler_main,uv).xyz)"),
        (
            "GetBlur1(uv)",
            "(tex2D(sampler_blur1,uv).xyz*_c5.x + _c5.y)",
        ),
        (
            "GetBlur2(uv)",
            "(tex2D(sampler_blur2,uv).xyz*_c5.z + _c5.w)",
        ),
        (
            "GetBlur3(uv)",
            "(tex2D(sampler_blur3,uv).xyz*_c6.x + _c6.y)",
        ),
        ("lum(x)", "(dot(x,float3(0.32,0.49,0.29)))"),
        ("tex2d", "tex2D"),
        ("tex3d", "tex3D"),
    ];
    for (name, value) in defines {
        let _ = writeln!(p, "#define {name} {value}");
    }
    for i in 0..32 {
        let bank = "abcdefgh".as_bytes()[i / 4] as char;
        let _ = writeln!(
            p,
            "#define q{} _q{bank}.{}",
            i + 1,
            ["x", "y", "z", "w"][i % 4]
        );
    }
    p
}

/// MilkDrop 1's warp, for presets without a warp shader: the previous frame darkened by
/// `decay`, repeating or clamped at the edges by `wrap`.
pub const DEFAULT_WARP: &str = "\
shader_body
{
    float3 wrapped = tex2D(sampler_fw_main, uv).xyz;
    float3 clamped = tex2D(sampler_fc_main, uv).xyz;
    ret = (_d2.z != 0 ? wrapped : clamped) * _d0.x;
}
";

/// MilkDrop 1's composite, for presets without a composite shader: video echo, gamma, the
/// hue shader, and brighten, darken, solarize and invert.
pub const DEFAULT_COMP: &str = "\
shader_body
{
    float orient = _d1.x;
    float2 flip = float2(fmod(orient, 2) >= 1 ? -1 : 1, orient >= 2 ? -1 : 1);
    float2 uv_echo = (uv - 0.5) / _d0.z * flip + 0.5;
    ret = lerp(tex2D(sampler_main, uv).xyz, tex2D(sampler_main, uv_echo).xyz, _d0.w);
    ret = saturate(ret * _d0.y);
    float hue = _d1.y;
    if (hue >= 1)
        ret *= hue_shader;
    else if (hue > 0.001)
        ret *= (1 - hue) + hue * hue_shader;
    if (_d1.z != 0) ret = sqrt(ret);
    if (_d1.w != 0) ret *= ret;
    if (_d2.x != 0) ret = ret * (1 - ret) * 4;
    if (_d2.y != 0) ret = 1 - ret;
}
";

/// Translate a preset's shader to WGSL with a vertex and a fragment entry point (`vs_main`,
/// `fs_main`).
///
/// # Errors
///
/// When the shader uses HLSL that NeoAmp does not understand, or is not valid HLSL.
pub fn translate(src: &str, stage: Stage) -> Result<Translated, Error> {
    let mut pp = pre::Preprocessor::default();
    let prelude = pp.run(&prelude(), 0)?;
    let user = pp.run(src, 1)?;
    let prelude_toks = syntax::lex(&prelude, 0)?;
    let toks = syntax::lex(&user, 1)?;

    // The header runs to `shader_body`; the body is the `{ }` after it. Anything after the
    // body's closing brace is ignored, as MilkDrop did.
    let Some(k) = toks
        .iter()
        .position(|t| t.tok == syntax::Tok::Ident("shader_body".into()))
    else {
        return Err(Error {
            line: 1,
            message: "no shader_body".into(),
        });
    };
    let mut depth = 0;
    let mut end = None;
    for (i, t) in toks.iter().enumerate().skip(k + 1) {
        match t.tok {
            syntax::Tok::Punct("{") => depth += 1,
            syntax::Tok::Punct("}") => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ if depth == 0 => {
                return Err(Error {
                    line: t.line,
                    message: "expected '{' after shader_body".into(),
                });
            }
            _ => {}
        }
    }
    let Some(end) = end else {
        return Err(Error {
            line: toks[k].line,
            message: "shader_body has no closing brace".into(),
        });
    };
    let body_toks = toks[k + 1..=end].to_vec();
    let header_toks = toks[..k].to_vec();

    let mut items = syntax::Parser::new(prelude_toks).items()?;
    items.extend(syntax::Parser::new(header_toks).items()?);
    let body = syntax::Parser::new(body_toks).block()?;

    let mut e = emit::Emitter::new();
    e.find_assigned(&items, &body);
    e.items(&items)?;
    let body = e.body(&body, stage == Stage::Comp)?;
    if e.used.user.len() > MAX_USER_TEXTURES {
        return Err(Error {
            line: 1,
            message: format!("more than {MAX_USER_TEXTURES} textures"),
        });
    }
    Ok(Translated {
        wgsl: assemble(&e, &body),
        user_textures: e.used.user.clone(),
        blur: e.used.blur,
    })
}

/// The whole WGSL module around the translated functions.
fn assemble(e: &emit::Emitter, body: &str) -> String {
    let mut w = String::from(
        "diagnostic(off, derivative_uniformity);\n\n\
         struct Inputs {\n\
         \x20   c: array<vec4<f32>, 14>,\n\
         \x20   q: array<vec4<f32>, 8>,\n\
         \x20   d: array<vec4<f32>, 3>,\n\
         \x20   rand_frame: vec4<f32>,\n\
         \x20   rand_preset: vec4<f32>,\n\
         \x20   rot: array<mat4x3<f32>, 24>,\n\
         }\n\n",
    );
    let _ = writeln!(
        w,
        "@group(0) @binding({INPUTS_BINDING}) var<uniform> inp: Inputs;"
    );
    let mut textures: Vec<&Texture> = e.used.textures.iter().collect();
    textures.sort();
    for t in textures {
        let kind = if t.is_volume() {
            "texture_3d<f32>"
        } else {
            "texture_2d<f32>"
        };
        let name = t.binding_name();
        match (binding(t), t) {
            (Some(b), _) => {
                let _ = writeln!(w, "@group(0) @binding({b}) var {name}: {kind};");
            }
            (None, Texture::User(i)) => {
                let _ = writeln!(w, "@group(1) @binding({i}) var {name}: {kind};");
            }
            (None, _) => {}
        }
    }
    let mut filters: Vec<&Filter> = e.used.filters.iter().collect();
    filters.sort();
    for f in filters {
        let _ = writeln!(
            w,
            "@group(0) @binding({}) var {}: sampler;",
            sampler_binding(*f),
            f.binding_name()
        );
    }
    w.push('\n');
    w.push_str(&e.private_decls);
    w.push_str(
        "\nstruct VertexOut {\n\
         \x20   @builtin(position) position: vec4<f32>,\n\
         \x20   @location(0) color: vec4<f32>,\n\
         \x20   @location(1) uv: vec2<f32>,\n\
         \x20   @location(2) uv_orig: vec2<f32>,\n\
         }\n\n\
         @vertex\n\
         fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, \
         @location(2) color: vec4<f32>) -> VertexOut {\n\
         \x20   var out: VertexOut;\n\
         \x20   out.position = vec4<f32>(pos, 0.0, 1.0);\n\
         \x20   out.color = color;\n\
         \x20   out.uv = uv;\n\
         \x20   out.uv_orig = vec2<f32>(pos.x * 0.5 + 0.5, 0.5 - pos.y * 0.5);\n\
         \x20   return out;\n\
         }\n\n",
    );
    w.push_str(&e.function_code);
    w.push_str(body);
    let _ = write!(w, "\nfn init_statics() {{\n{}}}\n\n", e.init);
    // rad and ang as MilkDrop's UvToMathSpace computed them for its meshes: 0 to 1 from the
    // centre to the corners, and 0 to 2pi counterclockwise from three o'clock.
    w.push_str(
        "@fragment\n\
         fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {\n\
         \x20   init_statics();\n\
         \x20   let p = (in.uv_orig * 2.0 - 1.0) * inp.c[0].xy;\n\
         \x20   var ang = atan2(p.y, p.x);\n\
         \x20   if (ang < 0.0) { ang += 6.283185307179586; }\n\
         \x20   let rad = length(p) / length(inp.c[0].xy);\n\
         \x20   let ret = shader_body(in.uv, in.uv_orig, rad, ang, in.color.rgb);\n\
         \x20   return vec4<f32>(ret, in.color.a);\n\
         }\n",
    );
    w
}

/// Check WGSL as wgpu will, returning naga's module.
///
/// # Errors
///
/// naga's description of what is wrong, with the source line.
pub fn validate(wgsl: &str) -> Result<naga::Module, String> {
    let module = naga::front::wgsl::parse_str(wgsl).map_err(|e| e.emit_to_string(wgsl))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|e| e.emit_to_string(wgsl))?;
    Ok(module)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ok(src: &str, stage: Stage) -> Translated {
        let t = translate(src, stage).unwrap_or_else(|e| panic!("{e}\n{src}"));
        if let Err(e) = validate(&t.wgsl) {
            panic!("{e}\n{}", t.wgsl);
        }
        t
    }

    #[test]
    fn the_built_in_shaders_translate() {
        ok(DEFAULT_WARP, Stage::Warp);
        ok(DEFAULT_COMP, Stage::Comp);
    }

    #[test]
    fn the_uniform_block_size_matches_its_layout() {
        // c + q + d + two vec4s, then the rotations at a 16-byte boundary.
        assert_eq!(INPUTS_SIZE, 432 + 1536);
    }

    #[test]
    fn a_typical_warp_shader_translates() {
        let t = ok(
            "shader_body\n{\n\
               float2 uv2 = uv;\n\
               float ss = 0.1;\n\
               float3 blur = 0.25*(\n\
                 tex2D(sampler_main, uv2+ss*texsize.zw*float2( 1, 1)).xyz +\n\
                 tex2D(sampler_main, uv2+ss*texsize.zw*float2(-1, 1)).xyz);\n\
               ret = blur;\n\
               float3 z = ret;\n\
               z.x += (GetBlur1(uv2) - GetPixel(uv2))*0.1;\n\
               ret = z;\n\
               ret += 0.05*(tex2d(sampler_noise_lq, uv*texsize.xy*texsize_noise_lq.zw + rand_frame.xy).xyz*2-1);\n\
             }\n",
            Stage::Warp,
        );
        assert_eq!(t.blur, 1);
        assert!(t.user_textures.is_empty());
    }

    #[test]
    fn headers_functions_and_textures_translate() {
        let t = ok(
            "sampler sampler_worms;\n\
             sampler2D sampler_fw_clouds;\n\
             static const float3x3 RotMat = float3x3(1,0,0, 0,1,0, 0,0,1);\n\
             static float2 sunpos = float2(q1, q2) * 0.5;\n\
             float3 tint(float3 c, float amount) { return lerp(c, lum(c), amount); }\n\
             void swap(inout float a, inout float b) { float t = a; a = b; b = t; }\n\
             shader_body\n{\n\
               float3 col = tex2D(sampler_worms, uv).xyz + tex2D(sampler_fw_clouds, uv * 2).rgb;\n\
               col = mul(col, RotMat);\n\
               float2x2 rot = float2x2(cos(time), -sin(time), sin(time), cos(time));\n\
               float2 p = mul(uv - 0.5, rot) + sunpos;\n\
               float a = p.x, b = p.y;\n\
               swap(a, b);\n\
               int n = 0;\n\
               [unroll] for (int i = 0; i < 4; i++) { n += i; if (n > 3) break; }\n\
               while (n <= 10) { n++; }\n\
               ret = tint(col, 0.5) * (a > b) + n * 0.01;\n\
               ret.xy *= 0.5;\n\
               ret.z = saturate(ret.z);\n\
               ret = (rad < 0.5) ? ret : 1 - ret;\n\
               ret.gb = mul(float4(ret, 1), rot_s1).xy;\n\
             }\n",
            Stage::Comp,
        );
        assert_eq!(t.user_textures, ["worms", "clouds"]);
    }

    #[test]
    fn preprocessor_conditionals_and_arrays_translate() {
        ok(
            "#define STEPS 3\n\
             shader_body\n{\n\
             #if STEPS > 2\n\
               const float4 samples[2] = { float4(1,2,3,4), float4(5,6,7,8) };\n\
             #else\n\
               garbage here;\n\
             #endif\n\
               float3 m[3];\n\
               for (int k = 0; k < 2; k++) m[k] = samples[k].xyz;\n\
               ret = m[0] + m[1] * q3 + float3(tex3D(sampler_noisevol_hq, float3(uv, time)).x, 0, 0);\n\
             }\n",
            Stage::Warp,
        );
    }

    #[test]
    fn unsupported_shaders_say_where() {
        let e =
            translate("shader_body\n{\n  ret = undefined_thing;\n}\n", Stage::Warp).unwrap_err();
        assert_eq!(e.line, 3);
        assert!(translate("float x;\n", Stage::Warp).is_err());
        assert!(translate("shader_body { ret = noise(uv); }", Stage::Warp).is_err());
    }

    /// Run by hand with `NEOAMP_MILK_FILE` set to a preset: prints why its shaders fail.
    #[test]
    #[ignore = "needs NEOAMP_MILK_FILE"]
    fn one_presets_shaders() {
        use crate::milkdrop::preset::Preset;
        let Some(file) = std::env::var_os("NEOAMP_MILK_FILE") else {
            return;
        };
        let bytes = std::fs::read(file).unwrap();
        let p = Preset::parse(&String::from_utf8_lossy(&bytes));
        for (src, stage) in [(&p.warp, Stage::Warp), (&p.comp, Stage::Comp)] {
            let Some(src) = src else { continue };
            match translate(src, stage) {
                Err(e) => eprintln!("{stage:?}: {e}\n{src}"),
                Ok(t) => match validate(&t.wgsl) {
                    Err(e) => eprintln!("{stage:?}: {e}\n{}", t.wgsl),
                    Ok(_) => eprintln!("{stage:?}: ok"),
                },
            }
        }
    }

    /// Run by hand with `NEOAMP_MILK_DIR` set to a folder of presets: translates and checks
    /// every warp and composite shader, and prints why the ones that fail do.
    #[test]
    #[ignore = "needs NEOAMP_MILK_DIR"]
    fn every_presets_shaders_translate() {
        use crate::milkdrop::preset::Preset;
        use std::collections::BTreeMap;
        let Some(dir) = std::env::var_os("NEOAMP_MILK_DIR") else {
            return;
        };
        let mut files = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("milk"))
                {
                    files.push(p);
                }
            }
        }
        files.sort();
        let (mut total, mut failed) = (0, 0);
        let mut reasons: BTreeMap<String, (usize, String)> = BTreeMap::new();
        for f in &files {
            let Ok(bytes) = std::fs::read(f) else {
                continue;
            };
            let p = Preset::parse(&String::from_utf8_lossy(&bytes));
            for (src, stage) in [(&p.warp, Stage::Warp), (&p.comp, Stage::Comp)] {
                let Some(src) = src else { continue };
                total += 1;
                let result = translate(src, stage)
                    .map_err(|e| e.to_string())
                    .and_then(|t| {
                        validate(&t.wgsl).map(|_| ()).map_err(|e| {
                            e.lines()
                                .find(|l| l.contains("error"))
                                .unwrap_or("invalid")
                                .to_owned()
                        })
                    });
                if let Err(e) = result {
                    failed += 1;
                    let key: String = e
                        .split(['\'', '`'])
                        .next()
                        .unwrap_or(&e)
                        .chars()
                        .filter(|c| !c.is_ascii_digit())
                        .collect();
                    let entry = reasons.entry(key).or_insert((0, String::new()));
                    entry.0 += 1;
                    if entry.1.is_empty() {
                        entry.1 = format!("{}: {e}", f.display());
                    }
                }
            }
        }
        let mut sorted: Vec<_> = reasons.into_iter().collect();
        sorted.sort_by_key(|r| std::cmp::Reverse(r.1.0));
        for (reason, (n, example)) in sorted.iter().take(40) {
            eprintln!("{n:6}  {reason}\n        e.g. {example}");
        }
        eprintln!("{total} shaders, {failed} failed");
    }
}
