//! The presets MilkDrop can show: those built into NeoAmp, then the `.milk` files in the
//! user's presets folder and the folders inside it.

use std::path::{Path, PathBuf};

use super::built_in::{BuiltIn, Category};
use super::preset::Preset;

/// A preset file larger than this is not one: it is skipped rather than read.
const MAX_FILE_BYTES: u64 = 1 << 20;
/// How deep into folders inside the presets folder files are looked for.
const MAX_DEPTH: usize = 8;
/// At most this many files are listed, however many the folder holds.
const MAX_FILES: usize = 100_000;

#[derive(Clone, Debug)]
enum Source {
    BuiltIn(&'static str),
    File(PathBuf),
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    source: Source,
    /// A built-in preset's category; the user's have none.
    category: Option<Category>,
}

/// The presets in the order they are listed: the built-in ones, then the user's, each sorted
/// by name.
#[derive(Clone, Debug, Default)]
pub struct Presets {
    entries: Vec<Entry>,
    built_in: usize,
}

/// Whether `path` names a `.milk` file, whatever the case of its extension.
fn is_milk(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("milk"))
}

/// Add the `.milk` files under `dir` to `out`, `depth` folders down.
fn scan(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // Links are followed only to files, so a link to a folder above cannot loop.
        let is_dir = kind.is_dir();
        if is_dir && depth < MAX_DEPTH {
            scan(&path, depth + 1, out);
        } else if !is_dir && is_milk(&path) && path.is_file() {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(Entry {
                name,
                source: Source::File(path),
                category: None,
            });
        }
    }
}

fn sort(entries: &mut [Entry]) {
    entries.sort_by_cached_key(|e| e.name.to_lowercase());
}

impl Presets {
    /// Just `built_in`.
    #[must_use]
    pub fn new(built_in: &[BuiltIn]) -> Presets {
        let mut entries: Vec<Entry> = built_in
            .iter()
            .map(|p| Entry {
                name: p.name.to_owned(),
                source: Source::BuiltIn(p.text),
                category: Some(p.category),
            })
            .collect();
        sort(&mut entries);
        Presets {
            built_in: entries.len(),
            entries,
        }
    }

    /// `built_in`, then every `.milk` file under `folder`, if there is one.
    #[must_use]
    pub fn with_folder(built_in: &[BuiltIn], folder: &Path) -> Presets {
        let mut presets = Presets::new(built_in);
        let mut found = Vec::new();
        scan(folder, 0, &mut found);
        sort(&mut found);
        presets.entries.extend(found);
        presets
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many come first because they are built in.
    #[must_use]
    pub fn built_in_count(&self) -> usize {
        self.built_in
    }

    /// Preset `i`'s name: a built-in preset's title, or a file's name without `.milk`.
    #[must_use]
    pub fn name(&self, i: usize) -> Option<&str> {
        self.entries.get(i).map(|e| e.name.as_str())
    }

    /// Preset `i`'s category, if it is built in.
    #[must_use]
    pub fn category(&self, i: usize) -> Option<Category> {
        self.entries.get(i).and_then(|e| e.category)
    }

    /// Every name, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// The first preset called `name`.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    /// The file preset `i` is read from; `None` for a built-in one.
    #[must_use]
    pub fn path(&self, i: usize) -> Option<&Path> {
        match &self.entries.get(i)?.source {
            Source::File(path) => Some(path),
            Source::BuiltIn(_) => None,
        }
    }

    /// Read preset `i`. `None` (logged) if its file cannot be read or is too large.
    #[must_use]
    pub fn load(&self, i: usize) -> Option<Preset> {
        let entry = self.entries.get(i)?;
        match &entry.source {
            Source::BuiltIn(text) => Some(Preset::parse(text)),
            Source::File(path) => {
                let read = std::fs::metadata(path).and_then(|m| {
                    if m.len() > MAX_FILE_BYTES {
                        Err(std::io::Error::other("too large to be a preset"))
                    } else {
                        std::fs::read(path)
                    }
                });
                match read {
                    // Presets are often in Windows' Latin-1; their text is ASCII where it matters.
                    Ok(bytes) => Some(Preset::parse(&String::from_utf8_lossy(&bytes))),
                    Err(e) => {
                        log::warn!("milkdrop: cannot read {}: {e}", path.display());
                        None
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const BUILT_IN: &[BuiltIn] = &[
        BuiltIn {
            name: "zebra",
            category: Category::Waveform,
            text: "[preset00]\nzoom=1.1\n",
        },
        BuiltIn {
            name: "Aardvark",
            category: Category::Fractal,
            text: "[preset00]\nzoom=0.9\n",
        },
    ];

    #[test]
    fn built_in_presets_come_first_sorted() {
        let p = Presets::new(BUILT_IN);
        assert_eq!(p.names().collect::<Vec<_>>(), ["Aardvark", "zebra"]);
        assert_eq!(p.built_in_count(), 2);
        assert_eq!(p.find("zebra"), Some(1));
        assert!((p.load(0).unwrap().values.get("zoom") - 0.9).abs() < 1e-9);
        assert!(p.path(0).is_none());
        assert!(p.load(2).is_none());
        assert_eq!(p.category(0), Some(Category::Fractal));
        assert_eq!(p.category(2), None);
    }

    #[test]
    fn the_folder_and_the_folders_in_it_are_read() {
        let dir = std::env::temp_dir().join(format!("neoamp-milk-{}", std::process::id()));
        let sub = dir.join("Sub folder");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("Mine.MILK"), "[preset00]\r\nzoom=1.5\r\n").unwrap();
        std::fs::write(sub.join("deep.milk"), "[preset00]\nzoom=0.5\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a preset").unwrap();
        let p = Presets::with_folder(BUILT_IN, &dir);
        let mine = p.load(3);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            p.names().collect::<Vec<_>>(),
            ["Aardvark", "zebra", "deep", "Mine"]
        );
        assert_eq!(p.built_in_count(), 2);
        assert!(p.path(3).is_some());
        assert_eq!(p.category(3), None, "the user's presets have no category");
        assert!((mine.unwrap().values.get("zoom") - 1.5).abs() < 1e-9);
        // The files are gone now, so they cannot be read.
        assert!(p.load(2).is_none());
    }

    #[test]
    fn a_missing_folder_leaves_the_built_in_presets() {
        let missing = std::env::temp_dir().join("neoamp-milk-no-such-folder");
        let p = Presets::with_folder(BUILT_IN, &missing);
        assert_eq!(p.len(), 2);
    }
}
