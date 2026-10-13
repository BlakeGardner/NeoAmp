//! Command-line entry point. All logic lives in the `winamp` library crate.

use std::path::PathBuf;

use anyhow::{Result, bail};
use winamp::app::{self, Options};

const USAGE: &str = "\
usage: winamp [--skin <file.wsz>] [--scale <1|1.5|2|2.5|3>] [--screenshot <out.png>]
              [<file.wsz>] [<audio file or folder>...]

  <file.wsz>           load a classic Winamp 2.x skin (also: drop one onto a window)
  <audio file/folder>  add to the playlist and play the first (also: drop onto a window);
                       folders add their MP3, WAV, FLAC and Ogg Vorbis files
  --scale <factor>     zoom the windows (default: from the screen height, 1.5 at 1440p)
  --screenshot <png>   render one frame at 1x to a PNG and exit

keys: Z/X/C/V/B transport, L open files, S shuffle, R repeat, Ctrl+T time mode,
      Ctrl+D double size, Ctrl+= / Ctrl+- / Ctrl+0 zoom in / out / reset,
      Alt+G equalizer, Alt+E playlist, arrows volume/seek";

/// Classic skins are `.wsz` files; any other path is audio for the playlist.
fn is_skin(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wsz"))
}

/// Parse the command line; `Ok(None)` means help was requested.
fn parse_args(args: &[String]) -> Result<Option<Options>> {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--screenshot" => match it.next() {
                Some(v) => opts.screenshot = Some(PathBuf::from(v)),
                None => bail!("--screenshot needs an output path"),
            },
            "--scale" => match it.next() {
                Some(v) => opts.zoom = Some(v.parse()?),
                None => bail!("--scale needs a factor: 1, 1.5, 2, 2.5 or 3"),
            },
            "--skin" => match it.next() {
                Some(v) => opts.skin = Some(PathBuf::from(v)),
                None => bail!("--skin needs a path to a .wsz file"),
            },
            flag if flag.starts_with('-') => bail!("unknown option {flag}\n\n{USAGE}"),
            path if is_skin(path) => opts.skin = Some(PathBuf::from(path)),
            path => opts.files.push(PathBuf::from(path)),
        }
    }
    Ok(Some(opts))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,sctk_adwaita=error"),
    )
    .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args)? {
        Some(opts) => app::run(opts),
        None => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
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
    fn audio_files_and_folders_go_to_the_playlist() {
        let opts = parse_args(&args(&["a.mp3", "Skin.WSZ", "music/", "b.wav"]))
            .unwrap()
            .unwrap();
        assert_eq!(opts.skin.as_deref(), Some(std::path::Path::new("Skin.WSZ")));
        assert_eq!(
            opts.files,
            ["a.mp3", "music/", "b.wav"].map(PathBuf::from).to_vec()
        );
    }

    #[test]
    fn scale_is_parsed() {
        let opts = parse_args(&args(&["--scale", "1.5"])).unwrap().unwrap();
        assert_eq!(opts.zoom, winamp::zoom::Zoom::try_from(1.5).ok());
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
