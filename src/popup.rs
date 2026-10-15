//! The window layer for popup menus: each open level of a [`Menus`] is a borderless OS window
//! drawn in software with softbuffer.
//!
//! Menus open and close as the pointer moves, so their windows must appear at once. The main
//! windows present through `pixels`, which sets up a GPU device per window, and that can take
//! a tenth of a second; softbuffer only copies pixels to the window.
//!
//! On X11 a menu is an override-redirect window, as toolkit menus are, so it lands exactly
//! where it is put; elsewhere it is a borderless window kept above the others. Wayland lets
//! no window choose its position, so there menus appear wherever the compositor puts them.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow};
use softbuffer::{Context, Surface};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::window::{Window, WindowId, WindowLevel};

use crate::canvas::{Image, Rect};
use crate::font::TtfFont;
use crate::menu::{self, Anchor, Item, Key, Menus, Outcome};
use crate::menus::Command;

/// One menu level's window.
struct Popup {
    surface: Surface<OwnedDisplayHandle, Arc<Window>>,
    canvas: Image,
    /// Where it was put, in screen pixels.
    rect: Rect,
    cursor: PhysicalPosition<f64>,
    focused: bool,
}

impl Popup {
    fn window(&self) -> &Window {
        self.surface.window()
    }

    /// The pointer in window pixels.
    fn pointer(&self) -> (i32, i32) {
        // Window coordinates are far inside `i32`; the casts only drop the fraction.
        (self.cursor.x.floor() as i32, self.cursor.y.floor() as i32)
    }
}

/// The menus on screen, if any, and their windows.
#[derive(Default)]
pub(crate) struct Popups {
    /// Created with the first menu.
    context: Option<Context<OwnedDisplayHandle>>,
    /// The menu font at the open menu's scale; loaded with the first menu.
    font: Option<TtfFont>,
    menus: Option<Menus<Command>>,
    windows: Vec<Popup>,
}

impl std::fmt::Debug for Popups {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Popups")
            .field("menus", &self.menus)
            .field("windows", &self.windows.len())
            .finish_non_exhaustive()
    }
}

/// `v` as a window dimension, at least one pixel.
fn dimension(v: i32) -> NonZeroU32 {
    NonZeroU32::new(v.max(1).unsigned_abs()).unwrap_or(NonZeroU32::MIN)
}

fn create_window(
    el: &ActiveEventLoop,
    context: &Context<OwnedDisplayHandle>,
    rect: Rect,
) -> Result<Popup> {
    let (w, h) = (dimension(rect.w), dimension(rect.h));
    #[allow(unused_mut)] // only X11 and Windows add to it
    let mut attrs = Window::default_attributes()
        .with_title("NeoAmp menu")
        .with_decorations(false)
        .with_resizable(false)
        .with_active(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_inner_size(PhysicalSize::new(w.get(), h.get()))
        .with_position(PhysicalPosition::new(rect.x, rect.y));
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        use winit::platform::x11::{WindowAttributesExtX11, WindowType};
        attrs = attrs
            .with_override_redirect(true)
            .with_x11_window_type(vec![WindowType::PopupMenu]);
    }
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        attrs = attrs.with_skip_taskbar(true);
    }
    let window = Arc::new(el.create_window(attrs).context("creating a menu window")?);
    let mut surface =
        Surface::new(context, window).map_err(|e| anyhow!("creating a menu surface: {e}"))?;
    surface
        .resize(w, h)
        .map_err(|e| anyhow!("sizing a menu surface: {e}"))?;
    Ok(Popup {
        surface,
        canvas: Image::new(rect.w, rect.h),
        rect,
        cursor: PhysicalPosition::new(-1.0, -1.0),
        focused: false,
    })
}

impl Popups {
    #[must_use]
    pub(crate) fn is_open(&self) -> bool {
        self.menus.is_some()
    }

    /// Whether `id` is one of the menu windows.
    #[must_use]
    pub(crate) fn owns(&self, id: WindowId) -> bool {
        self.windows.iter().any(|p| p.window().id() == id)
    }

    /// A menu window has the keyboard focus (Windows and macOS give it to a clicked one).
    #[must_use]
    pub(crate) fn has_focus(&self) -> bool {
        self.windows.iter().any(|p| p.focused)
    }

    /// Show `items` at `anchor` on `screen`, `scale` times Windows' sizes at 96 DPI, closing
    /// any menu already open.
    pub(crate) fn open(
        &mut self,
        el: &ActiveEventLoop,
        items: Vec<Item<Command>>,
        anchor: Anchor,
        screen: Rect,
        scale: f64,
        keyboard: bool,
    ) {
        self.close();
        let px = menu::font_px(scale);
        let font = match self.font.take() {
            Some(font) if font.px() == px => Some(font),
            Some(font) => Some(font.resized(px)),
            None => TtfFont::load_menu(px),
        };
        let Some(font) = font else {
            return;
        };
        self.menus = Some(Menus::open(items, anchor, screen, scale, keyboard, &font));
        self.font = Some(font);
        self.sync(el);
    }

    pub(crate) fn close(&mut self) {
        self.menus = None;
        self.windows.clear();
    }

    /// Handle an event for menu window `id`; returns the command chosen, if one was.
    pub(crate) fn window_event(
        &mut self,
        el: &ActiveEventLoop,
        id: WindowId,
        event: &WindowEvent,
        now: Instant,
    ) -> Option<Command> {
        let level = self.windows.iter().position(|p| p.window().id() == id)?;
        let (Some(menus), Some(font)) = (self.menus.as_mut(), self.font.as_ref()) else {
            return None;
        };
        let popup = &mut self.windows[level];
        let outcome = match event {
            WindowEvent::CursorMoved { position, .. } => {
                popup.cursor = *position;
                let (x, y) = popup.pointer();
                menus.pointer_moved(level, x, y, now)
            }
            WindowEvent::CursorLeft { .. } => {
                popup.cursor = PhysicalPosition::new(-1.0, -1.0);
                menus.pointer_left(level)
            }
            WindowEvent::MouseInput { state, .. } => {
                let (x, y) = popup.pointer();
                match state {
                    ElementState::Pressed => menus.pressed(level, x, y, font),
                    ElementState::Released => menus.released(level, x, y, font),
                }
            }
            WindowEvent::Focused(focused) => {
                popup.focused = *focused;
                Outcome::None
            }
            WindowEvent::RedrawRequested => {
                self.redraw(level);
                Outcome::None
            }
            _ => Outcome::None,
        };
        self.apply(el, outcome)
    }

    /// A key pressed while a menu is open; returns the command chosen, if one was.
    pub(crate) fn key(&mut self, el: &ActiveEventLoop, key: Key) -> Option<Command> {
        let (Some(menus), Some(font)) = (self.menus.as_mut(), self.font.as_ref()) else {
            return None;
        };
        let outcome = menus.key(key, font);
        self.apply(el, outcome)
    }

    /// Open or close the submenu the pointer has rested on. Call every frame.
    pub(crate) fn tick(&mut self, el: &ActiveEventLoop, now: Instant) {
        let (Some(menus), Some(font)) = (self.menus.as_mut(), self.font.as_ref()) else {
            return;
        };
        let outcome = menus.tick(now, font);
        // Resting the pointer never chooses a command.
        let _ = self.apply(el, outcome);
    }

    fn apply(&mut self, el: &ActiveEventLoop, outcome: Outcome<Command>) -> Option<Command> {
        match outcome {
            Outcome::None => None,
            Outcome::Changed => {
                self.sync(el);
                None
            }
            Outcome::Chosen(command) => {
                self.close();
                Some(command)
            }
            Outcome::Close => {
                self.close();
                None
            }
        }
    }

    /// Give each open menu level a window where it belongs, close the rest, and redraw.
    fn sync(&mut self, el: &ActiveEventLoop) {
        let Some(menus) = &self.menus else {
            self.windows.clear();
            return;
        };
        let levels = menus.levels();
        let rects: Vec<Rect> = (0..levels).filter_map(|i| menus.rect(i)).collect();
        self.windows.truncate(levels);
        if self.context.is_none() {
            match Context::new(el.owned_display_handle()) {
                Ok(context) => self.context = Some(context),
                Err(e) => {
                    log::error!("cannot draw menus: {e}");
                    self.close();
                    return;
                }
            }
        }
        let Some(context) = &self.context else {
            return;
        };
        for (i, rect) in rects.into_iter().enumerate() {
            if self.windows.get(i).is_some_and(|p| p.rect == rect) {
                continue;
            }
            // Another submenu at this level: a new window in its place.
            self.windows.truncate(i);
            match create_window(el, context, rect) {
                Ok(popup) => self.windows.push(popup),
                Err(e) => {
                    log::error!("cannot show a menu: {e:#}");
                    break;
                }
            }
        }
        for i in 0..self.windows.len() {
            self.redraw(i);
        }
    }

    fn redraw(&mut self, level: usize) {
        let (Some(menus), Some(font)) = (&self.menus, &self.font) else {
            return;
        };
        let Some(popup) = self.windows.get_mut(level) else {
            return;
        };
        menus.draw(level, &mut popup.canvas, font);
        let mut buffer = match popup.surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(e) => {
                log::warn!("cannot draw a menu: {e}");
                return;
            }
        };
        // softbuffer wants 0x00RRGGBB.
        for (dst, src) in buffer.iter_mut().zip(popup.canvas.data.chunks_exact(4)) {
            *dst = u32::from(src[0]) << 16 | u32::from(src[1]) << 8 | u32::from(src[2]);
        }
        if let Err(e) = buffer.present() {
            log::warn!("cannot show a menu: {e}");
        }
    }
}
