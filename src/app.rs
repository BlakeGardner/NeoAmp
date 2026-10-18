//! The desktop application: one borderless OS window per NeoAmp window (main, equalizer,
//! playlist), each presented through `pixels`, with Winamp-style magnetic docking.
//!
//! Docking needs to read and set window positions. X11 (including XWayland) allows that, so
//! the X11 backend is preferred whenever `DISPLAY` is set; pure Wayland forbids it, in which
//! case title-bar drags are handed to the compositor and no snapping happens.
//!
//! Windows are sized in physical pixels: skin size times the UI scale times the desktop's
//! scale factor, rounded. Each is drawn at that scale rounded up (see [`Density::covering`]),
//! and `upscale::Upscaler` stretches the canvas over the whole window: exact at whole-number
//! scales, shrunk by averaging in between.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use pixels::wgpu;
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement as _;
use winit::window::{Cursor, CursorIcon, CustomCursor, Window, WindowId, WindowLevel};

use crate::audio::{self, Location, OutputDevice, Player, Scanner};
use crate::canvas::{Canvas, Density, Rect};
use crate::cursor::SkinCursor;
use crate::dock::{self, SNAP_DISTANCE};
use crate::eqf::{self, AutoLoad};
use crate::file_dialog::{FileDialog, Purpose};
use crate::font::TtfFont;
use crate::info::Outcome as InfoOutcome;
use crate::info_window::InfoWindow;
use crate::instance::{self, Claim};
use crate::jump::Outcome as JumpOutcome;
use crate::jump_window::JumpWindow;
use crate::library::{self, Entry};
use crate::menu::{self, Anchor};
use crate::menus::{self, AppCommand, Command, Library};
use crate::now_playing::{self, NowPlaying};
use crate::playlist::SortKey;
use crate::playlist_file;
use crate::popup::Popups;
use crate::settings::{Saver, Settings, Store, WindowLayout};
use crate::shortcut::Shortcut;
use crate::skin::Skin;
use crate::sprites::pl::TRACK_H;
use crate::ui::{
    Action, CursorMove, InfoOf, KeyCmd, ListCmd, MenuRequest, Mods, OutputChoice, PlayState, Track,
    Transport, Ui, WindowKind,
};
use crate::upscale::{Upscaler, window_to_canvas};
use crate::url_dialog::{Outcome as UrlOutcome, Purpose as UrlPurpose};
use crate::url_window::UrlWindow;
use crate::vis;
use crate::window_icon;
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
    /// Audio files, folders, playlist files and streams. They replace the saved playlist, and
    /// the first one plays.
    pub files: Vec<Location>,
    /// Add `files` to the end of the playlist instead, without playing them.
    pub enqueue: bool,
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

/// The canvas of a window `(w, h)` skin pixels in size; the GPU stretches it over the window.
/// Windows are drawn at [`Density::covering`] their scale: as many image pixels per skin
/// pixel as the screen shows, rounded up, so the playlist text is sharp and the window shapes
/// smooth, while the skin's bitmaps are enlarged pixel for pixel.
fn window_canvas((w, h): (i32, i32), density: Density) -> Canvas {
    Canvas::new(w, h, density)
}

/// The size of `canvas`'s pixels, as the GPU buffer that shows them is sized.
fn buffer_size(canvas: &Canvas) -> Result<(u32, u32)> {
    let image = canvas.image();
    Ok((u32::try_from(image.w)?, u32::try_from(image.h)?))
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

/// Whether the modifier of Winamp's Ctrl shortcuts is held. On macOS that is Cmd, as in Mac
/// apps; the menus write it as ⌘ there (see [`crate::shortcut`]).
pub(crate) fn ctrl_held(m: ModifiersState) -> bool {
    if cfg!(target_os = "macos") {
        m.super_key()
    } else {
        m.control_key()
    }
}

/// The key a shortcut is matched against. On macOS, Option types other characters (Option+G
/// is "©" and Option+E starts an accent), so while it is held the key is read without it.
pub(crate) fn shortcut_key(event: &KeyEvent, m: ModifiersState) -> Key {
    if cfg!(target_os = "macos") && m.alt_key() {
        event.key_without_modifiers()
    } else {
        event.logical_key.clone()
    }
}

/// A clipboard shortcut in a dialog's edit box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EditShortcut {
    /// Ctrl+V (Cmd+V on macOS).
    Paste,
    /// Ctrl+A (Cmd+A on macOS).
    SelectAll,
}

/// The edit box shortcut `event` is, if it is one.
pub(crate) fn edit_shortcut(event: &KeyEvent, m: ModifiersState) -> Option<EditShortcut> {
    if !ctrl_held(m) || m.alt_key() {
        return None;
    }
    match &shortcut_key(event, m) {
        Key::Character(s) if s.eq_ignore_ascii_case("v") => Some(EditShortcut::Paste),
        Key::Character(s) if s.eq_ignore_ascii_case("a") => Some(EditShortcut::SelectAll),
        _ => None,
    }
}

/// Whether `event` is for the 3 key of the row of digits (Alt+3 is file info). It goes by
/// position, like the playlist's sort keys, so it works on layouts where 3 needs Shift.
fn is_digit_3(event: &KeyEvent) -> bool {
    event.physical_key == PhysicalKey::Code(KeyCode::Digit3)
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
    // Paths are remembered, and may be handed to a NeoAmp running elsewhere, so they must not
    // depend on the folder this one was started in.
    let skin_path = opts.skin.map(absolute);
    let files: Vec<Location> = opts
        .files
        .into_iter()
        .map(|item| match item {
            Location::File(path) => Location::File(absolute(path)),
            url @ Location::Url(_) => url,
        })
        .collect();
    let store = Store::platform();
    let instance = match (&opts.screenshot, &store) {
        (None, Some(store)) => {
            let request = instance::Request {
                files: files.clone(),
                skin: skin_path.clone(),
                enqueue: opts.enqueue,
            };
            match instance::claim(store.dir(), &request) {
                Claim::Forwarded => {
                    log::info!("handed over to the NeoAmp already running");
                    return Ok(());
                }
                Claim::First(server) => Some(server),
                Claim::Alone => None,
            }
        }
        _ => None,
    };

    let base = Skin::from_wsz(BASE_SKIN, "base-2.91").context("embedded base skin")?;
    let custom = skin_path
        .as_deref()
        .map(|p| load_skin(p, &base))
        .transpose()?;

    if let Some(out) = &opts.screenshot {
        let skin = custom.as_ref().unwrap_or(&base);
        let font = TtfFont::load_playlist(&skin.pledit.font, TtfFont::DEFAULT_PX);
        return render_screenshot(Ui::demo(), skin, Some(&font), out);
    }

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
    let font_family = custom.as_ref().unwrap_or(&base).pledit.font.clone();
    let font = TtfFont::load_playlist(&font_family, TtfFont::DEFAULT_PX);

    let mut ui = Ui::new();
    settings.apply(&mut ui);
    let player = match Player::new(ui.output.as_ref().map(|o| o.id.clone())) {
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
        font_family,
        pointers: HashMap::new(),
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
        jump: None,
        info: None,
        url: None,
        menu_skins: Vec::new(),
        menu_presets: Vec::new(),
        menu_devices: Vec::new(),
        now_playing: None,
        instance,
        focused: false,
        focus_lost: None,
        title: crate::ui::APP_NAME.to_owned(),
        title_scratch: String::new(),
        swallow_release: false,
        wheel_rest: (WindowKind::Main, 0.0),
    };
    let saved_tracks = if files.is_empty() || opts.enqueue {
        let tracks = store.as_ref().and_then(Store::load_playlist);
        if let Some(tracks) = &tracks {
            app.restore_playlist(tracks, settings.playlist.current);
        }
        let _ = app.add_tracks(playlist_file::expand(files), false, &mut Vec::new());
        tracks
    } else {
        let mut actions = Vec::new();
        app.add_tracks(playlist_file::expand(files), true, &mut actions);
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
    let mut canvas = window_canvas(ui.layout().size, Density::ONE);
    ui.vis.set_sample_rate(RATE);
    let mut signal = vec![0.0; usize::try_from(u128::from(RATE) * FRAME.as_millis() / 1000)?];
    for k in 0..70 {
        vis::demo_signal(f64::from(k) * FRAME.as_secs_f64(), RATE, &mut signal);
        ui.vis.push_samples(&signal);
        ui.draw_stacked(&mut canvas, skin, font, start + FRAME * k);
    }
    let (pw, ph) = buffer_size(&canvas)?;
    image::save_buffer(out, &canvas.image().data, pw, ph, image::ColorType::Rgba8)
        .with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} ({pw}x{ph})", out.display());
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

/// A mouse pointer a window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pointer {
    /// One of the skin's.
    Skin(SkinCursor),
    /// The system's, where the skin has none.
    System(CursorIcon),
}

/// The system's pointer for where the skin has none of its own: a resize arrow over the
/// playlist's grip (only sideways in its windowshade bar, whose grip only sets the width), and
/// the arrow everywhere else.
fn system_pointer(which: SkinCursor) -> CursorIcon {
    match which {
        SkinCursor::PSize => CursorIcon::NwseResize,
        SkinCursor::PWsSize => CursorIcon::EwResize,
        _ => CursorIcon::Default,
    }
}

/// How many times larger to make the skin's pointers on a desktop scaled by `scale_factor`, so
/// that they keep their size next to the system's: whole steps, since they are pixel art. macOS
/// scales them itself.
fn pointer_scale(scale_factor: f64) -> u16 {
    if cfg!(target_os = "macos") {
        1
    } else {
        // A small whole number, so the cast cannot truncate.
        scale_factor.round().clamp(1.0, 8.0) as u16
    }
}

/// One OS window hosting one NeoAmp window.
struct Pane {
    kind: WindowKind,
    upscaler: Upscaler,
    /// Declared before `window` so the GPU surface is dropped first.
    pixels: Pixels<'static>,
    window: Arc<Window>,
    canvas: Canvas,
    cursor: PhysicalPosition<f64>,
    /// The pointer the window shows; `None` until one is set, or once the skin changes.
    pointer: Option<Pointer>,
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

    /// Give the window a canvas `size` skin pixels big at `density`, and a GPU buffer to match,
    /// unless it already has them.
    fn fit_canvas(&mut self, size: (i32, i32), density: Density) {
        if self.canvas.size() == size && self.canvas.density() == density {
            return;
        }
        self.canvas = window_canvas(size, density);
        let resized =
            buffer_size(&self.canvas).and_then(|(w, h)| Ok(self.pixels.resize_buffer(w, h)?));
        match resized {
            Ok(()) => self.upscaler.rebind(&self.pixels),
            Err(e) => log::error!("resize_buffer: {e:#}"),
        }
    }

    /// The last pointer position in canvas (skin pixel) coordinates.
    fn canvas_pos(&self) -> (i32, i32) {
        let size = self.window.inner_size();
        window_to_canvas(
            (self.cursor.x, self.cursor.y),
            (size.width, size.height),
            self.canvas.size(),
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
    /// The playlist's font: the built-in one for the family the skin names.
    font: TtfFont,
    /// The family the skin named when `font` was loaded.
    font_family: String,
    /// The skin's pointers made ready for the windows, at each scale a window has needed.
    pointers: HashMap<(SkinCursor, u16), CustomCursor>,
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
    /// The Jump to file dialog, while it is open.
    jump: Option<JumpWindow>,
    /// The File info dialog, while it is open.
    info: Option<InfoWindow>,
    /// The Open URL (or Add URL) dialog, while it is open.
    url: Option<UrlWindow>,
    /// The skins, preset files and output devices the open menu lists; its commands number
    /// them.
    menu_skins: Vec<Entry>,
    menu_presets: Vec<Entry>,
    menu_devices: Vec<OutputDevice>,
    /// The system's media keys and "now playing" display, once the main window is open.
    now_playing: Option<NowPlaying>,
    /// Where NeoAmp started again hands over what it was asked to open.
    instance: Option<instance::Server>,
    /// One of our windows, menus included, has the keyboard focus.
    focused: bool,
    /// When the last of our windows lost the focus.
    focus_lost: Option<Instant>,
    /// The main window's title as last set: the current track, as in Winamp's taskbar entry.
    title: String,
    /// Where the title is written each frame, to be compared with `title` without allocating.
    title_scratch: String,
    /// A click that closed a menu went down; its release does nothing either.
    swallow_release: bool,
    /// Mouse wheel travel over a window not yet turned into whole steps.
    wheel_rest: (WindowKind, f64),
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
                self.skin_changed();
            }
            Err(e) => log::error!("cannot load skin {}: {e:#}", path.display()),
        }
    }

    fn skin(&self) -> &Skin {
        self.custom.as_ref().unwrap_or(&self.base)
    }

    /// Another skin is showing: forget the old one's pointers, and load the playlist font the
    /// new one names, unless that is the one loaded already.
    fn skin_changed(&mut self) {
        self.pointers.clear();
        for p in &mut self.panes {
            p.pointer = None;
        }
        let family = &self.skin().pledit.font;
        if family.to_lowercase() == self.font_family.to_lowercase() {
            return;
        }
        let family = family.clone();
        self.font = TtfFont::load_playlist(&family, TtfFont::DEFAULT_PX);
        self.font_family = family;
    }

    /// The current file's name, which auto-load presets go by.
    fn track_file_name(&self) -> Option<String> {
        let name = self.ui.track()?.location.path()?.file_name()?;
        Some(name.to_string_lossy().into_owned())
    }

    /// With AUTO on, load playlist entry `i`'s auto-load preset, if it has one.
    fn load_auto_preset(&mut self, i: usize) {
        if !self.ui.eq_auto {
            return;
        }
        let Some(name) = self
            .ui
            .tracks
            .get(i)
            .and_then(|t| t.location.path()?.file_name())
        else {
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
    fn app_command(&mut self, command: AppCommand, el: &ActiveEventLoop) {
        match command {
            AppCommand::BaseSkin => {
                self.custom = None;
                self.skin_path = None;
                self.skin_changed();
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
            AppCommand::AddFiles => self.show_dialog(Purpose::AddFiles),
            AppCommand::AddFolder => self.show_dialog(Purpose::AddFolder),
            AppCommand::PlayUrl => self.open_url(UrlPurpose::Play, el),
            AppCommand::AddUrl => self.open_url(UrlPurpose::Add, el),
            AppCommand::LoadList => self.show_dialog(Purpose::LoadList),
            AppCommand::SaveList => self.show_dialog(Purpose::SaveList),
            AppCommand::JumpToFile => self.open_jump(el),
            AppCommand::FileInfo(of) => self.open_info(of, el),
            AppCommand::DefaultOutput => self.choose_output(None),
            AppCommand::Output(i) => {
                if let Some(device) = self.menu_devices.get(i) {
                    self.choose_output(Some(OutputChoice {
                        id: device.id.clone(),
                        name: device.name.clone(),
                    }));
                }
            }
            AppCommand::KeepOutput => {}
        }
    }

    /// Play through `choice`, or the system's default device, from now on.
    fn choose_output(&mut self, choice: Option<OutputChoice>) {
        if let Some(player) = self.player.as_mut() {
            player.set_device(choice.as_ref().map(|c| c.id.clone()));
            self.ui.vis.set_sample_rate(player.sample_rate());
        }
        self.ui.output = choice;
    }

    /// Show the File info dialog about the track `of` names, or that track in the dialog if it
    /// is open already. With an empty playlist there is nothing to show.
    fn open_info(&mut self, of: InfoOf, el: &ActiveEventLoop) {
        let Some(location) = self.ui.info_track(of).map(|t| t.location.clone()) else {
            return;
        };
        let stream = self.ui.stream_details(&location);
        if let Some(info) = self.info.as_mut() {
            info.show(location, stream);
            return;
        }
        let scale_factor = self
            .pane(WindowKind::Main)
            .map_or(1.0, |p| p.window.scale_factor());
        // Like the Jump to file dialog, it follows the zoom but not double size.
        let opened = InfoWindow::open(
            el,
            self.ui.zoom.factor(),
            scale_factor,
            self.ui.always_on_top,
            location,
            stream,
        );
        match opened {
            Ok(info) => self.info = Some(info),
            Err(e) => log::error!("cannot show the File info dialog: {e:#}"),
        }
    }

    /// An event for the File info dialog's window.
    fn info_event(&mut self, event: WindowEvent, now: Instant) {
        let Some(info) = self.info.as_mut() else {
            return;
        };
        let focus_event = matches!(event, WindowEvent::Focused(_));
        if info.window_event(event) == InfoOutcome::Close {
            // A closed window reports no more focus changes, so the focus it had is lost now.
            self.info = None;
        }
        if self.info.is_none() || focus_event {
            self.focus_changed(now);
        }
    }

    /// Show the Open URL dialog (or Add URL, for `purpose`), or bring it to the front if it is
    /// open for that already.
    fn open_url(&mut self, purpose: UrlPurpose, el: &ActiveEventLoop) {
        if let Some(url) = &self.url
            && url.purpose() == purpose
        {
            url.raise();
            return;
        }
        let scale_factor = self
            .pane(WindowKind::Main)
            .map_or(1.0, |p| p.window.scale_factor());
        // Like the other dialogs, it follows the zoom but not double size.
        let opened = UrlWindow::open(
            el,
            purpose,
            self.ui.zoom.factor(),
            scale_factor,
            self.ui.always_on_top,
            Instant::now(),
        );
        match opened {
            Ok(url) => self.url = Some(url),
            Err(e) => log::error!("cannot show the {} dialog: {e:#}", purpose.title()),
        }
    }

    /// An event for the Open URL dialog's window.
    fn url_event(&mut self, event: WindowEvent, now: Instant, el: &ActiveEventLoop) {
        let Some(url) = self.url.as_mut() else {
            return;
        };
        let focus_event = matches!(event, WindowEvent::Focused(_));
        let purpose = url.purpose();
        let outcome = url.window_event(event, now);
        if matches!(outcome, UrlOutcome::Open(_) | UrlOutcome::Close) {
            // A closed window reports no more focus changes, so the focus it had is lost now.
            self.url = None;
        }
        if let UrlOutcome::Open(location) = outcome {
            self.open_location(location, purpose, el);
        }
        if self.url.is_none() || focus_event {
            self.focus_changed(now);
        }
    }

    /// Play what the Open URL dialog was given, in place of the playlist, as Winamp's did; or
    /// for Add URL, add it to the end. A file there, a folder or a playlist file works too.
    fn open_location(&mut self, location: Location, purpose: UrlPurpose, el: &ActiveEventLoop) {
        log::info!("opening {location}");
        let tracks = playlist_file::expand([location]);
        if tracks.is_empty() {
            log::info!("nothing playable there");
            return;
        }
        let mut actions = Vec::new();
        let play = purpose == UrlPurpose::Play;
        if play {
            self.ui.clear_playlist(&mut actions);
        }
        let _ = self.add_tracks(tracks, play, &mut actions);
        self.handle(actions, el);
    }

    /// Show the Jump to file dialog, or bring it to the front if it is open.
    fn open_jump(&mut self, el: &ActiveEventLoop) {
        if let Some(jump) = &self.jump {
            jump.raise();
            return;
        }
        let scale_factor = self
            .pane(WindowKind::Main)
            .map_or(1.0, |p| p.window.scale_factor());
        // Like the menus, the dialog follows the zoom but not double size.
        let opened = JumpWindow::open(
            el,
            self.ui.zoom.factor(),
            scale_factor,
            self.ui.always_on_top,
            self.ui.current_row(),
            Instant::now(),
        );
        match opened {
            Ok(jump) => self.jump = Some(jump),
            Err(e) => log::error!("cannot show the Jump to file dialog: {e:#}"),
        }
    }

    /// An event for the Jump to file dialog's window.
    fn jump_event(&mut self, event: WindowEvent, now: Instant, el: &ActiveEventLoop) {
        let Some(jump) = self.jump.as_mut() else {
            return;
        };
        let focus_event = matches!(event, WindowEvent::Focused(_));
        let outcome = jump.window_event(event, &self.ui.tracks, now);
        if matches!(outcome, JumpOutcome::Jump(_) | JumpOutcome::Close) {
            // A closed window reports no more focus changes, so the focus it had is lost now.
            self.jump = None;
        }
        if let JumpOutcome::Jump(i) = outcome {
            let mut actions = Vec::new();
            self.ui.play_entry(i, &mut actions);
            self.handle(actions, el);
        }
        if self.jump.is_none() || focus_event {
            self.focus_changed(now);
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
        if menus::lists_devices(req.kind) {
            self.menu_devices = audio::output_devices();
        }
        let lib = Library {
            skins: &self.menu_skins,
            skin: self.skin_path.as_deref(),
            presets: &self.menu_presets,
            has_auto_preset,
            devices: &self.menu_devices,
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

    /// Whether any of our windows, menus and dialogs included, has the keyboard focus.
    fn has_focus(&self) -> bool {
        self.panes.iter().any(|p| p.focused)
            || self.popups.has_focus()
            || self.jump.as_ref().is_some_and(JumpWindow::has_focus)
            || self.info.as_ref().is_some_and(InfoWindow::has_focus)
            || self.url.as_ref().is_some_and(UrlWindow::has_focus)
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

    /// Append entries to the playlist and have the tags read of those whose length is unknown
    /// (entries from a playlist file may know theirs). With `play`, the first one starts if
    /// nothing is playing. Returns the indices of the new entries.
    fn add_tracks(
        &mut self,
        tracks: Vec<Track>,
        play: bool,
        actions: &mut Vec<Action>,
    ) -> std::ops::Range<usize> {
        let added = self.ui.add_tracks(tracks);
        for (i, track) in self.ui.tracks.iter().enumerate().skip(added.start) {
            // Streams are not scanned: only playing one tells what it is.
            if let (None, Some(path)) = (track.seconds, track.location.path()) {
                self.scanner.scan(i, path.to_path_buf());
            }
        }
        if play && self.ui.state == PlayState::Stopped && !added.is_empty() {
            self.ui.play_track(added.start, actions);
        }
        added
    }

    /// Put the saved playlist back, stopped on entry `current`. Only entries whose length was
    /// not known when it was saved have their tags read again.
    fn restore_playlist(&mut self, tracks: &[Track], current: usize) {
        let _ = self.add_tracks(tracks.to_vec(), false, &mut Vec::new());
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
        // Positions from the window layer; which windows are open, shaded and how big, from the
        // interface.
        let mut windows = self.layout.clone();
        windows.main_shade = settings.windows.main_shade;
        for kind in [WindowKind::Eq, WindowKind::Playlist] {
            if let (Some(to), Some(from)) =
                (windows.window_mut(kind), settings.windows.window(kind))
            {
                to.open = from.open;
                to.shade = from.shade;
                to.size = from.size;
            }
        }
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

    /// A file dropped on any window: a skin is applied; audio files, folders and playlists
    /// join the playlist, starting to play if nothing was.
    fn drop_file(&mut self, path: PathBuf, el: &ActiveEventLoop) {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wsz"))
        {
            self.load_skin(&path);
            return;
        }
        let mut actions = Vec::new();
        let tracks = playlist_file::expand([Location::File(path)]);
        let _ = self.add_tracks(tracks, true, &mut actions);
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
            Purpose::PlayFiles
            | Purpose::PlayFolder
            | Purpose::AddFiles
            | Purpose::AddFolder
            | Purpose::LoadList
            | Purpose::SaveList => None,
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

    /// Once a file dialog closes, act on what was chosen. Files, a folder or a playlist file to
    /// play replace the playlist and the first entry plays, as in Winamp; those to add join the
    /// end of it. Cancelling changes nothing.
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
            Purpose::PlayFiles
            | Purpose::PlayFolder
            | Purpose::LoadList
            | Purpose::AddFiles
            | Purpose::AddFolder => {
                let tracks = playlist_file::expand(paths.into_iter().map(Location::File));
                if tracks.is_empty() {
                    log::info!("nothing playable there");
                    return;
                }
                let mut actions = Vec::new();
                let replace = matches!(
                    purpose,
                    Purpose::PlayFiles | Purpose::PlayFolder | Purpose::LoadList
                );
                if replace {
                    self.ui.clear_playlist(&mut actions);
                }
                let _ = self.add_tracks(tracks, replace, &mut actions);
                self.handle(actions, el);
            }
            Purpose::SaveList => match playlist_file::save(first, &self.ui.tracks) {
                Ok(path) => log::info!("saved the playlist to {}", path.display()),
                Err(e) => log::error!("cannot save the playlist: {e:#}"),
            },
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
                    log::info!("playing {}", track.location);
                    player.open(&track.location);
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

    /// Once per frame: take in scan results and engine events, move a fade-out on, hand the
    /// engine the current settings, and read back the position, bitrate and visualiser
    /// samples.
    fn sync_audio(&mut self, el: &ActiveEventLoop, now: Instant) {
        while let Some((i, path, info)) = self.scanner.poll() {
            self.ui.set_track_info(i, &path, &info);
        }
        let mut actions = self.ui.advance_fade(now);
        if let Some(player) = self.player.as_mut() {
            if player.maintain(now) {
                self.ui.vis.set_sample_rate(player.sample_rate());
            }
            while let Some(event) = player.poll() {
                self.ui.audio_event(event, &mut actions);
            }
        }
        // A finished track may have started the next one: send that before reading positions.
        self.handle(actions, el);

        let Some(player) = self.player.as_mut() else {
            return;
        };
        player.preload(self.ui.upcoming());
        player.set_volume(self.ui.output_volume(now));
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

    /// Bring the windows back from the taskbar, and the main window to the front.
    fn raise(&self) {
        for p in &self.panes {
            p.window.set_minimized(false);
        }
        if let Some(p) = self.pane(WindowKind::Main) {
            p.window.focus_window();
        }
    }

    /// Open what a later copy of NeoAmp was started with, and handed over before closing:
    /// files replace the playlist and play (or join it, with `--enqueue`), and a skin is
    /// shown. Started again with nothing to open, it brings this one to the front.
    fn take_requests(&mut self, el: &ActiveEventLoop) {
        while let Some(request) = self.instance.as_ref().and_then(instance::Server::poll) {
            if let Some(skin) = &request.skin {
                let path = install_skin(skin, self.store.as_ref());
                self.load_skin(&path);
            }
            if request.files.is_empty() {
                if request.skin.is_none() {
                    self.raise();
                }
                continue;
            }
            let tracks = playlist_file::expand(request.files);
            if tracks.is_empty() {
                log::info!("nothing playable there");
                continue;
            }
            let mut actions = Vec::new();
            if !request.enqueue {
                self.ui.clear_playlist(&mut actions);
            }
            let _ = self.add_tracks(tracks, !request.enqueue, &mut actions);
            self.handle(actions, el);
        }
    }

    /// Carry out what the system's media controls ask, and show them what plays.
    fn sync_now_playing(&mut self, el: &ActiveEventLoop, now: Instant) {
        let Some(np) = self.now_playing.as_mut() else {
            return;
        };
        let mut requests = Vec::new();
        while let Some(request) = np.poll() {
            requests.push(request);
        }
        for request in requests {
            match request {
                now_playing::Request::Ui(r) => {
                    let actions = self.ui.media(r);
                    self.handle(actions, el);
                }
                now_playing::Request::Raise => self.raise(),
                now_playing::Request::Quit => el.exit(),
            }
        }
        if let Some(np) = self.now_playing.as_mut() {
            np.update(&self.ui, now);
        }
    }

    /// Set the zoom from the command line, or else the settings, or else the height of the
    /// screen the main window opens on. The last is the default that Ctrl+0 (Cmd+0 on macOS)
    /// goes back to.
    fn choose_zoom(&mut self, el: &ActiveEventLoop) {
        let monitor = el
            .primary_monitor()
            .or_else(|| el.available_monitors().next());
        let default = monitor.map_or(Zoom::MIN, |m| {
            Zoom::for_screen_height(f64::from(m.size().height) / m.scale_factor())
        });
        let zoom = self.startup_zoom.unwrap_or(default);
        log::info!(
            "zoom {zoom} ({} and {} change it, {} resets to {default})",
            Shortcut::char('=').ctrl(),
            Shortcut::char('-').ctrl(),
            Shortcut::char('0').ctrl()
        );
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
            WindowKind::Main => self.title.as_str(),
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
        attrs = window_icon::with_icon(attrs, el);
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
        let density = Density::covering(ui_scale * window.scale_factor());
        let canvas = window_canvas((w, h), density);
        let (bw, bh) = buffer_size(&canvas)?;
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
            canvas,
            cursor: PhysicalPosition::new(0.0, 0.0),
            pointer: None,
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
            let scale = pane.scale(ui_scale);
            pane.fit_canvas((w, h), Density::covering(scale));
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

    /// Window `kind` went into or out of windowshade mode: resize it, and move the windows
    /// hanging below it by as much as it shrank or grew, so that they stay docked to it, as
    /// in Webamp. A closed window that would hang below it has the place it reopens at moved
    /// the same way. Without docking, the window only changes size.
    fn reshade(&mut self, kind: WindowKind, el: &ActiveEventLoop) {
        // Heights in screen pixels before and after, from the sizes asked for rather than
        // those the window reports, which may lag behind a request.
        let Some(scale) = self.pane(kind).map(|p| p.scale(self.ui.scale())) else {
            self.sync_panes(el);
            return;
        };
        let shaded = self.ui.is_shaded(kind);
        let height = |shaded| {
            let size = scaled_size(self.ui.window_size_shaded(kind, shaded), scale);
            i32::try_from(size.height).unwrap_or(0)
        };
        let (before, after) = (height(!shaded), height(shaded));
        let below = self.hanging_below(kind, before);
        self.sync_panes(el);
        let dh = after - before;
        if dh == 0 {
            return;
        }
        for (k, r) in below {
            if let Some(p) = self.pane(k) {
                p.window
                    .set_outer_position(PhysicalPosition::new(r.x, r.y.saturating_add(dh)));
            } else if let Some((dx, dy)) = self.offsets[kind_index(k)] {
                self.offsets[kind_index(k)] = Some((dx, dy.saturating_add(dh)));
            }
        }
    }

    /// The windows, open or closed, that hang below `kind` (see [`dock::hanging_below`]),
    /// with where they are now, taking `kind` to be `height` screen pixels high.
    fn hanging_below(&self, kind: WindowKind, height: i32) -> Vec<(WindowKind, Rect)> {
        if !self.docking || self.redock.contains(&WindowKind::Main) {
            return Vec::new();
        }
        let Some(main) = self.pane(WindowKind::Main).and_then(Pane::rect) else {
            return Vec::new();
        };
        let rects: Vec<(WindowKind, Rect)> = WindowKind::ALL
            .into_iter()
            .filter_map(|k| {
                let r = self.window_rect(k, main)?;
                let h = if k == kind { height } else { r.h };
                Some((k, Rect::new(r.x, r.y, r.w, h)))
            })
            .collect();
        let Some(root) = rects.iter().position(|(k, _)| *k == kind) else {
            return Vec::new();
        };
        let only: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        dock::hanging_below(root, &only)
            .into_iter()
            .map(|i| rects[i])
            .collect()
    }

    /// Where window `kind` is, with the main window at `main`: an open window's rectangle, or
    /// for a closed one, the place it would reopen at. `None` for a window whose place is not
    /// known yet, such as one waiting to be re-docked.
    fn window_rect(&self, kind: WindowKind, main: Rect) -> Option<Rect> {
        if self.redock.contains(&kind) {
            return None;
        }
        if let Some(pane) = self.pane(kind) {
            return pane.rect();
        }
        let (dx, dy) = self.offsets[kind_index(kind)]?;
        let size = scaled_size(self.ui.window_size(kind), self.applied_scale);
        Some(Rect::new(
            main.x.saturating_add(dx),
            main.y.saturating_add(dy),
            i32::try_from(size.width).ok()?,
            i32::try_from(size.height).ok()?,
        ))
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
        // The windowshade bar keeps its height, and the height the playlist opens out to.
        let size = (
            w.saturating_add(travel(pointer.x, resize.start.x)),
            if self.ui.pl_shade {
                h
            } else {
                h.saturating_add(travel(pointer.y, resize.start.y))
            },
        );
        if self.ui.resize_playlist(size) {
            self.sync_panes(el);
        }
    }

    /// Show the skin's pointer for where the mouse is over window `kind`, or the system's
    /// where the skin has none (see [`Ui::cursor_at`]). `pos` is the pointer in canvas
    /// coordinates. While a control, a window or the playlist's grip is being dragged, the
    /// pointer stays as it was when the drag began, as under Windows' mouse capture.
    fn update_cursor(&mut self, kind: WindowKind, pos: (i32, i32), el: &ActiveEventLoop) {
        if self.ui.pressed.is_some() || self.drag.is_some() || self.resize.is_some() {
            return;
        }
        let which = self.ui.cursor_at(kind, pos.0, pos.1);
        let want = if self.skin().cursor(which).is_some() {
            Pointer::Skin(which)
        } else {
            Pointer::System(system_pointer(which))
        };
        let Some(pane) = self.pane(kind) else {
            return;
        };
        if pane.pointer == Some(want) {
            return;
        }
        let k = pointer_scale(pane.window.scale_factor());
        let cursor: Cursor = match want {
            Pointer::Skin(which) => match self.skin_pointer(which, k, el) {
                Some(custom) => custom.into(),
                None => system_pointer(which).into(),
            },
            Pointer::System(icon) => icon.into(),
        };
        if let Some(pane) = self.pane_mut(kind) {
            pane.window.set_cursor(cursor);
            pane.pointer = Some(want);
        }
    }

    /// The skin's pointer `which`, `k` times larger, made ready for the windows the first time
    /// it is needed. `None` if the skin has none, or the platform refuses it (logged).
    fn skin_pointer(
        &mut self,
        which: SkinCursor,
        k: u16,
        el: &ActiveEventLoop,
    ) -> Option<CustomCursor> {
        if let Some(cursor) = self.pointers.get(&(which, k)) {
            return Some(cursor.clone());
        }
        let image = self.skin().cursor(which)?.scaled(k);
        let (hx, hy) = image.hotspot;
        match CustomCursor::from_rgba(image.rgba, image.w, image.h, hx, hy) {
            Ok(source) => {
                let cursor = el.create_custom_cursor(source);
                self.pointers.insert((which, k), cursor.clone());
                Some(cursor)
            }
            Err(e) => {
                log::warn!("cannot use the skin's {}: {e}", which.file_name());
                None
            }
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
                Action::Reshade(kind) => self.reshade(kind, el),
                Action::OpenFiles => self.show_dialog(Purpose::PlayFiles),
                Action::OpenFolder => self.show_dialog(Purpose::PlayFolder),
                Action::AlwaysOnTop => self.apply_window_level(),
                Action::OpenMenu(req) => self.open_menu(req, el),
                Action::App(command) => self.app_command(command, el),
                Action::Audio(t) => self.transport(t),
                Action::Advanced(i) => {
                    if let Some(track) = self.ui.tracks.get(i) {
                        log::info!("playing {} (gapless)", track.location);
                    }
                    self.load_auto_preset(i);
                }
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
        self.update_cursor(kind, (x, y), el);
    }

    /// What a key pressed in window `kind` does. The playlist has keys of its own, as in
    /// Winamp: there the arrows move through the entries rather than change the volume,
    /// Ctrl+A selects every entry rather than keeping the windows on top, and Ctrl+W shades
    /// the playlist rather than the main window. On macOS, Cmd takes Ctrl's place (see
    /// [`ctrl_held`]).
    fn key_cmd(&self, kind: WindowKind, event: &KeyEvent) -> Option<KeyCmd> {
        if kind == WindowKind::Playlist
            && let Some(cmd) = self.playlist_key(event)
        {
            return Some(cmd);
        }
        let ctrl = ctrl_held(self.modifiers);
        let alt = self.modifiers.alt_key();
        let shift = self.modifiers.shift_key();
        if alt && !ctrl && is_digit_3(event) {
            return Some(KeyCmd::FileInfo(InfoOf::Current));
        }
        Some(match &shortcut_key(event, self.modifiers) {
            Key::Character(s) => match (s.to_ascii_lowercase().as_str(), ctrl, alt) {
                ("a", true, _) => KeyCmd::ToggleAlwaysOnTop,
                ("d", true, _) => KeyCmd::ToggleDoubleSize,
                // Windowshade for the main window, or in the playlist for the playlist, as in
                // Winamp. Held down, it toggles once rather than flickering.
                ("w", true, false) if !event.repeat => {
                    let shaded = if kind == WindowKind::Playlist {
                        WindowKind::Playlist
                    } else {
                        WindowKind::Main
                    };
                    KeyCmd::ToggleShade(shaded)
                }
                ("f", _, true) => KeyCmd::MainMenu,
                ("s", _, true) => KeyCmd::SkinsMenu,
                ("l", false, false) if shift => KeyCmd::OpenFolder,
                ("l", true, false) => KeyCmd::OpenUrl,
                ("=" | "+", true, _) => KeyCmd::ZoomIn,
                ("-" | "_", true, _) => KeyCmd::ZoomOut,
                ("0", true, _) => KeyCmd::ZoomReset,
                ("t", true, _) => KeyCmd::ToggleTimeMode,
                ("g", _, true) => KeyCmd::ToggleEq,
                ("e", _, true) => KeyCmd::TogglePl,
                ("z", false, false) => KeyCmd::Prev,
                ("x", false, false) => KeyCmd::Play,
                ("c", false, false) => KeyCmd::Pause,
                ("v", false, false) if shift => KeyCmd::FadeOut,
                ("v", false, false) => KeyCmd::Stop,
                ("b", false, false) => KeyCmd::Next,
                ("l", false, false) => KeyCmd::OpenFiles,
                ("r", false, false) => KeyCmd::ToggleRepeat,
                ("s", false, false) => KeyCmd::ToggleShuffle,
                ("j", false, false) => KeyCmd::JumpToFile,
                _ => return None,
            },
            Key::Named(NamedKey::ArrowUp) => KeyCmd::VolumeUp,
            Key::Named(NamedKey::ArrowDown) => KeyCmd::VolumeDown,
            Key::Named(NamedKey::ArrowLeft) => KeyCmd::SeekBack,
            Key::Named(NamedKey::ArrowRight) => KeyCmd::SeekForward,
            _ => return None,
        })
    }

    /// The playlist window's own keys (Winamp's playlist editor shortcuts).
    fn playlist_key(&self, event: &KeyEvent) -> Option<KeyCmd> {
        let ctrl = ctrl_held(self.modifiers);
        let alt = self.modifiers.alt_key();
        let shift = self.modifiers.shift_key();
        let list = |cmd| Some(KeyCmd::List(cmd));
        let cursor = |to| list(ListCmd::Cursor { to, extend: shift });
        // Ctrl+Shift+1 sorts by title: with Shift held the key may report "!", so the sort
        // keys go by position on the keyboard.
        if ctrl && shift {
            let key = match event.physical_key {
                PhysicalKey::Code(KeyCode::Digit1) => Some(SortKey::Title),
                PhysicalKey::Code(KeyCode::Digit2) => Some(SortKey::FileName),
                PhysicalKey::Code(KeyCode::Digit3) => Some(SortKey::Path),
                _ => None,
            };
            if let Some(key) = key {
                return list(ListCmd::Sort(key));
            }
        }
        if alt && !ctrl && is_digit_3(event) {
            return Some(KeyCmd::FileInfo(InfoOf::Selection));
        }
        match &event.logical_key {
            Key::Character(s) if ctrl && !alt => match s.to_ascii_lowercase().as_str() {
                "a" => list(ListCmd::SelectAll),
                "i" => list(ListCmd::InvertSelection),
                "r" if shift => list(ListCmd::Randomize),
                "r" => list(ListCmd::Reverse),
                "n" => list(ListCmd::Clear),
                "o" => Some(KeyCmd::LoadList),
                "s" => Some(KeyCmd::SaveList),
                _ => None,
            },
            Key::Named(NamedKey::Delete) if ctrl => list(ListCmd::Crop),
            Key::Named(NamedKey::Delete) if alt => list(ListCmd::RemoveDead),
            Key::Named(NamedKey::Delete) => list(ListCmd::RemoveSelected),
            Key::Named(NamedKey::ArrowUp) if alt => list(ListCmd::MoveUp),
            Key::Named(NamedKey::ArrowDown) if alt => list(ListCmd::MoveDown),
            Key::Named(NamedKey::ArrowUp) => cursor(CursorMove::Up),
            Key::Named(NamedKey::ArrowDown) => cursor(CursorMove::Down),
            Key::Named(NamedKey::PageUp) => cursor(CursorMove::PageUp),
            Key::Named(NamedKey::PageDown) => cursor(CursorMove::PageDown),
            Key::Named(NamedKey::Home) => cursor(CursorMove::Home),
            Key::Named(NamedKey::End) => cursor(CursorMove::End),
            Key::Named(NamedKey::Enter) => list(ListCmd::PlayCursor),
            _ => None,
        }
    }

    /// The mouse wheel over window `kind`: the playlist scrolls a row per step, three steps a
    /// notch; over the main window the volume moves a step a notch.
    fn wheel(&mut self, kind: WindowKind, delta: MouseScrollDelta) {
        let Some(pane) = self.pane(kind) else { return };
        let per_notch = if kind == WindowKind::Playlist {
            3.0
        } else {
            1.0
        };
        let steps = match delta {
            MouseScrollDelta::LineDelta(_, y) => f64::from(y) * per_notch,
            // Touchpads report pixels: a playlist row's height makes a step.
            MouseScrollDelta::PixelDelta(d) => {
                d.y / (f64::from(TRACK_H) * pane.scale(self.ui.scale()))
            }
        };
        if self.wheel_rest.0 != kind {
            self.wheel_rest = (kind, 0.0);
        }
        let total = self.wheel_rest.1 + steps;
        let whole = total.trunc();
        self.wheel_rest.1 = total - whole;
        // A few steps per event, far inside `i32`.
        let whole = whole as i32;
        if whole != 0 {
            self.ui.wheel(kind, whole);
        }
    }

    /// Show the current track in the main window's title, for the taskbar and the window
    /// switcher, when it has changed.
    fn sync_title(&mut self) {
        self.title_scratch.clear();
        self.ui.write_window_title(&mut self.title_scratch);
        if self.title_scratch == self.title {
            return;
        }
        std::mem::swap(&mut self.title, &mut self.title_scratch);
        if let Some(p) = self.pane(WindowKind::Main) {
            p.window.set_title(&self.title);
        }
    }

    fn render(&mut self, kind: WindowKind, now: Instant) {
        let skin = self.custom.as_ref().unwrap_or(&self.base);
        let ui = &self.ui;
        let font = Some(&self.font);
        let Some(pane) = self.panes.iter_mut().find(|p| p.kind == kind) else {
            return;
        };
        pane.fit_canvas(
            ui.window_size(kind),
            Density::covering(pane.scale(ui.scale())),
        );
        ui.draw_window(kind, &mut pane.canvas, skin, font, now);
        let frame = pane.pixels.frame_mut();
        let drawn = &pane.canvas.image().data;
        if frame.len() == drawn.len() {
            frame.copy_from_slice(drawn);
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
        self.now_playing = self
            .pane(WindowKind::Main)
            .and_then(|p| NowPlaying::new(&p.window));
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.save();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let now = Instant::now();
        if self.jump.as_ref().is_some_and(|j| j.id() == id) {
            self.jump_event(event, now, el);
            return;
        }
        if self.info.as_ref().is_some_and(|i| i.id() == id) {
            self.info_event(event, now);
            return;
        }
        if self.url.as_ref().is_some_and(|u| u.id() == id) {
            self.url_event(event, now, el);
            return;
        }
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
                if let Some(p) = self.pane_mut(kind) {
                    // The skin's pointers are sized for the desktop's scale.
                    p.pointer = None;
                }
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
            WindowEvent::ModifiersChanged(m) => {
                let state = m.state();
                self.modifiers = state;
                self.ui.mods = Mods {
                    shift: state.shift_key(),
                    // macOS turns Ctrl-click into a right-click; Cmd-click selects there.
                    ctrl: state.control_key() || (cfg!(target_os = "macos") && state.super_key()),
                };
            }
            WindowEvent::MouseWheel { delta, .. } if !self.popups.is_open() => {
                self.wheel(kind, delta);
            }
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
                self.update_cursor(kind, (x, y), el);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_input(kind, state, button, now, el);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.popups.is_open() {
                    self.menu_key(&event, el);
                } else if event.state == ElementState::Pressed
                    && let Some(cmd) = self.key_cmd(kind, &event)
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
            self.take_requests(el);
            self.sync_audio(el, now);
            self.sync_now_playing(el, now);
            self.ui.tick(now);
            self.sync_title();
            if let Some(jump) = self.jump.as_mut() {
                jump.tick(&self.ui.tracks, self.ui.playlist_version(), now);
            }
            if let Some(url) = self.url.as_mut() {
                url.tick(now);
            }
            if let Some(info) = self.info.as_mut() {
                info.tick();
                // A stream's details arrive as it plays.
                if let Some(location) = self.ui.track().map(|t| &t.location)
                    && let (Some(url), Some(details)) =
                        (location.url(), self.ui.stream_details(location))
                {
                    info.update_stream(url, details);
                }
            }
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
