//! The presets built into NeoAmp, from projectM's "cream of the crop" collection. Their files
//! are in `milkdrop/presets`, and `milkdrop/README.md` says who made them.

/// A preset file of `milkdrop/presets`, by its name.
macro_rules! preset {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!("../../milkdrop/presets/", $name, ".milk")),
        )
    };
}

/// Each preset's name and text.
pub const PRESETS: &[(&str, &str)] = &[
    preset!("EoS - glowsticks v2 05 and proton lights (+Krash's beat code) _Phat_remix07"),
    preset!("Flexi - jellyfish jam"),
];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::milkdrop::eel::{Program, Vars};
    use crate::milkdrop::hlsl::{self, Stage};
    use crate::milkdrop::preset::Preset;

    /// The equations of every built-in preset compile, and its shaders translate to WGSL that
    /// naga accepts, using no textures but MilkDrop's own.
    #[test]
    fn every_built_in_preset_works() {
        for (name, text) in PRESETS {
            let p = Preset::parse(text);
            let mut code: Vec<&str> = vec![&p.init, &p.per_frame, &p.per_pixel];
            for w in &p.waves {
                code.extend([w.init.as_str(), &w.per_frame, &w.per_point]);
            }
            for s in &p.shapes {
                code.extend([s.init.as_str(), &s.per_frame]);
            }
            for src in code {
                let compiled = Program::compile(src, &mut Vars::new(), p.dialect());
                assert!(compiled.is_ok(), "{name}: {:?}", compiled.err());
            }
            for (src, stage) in [(&p.warp, Stage::Warp), (&p.comp, Stage::Comp)] {
                let Some(src) = src else { continue };
                let t = hlsl::translate(src, stage).unwrap();
                assert!(t.user_textures.is_empty(), "{name} needs textures");
                let checked = hlsl::validate(&t.wgsl);
                assert!(checked.is_ok(), "{name}: {:?}", checked.err());
            }
        }
    }
}
