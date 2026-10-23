//! The pictures presets name as textures (`sampler_worms`, `sampler_rand03`...), found among
//! the image files in the presets folder and the folders inside it, such as projectM's
//! texture pack unpacked there.

use std::path::{Path, PathBuf};

use crate::playlist::Rng;

/// How deep into folders inside the presets folder pictures are looked for.
const MAX_DEPTH: usize = 8;
/// At most this many pictures are listed.
const MAX_FILES: usize = 20_000;
/// Pictures larger than this along either side are not loaded.
const MAX_SIDE: u32 = 4096;
/// The kinds of picture read, by extension.
const EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "bmp", "tga"];

/// The pictures found, by name.
#[derive(Debug, Default)]
pub struct Textures {
    /// Each picture's name in lower case, without its extension, and where it is; sorted by
    /// name, so that the first of two with the same name is always the one used.
    files: Vec<(String, PathBuf)>,
}

fn scan(dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
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
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                scan(&path, depth + 1, out);
            }
            continue;
        }
        let picture = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)));
        if let (true, Some(stem)) = (picture, path.file_stem()) {
            out.push((stem.to_string_lossy().to_lowercase(), path));
        }
    }
}

/// For `randNN` and `randNN_prefix`: the prefix (empty for none). MilkDrop gives such a
/// sampler a picture chosen at random, from those whose names start with the prefix.
fn random_prefix(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("rand")?;
    let digits = rest.get(..2)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match &rest[2..] {
        "" => Some(""),
        tail => tail.strip_prefix('_'),
    }
}

impl Textures {
    /// The pictures under `folder`, if there is one.
    #[must_use]
    pub fn scan(folder: &Path) -> Textures {
        let mut files = Vec::new();
        scan(folder, 0, &mut files);
        files.sort();
        Textures { files }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Where the picture a preset calls `name` (in lower case) is, choosing with `rng` for the
    /// random ones.
    #[must_use]
    pub fn find(&self, name: &str, rng: &mut Rng) -> Option<&Path> {
        if let Some(prefix) = random_prefix(name) {
            let choices: Vec<&PathBuf> = self
                .files
                .iter()
                .filter(|(n, _)| n.starts_with(prefix))
                .map(|(_, p)| p)
                .collect();
            return choices.get(rng.below(choices.len())).map(|p| p.as_path());
        }
        let i = self.files.partition_point(|(n, _)| n.as_str() < name);
        self.files
            .get(i)
            .filter(|(n, _)| n == name)
            .map(|(_, p)| p.as_path())
    }

    /// The picture a preset calls `name`, as RGBA rows and its size. `None` when there is
    /// none, or it cannot be read (logged).
    #[must_use]
    pub fn load(&self, name: &str, rng: &mut Rng) -> Option<(Vec<u8>, u32, u32)> {
        let path = self.find(name, rng)?;
        let read = || -> Result<image::RgbaImage, image::ImageError> {
            let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_SIDE);
            limits.max_image_height = Some(MAX_SIDE);
            reader.limits(limits);
            Ok(reader.decode()?.to_rgba8())
        };
        match read() {
            Ok(img) => {
                let (w, h) = img.dimensions();
                Some((img.into_raw(), w, h))
            }
            Err(e) => {
                log::warn!("milkdrop: cannot read {}: {e}", path.display());
                None
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn random_names() {
        assert_eq!(random_prefix("rand00"), Some(""));
        assert_eq!(random_prefix("rand13_smalltiled"), Some("smalltiled"));
        assert_eq!(random_prefix("rand1"), None);
        assert_eq!(random_prefix("randomness"), None);
        assert_eq!(random_prefix("worms"), None);
    }

    #[test]
    fn pictures_are_found_by_name_in_any_folder() {
        let dir = std::env::temp_dir().join(format!("neoamp-tex-{}", std::process::id()));
        let sub = dir.join("textures");
        std::fs::create_dir_all(&sub).unwrap();
        let png = |path: &Path| {
            image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]))
                .save(path)
                .unwrap();
        };
        png(&sub.join("Worms.PNG"));
        png(&dir.join("smalltiled_a.png"));
        std::fs::write(dir.join("broken.jpg"), b"not a picture").unwrap();
        std::fs::write(dir.join("readme.txt"), b"hi").unwrap();
        let t = Textures::scan(&dir);
        let mut rng = Rng::with_seed(3);
        let worms = t.load("worms", &mut rng);
        let random = t.find("rand02_smalltiled", &mut rng).map(Path::to_path_buf);
        let broken = t.load("broken", &mut rng);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(t.len(), 3);
        let (data, w, h) = worms.unwrap();
        assert_eq!((w, h, data.len()), (2, 3, 24));
        assert_eq!(random, Some(dir.join("smalltiled_a.png")));
        assert!(broken.is_none());
        assert!(t.find("clouds", &mut rng).is_none());
        assert!(t.find("rand05_nothing", &mut rng).is_none());
    }
}
