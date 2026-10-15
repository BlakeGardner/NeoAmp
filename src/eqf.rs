//! Equalizer presets: Winamp's built-in ones, and its `.eqf` file format.
//!
//! An `.eqf` file is the header `Winamp EQ library file v1.1`, the bytes `1A 21 2D 2D`
//! (Ctrl+Z, `!--`), then any number of presets: a 257-byte name padded with zeros, and 11
//! bytes for the 60 Hz to 16 kHz bands and the preamp. A byte holds `64 - v`, where `v` runs
//! from 1 (the bottom of the slider) to 64 (the top). Winamp's own preset files `winamp.q1`
//! (presets) and `winamp.q2` (auto-load presets) share the format.
//!
//! Winamp wrote names in the Windows code page. NeoAmp writes UTF-8, which is the same for
//! plain ASCII names, and reads names as UTF-8 where they are valid and as Latin-1 otherwise.

use anyhow::{Result, bail};

use crate::audio::{BANDS, EQ_RANGE_DB};

const HEADER: &[u8] = b"Winamp EQ library file v1.1\x1a!--";
const NAME_LEN: usize = 257;
const ENTRY_LEN: usize = NAME_LEN + BANDS + 1;
/// The file value at the middle of the slider: 0 dB. Winamp's flat presets use it.
const FLAT: i32 = 33;
/// File values from the middle to the top of the slider (33 to 64), covering `EQ_RANGE_DB`.
const STEPS: f32 = 31.0;

/// One equalizer setting, in decibels.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    pub preamp: f32,
    /// From 60 Hz to 16 kHz.
    pub bands: [f32; BANDS],
}

/// Decibels for a file value. Values outside 1..=64 are brought onto the slider.
fn value_db(v: i32) -> f32 {
    // Within a byte, so the conversion is exact.
    ((v - FLAT) as f32 * EQ_RANGE_DB / STEPS).clamp(-EQ_RANGE_DB, EQ_RANGE_DB)
}

/// The file value nearest `db`.
fn db_value(db: f32) -> i32 {
    let db = if db.is_finite() { db } else { 0.0 };
    // Clamped to 1..=64 first, so the cast cannot truncate.
    ((db * STEPS / EQ_RANGE_DB).round() as i32 + FLAT).clamp(1, 64)
}

impl Preset {
    /// A preset from file values (bands, then the preamp).
    fn from_values(name: &str, values: [u8; BANDS + 1]) -> Preset {
        Preset {
            name: name.to_owned(),
            preamp: value_db(i32::from(values[BANDS])),
            bands: std::array::from_fn(|i| value_db(i32::from(values[i]))),
        }
    }

    /// Every band and the preamp at 0 dB.
    #[must_use]
    pub fn flat(name: &str) -> Preset {
        Preset {
            name: name.to_owned(),
            preamp: 0.0,
            bands: [0.0; BANDS],
        }
    }
}

/// Winamp's presets (its `winamp.q1`), in file values, sorted by name.
const BUILT_IN: [(&str, [u8; BANDS + 1]); 17] = [
    ("Classical", [33, 33, 33, 33, 33, 33, 20, 20, 20, 16, 33]),
    ("Club", [33, 33, 38, 42, 42, 42, 38, 33, 33, 33, 33]),
    ("Dance", [48, 44, 36, 32, 32, 22, 20, 20, 32, 32, 33]),
    ("Full Bass", [48, 48, 48, 42, 35, 25, 18, 15, 14, 14, 33]),
    (
        "Full Bass & Treble",
        [44, 42, 33, 20, 24, 35, 46, 50, 52, 52, 33],
    ),
    ("Full Treble", [16, 16, 16, 25, 37, 50, 58, 58, 58, 60, 33]),
    (
        "Laptop speakers/headphones",
        [40, 50, 41, 26, 28, 35, 40, 48, 53, 56, 33],
    ),
    ("Large hall", [49, 49, 42, 42, 33, 24, 24, 24, 33, 33, 33]),
    ("Live", [24, 33, 39, 41, 42, 42, 39, 37, 37, 36, 33]),
    ("Party", [44, 44, 33, 33, 33, 33, 33, 33, 44, 44, 33]),
    ("Pop", [29, 40, 44, 45, 41, 30, 28, 28, 29, 29, 33]),
    ("Reggae", [33, 33, 31, 22, 33, 43, 43, 33, 33, 33, 33]),
    ("Rock", [45, 40, 23, 19, 26, 39, 47, 50, 50, 50, 33]),
    ("Ska", [28, 24, 25, 31, 39, 42, 47, 48, 50, 48, 33]),
    ("Soft", [40, 35, 30, 28, 30, 39, 46, 48, 50, 52, 33]),
    ("Soft Rock", [39, 39, 36, 31, 25, 23, 26, 31, 37, 47, 33]),
    ("Techno", [45, 42, 33, 23, 24, 33, 45, 48, 48, 47, 33]),
];

/// The names of Winamp's built-in presets, in the order [`built_in`] numbers them.
pub fn built_in_names() -> impl Iterator<Item = &'static str> {
    BUILT_IN.iter().map(|(name, _)| *name)
}

/// Built-in preset `i`.
#[must_use]
pub fn built_in(i: usize) -> Option<Preset> {
    BUILT_IN
        .get(i)
        .map(|(name, values)| Preset::from_values(name, *values))
}

/// The presets in an `.eqf` file, in order. A partial preset at the end is ignored.
///
/// # Errors
/// Fails if `bytes` does not start with the `.eqf` header.
pub fn parse(bytes: &[u8]) -> Result<Vec<Preset>> {
    let Some(body) = bytes.strip_prefix(HEADER) else {
        bail!("not a Winamp EQ preset file");
    };
    Ok(body
        .chunks_exact(ENTRY_LEN)
        .map(|entry| {
            let (name, values) = entry.split_at(NAME_LEN);
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(NAME_LEN)];
            let name = match std::str::from_utf8(name) {
                Ok(s) => s.to_owned(),
                Err(_) => name.iter().map(|&b| char::from(b)).collect(),
            };
            let value = |b: u8| 64 - i32::from(b);
            Preset {
                name,
                preamp: value_db(value(values[BANDS])),
                bands: std::array::from_fn(|i| value_db(value(values[i]))),
            }
        })
        .collect())
}

/// `name` cut to what fits in a preset's name field (with its terminating zero), at a
/// character boundary.
fn fitted(name: &str) -> &str {
    let mut end = name.len().min(NAME_LEN - 1);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// `presets` as an `.eqf` file.
#[must_use]
pub fn write(presets: &[Preset]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER.len() + presets.len() * ENTRY_LEN);
    out.extend_from_slice(HEADER);
    for p in presets {
        let name = fitted(&p.name).as_bytes();
        out.extend_from_slice(name);
        out.resize(out.len() + NAME_LEN - name.len(), 0);
        for db in p.bands.iter().chain([&p.preamp]) {
            // 64 - (1..=64) is 0..=63, so the conversion cannot fail.
            out.push(u8::try_from(64 - db_value(*db)).unwrap_or(0));
        }
    }
    out
}

/// Auto-load presets, one per file name, as Winamp kept them in `winamp.q2`: with AUTO on,
/// a file's preset is loaded as it starts. Names compare without regard to case, since
/// Windows and macOS file names do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoLoad {
    presets: Vec<Preset>,
}

/// Whether two file names are the same once cut to fit a preset name, ignoring case.
fn same_name(a: &str, b: &str) -> bool {
    fn lower(s: &str) -> impl Iterator<Item = char> + '_ {
        fitted(s).chars().flat_map(char::to_lowercase)
    }
    lower(a).eq(lower(b))
}

impl AutoLoad {
    #[must_use]
    pub fn new(presets: Vec<Preset>) -> AutoLoad {
        AutoLoad { presets }
    }

    #[must_use]
    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    /// The preset for files named `file_name`.
    #[must_use]
    pub fn find(&self, file_name: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| same_name(&p.name, file_name))
    }

    /// Make `preset` the one for files named `file_name`, replacing any there was.
    pub fn set(&mut self, file_name: &str, preset: Preset) {
        let preset = Preset {
            name: fitted(file_name).to_owned(),
            ..preset
        };
        match self
            .presets
            .iter_mut()
            .find(|p| same_name(&p.name, file_name))
        {
            Some(old) => *old = preset,
            None => self.presets.push(preset),
        }
    }

    /// Forget the preset for files named `file_name`. True if there was one.
    pub fn remove(&mut self, file_name: &str) -> bool {
        let before = self.presets.len();
        self.presets.retain(|p| !same_name(&p.name, file_name));
        self.presets.len() != before
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn file_values_map_onto_the_slider_with_33_flat() {
        assert_eq!(value_db(33), 0.0);
        assert_eq!(value_db(64), EQ_RANGE_DB);
        assert_eq!(value_db(2), -EQ_RANGE_DB);
        assert_eq!(value_db(1), -EQ_RANGE_DB, "kept on the slider");
        for v in 2..=64 {
            assert_eq!(db_value(value_db(v)), v, "round trip of {v}");
        }
        assert_eq!(db_value(f32::NAN), FLAT);
    }

    #[test]
    fn built_in_presets_are_winamps() {
        let names: Vec<&str> = built_in_names().collect();
        assert_eq!(names.len(), 17);
        assert!(names.windows(2).all(|w| w[0] < w[1]), "sorted");
        let classical = built_in(0).unwrap();
        assert_eq!(classical.name, "Classical");
        assert_eq!(classical.preamp, 0.0);
        assert_eq!(classical.bands[0], 0.0);
        assert!(classical.bands[9] < -6.0, "treble cut");
        assert_eq!(built_in(17), None);
    }

    #[test]
    fn files_round_trip_and_match_winamps_layout() {
        let rock = built_in(12).unwrap();
        let bytes = write(&[rock.clone(), Preset::flat("Flat")]);
        assert_eq!(bytes.len(), 31 + 2 * 268);
        assert!(bytes.starts_with(b"Winamp EQ library file v1.1\x1a!--"));
        // Rock's 60 Hz band is 45, stored as 64 - 45.
        assert_eq!(bytes[31 + 257], 19);
        assert_eq!(parse(&bytes).unwrap(), vec![rock, Preset::flat("Flat")]);
    }

    #[test]
    fn names_are_read_as_utf8_or_latin1() {
        let mut bytes = write(&[Preset::flat("Ünïcode")]);
        assert_eq!(parse(&bytes).unwrap()[0].name, "Ünïcode");
        // Winamp's Windows code page: 0xE9 alone is not UTF-8.
        bytes[31] = 0xE9;
        bytes[32] = 0;
        assert_eq!(parse(&bytes).unwrap()[0].name, "é");
    }

    #[test]
    fn bad_files_are_refused_and_partial_presets_dropped() {
        assert!(parse(b"hello").is_err());
        let mut bytes = write(&[Preset::flat("A"), Preset::flat("B")]);
        bytes.truncate(bytes.len() - 5);
        assert_eq!(parse(&bytes).unwrap().len(), 1);
    }

    #[test]
    fn long_names_are_cut_at_a_character_boundary() {
        let long = "é".repeat(200);
        let bytes = write(&[Preset::flat(&long)]);
        let name = &parse(&bytes).unwrap()[0].name;
        assert_eq!(name.chars().count(), 128);
        assert!(long.starts_with(name.as_str()));
    }

    #[test]
    fn auto_load_presets_are_kept_per_file_name_ignoring_case() {
        let mut auto = AutoLoad::default();
        auto.set("Song.MP3", built_in(12).unwrap());
        assert_eq!(auto.find("song.mp3").unwrap().name, "Song.MP3");
        assert_eq!(
            auto.find("song.mp3").unwrap().bands,
            built_in(12).unwrap().bands
        );
        auto.set("SONG.mp3", Preset::flat("ignored"));
        assert_eq!(auto.presets().len(), 1, "replaced");
        assert_eq!(auto.find("Song.mp3").unwrap().bands, [0.0; BANDS]);
        assert!(auto.find("other.mp3").is_none());
        assert!(auto.remove("song.MP3"));
        assert!(!auto.remove("song.mp3"));
        let long = format!("{}.mp3", "x".repeat(300));
        auto.set(&long, Preset::flat(""));
        assert!(auto.find(&long).is_some(), "found by its cut name");
    }
}
