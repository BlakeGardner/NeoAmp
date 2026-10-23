//! The presets built into NeoAmp, from projectM's "cream of the crop" collection. Their files
//! are in `milkdrop/presets`, and `milkdrop/README.md` says who made them.

/// The collection's categories, the folders its presets were sorted into. The presets menu
/// groups the built-in presets by them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Dancer,
    Drawing,
    Fractal,
    Geometric,
    Hypnotic,
    Particles,
    Reaction,
    Sparkle,
    Supernova,
    Waveform,
}

impl Category {
    /// Every category, in the order the menu lists them.
    pub const ALL: [Category; 10] = [
        Category::Dancer,
        Category::Drawing,
        Category::Fractal,
        Category::Geometric,
        Category::Hypnotic,
        Category::Particles,
        Category::Reaction,
        Category::Sparkle,
        Category::Supernova,
        Category::Waveform,
    ];

    /// The category's name, as the menu shows it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Category::Dancer => "Dancer",
            Category::Drawing => "Drawing",
            Category::Fractal => "Fractal",
            Category::Geometric => "Geometric",
            Category::Hypnotic => "Hypnotic",
            Category::Particles => "Particles",
            Category::Reaction => "Reaction",
            Category::Sparkle => "Sparkle",
            Category::Supernova => "Supernova",
            Category::Waveform => "Waveform",
        }
    }
}

/// A preset built into NeoAmp.
#[derive(Clone, Copy, Debug)]
pub struct BuiltIn {
    /// The file's name, without `.milk`.
    pub name: &'static str,
    /// Where the collection sorted it.
    pub category: Category,
    /// The file's text.
    pub text: &'static str,
}

/// A preset file of `milkdrop/presets`, by its category and name.
macro_rules! preset {
    ($category:ident, $name:literal) => {
        BuiltIn {
            name: $name,
            category: Category::$category,
            text: include_str!(concat!("../../milkdrop/presets/", $name, ".milk")),
        }
    };
}

/// Every built-in preset.
pub const PRESETS: &[BuiltIn] = &[
    preset!(
        Fractal,
        "A Tribute to Martin - bombyx mori - Ft Flexi - AdamFX - StahlRegen - Hd In Milk Ailen FX H"
    ),
    preset!(
        Reaction,
        "Adam Eatit Mashup FX 2 martin - disco mix + Lodus + Geiss + Ludicrous speed + Eos Ft Flexi Gimmie color 4"
    ),
    preset!(Dancer, "Adam Fx 2 Zylot -  FierceFX Glowworld 7"),
    preset!(Reaction, "AdamFX 2 Geiss - Crystal Run 2"),
    preset!(
        Drawing,
        "AdamFX CollaborationFX of the best Remake  - Ft Martin - Orb - Flexi - WitchCraft Infection"
    ),
    preset!(
        Geometric,
        "amandio c - dynacube (shifter wave) - inf fin bnd"
    ),
    preset!(Geometric, "amandio c - fume (ATI)"),
    preset!(
        Drawing,
        "amandio c - prime forms 2 loavthepsyckuel seananners vs aquateen"
    ),
    preset!(
        Geometric,
        "amandio c - prime forms keylontiq pseudoskienke spot on blobs"
    ),
    preset!(Reaction, "amandio c - quiet life 2 shf"),
    preset!(
        Reaction,
        "bdrv + al Rovastar - Explosive Minds Bug Mix wait a moment  rmx"
    ),
    preset!(
        Fractal,
        "Beat it Adam FX 2  flexi + fishbrain -  operation fatcap [ebb and flow of memetic ov diffusion - fnord] + EoS 11"
    ),
    preset!(
        Supernova,
        "Benjam and Zylot - Tie-Dye Supernova (Sunspots Mix) - a million dull personalities nz+12"
    ),
    preset!(
        Fractal,
        "CantStop AdamFX 2 Stahlregen & Bmelgren + EoS + Geiss + Krash + Rocke + Rovastar - Glowstick Heart (MashUp 7) + Phat EOS"
    ),
    preset!(
        Fractal,
        "Collaboration Milk Celebration AdamFX  ExtreemFX FT flexi - divine struggle - Also Ft AdamFX n Martin (Divide Between Divine StruggleFX)A"
    ),
    preset!(
        Reaction,
        "cope + flexi - colorful marble (ghost mix) - rand wave 4"
    ),
    preset!(
        Dancer,
        "EoS + Phat - chasers 11 sentinel C_poltergeist_Blue_mix_22 malalol tweak"
    ),
    preset!(
        Dancer,
        "EoS - glowsticks v2 03 music shifter edit b (Stahl's Reactive RMX) [+ flexi's reflections] [roaming b]"
    ),
    preset!(
        Dancer,
        "EoS - glowsticks v2 05 and proton lights (+Krash's beat code) _Phat_remix07"
    ),
    preset!(Geometric, "EoS - sarc e"),
    preset!(Geometric, "EoS - sarc g_Phats_Zoom_Time_mix"),
    preset!(
        Fractal,
        "EoS - skylight a3 [trip colors flux2]_phat_Multi_shaped2_zoe_colours3"
    ),
    preset!(Dancer, "EVET - Zion Matrix"),
    preset!(
        Reaction,
        "flexi + geiss + redi jedi + Stahlregen - thumbdrum tokamak [crossfiring aftermath mashup] (Jelly)"
    ),
    preset!(
        Drawing,
        "flexi + geiss - bipolar vs- reaction diffusion [mirror scoped] (Jelly 5-5) threx nz+ fency"
    ),
    preset!(
        Reaction,
        "flexi + shifter + stahlregen - totally spiraling out your eyes [spinning]"
    ),
    preset!(
        Fractal,
        "flexi - a julia fractal for hexcollie embossed (Jelly)"
    ),
    preset!(Waveform, "Flexi - jellyfish jam"),
    preset!(
        Reaction,
        "flexi - lorenz chaser (Stahl's rainbow multiply mix 2 - Jelly V2)"
    ),
    preset!(Hypnotic, "Flexi - spatialist"),
    preset!(
        Waveform,
        "Fumbling_Foo & Flexi, Martin, Orb - Star Forge v13c"
    ),
    preset!(
        Waveform,
        "Geiss - Cauldron - painterly 3 (saturation remix)"
    ),
    preset!(Waveform, "Geiss - Confetti (Jelly)"),
    preset!(Particles, "Geiss - Kaleidoscope 1 nickel advice"),
    preset!(Waveform, "Geiss - Liquid Beats (Reverse Jelly V3)"),
    preset!(Reaction, "Geiss - Mash-Up 1"),
    preset!(Reaction, "Geiss - Reaction Diffusion 2"),
    preset!(
        Hypnotic,
        "goody + flexi - emotive dissonance - integral anomaly - dissonant rift2 roam"
    ),
    preset!(Waveform, "Goody - Need - Transcendance remix"),
    preset!(
        Dancer,
        "HDFX New Creation Sensation - AdamFx,Flexi,AmandioC Martin - Hexocollie Shadow Harlequin - No Boundr4"
    ),
    preset!(
        Fractal,
        "Hexcollie - The Journey of a Mosquito (personal mash) nz+"
    ),
    preset!(Fractal, "Hexcollie, suksma n TEcHNO - Shoehorn shin"),
    preset!(
        Sparkle,
        "hexcollie, ti@n, suksma, zylot n orb - blarffff fire --- Isosceles edit"
    ),
    preset!(
        Sparkle,
        "Infused by Flexi with AdamFX 2 Stahlregen & Aderrasi + EoS + Geiss + Unchained + Zylot - Slowflowers F7"
    ),
    preset!(Waveform, "Jc - Flow with the Go"),
    preset!(Geometric, "Jc - Orbital System"),
    preset!(
        Dancer,
        "Liquid Glowsticks - ATwisted Pre Set Mix By AdamFX 2 martin - disco mix Ft Hexocollie including Flexi Remixed 4"
    ),
    preset!(Sparkle, "LuxXx - Add-em Crossfire b"),
    preset!(Fractal, "LuxXx - FlameFace I (subtle) ps4"),
    preset!(Reaction, "LuxXx - MoltenWheel i --- Isosceles edit8"),
    preset!(Fractal, "martin + stahlregen - martin in da mash 14"),
    preset!(Geometric, "martin - glass corridor nz2 incline"),
    preset!(
        Drawing,
        "martin - neon space ps2 (ati fix) - yaqui graph - flx food roams"
    ),
    preset!(Dancer, "martin - purple pulsator - mom - ns2w"),
    preset!(
        Reaction,
        "NeW Adam Master Mashup FX 2 Geiss and Zylot - Reaction Diffusion 3 (Overload Mix 2) AMAZING MASUP Various Workwithlines 2"
    ),
    preset!(
        Sparkle,
        "New Creation Sensation -  AdamFx, Amandio c n Martin - Star to Another World ft Hexocollie"
    ),
    preset!(Dancer, "ORB - Cloud Scope"),
    preset!(Fractal, "ORB - Cosmic Dream"),
    preset!(Dancer, "ORB - Sliding down light"),
    preset!(Dancer, "ORB - Smoke and Fire (Jelly)"),
    preset!(Geometric, "ORB - Space Time2"),
    preset!(Waveform, "ORB - Stahl - Tantalum Gran random tex - j4+"),
    preset!(
        Drawing,
        "PHAT HD FX 2 FiShbRaiN  -  Flexi  ft AdamFX Aderrasi , Martin n Rova - Witchcraft Take OverFX  A"
    ),
    preset!(Fractal, "phat_RippleNebula99h-EoS mod_mod6 - j4+"),
    preset!(Fractal, "Phat_Rovastar - 1337 boxes"),
    preset!(
        Waveform,
        "Rovastar & Geiss - Dynamic Swirls 3 (Broken Destiny Mix)"
    ),
    preset!(Fractal, "Rovastar - Explosive Minds [no fnord, good buy]"),
    preset!(Waveform, "Rovastar - Sunflower Passion planet"),
    preset!(Waveform, "Serge + martin - crystal palace star009"),
    preset!(
        Waveform,
        "ShadowHarlequin - mashup - Satin Sunburst (Neon Tokyo Mix) v2"
    ),
    preset!(
        Geometric,
        "shadowharlequin, goody - woven beads (ps2-0) [outlined]"
    ),
    preset!(
        Reaction,
        "shifter + Flexi - liquid circuitry from the lesser evil hive"
    ),
    preset!(
        Particles,
        "shifter - plasmic fact - witholding, with-a holding, widdaholting - ap3 nz"
    ),
    preset!(
        Dancer,
        "shifter - robotopia v2 molding chaos - little miss scale oven vs scala nz+"
    ),
    preset!(Geometric, "Shreyas - Carnival loavthephysyq"),
    preset!(
        Sparkle,
        "Stahlregen & bdrv + fiSHbRAiN + flexi + Geiss + shifter - Starcraft ( Hyperion RMX V2) [for the elderly]"
    ),
    preset!(
        Drawing,
        "Stahlregen & fiShbRaiN + flexi + Geiss + shifter - Stonecraft (dark tides mix)_2"
    ),
    preset!(
        Supernova,
        "Stahlregen & Geiss + ORB - Etheral Waves (3 layers of Rayleigh RMX) nz+ wishes to"
    ),
    preset!(
        Sparkle,
        "Stahlregen - Dots (Pixels - Blocky) (Reverse Jelly V3)"
    ),
    preset!(Fractal, "stahlregen - spiral beats"),
    preset!(Supernova, "stahlregen - splash! mexupa"),
    preset!(Waveform, "suksma - 20100413 0136-42 belgrade countryside"),
    preset!(
        Reaction,
        "suksma - Hexcollie - Julian Carnival - shimmy dumb grid"
    ),
    preset!(
        Waveform,
        "suksma - loosely involved in grey market dog openers"
    ),
    preset!(Waveform, "suksma - negative infinity for not flinching2(1)"),
    preset!(
        Dancer,
        "suksma - yin - 395 - Artificial Inspiration (brainstorming - outward) + martin + flexi + stahlregen - el"
    ),
    preset!(
        Sparkle,
        "The NG + fiShbRaiN + Flexi + martin - Last Dying Moments (Under a Bright Sun)"
    ),
    preset!(
        Hypnotic,
        "TonyMilkdrop - Nuclear [Flexi - help out + multiverse]"
    ),
    preset!(
        Hypnotic,
        "TonyMilkdrop - Tricolors [Flexi - pump up the jam + multiverse] --- Isosceles edit"
    ),
    preset!(Drawing, "Tripgnosis - Astralmetric"),
    preset!(Waveform, "Tripgnosis - Digital Sun"),
    preset!(Supernova, "Tripgnosis - Kaleidestrobe"),
    preset!(Dancer, "Tripgnosis - Photontrap"),
    preset!(Particles, "Tripgnosis - Warp 11"),
    preset!(Reaction, "various - whirygig42"),
    preset!(
        Geometric,
        "yin - 314 - Ocean of Light (ascending nadi mix) spher nz+"
    ),
    preset!(Fractal, "yin - 354 - Chromatron (oldschool remix)"),
    preset!(
        Geometric,
        "Zylot - True Visionary (Final Mix) false auralary nz+ munchkin"
    ),
];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::milkdrop::eel::{Program, Vars};
    use crate::milkdrop::hlsl::{self, Stage};
    use crate::milkdrop::preset::Preset;

    /// The preset files can be checked out on every platform: no two names differ only in
    /// case (Windows' and macOS's file systems would take them for one), and none uses a
    /// character Windows forbids or ends in a dot or a space.
    #[test]
    fn the_preset_files_suit_every_file_system() {
        let mut seen = std::collections::HashSet::new();
        for BuiltIn { name, .. } in PRESETS {
            assert!(seen.insert(name.to_lowercase()), "{name} twice");
            assert!(
                !name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']),
                "{name}"
            );
            assert!(
                !name.ends_with(['.', ' ']) && !name.starts_with(' '),
                "{name}"
            );
        }
    }

    /// Every category has presets, so the menu shows no empty ones.
    #[test]
    fn every_category_has_presets() {
        for c in Category::ALL {
            let n = PRESETS.iter().filter(|p| p.category == c).count();
            assert!(n > 0, "{}", c.name());
        }
    }

    /// The equations of every built-in preset compile, and its shaders translate to WGSL that
    /// naga accepts, using no textures but MilkDrop's own.
    #[test]
    fn every_built_in_preset_works() {
        for BuiltIn { name, text, .. } in PRESETS {
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
