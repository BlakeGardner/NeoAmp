//! The desktop application: one borderless OS window per NeoAmp window (main, equalizer,
//! playlist), each presented through `pixels`, with Winamp-style magnetic docking.
//!
//! Docking needs to read and set window positions. X11 (including XWayland) allows that, so
//! the X11 backend is preferred whenever `DISPLAY` is set; pure Wayland forbids it, in which
//! case title-bar drags are handed to the compositor and no snapping happens.
//!
//! Windows are sized in physical pixels: skin size times the UI scale times the desktop's
//! scale factor, rounded. `upscale::Upscaler` stretches each canvas over its whole window: exact
//! squares at whole-number scales, sharp-bilinear in between.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use pixels::wgpu;
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{CursorIcon, Window, WindowId, WindowLevel};

use crate::audio::{self, Player, Scanner};
use crate::canvas::{Image, Rect};
use crate::dock::{self, SNAP_DISTANCE};
use crate::eqf::{self, AutoLoad};
use crate::file_dialog::{FileDialog, Purpose};
use crate::font::TtfFont;
use crate::library::{self, Entry};
use crate::menu::{self, Anchor};
use crate::menus::{self, AppCommand, Command, Library};
use crate::popup::Popups;
use crate::settings::{Saver, Settings, Store, WindowLayout};
use crate::skin::Skin;
use crate::ui::{
    Action, Control, KeyCmd, MenuRequest, PlayState, Track, Transport, Ui, WindowKind,
};
use crate::upscale::{Upscaler, window_to_canvas};
use crate::vis;
use crate::zoom::Zoom;

/// The stock Winamp 2.91 skin, embedded as the fallback for every missing sheet.
pub const BASE_SKIN: &[u8] = include_bytes!("../skins/base-2.91.wsz");
const FRAME: Duration = Duration::from_millis(33);
/// Where the main window first appears, relative to the primary monitor.
const INITIAL_OFFSET: (i32, i32) = (64, 64);
/// Window managers may override a new window's position when mapping it; dock again after this.
const REDOCK_DELAY: Duration = Duration::from_millis(200);
/// After re-docking, wait this long before checking whether the window manager obeyed.
const PROBE_DELAY: Duration = Duration::from_millis(250);
/// How often frame statistics are logged at debug level.
const STATS_PERIOD: Duration = Duration::from_secs(1);
/// How often to check whether the settings or playlist changed and need saving.
const SAVE_CHECK: Duration = Duration::from_secs(2);
/// Skin pixels from a window's top edge to a point inside every title bar.
const TITLE_BAR_Y: i32 = 5;
/// With a menu open, focus may pass through "no window" on its way between two of ours; the
/// menu closes only when none of ours has had it for this long.
const FOCUS_GRACE: Duration = Duration::from_millis(150);

/// Per-second counters for diagnosing stalls (`RUST_LOG=neoamp=debug`).
#[derive(Debug)]
struct FrameStats {
    since: Instant,
    wakeups: u32,
    frames: [u32; 3],
    slowest_present: Duration,
}

impl FrameStats {
    fn new(now: Instant) -> Self {
        Self {
            since: now,
            wakeups: 0,
            frames: [0; 3],
            slowest_present: Duration::ZERO,
        }
    }
}

fn kind_index(kind: WindowKind) -> usize {
    match kind {
        WindowKind::Main => 0,
        WindowKind::Eq => 1,
        WindowKind::Playlist => 2,
    }
}

/// What to run: an optional skin to load, an optional screenshot-and-exit path, an optional
/// zoom and the files to play. Each overrides what the saved settings say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// A `.wsz` skin to show instead of the saved one (or the embedded base skin).
    pub skin: Option<PathBuf>,
    /// Render one frame to this PNG and exit instead of opening windows.
    pub screenshot: Option<PathBuf>,
    /// The zoom to start at; `None` keeps the saved zoom or picks one from the screen height.
    pub zoom: Option<Zoom>,
    /// Audio files and folders. They replace the saved playlist, and the first file plays.
    pub files: Vec<PathBuf>,
}

/// Round a length in screen pixels to whole pixels. Window geometry stays far inside `i32`,
/// so the range check only guards against nonsense input; within it the cast is exact.
fn round_px(v: f64) -> i32 {
    let r = v.round();
    if r.is_nan() {
        0
    } else {
        r.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    }
}

/// Window size in physical pixels for a `(w, h)` skin-pixel canvas at `scale`.
fn scaled_size((w, h): (i32, i32), scale: f64) -> PhysicalSize<u32> {
    let px = |v: i32| round_px(f64::from(v) * scale).max(1).unsigned_abs();
    PhysicalSize::new(px(w), px(h))
}

/// Carry a physical offset taken at scale `from` over to scale `to`, going through whole skin
/// pixels so docked edges land where the rounded window sizes put them.
fn rescale_offset(offset: i32, from: f64, to: f64) -> i32 {
    round_px((f64::from(offset) / from).round() * to)
}

/// Load the base skin, apply the options and either render a screenshot or open the windows.
///
/// Opening the windows restores the saved settings and playlist (see [`crate::settings`]),
/// with the options taking precedence, and saves them again as they change and on exit. A
/// screenshot neither reads nor writes them. Without a usable audio device the interface
/// still runs, silently.
///
/// # Errors
/// Fails if the embedded base skin or the requested skin cannot be parsed, if the screenshot
/// cannot be written, if no windowing backend can be initialised, or if the playlist scanner
/// thread cannot be started.
pub fn run(opts: Options) -> Result<()> {
    let base = Skin::from_wsz(BASE_SKIN, "base-2.91").context("embedded base skin")?;
    // Paths are remembered, so they must not depend on the folder NeoAmp was started in.
    let skin_path = opts.skin.map(absolute);
    let custom = skin_path
        .as_deref()
        .map(|p| load_skin(p, &base))
        .transpose()?;
    let font = TtfFont::load_system(TtfFont::DEFAULT_PX);

    if let Some(out) = &opts.screenshot {
        let skin = custom.as_ref().unwrap_or(&base);
        return render_screenshot(Ui::demo(), skin, font.as_ref(), out);
    }

    let store = Store::platform();
    match &store {
        Some(store) => log::info!("settings are kept in {}", store.dir().display()),
        None => log::warn!("this system has no config folder; nothing will be remembered"),
    }
    let loaded = store.as_ref().and_then(Store::load_settings);
    let settings = loaded.clone().unwrap_or_default();
    let (custom, skin_path) = match (custom, skin_path) {
        // A skin from the command line joins the Skins menu.
        (Some(skin), Some(path)) => (Some(skin), Some(install_skin(&path, store.as_ref()))),
        (Some(skin), None) => (Some(skin), None),
        (None, _) => restore_skin(settings.skin.as_deref(), &base),
    };
    let autoload = store.as_ref().map(Store::load_autoload).unwrap_or_default();

    let mut ui = Ui::new();
    settings.apply(&mut ui);
    let player = match Player::new() {
        Ok(p) => {
            ui.vis.set_sample_rate(p.sample_rate());
            Some(p)
        }
        Err(e) => {
            log::error!("no audio output, playing silently: {e:#}");
            None
        }
    };
    let scanner = Scanner::new()?;
    let event_loop = build_event_loop()?;
    let mut app = App {
        ui,
        player,
        scanner,
        base,
        custom,
        skin_path,
        font,
        panes: Vec::with_capacity(3),
        drag: None,
        resize: None,
        redock: Vec::new(),
        docking: false,
        unmanaged: unmanaged_preference() == Some(true),
        probe: None,
        probe_done: unmanaged_preference().is_some(),
        startup_zoom: opts.zoom.or(settings.zoom),
        applied_scale: 1.0,
        layout: settings.windows,
        offsets: [None; 3],
        restored_main: None,
        saver: None,
        store: store.clone(),
        autoload,
        next_save: Instant::now() + SAVE_CHECK,
        modifiers: ModifiersState::empty(),
        next_frame: Instant::now(),
        stats: FrameStats::new(Instant::now()),
        dialog: None,
        popups: Popups::default(),
        menu_skins: Vec::new(),
        menu_presets: Vec::new(),
        focused: false,
        focus_lost: None,
        swallow_release: false,
    };
    let saved_tracks = if opts.files.is_empty() {
        let tracks = store.as_ref().and_then(Store::load_playlist);
        if let Some(tracks) = &tracks {
            app.restore_playlist(tracks, settings.playlist.current);
        }
        tracks
    } else {
        let files = opts.files.into_iter().map(absolute);
        let mut actions = Vec::new();
        app.add_files(audio::audio_files(files), &mut actions);
        app.perform_audio(actions);
        // Replaces the saved playlist at the first save.
        None
    };
    app.saver = store.map(|store| Saver::new(store, loaded, saved_tracks));
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;
    Ok(())
}

/// `path` made absolute against the current folder, or unchanged if that cannot be read.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// Copy a skin into the skins folder, where the Skins menu lists it, and return the copy's
/// path. Without a config folder, or if copying fails (logged), the skin stays where it is.
fn install_skin(path: &Path, store: Option<&Store>) -> PathBuf {
    let Some(store) = store else {
        return path.to_path_buf();
    };
    match library::install(path, &store.skins_dir()) {
        Ok(copy) => copy,
        Err(e) => {
            log::warn!("cannot add the skin to the skins folder: {e:#}");
            path.to_path_buf()
        }
    }
}

/// Load the skin the settings name, if any. One that cannot be loaded any more (moved or
/// deleted, say) gives way to the base skin and is forgotten.
fn restore_skin(path: Option<&Path>, base: &Skin) -> (Option<Skin>, Option<PathBuf>) {
    let Some(path) = path else {
        return (None, None);
    };
    match load_skin(path, base) {
        Ok(skin) => (Some(skin), Some(path.to_path_buf())),
        Err(e) => {
            log::warn!("cannot load the saved skin, using the base skin: {e:#}");
            (None, None)
        }
    }
}

/// Draw a few frames of `ui` offscreen, stacked the classic way, with a synthetic signal in the
/// visualiser, and save a PNG.
///
/// # Errors
/// Fails if the PNG cannot be written.
pub fn render_screenshot(
    mut ui: Ui,
    skin: &Skin,
    font: Option<&TtfFont>,
    out: &Path,
) -> Result<()> {
    const RATE: u32 = 44_100;
    let start = Instant::now();
    let (w, h) = ui.layout().size;
    let mut canvas = Image::new(w, h);
    ui.vis.set_sample_rate(RATE);
    let mut signal = vec![0.0; usize::try_from(u128::from(RATE) * FRAME.as_millis() / 1000)?];
    for k in 0..70 {
        vis::demo_signal(f64::from(k) * FRAME.as_secs_f64(), RATE, &mut signal);
        ui.vis.push_samples(&signal);
        ui.draw_stacked(&mut canvas, skin, font, start + FRAME * k);
    }
    let (pw, ph) = (u32::try_from(w)?, u32::try_from(h)?);
    image::save_buffer(out, &canvas.data, pw, ph, image::ColorType::Rgba8)
        .with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} ({w}x{h})", out.display());
    Ok(())
}

fn load_skin(path: &Path, base: &Skin) -> Result<Skin> {
    let mut skin = Skin::from_path(path)?;
    skin.fill_missing_from(base);
    if path.to_str().is_none() {
        log::warn!(
            "{} is not valid Unicode, so the settings cannot remember this skin",
            path.display()
        );
    }
    Ok(skin)
}

/// A monitor's area in physical pixels.
fn monitor_rect(monitor: &MonitorHandle) -> Option<Rect> {
    let p = monitor.position();
    let s = monitor.size();
    Some(Rect::new(
        p.x,
        p.y,
        i32::try_from(s.width).ok()?,
        i32::try_from(s.height).ok()?,
    ))
}

/// Whether a window with its top-left corner at `at`, `width` skin pixels wide and drawn
/// `scale` screen pixels per skin pixel, has the middle of its title bar on one of `screens`,
/// where it can be grabbed and dragged.
fn title_bar_on_screen((x, y): (i32, i32), width: i32, scale: f64, screens: &[Rect]) -> bool {
    let px = x.saturating_add(round_px(f64::from(width) / 2.0 * scale));
    let py = y.saturating_add(round_px(f64::from(TITLE_BAR_Y) * scale));
    screens.iter().any(|s| s.contains(px, py))
}

/// Prefer X11 (which can position windows) unless `NEOAMP_BACKEND=wayland` asks otherwise.
fn build_event_loop() -> Result<EventLoop<()>> {
    let backend = std::env::var("NEOAMP_BACKEND")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let prefer_x11 = match backend.as_str() {
        "x11" => true,
        "wayland" => false,
        _ => std::env::var_os("DISPLAY").is_some(),
    };
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    if prefer_x11 {
        use winit::platform::x11::EventLoopBuilderExtX11;
        match EventLoop::builder().with_x11().build() {
            Ok(el) => return Ok(el),
            Err(e) => log::warn!("X11 unavailable ({e}); windows will not dock"),
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let _ = prefer_x11;
    EventLoop::new().context("creating the event loop")
}

/// `NEOAMP_UNMANAGED=1` forces override-redirect windows, `0` forbids them; unset means probe.
fn unmanaged_preference() -> Option<bool> {
    match std::env::var("NEOAMP_UNMANAGED").ok()?.trim() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// One OS window hosting one NeoAmp window.
struct Pane {
    kind: WindowKind,
    upscaler: Upscaler,
    /// Declared before `window` so the GPU surface is dropped first.
    pixels: Pixels<'static>,
    window: Arc<Window>,
    canvas: Image,
    cursor: PhysicalPosition<f64>,
    /// The diagonal resize cursor is showing instead of the default arrow.
    resize_cursor: bool,
    focused: bool,
    opened_at: Instant,
}

impl Pane {
    /// Position and size in physical screen pixels, when the backend can tell us.
    fn rect(&self) -> Option<Rect> {
        let p = self.window.outer_position().ok()?;
        let s = self.window.inner_size();
        Some(Rect::new(
            p.x,
            p.y,
            i32::try_from(s.width).ok()?,
            i32::try_from(s.height).ok()?,
        ))
    }

    /// Like [`Pane::rect`], but `None` while the window is minimized: Windows then reports a
    /// position far off the screen, which must not be remembered.
    fn placed_rect(&self) -> Option<Rect> {
        if self.window.is_minimized() == Some(true) {
            None
        } else {
            self.rect()
        }
    }

    /// Screen pixels per skin pixel: the UI scale times the desktop's scale for this window.
    fn scale(&self, ui_scale: f64) -> f64 {
        ui_scale * self.window.scale_factor()
    }

    /// The last pointer position in canvas (skin pixel) coordinates.
    fn canvas_pos(&self) -> (i32, i32) {
        let size = self.window.inner_size();
        window_to_canvas(
            (self.cursor.x, self.cursor.y),
            (size.width, size.height),
            (self.canvas.w, self.canvas.h),
        )
    }

    /// The monitor this window is on, as a physical rectangle.
    fn screen(&self) -> Option<Rect> {
        monitor_rect(&self.window.current_monitor()?)
    }
}

/// A title-bar drag in progress.
struct Drag {
    kind: WindowKind,
    /// Pointer offset from the dragged window's top-left corner, in physical pixels.
    grab: PhysicalPosition<f64>,
    /// Windows attached to the dragged one, with their offsets from its top-left corner.
    followers: Vec<(WindowKind, i32, i32)>,
}

/// A playlist resize from its grip in progress.
struct Resize {
    /// Where the grip was pressed, in physical pixels from the playlist window's top-left
    /// corner, which stays put while the window changes size.
    start: PhysicalPosition<f64>,
    /// The playlist's size in skin pixels at that moment.
    size: (i32, i32),
}

struct App {
    ui: Ui,
    /// `None` when no audio device could be opened.
    player: Option<Player>,
    scanner: Scanner,
    base: Skin,
    /// A user-chosen skin; `None` means the embedded base skin is shown.
    custom: Option<Skin>,
    /// The file `custom` came from, to be remembered.
    skin_path: Option<PathBuf>,
    font: Option<TtfFont>,
    panes: Vec<Pane>,
    drag: Option<Drag>,
    resize: Option<Resize>,
    /// Windows opened recently whose docked position must be re-applied once mapped.
    redock: Vec<WindowKind>,
    /// The backend reports window positions, so magnetic docking is possible.
    docking: bool,
    /// Windows bypass the window manager (X11 override-redirect) so moves are always honoured.
    /// Some compositors, COSMIC among them, ignore moves of managed X11 windows.
    unmanaged: bool,
    /// A re-dock whose outcome is still to be checked: (window, expected position, when).
    probe: Option<(WindowKind, PhysicalPosition<i32>, Instant)>,
    probe_done: bool,
    /// The zoom asked for on the command line, or else the saved one; `None` picks one from
    /// the screen size.
    startup_zoom: Option<Zoom>,
    /// The main window's scale when the panes were last sized (to re-dock after a change).
    applied_scale: f64,
    /// Where the windows were last known to be: loaded from the settings, then brought up to
    /// date at every save. Positions the backend cannot report keep their saved values.
    layout: WindowLayout,
    /// For the equalizer and playlist (by `kind_index`), the offset from the main window's
    /// top-left corner to theirs, in physical pixels at `applied_scale`. Taken from the
    /// settings at startup and whenever a window closes, so that it reopens where it was.
    offsets: [Option<(i32, i32)>; 3],
    /// The saved position the main window was opened at, if it was. Applied again once the
    /// window manager has mapped the window, and kept if the windows are recreated.
    restored_main: Option<PhysicalPosition<i32>>,
    /// Writes the settings and playlist as they change; `None` without a config folder.
    saver: Option<Saver>,
    /// The config folder, which also holds the skins and presets folders.
    store: Option<Store>,
    /// Equalizer presets loaded with each file while AUTO is on.
    autoload: AutoLoad,
    /// When to next check whether anything needs saving.
    next_save: Instant,
    modifiers: ModifiersState,
    next_frame: Instant,
    stats: FrameStats,
    /// A file dialog, while one is showing.
    dialog: Option<FileDialog>,
    /// The menus on screen.
    popups: Popups,
    /// The skins and preset files the open menu lists; its commands number them.
    menu_skins: Vec<Entry>,
    menu_presets: Vec<Entry>,
    /// One of our windows, menus included, has the keyboard focus.
    focused: bool,
    /// When the last of our windows lost the focus.
    focus_lost: Option<Instant>,
    /// A click that closed a menu went down; its release does nothing either.
    swallow_release: bool,
}

impl App {
    fn pane(&self, kind: WindowKind) -> Option<&Pane> {
        self.panes.iter().find(|p| p.kind == kind)
    }

    fn pane_mut(&mut self, kind: WindowKind) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|p| p.kind == kind)
    }

    fn kind_of(&self, id: WindowId) -> Option<WindowKind> {
        self.panes
            .iter()
            .find(|p| p.window.id() == id)
            .map(|p| p.kind)
    }

    /// Show the skin at `path`. One from outside the skins folder is copied there, so that the
    /// Skins menu lists it from now on.
    fn load_skin(&mut self, path: &Path) {
        match load_skin(path, &self.base) {
            Ok(s) => {
                log::info!("loaded skin {}", s.name);
                self.custom = Some(s);
                self.skin_path = Some(install_skin(path, self.store.as_ref()));
            }
            Err(e) => log::error!("cannot load skin {}: {e:#}", path.display()),
        }
    }

    /// The current file's name, which auto-load presets go by.
    fn track_file_name(&self) -> Option<String> {
        let name = self.ui.track()?.path.file_name()?;
        Some(name.to_string_lossy().into_owned())
    }

    /// With AUTO on, load playlist entry `i`'s auto-load preset, if it has one.
    fn load_auto_preset(&mut self, i: usize) {
        if !self.ui.eq_auto {
            return;
        }
        let Some(name) = self.ui.tracks.get(i).and_then(|t| t.path.file_name()) else {
            return;
        };
        if let Some(preset) = self.autoload.find(&name.to_string_lossy()) {
            log::info!("auto-load preset for {}", preset.name);
            self.ui.apply_preset(preset);
        }
    }

    fn save_autoload(&self) {
        if let Some(store) = &self.store
            && let Err(e) = store.save_autoload(&self.autoload)
        {
            log::error!("cannot save the auto-load presets: {e:#}");
        }
    }

    /// Set the equalizer from the first preset in the `.eqf` file at `path`.
    fn load_preset_file(&mut self, path: &Path) {
        let presets = std::fs::read(path)
            .with_context(|| format!("reading {}", path.display()))
            .and_then(|bytes| eqf::parse(&bytes));
        match presets.as_deref() {
            Ok([first, ..]) => self.ui.apply_preset(first),
            Ok([]) => log::warn!("{} holds no presets", path.display()),
            Err(e) => log::error!("cannot load the preset: {e:#}"),
        }
    }

    /// Save the equalizer as an `.eqf` file at `path`, named after the file.
    fn save_preset_file(&self, path: &Path) {
        let path = if path.extension().is_some() {
            path.to_path_buf()
        } else {
            path.with_extension("eqf")
        };
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = eqf::write(&[self.ui.eq_preset(&name)]);
        match std::fs::write(&path, bytes) {
            Ok(()) => log::info!("saved preset {}", path.display()),
            Err(e) => log::error!("cannot save the preset to {}: {e}", path.display()),
        }
    }

    /// Carry out a menu command.
    fn run_command(&mut self, command: Command, el: &ActiveEventLoop) {
        let actions = self.ui.command(command);
        self.handle(actions, el);
    }

    /// Carry out a menu command that needs files or dialogs.
    fn app_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::BaseSkin => {
                self.custom = None;
                self.skin_path = None;
            }
            AppCommand::Skin(i) => {
                if let Some(path) = self.menu_skins.get(i).map(|e| e.path.clone()) {
                    self.load_skin(&path);
                }
            }
            AppCommand::OpenSkin => self.show_dialog(Purpose::OpenSkin),
            AppCommand::UserPreset(i) => {
                if let Some(path) = self.menu_presets.get(i).map(|e| e.path.clone()) {
                    self.load_preset_file(&path);
                }
            }
            AppCommand::PresetFromFile => self.show_dialog(Purpose::LoadPreset),
            AppCommand::SavePreset => self.show_dialog(Purpose::SavePreset),
            AppCommand::SaveAutoPreset => {
                if let Some(name) = self.track_file_name() {
                    let preset = self.ui.eq_preset(&name);
                    self.autoload.set(&name, preset);
                    self.save_autoload();
                }
            }
            AppCommand::DeleteAutoPreset => {
                if let Some(name) = self.track_file_name()
                    && self.autoload.remove(&name)
                {
                    self.save_autoload();
                }
            }
        }
    }

    /// Show a menu where `req` asks, listing the skins and presets as they are on disk now.
    fn open_menu(&mut self, req: MenuRequest, el: &ActiveEventLoop) {
        let Some(pane) = self.pane(req.window) else {
            return;
        };
        // Pure Wayland cannot tell where a window is; the menu goes where the compositor
        // puts it then.
        let origin = pane.window.outer_position().unwrap_or_default();
        let s = pane.scale(self.ui.scale());
        let anchor = match req.below {
            Some(r) => Anchor::Below(Rect::new(
                origin.x + round_px(f64::from(r.x) * s),
                origin.y + round_px(f64::from(r.y) * s),
                round_px(f64::from(r.w) * s),
                round_px(f64::from(r.h) * s),
            )),
            None => Anchor::Point(
                origin.x + round_px(pane.cursor.x),
                origin.y + round_px(pane.cursor.y),
            ),
        };
        let (x, y) = match anchor {
            Anchor::Point(x, y) => (x, y),
            Anchor::Below(r) => (r.x, r.bottom()),
        };
        let screen = el
            .available_monitors()
            .filter_map(|m| monitor_rect(&m))
            .find(|s| s.contains(x, y))
            .or_else(|| pane.screen())
            .unwrap_or(Rect::new(0, 0, 1 << 16, 1 << 16));
        // Menus follow the zoom, as Windows' grow with the desktop's scale, but not double
        // size, which Winamp's never did.
        let scale = self.ui.zoom.factor() * pane.window.scale_factor();
        if let Some(store) = &self.store {
            self.menu_skins = library::list(&store.skins_dir(), "wsz");
            self.menu_presets = library::list(&store.presets_dir(), "eqf");
        }
        let has_auto_preset = self
            .track_file_name()
            .is_some_and(|name| self.autoload.find(&name).is_some());
        let lib = Library {
            skins: &self.menu_skins,
            skin: self.skin_path.as_deref(),
            presets: &self.menu_presets,
            has_auto_preset,
        };
        let items = menus::build(req.kind, &self.ui, &lib);
        self.focus_lost = None;
        self.popups
            .open(el, items, anchor, screen, scale, req.keyboard);
    }

    /// Keep the windows above other applications' while always on top is on. Override-redirect
    /// windows are above them anyway, and pure Wayland lets no application choose.
    fn apply_window_level(&self) {
        let level = self.window_level();
        for p in &self.panes {
            p.window.set_window_level(level);
        }
    }

    fn window_level(&self) -> WindowLevel {
        if self.ui.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        }
    }

    /// Whether any of our windows, menus included, has the keyboard focus.
    fn has_focus(&self) -> bool {
        self.panes.iter().any(|p| p.focused) || self.popups.has_focus()
    }

    /// A window of ours gained or lost the focus.
    fn focus_changed(&mut self, now: Instant) {
        let had = self.focused;
        self.focused = self.has_focus();
        self.ui.active = self.focused;
        if self.focused {
            self.focus_lost = None;
        } else if had {
            self.focus_lost = Some(now);
        }
    }

    /// Close the menus once the focus has gone to another application. Only a loss counts:
    /// a menu opened while none of our windows had the focus stays.
    fn check_menu_focus(&mut self, now: Instant) {
        if self.popups.is_open()
            && let Some(lost) = self.focus_lost
            && now.saturating_duration_since(lost) >= FOCUS_GRACE
            && !self.has_focus()
        {
            log::debug!("another application took the focus: closing the menu");
            self.popups.close();
            self.focus_lost = None;
        }
    }

    /// A key while a menu is open: it goes to the menu.
    fn menu_key(&mut self, event: &KeyEvent, el: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        let key = match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => menu::Key::Up,
            Key::Named(NamedKey::ArrowDown) => menu::Key::Down,
            Key::Named(NamedKey::ArrowLeft) => menu::Key::Left,
            Key::Named(NamedKey::ArrowRight) => menu::Key::Right,
            Key::Named(NamedKey::Enter) => menu::Key::Enter,
            Key::Named(NamedKey::Escape) => menu::Key::Escape,
            Key::Character(s) => match s.chars().next() {
                Some(ch) => menu::Key::Char(ch),
                None => return,
            },
            _ => return,
        };
        if let Some(command) = self.popups.key(el, key) {
            self.run_command(command, el);
        }
    }

    /// Append files to the playlist, have their tags read, and start the first one if nothing
    /// is playing.
    fn add_files(&mut self, paths: Vec<PathBuf>, actions: &mut Vec<Action>) {
        let added = self.ui.add_files(paths);
        for (i, track) in self.ui.tracks.iter().enumerate().skip(added.start) {
            self.scanner.scan(i, track.path.clone());
        }
        if self.ui.state == PlayState::Stopped && !added.is_empty() {
            self.ui.play_track(added.start, actions);
        }
    }

    /// Put the saved playlist back, stopped on entry `current`. Only entries whose length was
    /// not known when it was saved have their tags read again.
    fn restore_playlist(&mut self, tracks: &[Track], current: usize) {
        let added = self.ui.add_tracks(tracks.iter().cloned());
        for (i, track) in self.ui.tracks.iter().enumerate().skip(added.start) {
            if track.seconds.is_none() {
                self.scanner.scan(i, track.path.clone());
            }
        }
        if let Some(last) = self.ui.tracks.len().checked_sub(1) {
            self.ui.select(current.min(last));
        }
    }

    /// Everything to remember, as it is now.
    fn settings(&mut self) -> Settings {
        let mut settings = Settings::from_ui(&self.ui);
        // TOML cannot hold a path that is not valid Unicode (`load_skin` says so once).
        settings.skin = self.skin_path.clone().filter(|p| p.to_str().is_some());
        self.update_layout();
        let mut windows = self.layout.clone();
        windows.eq.open = settings.windows.eq.open;
        windows.playlist.open = settings.windows.playlist.open;
        windows.playlist.size = settings.windows.playlist.size;
        settings.windows = windows;
        settings
    }

    /// Bring `layout` and `offsets` up to date with where the windows are, when the backend
    /// can tell and the windows have been placed.
    fn update_layout(&mut self) {
        if !self.docking || self.redock.contains(&WindowKind::Main) {
            return;
        }
        let Some(main) = self.pane(WindowKind::Main).and_then(Pane::placed_rect) else {
            return;
        };
        self.layout.scale = self.applied_scale;
        self.layout.main = Some([main.x, main.y]);
        for kind in [WindowKind::Eq, WindowKind::Playlist] {
            let i = kind_index(kind);
            // A window waiting to be re-docked is not where it is going yet.
            if !self.redock.contains(&kind)
                && let Some(r) = self.pane(kind).and_then(Pane::placed_rect)
            {
                self.offsets[i] = Some((r.x - main.x, r.y - main.y));
            }
            if let Some(w) = self.layout.window_mut(kind) {
                w.offset = self.offsets[i].map(|(dx, dy)| [dx, dy]);
            }
        }
    }

    /// Write the settings and playlist if they changed since they were last saved.
    fn save(&mut self) {
        if self.saver.is_none() {
            return;
        }
        let settings = self.settings();
        if let Some(saver) = self.saver.as_mut() {
            saver.save(&settings, &self.ui.tracks);
        }
    }

    /// A file dropped on any window: a skin is applied, anything else joins the playlist.
    fn drop_file(&mut self, path: PathBuf, el: &ActiveEventLoop) {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wsz"))
        {
            self.load_skin(&path);
            return;
        }
        let mut actions = Vec::new();
        self.add_files(audio::audio_files([path]), &mut actions);
        self.handle(actions, el);
    }

    /// Show a file dialog over the main window, unless one is already showing. Skins and
    /// presets dialogs start in their folders.
    fn show_dialog(&mut self, purpose: Purpose) {
        if self.dialog.is_some() {
            return;
        }
        let Some(main) = self.pane(WindowKind::Main) else {
            return;
        };
        let folder = self.store.as_ref().and_then(|store| match purpose {
            Purpose::PlayFiles | Purpose::PlayFolder => None,
            Purpose::OpenSkin => Some(store.skins_dir()),
            Purpose::LoadPreset | Purpose::SavePreset => Some(store.presets_dir()),
        });
        if purpose == Purpose::SavePreset
            && let Some(dir) = &folder
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!("cannot create {}: {e}", dir.display());
        }
        let folder = folder.filter(|dir| dir.is_dir());
        match FileDialog::show(purpose, &main.window, folder.as_deref()) {
            Ok(dialog) => self.dialog = Some(dialog),
            Err(e) => log::error!("cannot show the file dialog: {e:#}"),
        }
    }

    /// Once a file dialog closes, act on what was chosen. Files or a folder to play replace
    /// the playlist and the first one plays, as in Winamp. Cancelling changes nothing.
    fn finish_dialog(&mut self, el: &ActiveEventLoop) {
        let Some(dialog) = &self.dialog else {
            return;
        };
        let Some(paths) = dialog.poll() else {
            return;
        };
        let purpose = dialog.purpose();
        self.dialog = None;
        let Some(first) = paths.first() else {
            return;
        };
        match purpose {
            Purpose::PlayFiles | Purpose::PlayFolder => {
                let files = if purpose == Purpose::PlayFolder {
                    audio::audio_files(paths)
                } else {
                    paths
                };
                if files.is_empty() {
                    log::info!("no playable files there");
                    return;
                }
                let mut actions = Vec::new();
                self.ui.clear_playlist(&mut actions);
                self.add_files(files, &mut actions);
                self.handle(actions, el);
            }
            Purpose::OpenSkin => {
                let path = first.clone();
                self.load_skin(&path);
            }
            Purpose::LoadPreset => {
                let path = first.clone();
                self.load_preset_file(&path);
            }
            Purpose::SavePreset => self.save_preset_file(first),
        }
    }

    /// Pass a playback command from the UI to the audio engine.
    fn transport(&mut self, t: Transport) {
        if let Transport::Open(i) = t {
            self.load_auto_preset(i);
        }
        let Some(player) = self.player.as_mut() else {
            return;
        };
        match t {
            Transport::Open(i) => {
                if let Some(track) = self.ui.tracks.get(i) {
                    log::info!("playing {}", track.path.display());
                    player.open(&track.path);
                }
            }
            Transport::Pause => player.set_paused(true),
            Transport::Resume => player.set_paused(false),
            Transport::Stop => player.stop(),
            Transport::Seek(secs) => player.seek(f64::from(secs)),
        }
    }

    /// Carry out the audio actions in `actions` (before the event loop runs, the only kind).
    fn perform_audio(&mut self, actions: Vec<Action>) {
        for a in actions {
            if let Action::Audio(t) = a {
                self.transport(t);
            }
        }
    }

    /// Once per frame: take in scan results and engine events, hand the engine the current
    /// settings, and read back the position, bitrate and visualiser samples.
    fn sync_audio(&mut self, el: &ActiveEventLoop) {
        while let Some((i, path, info)) = self.scanner.poll() {
            self.ui.set_track_info(i, &path, &info);
        }
        let mut actions = Vec::new();
        if let Some(player) = self.player.as_mut() {
            while let Some(event) = player.poll() {
                self.ui.audio_event(event, &mut actions);
            }
        }
        // A finished track may have started the next one: send that before reading positions.
        self.handle(actions, el);

        let Some(player) = self.player.as_mut() else {
            return;
        };
        player.set_volume(self.ui.volume);
        player.set_balance(self.ui.balance);
        player.set_eq(&self.ui.eq_settings());
        if self.ui.state != PlayState::Stopped {
            // Seconds into a track: f32 is plenty.
            self.ui.elapsed = player.position() as f32;
            let kbps = player.kbps();
            if kbps > 0 {
                self.ui.kbps = kbps;
            }
        }
        let vis = &mut self.ui.vis;
        player.drain_vis(|samples| vis.push_samples(samples));
    }

    /// Set the zoom from the command line, or else the settings, or else the height of the
    /// screen the main window opens on. The last is the default that Ctrl+0 goes back to.
    fn choose_zoom(&mut self, el: &ActiveEventLoop) {
        let monitor = el
            .primary_monitor()
            .or_else(|| el.available_monitors().next());
        let default = monitor.map_or(Zoom::MIN, |m| {
            Zoom::for_screen_height(f64::from(m.size().height) / m.scale_factor())
        });
        let zoom = self.startup_zoom.unwrap_or(default);
        log::info!("zoom {zoom} (Ctrl+= and Ctrl+- change it, Ctrl+0 resets to {default})");
        self.ui.zoom = zoom;
        self.ui.default_zoom = default;
    }

    /// The saved position of the main window, unless its title bar would now be off every
    /// screen (a monitor unplugged since, say), out of reach of the mouse.
    fn saved_main_position(&self, screens: &[Rect]) -> Option<PhysicalPosition<i32>> {
        let [x, y] = self.layout.main?;
        let scale = self.layout.usable_scale()?;
        let (w, _) = self.ui.window_size(WindowKind::Main);
        title_bar_on_screen((x, y), w, scale, screens).then(|| PhysicalPosition::new(x, y))
    }

    /// Take the saved offsets of the equalizer and playlist, carried over to the main window's
    /// scale now, for a main window opened at `main`. An offset that would leave the window's
    /// title bar off every screen is dropped, and that window docks the default way instead.
    fn restore_offsets(&mut self, main: PhysicalPosition<i32>, screens: &[Rect]) {
        let Some(saved_scale) = self.layout.usable_scale() else {
            return;
        };
        let scale = self.applied_scale;
        for kind in [WindowKind::Eq, WindowKind::Playlist] {
            let (w, _) = self.ui.window_size(kind);
            self.offsets[kind_index(kind)] = self
                .layout
                .window(kind)
                .and_then(|c| c.offset)
                .map(|[dx, dy]| {
                    (
                        rescale_offset(dx, saved_scale, scale),
                        rescale_offset(dy, saved_scale, scale),
                    )
                })
                .filter(|&(dx, dy)| {
                    let at = (main.x.saturating_add(dx), main.y.saturating_add(dy));
                    title_bar_on_screen(at, w, scale, screens)
                });
        }
    }

    fn open_pane(
        &mut self,
        el: &ActiveEventLoop,
        kind: WindowKind,
        at: Option<PhysicalPosition<i32>>,
    ) -> Result<()> {
        let title = match kind {
            WindowKind::Main => "NeoAmp",
            WindowKind::Eq => "NeoAmp Equalizer",
            WindowKind::Playlist => "NeoAmp Playlist",
        };
        // The desktop's scale factor is only known once the window exists; `sync_panes`
        // then trims the size to exactly what `scaled_size` asks for.
        let (w, h) = self.ui.window_size(kind);
        let ui_scale = self.ui.scale();
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(
                f64::from(w) * ui_scale,
                f64::from(h) * ui_scale,
            ))
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true);
        if let Some(p) = at {
            attrs = attrs.with_position(p);
        }
        attrs = attrs.with_window_level(self.window_level());
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        if self.unmanaged {
            use winit::platform::x11::WindowAttributesExtX11;
            attrs = attrs.with_override_redirect(true);
        }
        let window = Arc::new(
            el.create_window(attrs)
                .with_context(|| format!("creating the {title} window"))?,
        );
        if let Some(p) = at {
            window.set_outer_position(p);
        }
        let (bw, bh) = (u32::try_from(w)?, u32::try_from(h)?);
        let size = window.inner_size();
        let make_surface =
            || SurfaceTexture::new(size.width.max(1), size.height.max(1), window.clone());
        let pixels = PixelsBuilder::new(bw, bh, make_surface())
            .clear_color(wgpu::Color::TRANSPARENT)
            .alpha_mode(wgpu::CompositeAlphaMode::PreMultiplied)
            .build()
            .or_else(|e| {
                log::debug!("transparent surface unavailable ({e}); using an opaque one");
                PixelsBuilder::new(bw, bh, make_surface())
                    .clear_color(wgpu::Color::BLACK)
                    .build()
            })
            .context("creating the GPU surface")?;
        self.panes.push(Pane {
            kind,
            upscaler: Upscaler::new(&pixels),
            pixels,
            window,
            canvas: Image::new(w, h),
            cursor: PhysicalPosition::new(0.0, 0.0),
            resize_cursor: false,
            focused: false,
            opened_at: Instant::now(),
        });
        if at.is_some() {
            self.redock.push(kind);
        }
        Ok(())
    }

    fn close_pane(&mut self, kind: WindowKind) {
        self.remember_offset(kind);
        self.panes.retain(|p| p.kind != kind);
        self.redock.retain(|k| *k != kind);
        if self.drag.as_ref().is_some_and(|d| d.kind == kind) {
            self.drag = None;
        }
        if kind == WindowKind::Playlist {
            self.resize = None;
        }
    }

    /// Note where the equalizer or playlist sits relative to the main window, so that it
    /// reopens there.
    fn remember_offset(&mut self, kind: WindowKind) {
        if kind == WindowKind::Main || !self.docking || self.redock.contains(&kind) {
            return;
        }
        let main = self.pane(WindowKind::Main).and_then(Pane::placed_rect);
        let this = self.pane(kind).and_then(Pane::placed_rect);
        if let (Some(main), Some(r)) = (main, this) {
            self.offsets[kind_index(kind)] = Some((r.x - main.x, r.y - main.y));
        }
    }

    /// Re-apply docked positions to windows the window manager has had time to map.
    fn apply_pending_docks(&mut self, now: Instant) {
        let mut due: Vec<WindowKind> = self
            .redock
            .iter()
            .copied()
            .filter(|k| {
                self.pane(*k)
                    .is_some_and(|p| now >= p.opened_at + REDOCK_DELAY)
            })
            .collect();
        // The main window first: the others dock to where it is going, which the window
        // manager may not have reported yet.
        due.sort_by_key(|k| kind_index(*k));
        let mut main = self.pane(WindowKind::Main).and_then(Pane::rect);
        for kind in due {
            self.redock.retain(|k| *k != kind);
            let target = main.and_then(|m| self.docked_to(kind, m));
            if kind == WindowKind::Main
                && let (Some(at), Some(m)) = (target, main)
            {
                main = Some(Rect::new(at.x, at.y, m.w, m.h));
            }
            if let (Some(at), Some(pane)) = (target, self.pane(kind)) {
                log::debug!(
                    "redock {kind:?}: reported {:?}, docking at ({}, {})",
                    pane.rect(),
                    at.x,
                    at.y
                );
                if pane.rect().is_some_and(|r| (r.x, r.y) != (at.x, at.y)) {
                    pane.window.set_outer_position(at);
                    if !self.probe_done && self.probe.is_none() {
                        self.probe = Some((kind, at, now + PROBE_DELAY));
                    }
                }
            }
        }
    }

    /// Check whether the window manager obeyed our first re-dock. If it did not, recreate every
    /// window as override-redirect so that docking works regardless.
    fn check_probe(&mut self, el: &ActiveEventLoop, now: Instant) {
        let Some((kind, expected, when)) = self.probe else {
            return;
        };
        if now < when {
            return;
        }
        self.probe = None;
        self.probe_done = true;
        let moved = self
            .pane(kind)
            .and_then(Pane::rect)
            .is_some_and(|r| (r.x, r.y) == (expected.x, expected.y));
        if moved || self.unmanaged {
            return;
        }
        log::info!("the window manager ignores window moves; switching to unmanaged windows");
        self.unmanaged = true;
        self.recreate_panes(el);
    }

    /// Tear every window down and open it again (used when switching window styles). The
    /// window manager ignored where the windows were asked to go, so the main window goes back
    /// to its saved position if it has one, and the others to their saved offsets.
    fn recreate_panes(&mut self, el: &ActiveEventLoop) {
        let main_at = self.restored_main.or_else(|| {
            self.pane(WindowKind::Main)
                .and_then(Pane::rect)
                .map(|r| PhysicalPosition::new(r.x, r.y))
        });
        self.panes.clear();
        self.redock.clear();
        self.drag = None;
        self.resize = None;
        if let Err(e) = self.open_pane(el, WindowKind::Main, main_at) {
            log::error!("cannot reopen the main window: {e:#}");
            el.exit();
            return;
        }
        self.redock.clear();
        self.sync_panes(el);
    }

    /// Where a newly opened window goes. See [`App::docked_to`].
    fn docked_position(&self, kind: WindowKind) -> Option<PhysicalPosition<i32>> {
        let main = self.pane(WindowKind::Main)?.rect()?;
        self.docked_to(kind, main)
    }

    /// Where a newly opened window goes with the main window at `main`: the main window to its
    /// saved position, if it was opened at one; the equalizer and playlist back to where they
    /// last were relative to the main window. Without such a place, the equalizer docks under
    /// the main window and the playlist under the lowest of the two.
    fn docked_to(&self, kind: WindowKind, main: Rect) -> Option<PhysicalPosition<i32>> {
        if !self.docking {
            return None;
        }
        if kind == WindowKind::Main {
            return self.restored_main;
        }
        if let Some((dx, dy)) = self.offsets[kind_index(kind)] {
            return Some(PhysicalPosition::new(
                main.x.saturating_add(dx),
                main.y.saturating_add(dy),
            ));
        }
        let below = |r: Rect| PhysicalPosition::new(r.x, r.bottom());
        match kind {
            WindowKind::Main => None,
            WindowKind::Eq => Some(below(main)),
            WindowKind::Playlist => {
                // Hang under the equalizer when it is docked to the main window, or is about
                // to be (its move may not have been applied yet).
                let eq = self.pane(WindowKind::Eq).and_then(Pane::rect);
                let eq_docked = eq.filter(|eq| {
                    (eq.y == main.bottom() && eq.x == main.x)
                        || self.redock.contains(&WindowKind::Eq)
                });
                let anchor =
                    eq_docked.map_or(main, |eq| Rect::new(main.x, main.bottom(), eq.w, eq.h));
                Some(below(anchor))
            }
        }
    }

    /// Create, destroy and resize OS windows to match the UI state.
    fn sync_panes(&mut self, el: &ActiveEventLoop) {
        // Remember how the cluster hangs off the main window before anything resizes.
        let ui_scale = self.ui.scale();
        let main_scale = self
            .pane(WindowKind::Main)
            .map_or(self.applied_scale, |p| p.scale(ui_scale));
        // Bit-exact on purpose: this asks whether the value changed since it was stored, and
        // both sides come from the same computation.
        let scale_changed = main_scale != self.applied_scale;
        let cluster = if scale_changed {
            self.cluster_offsets()
        } else {
            Vec::new()
        };

        for kind in WindowKind::ALL {
            let open = self.ui.is_open(kind);
            let exists = self.pane(kind).is_some();
            if open && !exists {
                let at = self.docked_position(kind);
                if let Err(e) = self.open_pane(el, kind, at) {
                    log::error!("cannot open window: {e:#}");
                    self.ui.set_open(kind, false);
                }
            } else if !open && exists {
                self.close_pane(kind);
            }
        }

        for kind in WindowKind::ALL {
            let (w, h) = self.ui.window_size(kind);
            let Some(pane) = self.pane_mut(kind) else {
                continue;
            };
            if pane.canvas.w != w || pane.canvas.h != h {
                pane.canvas = Image::new(w, h);
                if let (Ok(bw), Ok(bh)) = (u32::try_from(w), u32::try_from(h)) {
                    match pane.pixels.resize_buffer(bw, bh) {
                        Ok(()) => pane.upscaler.rebind(&pane.pixels),
                        Err(e) => log::error!("resize_buffer: {e}"),
                    }
                }
            }
            let scale = pane.scale(ui_scale);
            let wanted = scaled_size((w, h), scale);
            if pane.window.inner_size() != wanted {
                let _ = pane.window.request_inner_size(wanted);
            }
            pane.window.request_redraw();
        }

        if scale_changed {
            log::info!("interface scale {main_scale}x");
            let from = self.applied_scale;
            self.applied_scale = main_scale;
            // Closed windows reopen where they were at the new scale. Open ones have their
            // offsets taken afresh when they close.
            for kind in [WindowKind::Eq, WindowKind::Playlist] {
                let i = kind_index(kind);
                if self.pane(kind).is_none()
                    && let Some((dx, dy)) = self.offsets[i]
                {
                    self.offsets[i] = Some((
                        rescale_offset(dx, from, main_scale),
                        rescale_offset(dy, from, main_scale),
                    ));
                }
            }
            if let Some(main) = self.pane(WindowKind::Main).and_then(Pane::rect) {
                for (kind, dx, dy) in cluster {
                    if let Some(p) = self.pane(kind) {
                        let x = main.x.saturating_add(rescale_offset(dx, from, main_scale));
                        let y = main.y.saturating_add(rescale_offset(dy, from, main_scale));
                        p.window.set_outer_position(PhysicalPosition::new(x, y));
                    }
                }
            }
        }
    }

    /// Windows attached to the main window and their offsets from its top-left corner.
    fn cluster_offsets(&self) -> Vec<(WindowKind, i32, i32)> {
        if !self.docking {
            return Vec::new();
        }
        let rects: Vec<(WindowKind, Rect)> = self
            .panes
            .iter()
            .filter_map(|p| Some((p.kind, p.rect()?)))
            .collect();
        let Some(root) = rects.iter().position(|(k, _)| *k == WindowKind::Main) else {
            return Vec::new();
        };
        let only: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        let main = only[root];
        dock::attached(root, &only)
            .into_iter()
            .map(|i| (rects[i].0, rects[i].1.x - main.x, rects[i].1.y - main.y))
            .collect()
    }

    /// Start moving `kind`: by hand when positions are known (so we can snap), otherwise
    /// handing the move to the compositor.
    fn begin_drag(&mut self, kind: WindowKind) {
        let Some(pane) = self.pane(kind) else { return };
        if !self.docking {
            if let Err(e) = pane.window.drag_window() {
                log::warn!("drag_window: {e}");
            }
            return;
        }
        let grab = pane.cursor;
        // The main window carries everything docked to it; the others leave on their own.
        let followers = if kind == WindowKind::Main {
            self.cluster_offsets()
        } else {
            Vec::new()
        };
        self.drag = Some(Drag {
            kind,
            grab,
            followers,
        });
    }

    /// Move the dragged window (and its followers) so the grab point stays under the pointer,
    /// snapping to the other windows and the screen edges.
    fn drag_to(&mut self, kind: WindowKind, pointer: PhysicalPosition<f64>) {
        let Some(drag) = self.drag.as_ref().filter(|d| d.kind == kind) else {
            return;
        };
        let Some(pane) = self.pane(kind) else { return };
        let Some(rect) = pane.rect() else { return };
        let screen = pane.screen();
        // Winamp snapped at 10 skin pixels, which is more screen pixels when zoomed.
        let snap = round_px(f64::from(SNAP_DISTANCE) * pane.scale(self.ui.scale()));
        // Pointer and grab offset in whole screen pixels.
        let pointer = (
            (f64::from(rect.x) + pointer.x).round() as i32,
            (f64::from(rect.y) + pointer.y).round() as i32,
        );
        let grab = (drag.grab.x.round() as i32, drag.grab.y.round() as i32);

        let followers: Vec<(WindowKind, i32, i32, i32, i32)> = drag
            .followers
            .iter()
            .filter_map(|&(fk, dx, dy)| {
                self.pane(fk)
                    .and_then(Pane::rect)
                    .map(|fr| (fk, dx, dy, fr.w, fr.h))
            })
            .collect();
        let offsets: Vec<(i32, i32, i32, i32)> = followers
            .iter()
            .map(|&(_, dx, dy, w, h)| (dx, dy, w, h))
            .collect();
        let statics: Vec<Rect> = self
            .panes
            .iter()
            .filter(|p| p.kind != kind && !followers.iter().any(|(fk, ..)| *fk == p.kind))
            .filter_map(Pane::rect)
            .collect();
        let (nx, ny) = dock::drag_target(rect, &offsets, pointer, grab, &statics, screen, snap);

        if (nx, ny) != (rect.x, rect.y) {
            pane.window
                .set_outer_position(PhysicalPosition::new(nx, ny));
            for (fk, dx, dy, _, _) in followers {
                if let Some(p) = self.pane(fk) {
                    p.window
                        .set_outer_position(PhysicalPosition::new(nx + dx, ny + dy));
                }
            }
        }
    }

    /// Start resizing the playlist from the grip under the pointer.
    fn begin_resize(&mut self) {
        let Some(pane) = self.pane(WindowKind::Playlist) else {
            return;
        };
        self.resize = Some(Resize {
            start: pane.cursor,
            size: self.ui.pl_size,
        });
    }

    /// Resize the playlist so its grip follows the pointer, a whole grid step at a time. Other
    /// windows stay where they are, even those docked to it, as in Webamp.
    ///
    /// This measures how far the pointer has travelled in screen pixels instead of mapping it
    /// onto the canvas: the pointer leaves the window as it grows, and the canvas changes size
    /// before the window manager has resized the window.
    fn resize_to(&mut self, pointer: PhysicalPosition<f64>, el: &ActiveEventLoop) {
        let Some(resize) = self.resize.as_ref() else {
            return;
        };
        let Some(pane) = self.pane(WindowKind::Playlist) else {
            return;
        };
        let scale = pane.scale(self.ui.scale());
        let travel = |now: f64, then: f64| round_px((now - then) / scale);
        let (w, h) = resize.size;
        let size = (
            w.saturating_add(travel(pointer.x, resize.start.x)),
            h.saturating_add(travel(pointer.y, resize.start.y)),
        );
        if self.ui.resize_playlist(size) {
            self.sync_panes(el);
        }
    }

    /// Show the diagonal resize cursor over the playlist's grip and while resizing, as Winamp
    /// did with its skins' `PSIZE.CUR`. `pos` is the pointer in canvas coordinates.
    fn update_cursor(&mut self, kind: WindowKind, pos: (i32, i32)) {
        let resizing = kind == WindowKind::Playlist && self.resize.is_some();
        // While another control or a window is being dragged, the cursor stays as it is.
        let idle = self.ui.pressed.is_none() && self.drag.is_none() && self.resize.is_none();
        let want = resizing || (idle && self.ui.hit(kind, pos.0, pos.1) == Some(Control::PlResize));
        let Some(pane) = self.pane_mut(kind) else {
            return;
        };
        if pane.resize_cursor != want {
            pane.resize_cursor = want;
            pane.window.set_cursor(if want {
                CursorIcon::NwseResize
            } else {
                CursorIcon::Default
            });
        }
    }

    fn handle(&mut self, actions: Vec<Action>, el: &ActiveEventLoop) {
        for a in actions {
            match a {
                Action::DragWindow(kind) => self.begin_drag(kind),
                Action::ResizePlaylist => self.begin_resize(),
                Action::Close => el.exit(),
                Action::Minimize => {
                    for p in &self.panes {
                        p.window.set_minimized(true);
                    }
                }
                Action::Relayout => self.sync_panes(el),
                Action::OpenFiles => self.show_dialog(Purpose::PlayFiles),
                Action::OpenFolder => self.show_dialog(Purpose::PlayFolder),
                Action::AlwaysOnTop => self.apply_window_level(),
                Action::OpenMenu(req) => self.open_menu(req, el),
                Action::App(command) => self.app_command(command),
                Action::Audio(t) => self.transport(t),
            }
        }
    }

    /// A mouse button in one of the NeoAmp windows.
    fn mouse_input(
        &mut self,
        kind: WindowKind,
        state: ElementState,
        button: MouseButton,
        now: Instant,
        el: &ActiveEventLoop,
    ) {
        // A click outside an open menu only closes it, as in Windows.
        if state == ElementState::Pressed && self.popups.is_open() {
            self.popups.close();
            self.swallow_release = true;
            return;
        }
        if state == ElementState::Released && std::mem::take(&mut self.swallow_release) {
            return;
        }
        let Some(p) = self.pane(kind) else { return };
        if self.unmanaged && state == ElementState::Pressed {
            // Nobody manages focus for override-redirect windows: claim it ourselves.
            p.window.focus_window();
        }
        let (x, y) = p.canvas_pos();
        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                let actions = self.ui.mouse_down(kind, x, y, now);
                self.handle(actions, el);
            }
            (MouseButton::Left, ElementState::Released) => {
                self.drag = None;
                self.resize = None;
                let actions = self.ui.mouse_up(kind, x, y);
                self.handle(actions, el);
            }
            // Context menus open when the right button is let go, as in Windows.
            (MouseButton::Right, ElementState::Released) => {
                let actions = self.ui.context_menu(kind, x, y);
                self.handle(actions, el);
            }
            _ => {}
        }
        self.update_cursor(kind, (x, y));
    }

    fn key_cmd(&self, key: &Key) -> Option<KeyCmd> {
        let ctrl = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();
        let shift = self.modifiers.shift_key();
        Some(match key {
            Key::Character(s) => match (s.to_ascii_lowercase().as_str(), ctrl, alt) {
                ("a", true, _) => KeyCmd::ToggleAlwaysOnTop,
                ("d", true, _) => KeyCmd::ToggleDoubleSize,
                ("f", _, true) => KeyCmd::MainMenu,
                ("s", _, true) => KeyCmd::SkinsMenu,
                ("l", false, false) if shift => KeyCmd::OpenFolder,
                ("=" | "+", true, _) => KeyCmd::ZoomIn,
                ("-" | "_", true, _) => KeyCmd::ZoomOut,
                ("0", true, _) => KeyCmd::ZoomReset,
                ("t", true, _) => KeyCmd::ToggleTimeMode,
                ("g", _, true) => KeyCmd::ToggleEq,
                ("e", _, true) => KeyCmd::TogglePl,
                ("z", false, false) => KeyCmd::Prev,
                ("x", false, false) => KeyCmd::Play,
                ("c", false, false) => KeyCmd::Pause,
                ("v", false, false) => KeyCmd::Stop,
                ("b", false, false) => KeyCmd::Next,
                ("l", false, false) => KeyCmd::OpenFiles,
                ("r", false, false) => KeyCmd::ToggleRepeat,
                ("s", false, false) => KeyCmd::ToggleShuffle,
                _ => return None,
            },
            Key::Named(NamedKey::ArrowUp) => KeyCmd::VolumeUp,
            Key::Named(NamedKey::ArrowDown) => KeyCmd::VolumeDown,
            Key::Named(NamedKey::ArrowLeft) => KeyCmd::SeekBack,
            Key::Named(NamedKey::ArrowRight) => KeyCmd::SeekForward,
            _ => return None,
        })
    }

    fn render(&mut self, kind: WindowKind, now: Instant) {
        let skin = self.custom.as_ref().unwrap_or(&self.base);
        let ui = &self.ui;
        let font = self.font.as_ref();
        let Some(pane) = self.panes.iter_mut().find(|p| p.kind == kind) else {
            return;
        };
        let (w, h) = ui.window_size(kind);
        if pane.canvas.w != w || pane.canvas.h != h {
            pane.canvas = Image::new(w, h);
        }
        ui.draw_window(kind, &mut pane.canvas, skin, font, now);
        let frame = pane.pixels.frame_mut();
        if frame.len() == pane.canvas.data.len() {
            frame.copy_from_slice(&pane.canvas.data);
        }
        let started = Instant::now();
        if let Err(e) = pane.upscaler.render(&pane.pixels) {
            log::error!("render: {e}");
        }
        let took = started.elapsed();
        self.stats.frames[kind_index(kind)] += 1;
        self.stats.slowest_present = self.stats.slowest_present.max(took);
    }

    fn log_stats(&mut self, now: Instant) {
        if now.saturating_duration_since(self.stats.since) < STATS_PERIOD {
            return;
        }
        let s = &self.stats;
        log::debug!(
            "frames/s main={} eq={} pl={} wakeups={} slowest_present={:.1}ms",
            s.frames[0],
            s.frames[1],
            s.frames[2],
            s.wakeups,
            s.slowest_present.as_secs_f64() * 1000.0
        );
        self.stats = FrameStats::new(now);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if !self.panes.is_empty() {
            return;
        }
        let origin = el
            .primary_monitor()
            .or_else(|| el.available_monitors().next())
            .map(|m| m.position())
            .unwrap_or(PhysicalPosition::new(0, 0));
        let screens: Vec<Rect> = el
            .available_monitors()
            .filter_map(|m| monitor_rect(&m))
            .collect();
        self.choose_zoom(el);
        self.restored_main = self.saved_main_position(&screens);
        let at = self.restored_main.unwrap_or_else(|| {
            PhysicalPosition::new(origin.x + INITIAL_OFFSET.0, origin.y + INITIAL_OFFSET.1)
        });
        if let Err(e) = self.open_pane(el, WindowKind::Main, Some(at)) {
            log::error!("cannot open a window: {e:#}");
            el.exit();
            return;
        }
        // A new main window goes where the window manager puts it, and only the others
        // re-dock; a restored one is put back once mapped, in case the manager moved it.
        if self.restored_main.is_none() {
            self.redock.clear();
        }
        self.docking = self.pane(WindowKind::Main).and_then(Pane::rect).is_some();
        if !self.docking {
            log::info!("this backend cannot position windows; docking is disabled");
        }
        self.applied_scale = self
            .pane(WindowKind::Main)
            .map_or(1.0, |p| p.scale(self.ui.scale()));
        self.restore_offsets(at, &screens);
        self.sync_panes(el);
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.save();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let now = Instant::now();
        if self.popups.owns(id) {
            match &event {
                WindowEvent::KeyboardInput { event, .. } => self.menu_key(event, el),
                WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
                _ => {
                    if let Some(command) = self.popups.window_event(el, id, &event, now) {
                        self.run_command(command, el);
                    }
                    if matches!(event, WindowEvent::Focused(_)) {
                        self.focus_changed(now);
                    }
                }
            }
            return;
        }
        let Some(kind) = self.kind_of(id) else { return };
        match event {
            WindowEvent::CloseRequested => {
                if kind == WindowKind::Main {
                    el.exit();
                } else {
                    self.ui.set_open(kind, false);
                    self.sync_panes(el);
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(p) = self.pane_mut(kind)
                    && let Err(e) = p
                        .pixels
                        .resize_surface(size.width.max(1), size.height.max(1))
                {
                    log::error!("resize_surface: {e}");
                }
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor,
                mut inner_size_writer,
            } => {
                // Moved to a monitor with a different desktop scale. Answer with our own size,
                // or winit applies its rounding of the old logical size after this returns.
                let wanted = scaled_size(self.ui.window_size(kind), self.ui.scale() * scale_factor);
                if let Err(e) = inner_size_writer.request_inner_size(wanted) {
                    log::warn!("keeping winit's size after a scale change: {e}");
                }
                self.sync_panes(el);
            }
            WindowEvent::Focused(f) => {
                if let Some(p) = self.pane_mut(kind) {
                    p.focused = f;
                }
                self.focus_changed(now);
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let Some(p) = self.pane_mut(kind) else { return };
                p.cursor = position;
                let (x, y) = p.canvas_pos();
                if self.drag.as_ref().is_some_and(|d| d.kind == kind) {
                    self.drag_to(kind, position);
                } else if kind == WindowKind::Playlist && self.resize.is_some() {
                    self.resize_to(position, el);
                } else {
                    let actions = self.ui.mouse_move(kind, x, y);
                    self.handle(actions, el);
                }
                self.update_cursor(kind, (x, y));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_input(kind, state, button, now, el);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.popups.is_open() {
                    self.menu_key(&event, el);
                } else if event.state == ElementState::Pressed
                    && let Some(cmd) = self.key_cmd(&event.logical_key)
                {
                    let actions = self.ui.key(cmd);
                    self.handle(actions, el);
                }
            }
            WindowEvent::DroppedFile(path) => self.drop_file(path, el),
            WindowEvent::RedrawRequested => self.render(kind, now),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        self.stats.wakeups += 1;
        self.log_stats(now);
        if now >= self.next_frame {
            self.next_frame = now + FRAME;
            self.apply_pending_docks(now);
            self.check_probe(el, now);
            self.finish_dialog(el);
            self.popups.tick(el, now);
            self.check_menu_focus(now);
            self.sync_audio(el);
            self.ui.tick(now);
            for p in &self.panes {
                p.window.request_redraw();
            }
        }
        if now >= self.next_save {
            self.next_save = now + SAVE_CHECK;
            self.save();
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_are_sized_to_the_nearest_screen_pixel() {
        assert_eq!(scaled_size((275, 116), 1.0), PhysicalSize::new(275, 116));
        assert_eq!(scaled_size((275, 116), 1.5), PhysicalSize::new(413, 174));
        assert_eq!(scaled_size((275, 116), 3.0), PhysicalSize::new(825, 348));
        assert_eq!(scaled_size((0, 0), 2.0), PhysicalSize::new(1, 1));
    }

    #[test]
    fn pixel_rounding_stays_in_range() {
        assert_eq!(round_px(412.5), 413);
        assert_eq!(round_px(-0.4), 0);
        assert_eq!(round_px(f64::NAN), 0);
        assert_eq!(round_px(1e12), i32::MAX);
        assert_eq!(scaled_size((275, 116), f64::NAN), PhysicalSize::new(1, 1));
    }

    #[test]
    fn windows_off_every_screen_are_not_restored_there() {
        let screens = [Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 2560, 1440)];
        assert!(title_bar_on_screen((64, 64), 275, 1.0, &screens));
        assert!(
            title_bar_on_screen((4000, 1300), 275, 1.0, &screens),
            "second screen"
        );
        // Hanging off the left edge is fine while the title bar's middle is still on screen.
        assert!(title_bar_on_screen((-100, 0), 275, 1.0, &screens));
        assert!(!title_bar_on_screen((-200, 0), 275, 1.0, &screens));
        // Below the first screen but beside the taller second one.
        assert!(!title_bar_on_screen((100, 1200), 275, 1.0, &screens));
        assert!(
            !title_bar_on_screen((0, 0), 275, 1.0, &[]),
            "no screens known"
        );
        // The point checked moves with the scale.
        assert!(!title_bar_on_screen((-200, 0), 275, 1.0, &screens));
        assert!(title_bar_on_screen((-200, 0), 275, 2.0, &screens));
    }

    #[test]
    fn docked_offsets_follow_the_rounded_window_sizes() {
        // A window docked right of a 275-pixel main window, at 1.5x and then 2x.
        let at_1_5 = rescale_offset(275, 1.0, 1.5);
        assert_eq!(
            u32::try_from(at_1_5),
            Ok(scaled_size((275, 116), 1.5).width)
        );
        assert_eq!(rescale_offset(at_1_5, 1.5, 2.0), 550);
        assert_eq!(rescale_offset(550, 2.0, 1.5), 413);
        // Under the main window and the equalizer.
        assert_eq!(rescale_offset(232, 1.0, 2.5), 580);
    }
}
