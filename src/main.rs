//! Command-line entry point. All logic lives in the `neoamp` library crate.

// On Windows a release build is a windowed program, so that starting it from Explorer or the
// Start menu opens no console window beside it. Debug builds keep the console for their log.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use neoamp::app::{self, Options};
use neoamp::audio::{self, Location};
use neoamp::playlist_file;

const USAGE: &str = "\
usage: neoamp [--skin <file.wsz>] [--scale <1|1.5|2|2.5|3>] [--screenshot <out.png>]
              [--enqueue] [<file.wsz>] [<audio file, folder, playlist or stream URL>...]

  <file.wsz>           load a classic Winamp 2.x skin (also: drop one onto a window)
  <audio file/folder>  replace the playlist and play the first (dropping onto a window
                       adds instead); folders add their MP3, MP2, AAC, M4A, WAV, AIFF, FLAC
                       and Ogg Vorbis files, and .m3u, .m3u8 and .pls playlists their entries
  <http(s)://...>      an internet radio stream, a playlist leading to one, or a file on
                       the web
  --enqueue            add the files to the end of the playlist instead, without playing
  --scale <factor>     zoom the windows (default: the saved zoom, else from the screen
                       height, 1.5 at 1440p)
  --screenshot <png>   render one frame at 1x to a PNG and exit

While NeoAmp runs, starting it again hands the files and skin to the running copy.
Settings and the playlist are remembered between runs; NEOAMP_CONFIG_DIR=<folder> keeps
them somewhere other than the platform's config folder (and is a NeoAmp of its own).

keys: Z/X/C/V/B transport, Shift+V fade out and stop, L open files, Shift+L open a
      folder, Ctrl+L open a URL, S shuffle, R repeat, J jump to file, Alt+3 file info,
      Ctrl+T time mode, Ctrl+D double size, Ctrl+A always on top, Ctrl+W windowshade,
      Ctrl+= / Ctrl+- / Ctrl+0 zoom in / out / reset,
      Alt+G equalizer, Alt+E playlist, Ctrl+Shift+K MilkDrop, Alt+F main menu (or
      right-click), Alt+S skins, arrows volume/seek, media keys play/pause, next,
      previous, stop (system-wide)
in the playlist: arrows, PgUp/PgDn, Home/End move (Shift selects), Enter plays,
      Ctrl+A / Ctrl+I select all / invert, Delete / Ctrl+Delete / Alt+Delete remove
      selected / crop / dead files, Alt+Up/Down move entries, Ctrl+Shift+1/2/3 sort,
      Ctrl+R reverse, Ctrl+Shift+R randomize, Ctrl+N / Ctrl+O / Ctrl+S new/load/save,
      Ctrl+W windowshade the playlist, Alt+3 file info for the selected entry
in MilkDrop: Space next preset, H hard cut, Backspace previous, R random order,
      Scroll Lock (F14 on macOS) lock the preset, L list the presets, Alt+Enter
      fullscreen, Esc leave it
on macOS, Cmd takes Ctrl's place and Option is Alt";

/// Classic skins are `.wsz` files; any other path is audio for the playlist.
fn is_skin(path: &OsStr) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wsz"))
}

/// Parse the command line; `Ok(None)` means help was requested. File names are taken as the
/// system gives them, Unicode or not; options and URLs are Unicode.
fn parse_args(args: &[OsString]) -> Result<Option<Options>> {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.to_str() {
            Some("-h" | "--help") => return Ok(None),
            Some("--screenshot") => match it.next() {
                Some(v) => opts.screenshot = Some(PathBuf::from(v)),
                None => bail!("--screenshot needs an output path"),
            },
            Some("--scale") => match it.next().and_then(|v| v.to_str()) {
                Some(v) => opts.zoom = Some(v.parse()?),
                None => bail!("--scale needs a factor: 1, 1.5, 2, 2.5 or 3"),
            },
            Some("--skin") => match it.next() {
                Some(v) => opts.skin = Some(PathBuf::from(v)),
                None => bail!("--skin needs a path to a .wsz file"),
            },
            Some("--enqueue") => opts.enqueue = true,
            // The process serial number macOS may add when the Finder opens the app for the
            // first time after it was downloaded. Refusing it would quit on that first launch.
            Some(psn) if cfg!(target_os = "macos") && psn.starts_with("-psn_") => {}
            Some(flag) if flag.starts_with('-') => bail!("unknown option {flag}\n\n{USAGE}"),
            Some(url) if audio::is_web_url(url) => opts.files.push(Location::from_arg(url)),
            _ => {
                // The Flatpak passes the files opened in the file manager as `file://` URLs
                // (the `%U` in the .desktop file).
                let path = arg
                    .to_str()
                    .and_then(playlist_file::file_url)
                    .unwrap_or_else(|| PathBuf::from(arg));
                if is_skin(path.as_os_str()) {
                    opts.skin = Some(path);
                } else {
                    opts.files.push(Location::File(path));
                }
            }
        }
    }
    Ok(Some(opts))
}

/// Whether this build has no console to write to: a windowed program on Windows (see the top
/// of this file), which a terminal neither waits for nor shows the output of.
const WINDOWED: bool = cfg!(all(windows, not(debug_assertions)));

/// Show `text` in a message box, for a build that has no console.
fn message_box(level: rfd::MessageLevel, title: &str, text: &str) {
    rfd::MessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,sctk_adwaita=error"),
    )
    .init();
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = match parse_args(&args) {
        Ok(Some(opts)) => app::run(opts),
        Ok(None) => {
            println!("{USAGE}");
            if WINDOWED {
                message_box(rfd::MessageLevel::Info, "NeoAmp", USAGE);
            }
            Ok(())
        }
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Printed even without a console, for when the output goes to a file.
            eprintln!("Error: {e:?}");
            if WINDOWED {
                message_box(rfd::MessageLevel::Error, "NeoAmp", &format!("{e:#}"));
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn bare_path_and_flags_are_accepted() {
        let opts = parse_args(&args(&["foo.wsz"])).unwrap().unwrap();
        assert_eq!(opts.skin.as_deref(), Some(std::path::Path::new("foo.wsz")));
        let opts = parse_args(&args(&["--skin", "a.wsz", "--screenshot", "out.png"]))
            .unwrap()
            .unwrap();
        assert_eq!(opts.skin.as_deref(), Some(std::path::Path::new("a.wsz")));
        assert_eq!(
            opts.screenshot.as_deref(),
            Some(std::path::Path::new("out.png"))
        );
    }

    #[test]
    fn audio_files_folders_and_streams_go_to_the_playlist() {
        let opts = parse_args(&args(&[
            "a.mp3",
            "Skin.WSZ",
            "music/",
            "https://radio.example/skins.wsz",
            "b.wav",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(opts.skin.as_deref(), Some(std::path::Path::new("Skin.WSZ")));
        let file = |p: &str| Location::File(PathBuf::from(p));
        assert_eq!(
            opts.files,
            [
                file("a.mp3"),
                file("music/"),
                Location::Url("https://radio.example/skins.wsz".into()),
                file("b.wav"),
            ]
        );
    }

    #[test]
    fn file_urls_from_a_file_manager_are_paths() {
        let opts = parse_args(&args(&[
            "file:///music/My%20Skin.wsz",
            "file:///music/a%20b.mp3",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            opts.skin.as_deref(),
            Some(std::path::Path::new("/music/My Skin.wsz"))
        );
        assert_eq!(
            opts.files,
            [Location::File(PathBuf::from("/music/a b.mp3"))]
        );
    }

    #[test]
    fn enqueue_is_a_flag() {
        let opts = parse_args(&args(&["--enqueue", "a.mp3"])).unwrap().unwrap();
        assert!(opts.enqueue);
        assert!(!parse_args(&args(&["a.mp3"])).unwrap().unwrap().enqueue);
    }

    #[test]
    fn the_finders_process_serial_number_is_ignored_on_macos_only() {
        let parsed = parse_args(&args(&["-psn_0_1234567", "a.mp3"]));
        if cfg!(target_os = "macos") {
            let opts = parsed.unwrap().unwrap();
            assert_eq!(opts.files, [Location::File(PathBuf::from("a.mp3"))]);
        } else {
            assert!(parsed.is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_names_need_not_be_unicode() {
        use std::os::unix::ffi::OsStrExt;
        let name = OsStr::from_bytes(b"caf\xe9.mp3");
        let opts = parse_args(&[name.to_os_string()]).unwrap().unwrap();
        assert_eq!(opts.files, [Location::File(PathBuf::from(name))]);
    }

    #[test]
    fn scale_is_parsed() {
        let opts = parse_args(&args(&["--scale", "1.5"])).unwrap().unwrap();
        assert_eq!(opts.zoom, neoamp::zoom::Zoom::try_from(1.5).ok());
        assert_eq!(parse_args(&args(&[])).unwrap().unwrap().zoom, None);
    }

    #[test]
    fn help_and_errors_are_reported() {
        assert!(parse_args(&args(&["--help"])).unwrap().is_none());
        assert!(parse_args(&args(&["--screenshot"])).is_err());
        assert!(parse_args(&args(&["--scale"])).is_err());
        assert!(parse_args(&args(&["--scale", "1.25"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
    }
}
