//! Extended M3U playlists (`.m3u8`): one path per line, each optionally preceded by an
//! `#EXTINF:<seconds>,<title>` line. Winamp kept its own playlist this way, in `winamp.m3u`.
//!
//! Lines may end in `\r\n` or `\n`, and a UTF-8 byte order mark is skipped. Paths are written as
//! the operating system spells them. On Unix-like systems that is raw bytes, so a file name that
//! is not valid UTF-8 still comes back exactly; on Windows a path that is not valid Unicode (an
//! unpaired surrogate, which almost never happens) is written with replacement characters.

use std::borrow::Cow;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::ui::Track;

const HEADER: &[u8] = b"#EXTM3U\n";
const EXTINF: &[u8] = b"EXTINF:";
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// The playlist as an extended M3U file. Entries whose path contains a line break cannot be
/// written as M3U and are left out (logged).
#[must_use]
pub fn write(tracks: &[Track]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER.len() + tracks.len() * 96);
    out.extend_from_slice(HEADER);
    for track in tracks {
        let path = path_bytes(&track.path);
        if path.contains(&b'\n') || path.contains(&b'\r') {
            log::warn!(
                "{} has a line break in its name; it cannot be saved in the playlist",
                track.path.display()
            );
            continue;
        }
        let seconds = track.seconds.map_or(-1, i64::from);
        // A line break in a tag would end the line early.
        let title = track.title.replace(['\r', '\n'], " ");
        writeln!(out, "#EXTINF:{seconds},{title}").expect("BUG: writing to a Vec never fails");
        out.extend_from_slice(&path);
        out.push(b'\n');
    }
    out
}

/// The entries of an M3U or extended M3U file. Relative paths are taken from `base`, the
/// playlist's folder. Comments and directives other than `#EXTINF` are skipped, and an entry
/// without a usable `#EXTINF` line is titled by its file name with its length unknown.
#[must_use]
pub fn parse(bytes: &[u8], base: &Path) -> Vec<Track> {
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    let mut tracks = Vec::new();
    let mut info = None;
    for line in bytes.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Some(directive) = line.strip_prefix(b"#") {
            if let Some(extinf) = strip_prefix_ignore_case(directive, EXTINF) {
                info = Some(parse_extinf(extinf));
            }
            continue;
        }
        let path = path_from_bytes(line);
        let path = if path.is_relative() {
            base.join(path)
        } else {
            path
        };
        let mut track = Track::from_path(path);
        if let Some((seconds, title)) = info.take() {
            track.seconds = seconds;
            if !title.is_empty() {
                track.title = title;
            }
        }
        tracks.push(track);
    }
    tracks
}

fn strip_prefix_ignore_case<'a>(bytes: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    let head = bytes.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &bytes[prefix.len()..])
}

/// `213,Artist - Title` into the length (unknown when not positive) and the title. Some
/// players put attributes after the length (`-1 tvg-id="x",Title`); they are ignored.
fn parse_extinf(text: &[u8]) -> (Option<u32>, String) {
    let text = String::from_utf8_lossy(text);
    let (length, title) = text.split_once(',').unwrap_or((&text, ""));
    let seconds = length
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&s| s > 0)
        .and_then(|s| u32::try_from(s).ok());
    (seconds, title.trim().to_owned())
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Cow<'_, [u8]> {
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
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, seconds: Option<u32>, path: &str) -> Track {
        Track {
            title: title.into(),
            seconds,
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn written_playlists_read_back_unchanged() {
        let tracks = vec![
            track("Band - Song, with a comma", Some(213), "/music/a.mp3"),
            track("b", None, "/music/b.flac"),
            track("Ünïcödé", Some(1), "/music/ü/c.ogg"),
        ];
        let bytes = write(&tracks);
        assert!(bytes.starts_with(b"#EXTM3U\n#EXTINF:213,Band - Song, with a comma\n"));
        assert_eq!(parse(&bytes, Path::new("/elsewhere")), tracks);
    }

    #[test]
    fn windows_line_endings_bom_and_plain_m3u_are_read() {
        let text = b"\xEF\xBB\xBF#EXTM3U\r\n#EXTINF:61,Band - Intro\r\n/music/01 Intro.mp3\r\n\r\n\
                     # a comment\r\n/music/02 Plain.wav\r\n#extinf:-1,Unknown length\r\n/x/y.mp3";
        let got = parse(text, Path::new("/"));
        assert_eq!(
            got,
            vec![
                track("Band - Intro", Some(61), "/music/01 Intro.mp3"),
                track("02 Plain", None, "/music/02 Plain.wav"),
                track("Unknown length", None, "/x/y.mp3"),
            ]
        );
    }

    #[test]
    fn relative_paths_are_taken_from_the_playlist_folder() {
        let base = Path::new("lists").join("rock");
        let got = parse(b"song.mp3\n../other.mp3\n", &base);
        assert_eq!(got[0].path, base.join("song.mp3"));
        assert_eq!(got[1].path, base.join("../other.mp3"));
    }

    #[test]
    fn extinf_attributes_and_bad_lengths_are_tolerated() {
        assert_eq!(
            parse_extinf(b"-1 tvg-id=\"x\",Radio"),
            (None, "Radio".into())
        );
        assert_eq!(parse_extinf(b"abc,Title"), (None, "Title".into()));
        assert_eq!(parse_extinf(b"99999999999,T"), (None, "T".into()));
        assert_eq!(parse_extinf(b"42"), (Some(42), String::new()));
    }

    #[test]
    fn line_breaks_in_titles_and_paths_cannot_break_the_file() {
        let tracks = [
            track("Two\nlines", Some(5), "/music/a.mp3"),
            track("bad", None, "/music/new\nline.mp3"),
        ];
        let got = parse(&write(&tracks), Path::new("/"));
        assert_eq!(got, vec![track("Two lines", Some(5), "/music/a.mp3")]);
    }

    #[cfg(unix)]
    #[test]
    fn file_names_that_are_not_utf8_round_trip() {
        use std::os::unix::ffi::OsStrExt;
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/music/caf\xE9.mp3"));
        let tracks = [Track::from_path(path.clone())];
        let got = parse(&write(&tracks), Path::new("/"));
        assert_eq!(got[0].path, path);
    }
}
