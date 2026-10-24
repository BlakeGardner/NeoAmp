//! Playlist files: loading and saving M3U, M3U8 and PLS by their extension, and turning what
//! the user opens (files, folders and playlists, mixed) into playlist entries.
//!
//! Playlists often come from Windows, so the parsers accept `\r\n` and `\n`, and text that is
//! not UTF-8 is read as Windows-1252, the codepage Winamp wrote on Western systems. Entries
//! may be absolute paths, paths relative to the playlist's folder (with `\` separators on
//! every system), `file://` URLs, or `http://` and `https://` URLs, which are internet radio
//! streams (or files on the web) and are kept as written. URLs of other kinds are skipped.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::audio::{self, Location};
use crate::ui::Track;
use crate::{m3u, pls};

/// The playlist formats NeoAmp reads and writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// Extended M3U. Written as UTF-8 whether the file ends in `.m3u` or `.m3u8`.
    M3u,
    /// Winamp's INI-style PLS.
    Pls,
}

/// Extensions of playlist files, for the file dialogs.
pub const EXTENSIONS: [&str; 3] = ["m3u", "m3u8", "pls"];

impl Format {
    /// The format a file's extension names, if it is a playlist.
    #[must_use]
    pub fn of(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "m3u" | "m3u8" => Some(Format::M3u),
            "pls" => Some(Format::Pls),
            _ => None,
        }
    }
}

/// Whether `path` names a playlist file NeoAmp can read.
#[must_use]
pub fn is_playlist(path: &Path) -> bool {
    Format::of(path).is_some()
}

/// The entries of the playlist file at `path`.
///
/// # Errors
/// Fails if `path` is not a playlist NeoAmp knows or cannot be read.
pub fn load(path: &Path) -> Result<Vec<Track>> {
    let Some(format) = Format::of(path) else {
        bail!("{} is not an M3U or PLS playlist", path.display());
    };
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new(""));
    Ok(match format {
        Format::M3u => m3u::parse(&bytes, base),
        Format::Pls => pls::parse(&bytes, base),
    })
}

/// Save `tracks` at `path`, as PLS if it ends in `.pls` and as extended M3U otherwise. A name
/// without an extension gets `.m3u8`. Files inside the playlist's folder are written relative
/// to it, so the folder can move as a whole. Returns where the file went.
///
/// # Errors
/// Fails if the file cannot be written.
pub fn save(path: &Path, tracks: &[Track]) -> Result<PathBuf> {
    let path = if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension("m3u8")
    };
    let base = path.parent();
    let bytes = match Format::of(&path) {
        Some(Format::Pls) => pls::write(tracks, base),
        Some(Format::M3u) | None => m3u::write(tracks, base),
    };
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Playlist entries for what the user opened: a folder adds its playable files (subfolders
/// included), a playlist file its entries, and any other file or a URL itself. A playlist
/// that cannot be read is skipped (logged).
#[must_use]
pub fn expand(items: impl IntoIterator<Item = Location>) -> Vec<Track> {
    let mut tracks = Vec::new();
    for item in items {
        let path = match item {
            Location::File(path) => path,
            url @ Location::Url(_) => {
                tracks.push(Track::new(url));
                continue;
            }
        };
        if path.is_dir() {
            tracks.extend(audio::audio_files([path]).into_iter().map(Track::from_path));
        } else if is_playlist(&path) {
            match load(&path) {
                Ok(entries) => tracks.extend(entries),
                Err(e) => log::warn!("cannot open the playlist: {e:#}"),
            }
        } else {
            tracks.push(Track::from_path(path));
        }
    }
    tracks
}

/// The file or stream a playlist entry names, with relative paths taken from `base`, the
/// playlist's folder. `None` for a URL NeoAmp cannot play (logged).
pub(crate) fn entry_location(raw: &[u8], base: &Path) -> Option<Location> {
    if let Some(rest) = strip_prefix_ignore_case(raw, b"file:") {
        return file_url_path(rest).map(Location::File);
    }
    let text = raw.trim_ascii();
    if is_url(text) {
        let url = decode_text(text);
        if audio::is_web_url(&url) {
            return Some(Location::Url(url.into_owned()));
        }
        log::info!("skipping {url}: only http and https streams can be played");
        return None;
    }
    entry_path(raw, base).map(Location::File)
}

/// The file a `file:` URL names, the way the Flatpak passes the files opened in the file
/// manager (the `%U` in NeoAmp's `.desktop` file): `file:///music/a%20b.mp3` gives
/// `/music/a b.mp3`. `None` for anything else, and for a network share outside Windows
/// (logged).
#[must_use]
pub fn file_url(text: &str) -> Option<PathBuf> {
    strip_prefix_ignore_case(text.as_bytes(), b"file:").and_then(file_url_path)
}

/// The file a playlist entry that is not a URL names, with relative paths taken from `base`.
fn entry_path(raw: &[u8], base: &Path) -> Option<PathBuf> {
    let path = local_path(raw, base);
    // A name that is not UTF-8 may be Windows-1252 from a Windows playlist rather than the
    // file system's own bytes. On Windows (and macOS) names are Unicode, so it must be.
    if std::str::from_utf8(raw).is_err() && (!cfg!(unix) || !path.exists()) {
        let decoded = local_path(decode_text(raw).as_bytes(), base);
        if !cfg!(unix) || decoded.exists() {
            return Some(decoded);
        }
    }
    Some(path)
}

/// A path written in a playlist, made absolute against `base` if it is relative.
fn local_path(raw: &[u8], base: &Path) -> PathBuf {
    let path = path_from_bytes(raw);
    if !path.is_relative() {
        return path;
    }
    // Winamp wrote relative paths with backslashes. Other systems take them as part of the
    // name, so they become separators there, unless the name has forward slashes too.
    if !cfg!(windows) && raw.contains(&b'\\') && !raw.contains(&b'/') {
        let fixed: Vec<u8> = raw
            .iter()
            .map(|&b| if b == b'\\' { b'/' } else { b })
            .collect();
        return tidy(&base.join(path_from_bytes(&fixed)));
    }
    tidy(&base.join(path))
}

/// `path` without `.` components, and with each `..` taking away the folder before it, as
/// Winamp read relative entries. (After a symbolic link, `..` leads elsewhere on disk;
/// playlists do not count on that.)
fn tidy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `scheme://...` for any scheme: a URL rather than a path. A Windows path such as `C:\x` has
/// no `//` after its colon.
fn is_url(raw: &[u8]) -> bool {
    let Some(colon) = raw.iter().position(|&b| b == b':') else {
        return false;
    };
    let scheme = &raw[..colon];
    scheme.len() > 1
        && scheme[0].is_ascii_alphabetic()
        && scheme
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        && raw[colon + 1..].starts_with(b"//")
}

/// The path in a `file:` URL, after the `file:`. `file:///music/a%20b.mp3` and
/// `file:/music/a.mp3` both give `/music/a b.mp3`; on Windows `file:///C:/Music/a.mp3` gives
/// `C:/Music/a.mp3` and `file://server/share/a.mp3` the network path `\\server\share\a.mp3`.
fn file_url_path(rest: &[u8]) -> Option<PathBuf> {
    let path = match rest.strip_prefix(b"//") {
        Some(after) => {
            let slash = after.iter().position(|&b| b == b'/').unwrap_or(after.len());
            let (host, path) = after.split_at(slash);
            if !host.is_empty() && !host.eq_ignore_ascii_case(b"localhost") {
                return network_path(host, path);
            }
            path
        }
        None => rest,
    };
    let mut bytes = percent_decode(path);
    // `/C:/Music` on Windows: the drive comes first there.
    if cfg!(windows)
        && let [b'/', drive, b':', ..] = bytes[..]
        && drive.is_ascii_alphabetic()
    {
        bytes.remove(0);
    }
    Some(path_from_bytes(&bytes))
}

/// `\\host\path` for a `file://host/path` URL: a network share, which only Windows names this
/// way.
fn network_path(host: &[u8], path: &[u8]) -> Option<PathBuf> {
    if !cfg!(windows) {
        log::info!(
            "skipping file://{}: network shares need Windows",
            decode_text(host)
        );
        return None;
    }
    let mut bytes = b"\\\\".to_vec();
    bytes.extend_from_slice(host);
    bytes.extend(
        percent_decode(path)
            .into_iter()
            .map(|b| if b == b'/' { b'\\' } else { b }),
    );
    Some(path_from_bytes(&bytes))
}

/// `%XX` escapes turned back into bytes; a `%` not followed by two hex digits stays.
fn percent_decode(text: &[u8]) -> Vec<u8> {
    fn hex(b: u8) -> Option<u8> {
        char::from(b)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    }
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while let Some(&b) = text.get(i) {
        if b == b'%'
            && let (Some(hi), Some(lo)) = (
                text.get(i + 1).copied().and_then(hex),
                text.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

pub(crate) fn strip_prefix_ignore_case<'a>(bytes: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    let head = bytes.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &bytes[prefix.len()..])
}

/// Text from a playlist: UTF-8 if it is valid UTF-8, else Windows-1252.
pub(crate) fn decode_text(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => Cow::Owned(bytes.iter().map(|&b| cp1252(b)).collect()),
    }
}

/// One Windows-1252 byte as a character. 0x80-0x9F hold typographic marks there; the five
/// bytes Windows leaves unassigned map to the C1 controls, as Windows itself maps them.
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[usize::from(b - 0x80)],
        // ASCII and Latin-1 are the first 256 code points.
        _ => char::from(b),
    }
}

/// A path as the operating system spells it, for writing to a playlist. On Unix-like systems
/// that is raw bytes, so a file name that is not valid UTF-8 still comes back exactly; on
/// Windows a path that is not valid Unicode (an unpaired surrogate, which almost never happens)
/// is written with replacement characters.
#[cfg(unix)]
pub(crate) fn path_bytes(path: &Path) -> Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
pub(crate) fn path_bytes(path: &Path) -> Cow<'_, [u8]> {
    match path.to_string_lossy() {
        Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
        Cow::Owned(s) => Cow::Owned(s.into_bytes()),
    }
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(decode_text(bytes).into_owned())
}

/// How `path` is written in a playlist kept in folder `base`: relative to it when inside it,
/// else as it is.
pub(crate) fn written_path<'a>(path: &'a Path, base: Option<&Path>) -> Cow<'a, [u8]> {
    match base.and_then(|b| path.strip_prefix(b).ok()) {
        Some(rel) if !rel.as_os_str().is_empty() => path_bytes(rel),
        _ => path_bytes(path),
    }
}

/// How an entry is written in a playlist kept in folder `base`: a file as [`written_path`]
/// writes it, a URL as it is.
pub(crate) fn written_location<'a>(location: &'a Location, base: Option<&Path>) -> Cow<'a, [u8]> {
    match location {
        Location::File(path) => written_path(path, base),
        Location::Url(url) => Cow::Borrowed(url.as_bytes()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A fresh, empty folder under the system's temporary folder.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("neoamp-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn formats_go_by_extension_in_any_case() {
        assert_eq!(Format::of(Path::new("a.M3U")), Some(Format::M3u));
        assert_eq!(Format::of(Path::new("a.m3u8")), Some(Format::M3u));
        assert_eq!(Format::of(Path::new("a.Pls")), Some(Format::Pls));
        assert_eq!(Format::of(Path::new("a.mp3")), None);
        assert!(!is_playlist(Path::new("m3u")));
    }

    fn file(bytes: &[u8]) -> Option<Location> {
        Some(Location::File(path_from_bytes(bytes)))
    }

    #[test]
    fn file_urls_become_paths() {
        let base = Path::new("/lists");
        assert_eq!(
            entry_location(b"file:///music/a%20b.mp3", base),
            file(b"/music/a b.mp3")
        );
        assert_eq!(
            entry_location(b"FILE://localhost/music/c.mp3", base),
            file(b"/music/c.mp3")
        );
        assert_eq!(
            entry_location(b"file:/music/d.mp3", base),
            file(b"/music/d.mp3")
        );
        assert_eq!(percent_decode(b"100%25 %zz%4"), b"100% %zz%4");
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_urls_have_drives_and_shares() {
        let base = Path::new("C:\\lists");
        assert_eq!(
            entry_location(b"file:///C:/Music/a.mp3", base),
            Some(Location::File(PathBuf::from("C:/Music/a.mp3")))
        );
        assert_eq!(
            entry_location(b"file://server/share/a.mp3", base),
            Some(Location::File(PathBuf::from("\\\\server\\share\\a.mp3")))
        );
    }

    #[test]
    fn web_urls_are_streams_others_are_skipped_and_drive_letters_are_not_urls() {
        let base = Path::new("/");
        assert_eq!(
            entry_location(b"http://radio.example:8000/stream \r", base),
            Some(Location::Url("http://radio.example:8000/stream".into()))
        );
        assert_eq!(
            entry_location(b"HTTPS://radio.example/a.pls", base),
            Some(Location::Url("HTTPS://radio.example/a.pls".into()))
        );
        assert_eq!(entry_location(b"rtsp://radio.example/live", base), None);
        assert_eq!(entry_location(b"mms://radio.example/live", base), None);
        assert!(is_url(b"https://x"));
        assert!(!is_url(b"C:\\Music\\a.mp3"));
        assert!(!is_url(b"C:/Music/a.mp3"));
        assert!(!is_url(b"song.mp3"));
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_relative_paths_use_backslashes() {
        let base = Path::new("/lists");
        assert_eq!(
            entry_path(b"Rock\\01 Song.mp3", base),
            Some(PathBuf::from("/lists/Rock/01 Song.mp3"))
        );
        // A name with both kinds keeps its backslash.
        assert_eq!(
            entry_path(b"a/b\\c.mp3", base),
            Some(PathBuf::from("/lists/a/b\\c.mp3"))
        );
        assert_eq!(
            entry_path(b".\\Rock\\..\\Pop\\02.mp3", base),
            Some(PathBuf::from("/lists/Pop/02.mp3"))
        );
    }

    #[test]
    fn relative_entries_lose_dot_folders() {
        let base = Path::new("music").join("lists");
        assert_eq!(entry_path(b"./a.mp3", &base), Some(base.join("a.mp3")));
        assert_eq!(
            entry_path(b"../../../up.mp3", &base),
            Some(Path::new("..").join("up.mp3")),
            "a .. with no folder left to take away stays"
        );
        assert_eq!(tidy(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
    }

    #[test]
    fn text_that_is_not_utf8_is_windows_1252() {
        assert_eq!(decode_text(b"caf\xE9 \x93hi\x94 \x80"), "café “hi” €");
        assert_eq!(decode_text("ü".as_bytes()), "ü");
    }

    #[test]
    fn windows_1252_names_are_found_on_disk() {
        let dir = scratch("cp1252");
        let file = dir.join("café.mp3");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(entry_path(b"caf\xE9.mp3", &dir), Some(file));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saved_lists_load_back_in_either_format_with_relative_paths() {
        let dir = scratch("roundtrip");
        let tracks = vec![
            Track {
                title: "Band - Song".into(),
                seconds: Some(61),
                location: Location::File(dir.join("sub").join("a.mp3")),
            },
            // Absolute on every system, and outside the playlist's folder.
            Track::from_path(std::env::temp_dir().join("neoamp-elsewhere").join("b.flac")),
            Track {
                title: "Radio Llama".into(),
                seconds: None,
                location: Location::Url("https://radio.example/llama?x=1&y=2".into()),
            },
            Track::from_url("http://radio.example:8000/".into()),
        ];
        for name in ["list.m3u", "list.M3U8", "list.pls"] {
            let saved = save(&dir.join(name), &tracks).unwrap();
            let text = std::fs::read(&saved).unwrap();
            let relative = Path::new("sub").join("a.mp3");
            let relative = path_bytes(&relative);
            assert!(
                text.windows(relative.len()).any(|w| w == &relative[..]),
                "{name} writes paths inside its folder relative to it"
            );
            assert_eq!(load(&saved).unwrap(), tracks, "{name}");
        }
        assert_eq!(
            save(&dir.join("bare"), &tracks).unwrap(),
            dir.join("bare.m3u8")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn opened_folders_and_playlists_are_expanded() {
        let dir = scratch("expand");
        std::fs::create_dir(dir.join("album")).unwrap();
        std::fs::write(dir.join("album").join("2.mp3"), b"").unwrap();
        std::fs::write(dir.join("album").join("1.flac"), b"").unwrap();
        std::fs::write(dir.join("album").join("cover.jpg"), b"").unwrap();
        std::fs::write(dir.join("list.m3u"), b"#EXTINF:5,Five\r\nx.mp3\r\n").unwrap();
        let got = expand([
            Location::File(dir.join("album")),
            Location::File(dir.join("list.m3u")),
            Location::File(dir.join("single.wav")),
            Location::Url("http://radio.example/live".into()),
            Location::File(dir.join("missing.pls")),
        ]);
        let titles: Vec<&str> = got.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            ["1", "2", "Five", "single", "http://radio.example/live"]
        );
        assert_eq!(got[2].location, Location::File(dir.join("x.mp3")));
        assert!(load(Path::new("song.mp3")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
