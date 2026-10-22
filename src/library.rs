//! The skins and equalizer presets NeoAmp keeps in folders beside its settings (see
//! [`crate::settings::Store`]), listed for the Skins and EQ presets menus.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// One file in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The file name without its extension.
    pub name: String,
    pub path: PathBuf,
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The files in `dir` whose extension is `ext` in any case, sorted by name without regard to
/// case. A folder that does not exist (yet) lists nothing.
#[must_use]
pub fn list(dir: &Path, ext: &str) -> Vec<Entry> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .map(|e| e.path())
        .filter(|p| has_extension(p, ext))
        .map(|path| Entry {
            name: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path,
        })
        .collect();
    entries.sort_by_cached_key(|e| (e.name.to_lowercase(), e.name.clone()));
    entries
}

/// Whether `file` sits directly in `dir`.
fn is_in(file: &Path, dir: &Path) -> bool {
    let parent = file.parent().and_then(|p| fs::canonicalize(p).ok());
    parent.is_some() && parent == fs::canonicalize(dir).ok()
}

/// `name` with ` (n)` before its extension, for `n` above 1.
fn numbered(name: &Path, n: u32) -> Option<OsString> {
    let whole = name.file_name()?;
    if n < 2 {
        return Some(whole.to_owned());
    }
    let mut numbered = name.file_stem()?.to_owned();
    numbered.push(format!(" ({n})"));
    if let Some(ext) = name.extension() {
        numbered.push(".");
        numbered.push(ext);
    }
    Some(numbered)
}

/// Copy `file` into `dir`, creating `dir` if need be, so that it stays listed there, and
/// return where the copy is. A file already in `dir` is left where it is. When `dir` holds a
/// different file of the same name, the copy is numbered: `Name (2).wsz`, and so on; an
/// identical one is used as it is.
///
/// # Errors
/// Fails if `file` has no file name, or cannot be read or copied.
pub fn install(file: &Path, dir: &Path) -> Result<PathBuf> {
    if is_in(file, dir) {
        return Ok(file.to_path_buf());
    }
    let Some(name) = file.file_name() else {
        bail!("{} has no file name", file.display());
    };
    install_as(file, Path::new(name), dir)
}

/// Copy `file` into `dir` as [`install`] does, named `name` (a file name, without folders)
/// rather than after `file`.
///
/// # Errors
/// Fails if `name` is not a plain file name, or `file` cannot be read or copied.
pub fn install_as(file: &Path, name: &Path, dir: &Path) -> Result<PathBuf> {
    if name.file_name() != Some(name.as_os_str()) {
        bail!("{} is not a file name", name.display());
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let contents = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    for n in 1..1000 {
        let Some(name) = numbered(name, n) else {
            bail!("{} has no file name", file.display());
        };
        let dest = dir.join(name);
        match fs::read(&dest) {
            Ok(existing) if existing == contents => return Ok(dest),
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::write(&dest, &contents)
                    .with_context(|| format!("copying {} to {}", file.display(), dest.display()))?;
                return Ok(dest);
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", dest.display())),
        }
    }
    bail!(
        "{} holds too many files named like {}",
        dir.display(),
        file.display()
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A scratch folder, removed again when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("neoamp-library-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn listing_keeps_one_extension_sorted_by_name() {
        let tmp = TempDir::new("list");
        for name in ["b.wsz", "A.WSZ", "c.txt", "a2.wsz"] {
            fs::write(tmp.0.join(name), b"x").unwrap();
        }
        fs::create_dir(tmp.0.join("dir.wsz")).unwrap();
        let names: Vec<String> = list(&tmp.0, "wsz").into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["A", "a2", "b"]);
        assert!(list(&tmp.0.join("missing"), "wsz").is_empty());
    }

    #[test]
    fn installing_copies_in_once_and_numbers_clashes() {
        let tmp = TempDir::new("install");
        let (outside, skins) = (tmp.0.join("outside"), tmp.0.join("Skins"));
        fs::create_dir(&outside).unwrap();
        let skin = outside.join("Cool.wsz");
        fs::write(&skin, b"one").unwrap();
        let copy = install(&skin, &skins).unwrap();
        assert_eq!(copy, skins.join("Cool.wsz"));
        assert_eq!(install(&skin, &skins).unwrap(), copy, "identical: reused");
        assert_eq!(install(&copy, &skins).unwrap(), copy, "already inside");
        fs::write(&skin, b"two").unwrap();
        assert_eq!(install(&skin, &skins).unwrap(), skins.join("Cool (2).wsz"));
        assert_eq!(fs::read(skins.join("Cool.wsz")).unwrap(), b"one");
        assert!(install(&outside.join("missing.wsz"), &skins).is_err());
    }

    #[test]
    fn installing_under_another_name_numbers_clashes_too() {
        let tmp = TempDir::new("install-as");
        let skins = tmp.0.join("Skins");
        let cached = tmp.0.join("0123abcd.wsz");
        fs::write(&cached, b"one").unwrap();
        let name = Path::new("Bento.wsz");
        let copy = install_as(&cached, name, &skins).unwrap();
        assert_eq!(copy, skins.join("Bento.wsz"));
        assert_eq!(install_as(&cached, name, &skins).unwrap(), copy, "reused");
        fs::write(&cached, b"two").unwrap();
        assert_eq!(
            install_as(&cached, name, &skins).unwrap(),
            skins.join("Bento (2).wsz")
        );
        for bad in ["../Bento.wsz", "a/b.wsz", ""] {
            assert!(
                install_as(&cached, Path::new(bad), &skins).is_err(),
                "{bad}"
            );
        }
    }
}
