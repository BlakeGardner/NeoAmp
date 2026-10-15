//! What NeoAmp remembers between runs. One folder in the platform's config directory holds:
//!
//! - `settings.toml`: volume, equalizer, windows, skin, zoom and the like ([`Settings`]).
//! - `playlist.m3u8`: the playlist, as an extended M3U file (see [`crate::m3u`]).
//! - `autoload.eqf`: the equalizer's auto-load presets (see [`crate::eqf::AutoLoad`]).
//! - The skins and presets folders, which the menus list (see [`crate::library`]).
//!
//! The folder is `~/.config/neoamp` on Linux (following `XDG_CONFIG_HOME`),
//! `%APPDATA%\NeoAmp` on Windows and `~/Library/Application Support/NeoAmp` on macOS.
//! `NEOAMP_CONFIG_DIR` names another one.
//!
//! `docs/SETTINGS.md` describes every key for users. Keep it in step with [`Settings`]: a test
//! there checks the leniencies it promises.
//!
//! Reading never keeps the app from starting. Missing files give the defaults, and so does an
//! unreadable `settings.toml`, which is first renamed `settings.toml.bad` so that the next save
//! does not destroy a hand edit gone wrong. [`Saver`] writes a file only when its contents have
//! changed, to a temporary file that then replaces the old one, so a crash part-way through a
//! save leaves either the old file or the new one.

use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::audio::{BANDS, EQ_RANGE_DB};
use crate::eqf::{self, AutoLoad};
use crate::m3u;
use crate::sprites::pl as p;
use crate::ui::{DEFAULT_VOLUME, Track, Ui, WindowKind, db_slider, slider_db};
use crate::vis::{AnalyzerOptions, ScopeOptions, VisMode};
use crate::zoom::Zoom;

/// Set to a folder to keep the settings and playlist there instead of the platform's folder.
pub const DIR_VAR: &str = "NEOAMP_CONFIG_DIR";
const SETTINGS_FILE: &str = "settings.toml";
const PLAYLIST_FILE: &str = "playlist.m3u8";
/// Auto-load equalizer presets, in Winamp's `.eqf` format (see [`eqf::AutoLoad`]).
const AUTOLOAD_FILE: &str = "autoload.eqf";

/// The folder's name, and those of the skins and presets folders in it: lowercase where that
/// is the custom, capitalised elsewhere.
#[cfg(any(target_os = "windows", target_os = "macos"))]
const DIR_NAME: &str = "NeoAmp";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const DIR_NAME: &str = "neoamp";
#[cfg(any(target_os = "windows", target_os = "macos"))]
const SKINS_DIR: &str = "Skins";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const SKINS_DIR: &str = "skins";
#[cfg(any(target_os = "windows", target_os = "macos"))]
const PRESETS_DIR: &str = "Presets";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const PRESETS_DIR: &str = "presets";

/// Everything `settings.toml` holds. A key missing from the file keeps its default, so files
/// written by older versions load unchanged. [`Settings::apply`] brings values into range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `0..=1`.
    pub volume: f32,
    /// `-1` (all left) to `1` (all right).
    pub balance: f32,
    pub shuffle: bool,
    pub repeat: bool,
    /// The time display counts down to the end of the track.
    pub time_remaining: bool,
    #[serde(deserialize_with = "or_default")]
    pub vis: VisMode,
    pub double_size: bool,
    /// The windows stay above other applications' (Ctrl+A).
    pub always_on_top: bool,
    /// A zoom picked with Ctrl+= / Ctrl+- or `--scale`. `None` follows the screen's height.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "or_default"
    )]
    pub zoom: Option<Zoom>,
    /// The `.wsz` skin shown. `None` is the built-in base skin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<PathBuf>,
    pub analyzer: AnalyzerOptions,
    pub oscilloscope: ScopeOptions,
    pub eq: EqConfig,
    pub playlist: PlaylistConfig,
    pub windows: WindowLayout,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            volume: DEFAULT_VOLUME,
            balance: 0.0,
            shuffle: false,
            repeat: false,
            time_remaining: false,
            vis: VisMode::default(),
            double_size: false,
            always_on_top: false,
            zoom: None,
            skin: None,
            analyzer: AnalyzerOptions::default(),
            oscilloscope: ScopeOptions::default(),
            eq: EqConfig::default(),
            playlist: PlaylistConfig::default(),
            windows: WindowLayout::default(),
        }
    }
}

/// The equalizer, in decibels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqConfig {
    pub on: bool,
    /// The AUTO button: load each file's auto-load preset as it starts.
    pub auto: bool,
    pub preamp: f32,
    /// From 60 Hz to 16 kHz.
    #[serde(deserialize_with = "or_default")]
    pub bands: [f32; BANDS],
}

impl Default for EqConfig {
    fn default() -> Self {
        EqConfig {
            on: true,
            auto: false,
            preamp: 0.0,
            bands: [0.0; BANDS],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaylistConfig {
    /// The current entry of `playlist.m3u8`, counting from 0.
    pub current: usize,
}

/// Where the windows were, in screen (physical) pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowLayout {
    /// Screen pixels per skin pixel of the main window when the positions were taken: the
    /// zoom, double size and the desktop's scale together. Offsets are carried over to another
    /// scale through it.
    pub scale: f64,
    /// The main window's top-left corner. `None` lets NeoAmp place it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main: Option<[i32; 2]>,
    pub eq: WindowConfig,
    pub playlist: WindowConfig,
}

impl Default for WindowLayout {
    fn default() -> Self {
        WindowLayout {
            scale: 1.0,
            main: None,
            eq: WindowConfig::default(),
            playlist: WindowConfig {
                size: Some([p::MIN_W, p::MIN_H]),
                ..WindowConfig::default()
            },
        }
    }
}

impl WindowLayout {
    /// The equalizer's or the playlist's entry; `None` for the main window.
    #[must_use]
    pub fn window(&self, kind: WindowKind) -> Option<&WindowConfig> {
        match kind {
            WindowKind::Main => None,
            WindowKind::Eq => Some(&self.eq),
            WindowKind::Playlist => Some(&self.playlist),
        }
    }

    /// The equalizer's or the playlist's entry; `None` for the main window.
    pub fn window_mut(&mut self, kind: WindowKind) -> Option<&mut WindowConfig> {
        match kind {
            WindowKind::Main => None,
            WindowKind::Eq => Some(&mut self.eq),
            WindowKind::Playlist => Some(&mut self.playlist),
        }
    }

    /// `scale`, unless a hand edit made it something positions cannot be converted with.
    #[must_use]
    pub fn usable_scale(&self) -> Option<f64> {
        (self.scale.is_finite() && self.scale > 0.0).then_some(self.scale)
    }
}

/// The equalizer or playlist window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub open: bool,
    /// From the main window's top-left corner to this window's. `None` docks it below the
    /// main window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<[i32; 2]>,
    /// In skin pixels. Only the playlist can be resized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[i32; 2]>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            open: true,
            offset: None,
            size: None,
        }
    }
}

/// For values whose type is right but whose content may not be: a zoom between the steps, a
/// visualizer mode this version does not know, a band list of the wrong length. Such a value
/// falls back to its default on its own instead of costing the whole file.
pub(crate) fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    Ok(lenient(deserializer)?.unwrap_or_default())
}

/// The value, or `None` (with a warning) when it is valid TOML but not a usable `T`. For
/// fields whose fallback is not `T::default()`.
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = toml::Value::deserialize(deserializer)?;
    match value.try_into() {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            log::warn!("settings: {e}; using the default instead");
            Ok(None)
        }
    }
}

/// `v` within `lo..=hi`, or `default` when it is not a number at all.
fn in_range(v: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        default
    }
}

impl Settings {
    /// Parse `settings.toml`.
    ///
    /// # Errors
    /// Fails on broken TOML and on values of the wrong type. A value of the right type that is
    /// still unusable falls back to its default instead, with a warning.
    pub fn from_toml(text: &str) -> Result<Settings, toml::de::Error> {
        toml::from_str(text)
    }

    /// The settings as `settings.toml`, under a comment explaining the units.
    ///
    /// # Errors
    /// Fails if the skin's path is not valid Unicode, which TOML cannot hold.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let body = toml::to_string(self)?;
        Ok(format!(
            "# NeoAmp settings, rewritten whenever they change. Edit them while NeoAmp is closed.\n\
             # volume: 0 to 1. balance: -1 (left) to 1 (right). eq: decibels, -{EQ_RANGE_DB} \
             to {EQ_RANGE_DB}.\n\
             # windows: screen pixels; eq and playlist offsets are from the main window's \
             corner.\n\n{body}"
        ))
    }

    /// The settings the interface holds. The window layer fills in the skin and where the
    /// windows are.
    #[must_use]
    pub fn from_ui(ui: &Ui) -> Settings {
        Settings {
            volume: ui.volume,
            balance: ui.balance,
            shuffle: ui.shuffle,
            repeat: ui.repeat,
            time_remaining: ui.remaining_mode,
            vis: ui.vis.mode,
            double_size: ui.double_size,
            always_on_top: ui.always_on_top,
            zoom: (ui.zoom != ui.default_zoom).then_some(ui.zoom),
            skin: None,
            analyzer: ui.vis.analyzer,
            oscilloscope: ScopeOptions {
                style: ui.vis.scope_style,
            },
            eq: EqConfig {
                on: ui.eq_on,
                auto: ui.eq_auto,
                preamp: slider_db(ui.preamp),
                bands: ui.bands.map(slider_db),
            },
            playlist: PlaylistConfig {
                current: ui.current,
            },
            windows: WindowLayout {
                eq: WindowConfig {
                    open: ui.eq_open,
                    ..WindowConfig::default()
                },
                playlist: WindowConfig {
                    open: ui.pl_open,
                    size: Some([ui.pl_size.0, ui.pl_size.1]),
                    offset: None,
                },
                ..WindowLayout::default()
            },
        }
    }

    /// Set the interface to these settings, bringing every value into range. The zoom, skin,
    /// window positions and current entry are left to the window layer, which knows the
    /// screen, the skin files and the playlist.
    pub fn apply(&self, ui: &mut Ui) {
        let defaults = Settings::default();
        ui.volume = in_range(self.volume, 0.0, 1.0, defaults.volume);
        ui.balance = in_range(self.balance, -1.0, 1.0, defaults.balance);
        ui.shuffle = self.shuffle;
        ui.repeat = self.repeat;
        ui.remaining_mode = self.time_remaining;
        ui.vis.mode = self.vis;
        ui.vis.analyzer = self.analyzer;
        ui.vis.scope_style = self.oscilloscope.style;
        ui.double_size = self.double_size;
        ui.always_on_top = self.always_on_top;
        ui.eq_on = self.eq.on;
        ui.eq_auto = self.eq.auto;
        fn slider(db: f32) -> f32 {
            db_slider(in_range(db, -EQ_RANGE_DB, EQ_RANGE_DB, 0.0))
        }
        ui.preamp = slider(self.eq.preamp);
        ui.bands = self.eq.bands.map(slider);
        ui.eq_open = self.windows.eq.open;
        ui.pl_open = self.windows.playlist.open;
        if let Some([w, h]) = self.windows.playlist.size {
            // Snaps to the resize grid and keeps the minimum size.
            ui.resize_playlist((w, h));
        }
    }
}

/// The folder the settings and playlist are kept in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    #[must_use]
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    /// The folder named by `NEOAMP_CONFIG_DIR`, otherwise NeoAmp's folder in the platform's
    /// config directory. `None` when the platform reports no such directory (no home folder).
    #[must_use]
    pub fn platform() -> Option<Store> {
        let dir = std::env::var_os(DIR_VAR)
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::config_dir().map(|d| d.join(DIR_NAME)))?;
        Some(Store { dir })
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the Skins menu finds skins, and where skins opened from elsewhere are copied.
    #[must_use]
    pub fn skins_dir(&self) -> PathBuf {
        self.dir.join(SKINS_DIR)
    }

    /// Where the EQ presets menu finds `.eqf` presets, and where it saves them.
    #[must_use]
    pub fn presets_dir(&self) -> PathBuf {
        self.dir.join(PRESETS_DIR)
    }

    /// The auto-load presets; none when there are none yet or they cannot be read (logged).
    /// A file that is not an `.eqf` file is renamed `autoload.eqf.bad`.
    #[must_use]
    pub fn load_autoload(&self) -> AutoLoad {
        let path = self.dir.join(AUTOLOAD_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return AutoLoad::default(),
            Err(e) => {
                log::warn!("cannot read {}: {e}", path.display());
                return AutoLoad::default();
            }
        };
        match eqf::parse(&bytes) {
            Ok(presets) => AutoLoad::new(presets),
            Err(e) => {
                let bad = with_suffix(&path, ".bad");
                log::warn!("{}: {e}; keeping it as {}", path.display(), bad.display());
                if let Err(e) = fs::rename(&path, &bad) {
                    log::warn!("cannot rename {}: {e}", path.display());
                }
                AutoLoad::default()
            }
        }
    }

    /// Replace the auto-load presets file.
    ///
    /// # Errors
    /// Fails if the file cannot be written.
    pub fn save_autoload(&self, auto: &AutoLoad) -> Result<()> {
        self.replace(AUTOLOAD_FILE, &eqf::write(auto.presets()))
    }

    /// The saved settings, or `None` when there are none yet or they cannot be read (logged).
    /// A file that is not valid settings is renamed `settings.toml.bad`.
    #[must_use]
    pub fn load_settings(&self) -> Option<Settings> {
        let path = self.dir.join(SETTINGS_FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => {
                log::warn!("cannot read {}, using the defaults: {e}", path.display());
                return None;
            }
        };
        match Settings::from_toml(&text) {
            Ok(settings) => Some(settings),
            Err(e) => {
                let bad = with_suffix(&path, ".bad");
                log::warn!(
                    "{} is not valid, using the defaults and keeping it as {}: {e}",
                    path.display(),
                    bad.display()
                );
                if let Err(e) = fs::rename(&path, &bad) {
                    log::warn!("cannot rename {}: {e}", path.display());
                }
                None
            }
        }
    }

    /// The saved playlist, or `None` when there is none yet or it cannot be read (logged).
    #[must_use]
    pub fn load_playlist(&self) -> Option<Vec<Track>> {
        let path = self.dir.join(PLAYLIST_FILE);
        match fs::read(&path) {
            Ok(bytes) => Some(m3u::parse(&bytes, &self.dir)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                log::warn!("cannot read {}: {e}", path.display());
                None
            }
        }
    }

    /// Replace `settings.toml`.
    ///
    /// # Errors
    /// Fails if the skin's path is not valid Unicode, or if the file cannot be written.
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let text = settings.to_toml().context("formatting the settings")?;
        self.replace(SETTINGS_FILE, text.as_bytes())
    }

    /// Replace `playlist.m3u8`.
    ///
    /// # Errors
    /// Fails if the file cannot be written.
    pub fn save_playlist(&self, tracks: &[Track]) -> Result<()> {
        // Absolute paths: the config folder is not where the music is.
        self.replace(PLAYLIST_FILE, &m3u::write(tracks, None))
    }

    /// Replace file `name` with `bytes`: written beside it under a temporary name, flushed to
    /// the disk, then renamed over it.
    fn replace(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let path = self.dir.join(name);
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;
        let tmp = with_suffix(&path, ".tmp");
        let written = write_synced(&tmp, bytes).and_then(|()| fs::rename(&tmp, &path));
        if written.is_err() {
            // Best effort: the temporary file may never have been created.
            let _ = fs::remove_file(&tmp);
        }
        written.with_context(|| format!("writing {}", path.display()))
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// `path` with `suffix` added to its file name: `settings.toml` becomes `settings.toml.bad`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Writes the settings and the playlist when they differ from what it last wrote.
#[derive(Debug)]
pub struct Saver {
    store: Store,
    /// What the files hold, as far as this run knows; `None` when unknown, which forces a write.
    settings: Option<Settings>,
    tracks: Option<Vec<Track>>,
    /// The last write failed. Until one succeeds, further failures are logged quietly.
    failing: bool,
}

impl Saver {
    /// `settings` and `tracks` are what was read from `store` at startup, if anything.
    #[must_use]
    pub fn new(store: Store, settings: Option<Settings>, tracks: Option<Vec<Track>>) -> Saver {
        Saver {
            store,
            settings,
            tracks,
            failing: false,
        }
    }

    /// Write whichever of the two has changed. A failed write is logged and tried again on the
    /// next call.
    pub fn save(&mut self, settings: &Settings, tracks: &[Track]) {
        if self.settings.as_ref() != Some(settings) {
            let written = self.store.save_settings(settings);
            if self.succeeded(written) {
                self.settings = Some(settings.clone());
            }
        }
        if self.tracks.as_deref() != Some(tracks) {
            let written = self.store.save_playlist(tracks);
            if self.succeeded(written) {
                let saved = self.tracks.get_or_insert_with(Vec::new);
                saved.clear();
                saved.extend_from_slice(tracks);
            }
        }
    }

    fn succeeded(&mut self, written: Result<()>) -> bool {
        match written {
            Ok(()) => {
                self.failing = false;
                true
            }
            Err(e) => {
                if self.failing {
                    log::debug!("still cannot save: {e:#}");
                } else {
                    log::warn!("cannot save: {e:#}");
                }
                self.failing = true;
                false
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::vis::{AnalyzerStyle, Bands, Falloff, ScopeStyle};

    /// A scratch folder, removed again when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("neoamp-settings-{}-{name}", std::process::id()));
            // Left over from an earlier run that crashed.
            let _ = fs::remove_dir_all(&dir);
            TempDir(dir)
        }

        fn store(&self) -> Store {
            Store::new(self.0.clone())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn changed() -> Settings {
        Settings {
            volume: 0.5,
            balance: -0.25,
            shuffle: true,
            repeat: true,
            time_remaining: true,
            vis: VisMode::Oscilloscope,
            double_size: true,
            always_on_top: true,
            zoom: Zoom::try_from(2.5).ok(),
            skin: Some(PathBuf::from("/skins/Classic.wsz")),
            analyzer: AnalyzerOptions {
                style: AnalyzerStyle::Fire,
                bands: Bands::Thin,
                peaks: false,
                falloff: Falloff::Fastest,
                peak_falloff: Falloff::Slowest,
            },
            oscilloscope: ScopeOptions {
                style: ScopeStyle::Solid,
            },
            eq: EqConfig {
                on: false,
                auto: true,
                preamp: -3.5,
                bands: [1.0, 2.0, 3.0, 4.0, 5.0, -5.0, -4.0, -3.0, -2.0, -1.0],
            },
            playlist: PlaylistConfig { current: 4 },
            windows: WindowLayout {
                scale: 1.5,
                main: Some([-100, 64]),
                eq: WindowConfig {
                    open: false,
                    offset: Some([0, 174]),
                    size: None,
                },
                playlist: WindowConfig {
                    open: true,
                    offset: Some([413, 0]),
                    size: Some([300, 145]),
                },
            },
        }
    }

    #[test]
    fn the_defaults_are_a_new_interface() {
        assert_eq!(Settings::default(), Settings::from_ui(&Ui::new()));
    }

    #[test]
    fn settings_survive_a_trip_through_toml() {
        let text = changed().to_toml().unwrap();
        assert!(text.starts_with("# NeoAmp settings"), "{text}");
        assert!(text.contains("vis = \"oscilloscope\""), "{text}");
        assert!(text.contains("zoom = 2.5"), "{text}");
        assert_eq!(Settings::from_toml(&text).unwrap(), changed());
        // Nothing optional is written while unset.
        let text = Settings::default().to_toml().unwrap();
        let keys: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once(" = ").map(|(k, _)| k))
            .collect();
        for unset in ["zoom", "skin", "main", "offset"] {
            assert!(!keys.contains(&unset), "{unset} in {text}");
        }
        assert_eq!(Settings::from_toml(&text).unwrap(), Settings::default());
    }

    #[test]
    fn missing_keys_keep_their_defaults() {
        let s = Settings::from_toml("volume = 0.5\r\n[eq]\r\npreamp = 3\r\n").unwrap();
        assert_eq!(s.volume, 0.5);
        assert_eq!(s.eq.preamp, 3.0);
        assert!(s.eq.on, "the equalizer defaults to on");
        assert!(s.windows.playlist.open);
        assert_eq!(s.balance, 0.0);
        assert_eq!(Settings::from_toml("").unwrap(), Settings::default());
    }

    #[test]
    fn unusable_values_fall_back_one_at_a_time() {
        let s = Settings::from_toml(
            "zoom = 1.3\nvis = \"fire\"\nvolume = 0.25\n[eq]\nbands = [1.0, 2.0]\npreamp = 2.0\n",
        )
        .unwrap();
        assert_eq!(s.zoom, None);
        assert_eq!(s.vis, VisMode::Analyzer);
        assert_eq!(s.eq.bands, [0.0; BANDS]);
        assert_eq!((s.volume, s.eq.preamp), (0.25, 2.0));
        assert_eq!(
            Settings::from_toml("zoom = 2").unwrap().zoom,
            Zoom::try_from(2.0).ok()
        );
    }

    #[test]
    fn visualizer_options_fall_back_one_at_a_time() {
        let s = Settings::from_toml(
            "[analyzer]\nstyle = \"plasma\"\nbands = \"thin\"\nfalloff = 3\n\
             peak_falloff = \"warp\"\n[oscilloscope]\nstyle = \"dots\"\n",
        )
        .unwrap();
        assert_eq!(s.analyzer.style, AnalyzerStyle::Normal);
        assert_eq!(s.analyzer.bands, Bands::Thin);
        assert_eq!(s.analyzer.falloff, Falloff::Moderate);
        assert_eq!(
            s.analyzer.peak_falloff,
            Falloff::Slow,
            "the peaks' own default"
        );
        assert!(s.analyzer.peaks);
        assert_eq!(s.oscilloscope.style, ScopeStyle::Dots);
    }

    #[test]
    fn auto_load_presets_are_kept_in_an_eqf_file() {
        let tmp = TempDir::new("autoload");
        let store = tmp.store();
        assert_eq!(store.load_autoload(), AutoLoad::default());
        let mut auto = AutoLoad::default();
        auto.set("song.mp3", eqf::Preset::flat(""));
        store.save_autoload(&auto).unwrap();
        assert_eq!(store.load_autoload(), auto);
        fs::write(tmp.0.join("autoload.eqf"), "not presets").unwrap();
        assert_eq!(store.load_autoload(), AutoLoad::default());
        assert!(tmp.0.join("autoload.eqf.bad").exists());
        assert_ne!(store.skins_dir(), store.presets_dir());
        assert!(store.skins_dir().starts_with(&tmp.0));
    }

    /// What docs/SETTINGS.md promises about reading and writing the file.
    #[test]
    fn the_documented_leniencies_hold() {
        let s = Settings::from_toml("volume = 1\nlouder = true\n[eq]\npreamp = -3\n").unwrap();
        assert_eq!(
            (s.volume, s.eq.preamp),
            (1.0, -3.0),
            "whole numbers for decimals"
        );
        assert_eq!(s.balance, 0.0, "unknown keys are ignored");
        let mut ui = Ui::new();
        Settings::from_toml("volume = nan").unwrap().apply(&mut ui);
        assert_eq!(ui.volume, DEFAULT_VOLUME, "nan counts as missing");
        let text = Settings::default().to_toml().unwrap();
        for line in [
            "volume = 0.78\n",
            "vis = \"analyzer\"\n",
            "[windows.playlist]\n",
        ] {
            assert!(text.contains(line), "{line:?} in {text}");
        }
        let windows = Settings {
            skin: Some(PathBuf::from(r"C:\Skins\Classic.wsz")),
            ..Settings::default()
        };
        let text = windows.to_toml().unwrap();
        assert!(text.contains(r"skin = 'C:\Skins\Classic.wsz'"), "{text}");
        assert_eq!(Settings::from_toml(&text).unwrap(), windows);
    }

    #[test]
    fn broken_files_and_wrong_types_are_rejected() {
        assert!(Settings::from_toml("volume = \"loud\"").is_err());
        assert!(Settings::from_toml("volume = ").is_err());
        assert!(Settings::from_toml("[windows]\nmain = 5").is_err());
    }

    #[test]
    fn applying_brings_values_into_range() {
        let s = Settings::from_toml(
            "volume = 5.0\nbalance = nan\n[eq]\npreamp = 40.0\nbands = [-99.0, 0, 0, 0, 0, \
             0, 0, 0, 0, inf]\n[windows.playlist]\nsize = [10, 10000]\n",
        )
        .unwrap();
        let mut ui = Ui::new();
        s.apply(&mut ui);
        assert_eq!(ui.volume, 1.0);
        assert_eq!(ui.balance, 0.0);
        assert_eq!(ui.preamp, 1.0);
        assert_eq!((ui.bands[0], ui.bands[1], ui.bands[9]), (0.0, 0.5, 0.5));
        assert_eq!(ui.pl_size.0, p::MIN_W);
        assert_eq!(
            (ui.pl_size.1 - p::MIN_H) % p::STEP_H,
            0,
            "on the resize grid"
        );
    }

    #[test]
    fn the_interface_survives_a_trip_through_settings() {
        let mut ui = Ui::new();
        changed().apply(&mut ui);
        let back = Settings::from_ui(&ui);
        let mut expected = changed();
        // Left to the window layer.
        expected.zoom = None;
        expected.skin = None;
        expected.playlist.current = 0;
        expected.windows.scale = 1.0;
        expected.windows.main = None;
        expected.windows.eq.offset = None;
        expected.windows.playlist.offset = None;
        // Decibels go through the slider and back.
        for (got, want) in back.eq.bands.iter().zip(expected.eq.bands) {
            assert!((got - want).abs() < 1e-4, "{got} vs {want}");
        }
        assert!((back.eq.preamp - expected.eq.preamp).abs() < 1e-4);
        expected.eq.bands = back.eq.bands;
        expected.eq.preamp = back.eq.preamp;
        assert_eq!(back, expected);
    }

    #[test]
    fn a_zoom_is_only_saved_when_it_differs_from_the_screens() {
        let mut ui = Ui::new();
        ui.default_zoom = Zoom::try_from(1.5).unwrap();
        ui.zoom = ui.default_zoom;
        assert_eq!(Settings::from_ui(&ui).zoom, None);
        ui.zoom = Zoom::MAX;
        assert_eq!(Settings::from_ui(&ui).zoom, Some(Zoom::MAX));
    }

    #[test]
    fn the_store_saves_and_loads_both_files() {
        let tmp = TempDir::new("store");
        let store = tmp.store();
        assert_eq!(store.load_settings(), None);
        assert_eq!(store.load_playlist(), None);
        store.save_settings(&changed()).unwrap();
        let tracks = vec![Track {
            title: "Band - Song".into(),
            seconds: Some(200),
            path: tmp.0.join("song.mp3"),
        }];
        store.save_playlist(&tracks).unwrap();
        assert_eq!(store.load_settings(), Some(changed()));
        assert_eq!(store.load_playlist(), Some(tracks));
        // Saving again replaces the file and leaves no temporary file behind.
        store.save_settings(&Settings::default()).unwrap();
        assert_eq!(store.load_settings(), Some(Settings::default()));
        let mut names: Vec<_> = fs::read_dir(&tmp.0)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["playlist.m3u8", "settings.toml"]);
    }

    #[test]
    fn an_unreadable_settings_file_is_set_aside() {
        let tmp = TempDir::new("bad");
        let store = tmp.store();
        fs::create_dir_all(&tmp.0).unwrap();
        fs::write(tmp.0.join("settings.toml"), "volume = [").unwrap();
        assert_eq!(store.load_settings(), None);
        assert!(!tmp.0.join("settings.toml").exists());
        assert_eq!(
            fs::read_to_string(tmp.0.join("settings.toml.bad")).unwrap(),
            "volume = ["
        );
    }

    #[test]
    fn the_saver_writes_only_what_changed() {
        let tmp = TempDir::new("saver");
        let settings_file = tmp.0.join("settings.toml");
        let playlist_file = tmp.0.join("playlist.m3u8");
        let mut saver = Saver::new(tmp.store(), None, None);
        let mut settings = Settings::default();
        let mut tracks = vec![Track::from_path(tmp.0.join("a.mp3"))];
        saver.save(&settings, &tracks);
        assert!(settings_file.exists() && playlist_file.exists());
        // Unchanged: nothing is written, so removed files stay removed.
        fs::remove_file(&settings_file).unwrap();
        fs::remove_file(&playlist_file).unwrap();
        saver.save(&settings, &tracks);
        assert!(!settings_file.exists() && !playlist_file.exists());
        settings.repeat = true;
        saver.save(&settings, &tracks);
        assert!(settings_file.exists() && !playlist_file.exists());
        tracks[0].seconds = Some(3);
        saver.save(&settings, &tracks);
        assert!(playlist_file.exists());
        assert_eq!(tmp.store().load_playlist(), Some(tracks));
    }

    #[test]
    fn a_failed_save_is_tried_again() {
        let tmp = TempDir::new("retry");
        // A file where the folder should be makes every write fail.
        fs::write(&tmp.0, b"in the way").unwrap();
        let mut saver = Saver::new(tmp.store(), None, None);
        saver.save(&Settings::default(), &[]);
        assert!(saver.failing);
        fs::remove_file(&tmp.0).unwrap();
        saver.save(&Settings::default(), &[]);
        assert!(!saver.failing);
        assert_eq!(tmp.store().load_settings(), Some(Settings::default()));
    }
}
