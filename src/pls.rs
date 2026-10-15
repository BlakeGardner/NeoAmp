//! PLS playlists, Winamp's INI-style format:
//!
//! ```text
//! [playlist]
//! File1=C:\Music\Song.mp3
//! Title1=Band - Song
//! Length1=213
//! NumberOfEntries=1
//! Version=2
//! ```
//!
//! Entries are numbered from 1 and may come in any order; a length of -1 means unknown. Keys
//! are read in any case, and lines may end in `\r\n` or `\n`.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;

use crate::playlist_file::{decode_text, entry_path, strip_prefix_ignore_case, written_path};
use crate::ui::Track;

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// The playlist as a PLS file, with `\r\n` line ends as Windows' INI files have. Paths inside
/// `base`, the playlist's folder, are written relative to it. Entries whose path contains a
/// line break cannot be written and are left out (logged).
#[must_use]
pub fn write(tracks: &[Track], base: Option<&Path>) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + tracks.len() * 128);
    out.extend_from_slice(b"[playlist]\r\n");
    let mut n = 0;
    for track in tracks {
        let path = written_path(&track.path, base);
        if path.contains(&b'\n') || path.contains(&b'\r') {
            log::warn!(
                "{} has a line break in its name; it cannot be saved in the playlist",
                track.path.display()
            );
            continue;
        }
        n += 1;
        out.extend_from_slice(format!("File{n}=").as_bytes());
        out.extend_from_slice(&path);
        let title = track.title.replace(['\r', '\n'], " ");
        let seconds = track.seconds.map_or(-1, i64::from);
        write!(out, "\r\nTitle{n}={title}\r\nLength{n}={seconds}\r\n")
            .expect("BUG: writing to a Vec never fails");
    }
    write!(out, "NumberOfEntries={n}\r\nVersion=2\r\n").expect("BUG: writing to a Vec never fails");
    out
}

/// One entry while its keys are being collected.
#[derive(Default)]
struct Entry {
    file: Option<Vec<u8>>,
    title: Option<String>,
    seconds: Option<u32>,
}

/// The entries of a PLS file, in the order of their numbers. Relative paths are taken from
/// `base`, the playlist's folder. An entry without a title is named by its file, and one
/// without a `File` line is skipped.
#[must_use]
pub fn parse(bytes: &[u8], base: &Path) -> Vec<Track> {
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    let mut entries: BTreeMap<u64, Entry> = BTreeMap::new();
    for line in bytes.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(eq) = line.iter().position(|&b| b == b'=') else {
            continue;
        };
        let (key, value) = (line[..eq].trim_ascii(), &line[eq + 1..]);
        // The entry number after `name`, if `key` is `name` followed by one.
        let number = |name: &[u8]| {
            strip_prefix_ignore_case(key, name)
                .and_then(|n| std::str::from_utf8(n).ok())
                .and_then(|n| n.parse::<u64>().ok())
        };
        if let Some(n) = number(b"file") {
            entries.entry(n).or_default().file = Some(value.to_vec());
        } else if let Some(n) = number(b"title") {
            entries.entry(n).or_default().title = Some(decode_text(value).trim().to_owned());
        } else if let Some(n) = number(b"length") {
            entries.entry(n).or_default().seconds = decode_text(value)
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|&s| s > 0)
                .and_then(|s| u32::try_from(s).ok());
        }
    }
    entries
        .into_values()
        .filter_map(|e| {
            let mut track = Track::from_path(entry_path(&e.file?, base)?);
            track.seconds = e.seconds;
            if let Some(title) = e.title.filter(|t| !t.is_empty()) {
                track.title = title;
            }
            Some(track)
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn winamp_pls_files_are_read_in_entry_order() {
        let text = b"\xEF\xBB\xBF[playlist]\r\nNumberOfEntries=3\r\n\
                     File2=two.mp3\r\nTitle2=Second\r\nLength2=-1\r\n\
                     file1=sub/one.mp3\r\ntitle1=First\r\nLENGTH1=213\r\n\
                     Title3=No file\r\nVersion=2\r\n\
                     File4=four.ogg\r\nLength4=oops\r\n";
        let got = parse(text, Path::new("/lists"));
        assert_eq!(
            got,
            vec![
                Track {
                    title: "First".into(),
                    seconds: Some(213),
                    path: Path::new("/lists").join("sub/one.mp3"),
                },
                Track {
                    title: "Second".into(),
                    seconds: None,
                    path: Path::new("/lists").join("two.mp3"),
                },
                Track::from_path(Path::new("/lists").join("four.ogg")),
            ]
        );
    }

    #[test]
    fn written_files_read_back_with_paths_relative_to_their_folder() {
        let base = std::env::temp_dir().join("lists");
        let tracks = vec![
            Track {
                title: "A = B, live".into(),
                seconds: Some(5),
                path: base.join("a.mp3"),
            },
            Track::from_path(std::env::temp_dir().join("b.flac")),
        ];
        let bytes = write(&tracks, Some(&base));
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            text.starts_with("[playlist]\r\nFile1=a.mp3\r\nTitle1=A = B, live\r\nLength1=5\r\n")
        );
        assert!(text.ends_with("NumberOfEntries=2\r\nVersion=2\r\n"));
        assert_eq!(parse(&bytes, &base), tracks);
    }

    #[test]
    fn streams_and_line_breaks_are_left_out() {
        let text = b"[playlist]\nFile1=http://radio.example:8000/\nFile2=ok.mp3\n";
        let got = parse(text, Path::new("/"));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, PathBuf::from("/ok.mp3"));
        let bad = [Track::from_path(PathBuf::from("/a\nb.mp3"))];
        assert!(
            String::from_utf8(write(&bad, None))
                .unwrap()
                .contains("NumberOfEntries=0")
        );
    }
}
