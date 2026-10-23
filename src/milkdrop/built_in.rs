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
    preset!(
        "A Tribute to Martin - bombyx mori - Ft Flexi - AdamFX - StahlRegen - Hd In Milk Ailen FX H"
    ),
    preset!(
        "Adam Eatit Mashup FX 2 martin - disco mix + Lodus + Geiss + Ludicrous speed + Eos Ft Flexi Gimmie color 4"
    ),
    preset!("Adam Fx 2 Zylot -  FierceFX Glowworld 7"),
    preset!("AdamFX 2 Geiss - Crystal Run 2"),
    preset!(
        "AdamFX CollaborationFX of the best Remake  - Ft Martin - Orb - Flexi - WitchCraft Infection"
    ),
    preset!("amandio c - dynacube (shifter wave) - inf fin bnd"),
    preset!("amandio c - fume (ATI)"),
    preset!("amandio c - prime forms 2 loavthepsyckuel seananners vs aquateen"),
    preset!("amandio c - prime forms keylontiq pseudoskienke spot on blobs"),
    preset!("amandio c - quiet life 2 shf"),
    preset!("bdrv + al Rovastar - Explosive Minds Bug Mix wait a moment  rmx"),
    preset!(
        "Beat it Adam FX 2  flexi + fishbrain -  operation fatcap [ebb and flow of memetic ov diffusion - fnord] + EoS 11"
    ),
    preset!(
        "Benjam and Zylot - Tie-Dye Supernova (Sunspots Mix) - a million dull personalities nz+12"
    ),
    preset!(
        "CantStop AdamFX 2 Stahlregen & Bmelgren + EoS + Geiss + Krash + Rocke + Rovastar - Glowstick Heart (MashUp 7) + Phat EOS"
    ),
    preset!(
        "Collaboration Milk Celebration AdamFX  ExtreemFX FT flexi - divine struggle - Also Ft AdamFX n Martin (Divide Between Divine StruggleFX)A"
    ),
    preset!("cope + flexi - colorful marble (ghost mix) - rand wave 4"),
    preset!("EoS + Phat - chasers 11 sentinel C_poltergeist_Blue_mix_22 malalol tweak"),
    preset!(
        "EoS - glowsticks v2 03 music shifter edit b (Stahl's Reactive RMX) [+ flexi's reflections] [roaming b]"
    ),
    preset!("EoS - glowsticks v2 05 and proton lights (+Krash's beat code) _Phat_remix07"),
    preset!("EoS - sarc e"),
    preset!("EoS - sarc g_Phats_Zoom_Time_mix"),
    preset!("EoS - skylight a3 [trip colors flux2]_phat_Multi_shaped2_zoe_colours3"),
    preset!("EVET - Zion Matrix"),
    preset!(
        "flexi + geiss + redi jedi + Stahlregen - thumbdrum tokamak [crossfiring aftermath mashup] (Jelly)"
    ),
    preset!(
        "flexi + geiss - bipolar vs- reaction diffusion [mirror scoped] (Jelly 5-5) threx nz+ fency"
    ),
    preset!("flexi + shifter + stahlregen - totally spiraling out your eyes [spinning]"),
    preset!("flexi - a julia fractal for hexcollie embossed (Jelly)"),
    preset!("Flexi - jellyfish jam"),
    preset!("flexi - lorenz chaser (Stahl's rainbow multiply mix 2 - Jelly V2)"),
    preset!("Flexi - spatialist"),
    preset!("Fumbling_Foo & Flexi, Martin, Orb - Star Forge v13c"),
    preset!("Geiss - Cauldron - painterly 3 (saturation remix)"),
    preset!("Geiss - Confetti (Jelly)"),
    preset!("Geiss - Kaleidoscope 1 nickel advice"),
    preset!("Geiss - Liquid Beats (Reverse Jelly V3)"),
    preset!("Geiss - Mash-Up 1"),
    preset!("Geiss - Reaction Diffusion 2"),
    preset!("goody + flexi - emotive dissonance - integral anomaly - dissonant rift2 roam"),
    preset!("Goody - Need - Transcendance remix"),
    preset!(
        "HDFX New Creation Sensation - AdamFx,Flexi,AmandioC Martin - Hexocollie Shadow Harlequin - No Boundr4"
    ),
    preset!("Hexcollie - The Journey of a Mosquito (personal mash) nz+"),
    preset!("Hexcollie, suksma n TEcHNO - Shoehorn shin"),
    preset!("hexcollie, ti@n, suksma, zylot n orb - blarffff fire --- Isosceles edit"),
    preset!(
        "Infused by Flexi with AdamFX 2 Stahlregen & Aderrasi + EoS + Geiss + Unchained + Zylot - Slowflowers F7"
    ),
    preset!("Jc - Flow with the Go"),
    preset!("Jc - Orbital System"),
    preset!(
        "Liquid Glowsticks - ATwisted Pre Set Mix By AdamFX 2 martin - disco mix Ft Hexocollie including Flexi Remixed 4"
    ),
    preset!("LuxXx - Add-em Crossfire b"),
    preset!("LuxXx - FlameFace I (subtle) ps4"),
    preset!("LuxXx - MoltenWheel i --- Isosceles edit8"),
    preset!("martin + stahlregen - martin in da mash 14"),
    preset!("martin - glass corridor nz2 incline"),
    preset!("martin - neon space ps2 (ati fix) - yaqui graph - flx food roams"),
    preset!("martin - purple pulsator - mom - ns2w"),
    preset!(
        "NeW Adam Master Mashup FX 2 Geiss and Zylot - Reaction Diffusion 3 (Overload Mix 2) AMAZING MASUP Various Workwithlines 2"
    ),
    preset!(
        "New Creation Sensation -  AdamFx, Amandio c n Martin - Star to Another World ft Hexocollie"
    ),
    preset!("ORB - Cloud Scope"),
    preset!("ORB - Cosmic Dream"),
    preset!("ORB - Sliding down light"),
    preset!("ORB - Smoke and Fire (Jelly)"),
    preset!("ORB - Space Time2"),
    preset!("ORB - Stahl - Tantalum Gran random tex - j4+"),
    preset!(
        "PHAT HD FX 2 FiShbRaiN  -  Flexi  ft AdamFX Aderrasi , Martin n Rova - Witchcraft Take OverFX  A"
    ),
    preset!("phat_RippleNebula99h-EoS mod_mod6 - j4+"),
    preset!("Phat_Rovastar - 1337 boxes"),
    preset!("Rovastar & Geiss - Dynamic Swirls 3 (Broken Destiny Mix)"),
    preset!("Rovastar - Explosive Minds [no fnord, good buy]"),
    preset!("Rovastar - Sunflower Passion planet"),
    preset!("Serge + martin - crystal palace star009"),
    preset!("ShadowHarlequin - mashup - Satin Sunburst (Neon Tokyo Mix) v2"),
    preset!("shadowharlequin, goody - woven beads (ps2-0) [outlined]"),
    preset!("shifter + Flexi - liquid circuitry from the lesser evil hive"),
    preset!("shifter - plasmic fact - witholding, with-a holding, widdaholting - ap3 nz"),
    preset!("shifter - robotopia v2 molding chaos - little miss scale oven vs scala nz+"),
    preset!("Shreyas - Carnival loavthephysyq"),
    preset!(
        "Stahlregen & bdrv + fiSHbRAiN + flexi + Geiss + shifter - Starcraft ( Hyperion RMX V2) [for the elderly]"
    ),
    preset!("Stahlregen & fiShbRaiN + flexi + Geiss + shifter - Stonecraft (dark tides mix)_2"),
    preset!("Stahlregen & Geiss + ORB - Etheral Waves (3 layers of Rayleigh RMX) nz+ wishes to"),
    preset!("Stahlregen - Dots (Pixels - Blocky) (Reverse Jelly V3)"),
    preset!("stahlregen - spiral beats"),
    preset!("stahlregen - splash! mexupa"),
    preset!("suksma - 20100413 0136-42 belgrade countryside"),
    preset!("suksma - Hexcollie - Julian Carnival - shimmy dumb grid"),
    preset!("suksma - loosely involved in grey market dog openers"),
    preset!("suksma - negative infinity for not flinching2(1)"),
    preset!(
        "suksma - yin - 395 - Artificial Inspiration (brainstorming - outward) + martin + flexi + stahlregen - el"
    ),
    preset!("The NG + fiShbRaiN + Flexi + martin - Last Dying Moments (Under a Bright Sun)"),
    preset!("TonyMilkdrop - Nuclear [Flexi - help out + multiverse]"),
    preset!("TonyMilkdrop - Tricolors [Flexi - pump up the jam + multiverse] --- Isosceles edit"),
    preset!("Tripgnosis - Astralmetric"),
    preset!("Tripgnosis - Digital Sun"),
    preset!("Tripgnosis - Kaleidestrobe"),
    preset!("Tripgnosis - Photontrap"),
    preset!("Tripgnosis - Warp 11"),
    preset!("various - whirygig42"),
    preset!("yin - 314 - Ocean of Light (ascending nadi mix) spher nz+"),
    preset!("yin - 354 - Chromatron (oldschool remix)"),
    preset!("Zylot - True Visionary (Final Mix) false auralary nz+ munchkin"),
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
        for (name, _) in PRESETS {
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
