//! The desktop application: one borderless OS window per Winamp window (main, equalizer,
//! playlist), each presented through `pixels`, with Winamp-style magnetic docking.
//!
//! Docking needs to read and set window positions. X11 (including XWayland) allows that, so
//! the X11 backend is preferred whenever `DISPLAY` is set; pure Wayland forbids it, in which
//! case title-bar drags are handed to the compositor and no snapping happens.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use pixels::wgpu;
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::canvas::{Image, Rect};
use crate::dock::{self, SNAP_DISTANCE};
use crate::font::TtfFont;
use crate::skin::Skin;
use crate::ui::{Action, KeyCmd, Ui, WindowKind};

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

/// Per-second counters for diagnosing stalls (`RUST_LOG=winamp=debug`).
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

/// What to run: an optional skin to load and an optional screenshot-and-exit path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub skin: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
}

/// Load the base skin, apply the options and either render a screenshot or open the windows.
///
/// # Errors
/// Fails if the embedded base skin or the requested skin cannot be parsed, if the screenshot
/// cannot be written, or if no windowing backend can be initialised.
pub fn run(opts: Options) -> Result<()> {
    let base = Skin::from_wsz(BASE_SKIN, "base-2.91").context("embedded base skin")?;
    let custom = opts
        .skin
        .as_deref()
        .map(|p| load_skin(p, &base))
        .transpose()?;
    let font = TtfFont::load_system(TtfFont::DEFAULT_PX);
    let ui = Ui::new();

    if let Some(out) = &opts.screenshot {
        return render_screenshot(ui, custom.as_ref().unwrap_or(&base), font.as_ref(), out);
    }

    let event_loop = build_event_loop()?;
    let mut app = App {
        ui,
        base,
        custom,
        font,
        panes: Vec::with_capacity(3),
        drag: None,
        redock: Vec::new(),
        docking: false,
        unmanaged: unmanaged_preference() == Some(true),
        probe: None,
        probe_done: unmanaged_preference().is_some(),
        applied_scale: 1,
        modifiers: ModifiersState::empty(),
        next_frame: Instant::now(),
        stats: FrameStats::new(Instant::now()),
    };
    event_loop
        .run_app(&mut app)
        .context("running the event loop")?;
    Ok(())
}

/// Draw a few frames of fake playback offscreen, stacked the classic way, and save a PNG.
///
/// # Errors
/// Fails if the PNG cannot be written.
pub fn render_screenshot(
    mut ui: Ui,
    skin: &Skin,
    font: Option<&TtfFont>,
    out: &Path,
) -> Result<()> {
    let start = Instant::now();
    let (w, h) = ui.layout().size;
    let mut canvas = Image::new(w, h);
    for k in 0..70 {
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
    Ok(skin)
}

/// Prefer X11 (which can position windows) unless `WINAMP_BACKEND=wayland` asks otherwise.
fn build_event_loop() -> Result<EventLoop<()>> {
    let backend = std::env::var("WINAMP_BACKEND")
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

/// `WINAMP_UNMANAGED=1` forces override-redirect windows, `0` forbids them; unset means probe.
fn unmanaged_preference() -> Option<bool> {
    match std::env::var("WINAMP_UNMANAGED").ok()?.trim() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// One OS window hosting one Winamp window.
struct Pane {
    kind: WindowKind,
    /// Declared before `window` so the GPU surface is dropped first.
    pixels: Pixels<'static>,
    window: Arc<Window>,
    canvas: Image,
    cursor: PhysicalPosition<f64>,
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

    /// The last pointer position in canvas (skin pixel) coordinates.
    fn canvas_pos(&self) -> (i32, i32) {
        let pos = (self.cursor.x as f32, self.cursor.y as f32);
        let (x, y) = match self.pixels.window_pos_to_pixel(pos) {
            Ok(xy) => xy,
            Err(xy) => self.pixels.clamp_pixel_pos(xy),
        };
        (
            i32::try_from(x).unwrap_or(i32::MAX),
            i32::try_from(y).unwrap_or(i32::MAX),
        )
    }

    /// The monitor this window is on, as a physical rectangle.
    fn screen(&self) -> Option<Rect> {
        let mon = self.window.current_monitor()?;
        let p = mon.position();
        let s = mon.size();
        Some(Rect::new(
            p.x,
            p.y,
            i32::try_from(s.width).ok()?,
            i32::try_from(s.height).ok()?,
        ))
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

struct App {
    ui: Ui,
    base: Skin,
    /// A user-chosen skin; `None` means the embedded base skin is shown.
    custom: Option<Skin>,
    font: Option<TtfFont>,
    panes: Vec<Pane>,
    drag: Option<Drag>,
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
    /// The scale the panes were last sized for (to re-dock after a double-size toggle).
    applied_scale: u8,
    modifiers: ModifiersState,
    next_frame: Instant,
    stats: FrameStats,
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

    fn load_skin(&mut self, path: &Path) {
        match load_skin(path, &self.base) {
            Ok(s) => {
                log::info!("loaded skin {}", s.name);
                self.custom = Some(s);
            }
            Err(e) => log::error!("cannot load skin {}: {e:#}", path.display()),
        }
    }

    /// Logical window size for `kind` at the current UI scale.
    fn logical_size(&self, kind: WindowKind) -> LogicalSize<f64> {
        let (w, h) = self.ui.window_size(kind);
        let scale = i32::from(self.ui.scale());
        LogicalSize::new(f64::from(w * scale), f64::from(h * scale))
    }

    fn open_pane(
        &mut self,
        el: &ActiveEventLoop,
        kind: WindowKind,
        at: Option<PhysicalPosition<i32>>,
    ) -> Result<()> {
        let title = match kind {
            WindowKind::Main => "Winamp",
            WindowKind::Eq => "Winamp Equalizer",
            WindowKind::Playlist => "Winamp Playlist",
        };
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(self.logical_size(kind))
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true);
        if let Some(p) = at {
            attrs = attrs.with_position(p);
        }
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
        let (w, h) = self.ui.window_size(kind);
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
            pixels,
            window,
            canvas: Image::new(w, h),
            cursor: PhysicalPosition::new(0.0, 0.0),
            focused: false,
            opened_at: Instant::now(),
        });
        if at.is_some() {
            self.redock.push(kind);
        }
        Ok(())
    }

    fn close_pane(&mut self, kind: WindowKind) {
        self.panes.retain(|p| p.kind != kind);
        self.redock.retain(|k| *k != kind);
        if self.drag.as_ref().is_some_and(|d| d.kind == kind) {
            self.drag = None;
        }
    }

    /// Re-apply docked positions to windows the window manager has had time to map.
    fn apply_pending_docks(&mut self, now: Instant) {
        let due: Vec<WindowKind> = self
            .redock
            .iter()
            .copied()
            .filter(|k| {
                self.pane(*k)
                    .is_some_and(|p| now >= p.opened_at + REDOCK_DELAY)
            })
            .collect();
        for kind in due {
            self.redock.retain(|k| *k != kind);
            if let (Some(at), Some(pane)) = (self.docked_position(kind), self.pane(kind)) {
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

    /// Tear every window down and open it again (used when switching window styles).
    fn recreate_panes(&mut self, el: &ActiveEventLoop) {
        let main_at = self
            .pane(WindowKind::Main)
            .and_then(Pane::rect)
            .map(|r| PhysicalPosition::new(r.x, r.y));
        self.panes.clear();
        self.redock.clear();
        self.drag = None;
        if let Err(e) = self.open_pane(el, WindowKind::Main, main_at) {
            log::error!("cannot reopen the main window: {e:#}");
            el.exit();
            return;
        }
        self.redock.clear();
        self.sync_panes(el);
    }

    /// Where a newly opened window docks: the equalizer under the main window, the playlist
    /// under the lowest of the two.
    fn docked_position(&self, kind: WindowKind) -> Option<PhysicalPosition<i32>> {
        if !self.docking {
            return None;
        }
        let main = self.pane(WindowKind::Main)?.rect()?;
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
        let scale_changed = self.ui.scale() != self.applied_scale;
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
            let wanted = self.logical_size(kind);
            let (w, h) = self.ui.window_size(kind);
            let Some(pane) = self.pane_mut(kind) else {
                continue;
            };
            if pane.canvas.w != w || pane.canvas.h != h {
                pane.canvas = Image::new(w, h);
                if let (Ok(bw), Ok(bh)) = (u32::try_from(w), u32::try_from(h))
                    && let Err(e) = pane.pixels.resize_buffer(bw, bh)
                {
                    log::error!("resize_buffer: {e}");
                }
            }
            let current = pane
                .window
                .inner_size()
                .to_logical::<f64>(pane.window.scale_factor());
            if (current.width - wanted.width).abs() > 0.5
                || (current.height - wanted.height).abs() > 0.5
            {
                let _ = pane.window.request_inner_size(wanted);
            }
            pane.window.request_redraw();
        }

        if scale_changed {
            let ratio = f64::from(self.ui.scale()) / f64::from(self.applied_scale);
            self.applied_scale = self.ui.scale();
            if let Some(main) = self.pane(WindowKind::Main).and_then(Pane::rect) {
                for (kind, dx, dy) in cluster {
                    if let Some(p) = self.pane(kind) {
                        let x = main.x + (f64::from(dx) * ratio).round() as i32;
                        let y = main.y + (f64::from(dy) * ratio).round() as i32;
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
        let (nx, ny) = dock::drag_target(
            rect,
            &offsets,
            pointer,
            grab,
            &statics,
            screen,
            SNAP_DISTANCE,
        );

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

    fn handle(&mut self, actions: Vec<Action>, el: &ActiveEventLoop) {
        for a in actions {
            match a {
                Action::DragWindow(kind) => self.begin_drag(kind),
                Action::Close => el.exit(),
                Action::Minimize => {
                    for p in &self.panes {
                        p.window.set_minimized(true);
                    }
                }
                Action::Relayout => self.sync_panes(el),
            }
        }
    }

    fn key_cmd(&self, key: &Key) -> Option<KeyCmd> {
        let ctrl = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();
        Some(match key {
            Key::Character(s) => match (s.to_ascii_lowercase().as_str(), ctrl, alt) {
                ("d", true, _) => KeyCmd::ToggleDoubleSize,
                ("t", true, _) => KeyCmd::ToggleTimeMode,
                ("g", _, true) => KeyCmd::ToggleEq,
                ("e", _, true) => KeyCmd::TogglePl,
                ("z", false, false) => KeyCmd::Prev,
                ("x", false, false) => KeyCmd::Play,
                ("c", false, false) => KeyCmd::Pause,
                ("v", false, false) => KeyCmd::Stop,
                ("b", false, false) => KeyCmd::Next,
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
        if let Err(e) = pane.pixels.render() {
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
        let at = PhysicalPosition::new(origin.x + INITIAL_OFFSET.0, origin.y + INITIAL_OFFSET.1);
        if let Err(e) = self.open_pane(el, WindowKind::Main, Some(at)) {
            log::error!("cannot open a window: {e:#}");
            el.exit();
            return;
        }
        // The main window goes where the window manager puts it; only the others re-dock.
        self.redock.clear();
        self.docking = self.pane(WindowKind::Main).and_then(Pane::rect).is_some();
        if !self.docking {
            log::info!("this backend cannot position windows; docking is disabled");
        }
        self.applied_scale = self.ui.scale();
        self.sync_panes(el);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let now = Instant::now();
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
            WindowEvent::Focused(f) => {
                if let Some(p) = self.pane_mut(kind) {
                    p.focused = f;
                }
                self.ui.active = self.panes.iter().any(|p| p.focused);
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let Some(p) = self.pane_mut(kind) else { return };
                p.cursor = position;
                let (x, y) = p.canvas_pos();
                if self.drag.as_ref().is_some_and(|d| d.kind == kind) {
                    self.drag_to(kind, position);
                } else {
                    let actions = self.ui.mouse_move(kind, x, y, now);
                    self.handle(actions, el);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let Some(p) = self.pane(kind) else { return };
                if self.unmanaged && state == ElementState::Pressed {
                    // Nobody manages focus for override-redirect windows: claim it ourselves.
                    p.window.focus_window();
                }
                let (x, y) = p.canvas_pos();
                let actions = match state {
                    ElementState::Pressed => self.ui.mouse_down(kind, x, y, now),
                    ElementState::Released => {
                        self.drag = None;
                        self.ui.mouse_up(kind, x, y, now)
                    }
                };
                self.handle(actions, el);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && let Some(cmd) = self.key_cmd(&event.logical_key)
                {
                    let actions = self.ui.key(cmd, now);
                    self.handle(actions, el);
                }
            }
            WindowEvent::DroppedFile(path) => self.load_skin(&path),
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
            self.ui.tick(now);
            for p in &self.panes {
                p.window.request_redraw();
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}
