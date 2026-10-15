//! The Jump to file dialog's OS window. Unlike the menus it is an ordinary window with the
//! system's title bar, as Winamp's dialog was, so the window manager places it, moves it and
//! gives it the keyboard. It is drawn in software with softbuffer, like the menus.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow};
use softbuffer::{Context, Surface};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

use crate::canvas::Image;
use crate::classic::FACE;
use crate::font::TtfFont;
use crate::jump::{self, Jump, Key, Outcome};
use crate::menu;
use crate::ui::Track;

/// The dialog and its window.
pub(crate) struct JumpWindow {
    surface: Surface<OwnedDisplayHandle, Arc<Window>>,
    canvas: Image,
    font: TtfFont,
    jump: Jump,
    /// NeoAmp's zoom; the dialog's scale is this times the desktop's.
    zoom: f64,
    cursor: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    focused: bool,
    /// Whether the caret showed in the last frame drawn.
    caret_drawn: bool,
    /// Wheel travel not yet turned into whole notches.
    wheel_rest: f64,
}

impl std::fmt::Debug for JumpWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JumpWindow")
            .field("jump", &self.jump)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

fn dimension(v: i32) -> NonZeroU32 {
    NonZeroU32::new(v.max(1).unsigned_abs()).unwrap_or(NonZeroU32::MIN)
}

impl JumpWindow {
    /// Open the dialog at `zoom` on a desktop scaled by `scale_factor`, highlighting playlist
    /// entry `current`.
    ///
    /// # Errors
    /// Fails if no font is found or the window cannot be created.
    pub(crate) fn open(
        el: &ActiveEventLoop,
        zoom: f64,
        scale_factor: f64,
        on_top: bool,
        current: Option<usize>,
        now: Instant,
    ) -> Result<JumpWindow> {
        let scale = zoom * scale_factor;
        let font = TtfFont::load_menu(menu::font_px(scale))
            .context("no font for the Jump to file dialog")?;
        let jump = Jump::new(scale, &font, current, now);
        let (w, h) = jump.size();
        let level = if on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
        let attrs = Window::default_attributes()
            .with_title(jump::TITLE)
            .with_resizable(false)
            .with_window_level(level)
            .with_inner_size(PhysicalSize::new(dimension(w).get(), dimension(h).get()));
        let window = Arc::new(
            el.create_window(attrs)
                .context("creating the dialog window")?,
        );
        let context = Context::new(el.owned_display_handle())
            .map_err(|e| anyhow!("drawing the dialog: {e}"))?;
        let mut surface =
            Surface::new(&context, window).map_err(|e| anyhow!("creating its surface: {e}"))?;
        let size = surface.window().inner_size();
        surface
            .resize(dimension_u32(size.width), dimension_u32(size.height))
            .map_err(|e| anyhow!("sizing its surface: {e}"))?;
        Ok(JumpWindow {
            surface,
            canvas: Image::new(w, h),
            font,
            jump,
            zoom,
            cursor: PhysicalPosition::new(-1.0, -1.0),
            modifiers: ModifiersState::empty(),
            focused: false,
            caret_drawn: false,
            wheel_rest: 0.0,
        })
    }

    pub(crate) fn id(&self) -> WindowId {
        self.surface.window().id()
    }

    pub(crate) fn has_focus(&self) -> bool {
        self.focused
    }

    /// Bring the dialog to the front with the keyboard focus.
    pub(crate) fn raise(&self) {
        self.surface.window().focus_window();
    }

    fn pointer(&self) -> (i32, i32) {
        // Window coordinates are far inside `i32`; the casts only drop the fraction.
        (self.cursor.x.floor() as i32, self.cursor.y.floor() as i32)
    }

    /// Handle an event for the dialog's window, given the playlist.
    pub(crate) fn window_event(
        &mut self,
        event: WindowEvent,
        tracks: &[Track],
        now: Instant,
    ) -> Outcome {
        let outcome = match event {
            WindowEvent::CloseRequested => Outcome::Close,
            WindowEvent::Focused(f) => {
                self.focused = f;
                Outcome::None
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                Outcome::None
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                let (x, y) = self.pointer();
                self.jump.pointer_moved(x, y)
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.pointer();
                match state {
                    ElementState::Pressed => self.jump.pointer_down(x, y, now, &self.font),
                    ElementState::Released => self.jump.pointer_up(x, y),
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel_rest += match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    // A notch is about three rows.
                    MouseScrollDelta::PixelDelta(p) => p.y / (42.0 * self.jump.scale()),
                };
                let notches = self.wheel_rest.trunc();
                self.wheel_rest -= notches;
                // Whole notches from one event: a handful at most.
                let notches = notches as i32;
                if notches == 0 {
                    Outcome::None
                } else {
                    self.jump.wheel(notches)
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let key = match &event.logical_key {
                    WinitKey::Named(NamedKey::ArrowUp) => Some(Key::Up),
                    WinitKey::Named(NamedKey::ArrowDown) => Some(Key::Down),
                    WinitKey::Named(NamedKey::PageUp) => Some(Key::PageUp),
                    WinitKey::Named(NamedKey::PageDown) => Some(Key::PageDown),
                    WinitKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
                    WinitKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
                    WinitKey::Named(NamedKey::Home) => Some(Key::Home),
                    WinitKey::Named(NamedKey::End) => Some(Key::End),
                    WinitKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
                    WinitKey::Named(NamedKey::Delete) => Some(Key::Delete),
                    WinitKey::Named(NamedKey::Enter) => Some(Key::Enter),
                    WinitKey::Named(NamedKey::Escape) => Some(Key::Escape),
                    _ => None,
                };
                let shortcut = self.modifiers.control_key()
                    || self.modifiers.alt_key()
                    || self.modifiers.super_key();
                match (key, &event.text) {
                    (Some(key), _) => self.jump.key(key, tracks, now),
                    (None, Some(text)) if !shortcut => self.jump.text(text, tracks, now),
                    _ => Outcome::None,
                }
            }
            WindowEvent::Resized(size) => {
                if let Err(e) = self
                    .surface
                    .resize(dimension_u32(size.width), dimension_u32(size.height))
                {
                    log::warn!("cannot resize the dialog: {e}");
                }
                Outcome::Redraw
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor,
                mut inner_size_writer,
            } => {
                // Moved to a screen with another desktop scale: lay out afresh, and answer with
                // the new size, or winit resizes the window by the ratio of the scales.
                let scale = self.zoom * scale_factor;
                self.font = self.font.resized(menu::font_px(scale));
                self.jump.rescale(scale, &self.font);
                let (w, h) = self.jump.size();
                self.canvas = Image::new(w, h);
                let size = PhysicalSize::new(dimension(w).get(), dimension(h).get());
                if let Err(e) = inner_size_writer.request_inner_size(size) {
                    log::warn!("cannot resize the dialog for the new scale: {e}");
                }
                Outcome::Redraw
            }
            WindowEvent::RedrawRequested => Outcome::Redraw,
            _ => Outcome::None,
        };
        if outcome == Outcome::Redraw {
            self.redraw(tracks, now);
        }
        outcome
    }

    /// Call every frame: match the playlist again if it changed, and blink the caret.
    pub(crate) fn tick(&mut self, tracks: &[Track], version: u64, now: Instant) {
        let changed = self.jump.refresh(tracks, version);
        if changed || self.jump.caret_on(now) != self.caret_drawn {
            self.redraw(tracks, now);
        }
    }

    fn redraw(&mut self, tracks: &[Track], now: Instant) {
        self.jump.draw(&mut self.canvas, &self.font, tracks, now);
        self.caret_drawn = self.jump.caret_on(now);
        let size = self.surface.window().inner_size();
        let mut buffer = match self.surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(e) => {
                log::warn!("cannot draw the dialog: {e}");
                return;
            }
        };
        // The window manager may have sized the window differently; the rest is plain face.
        let face = pixel(FACE);
        let bw = usize::try_from(size.width).unwrap_or(0);
        let cw = usize::try_from(self.canvas.w).unwrap_or(0);
        for (y, row) in buffer.chunks_mut(bw.max(1)).enumerate() {
            let src = self
                .canvas
                .data
                .get(y * cw * 4..(y + 1) * cw * 4)
                .unwrap_or(&[]);
            let mut src = src.chunks_exact(4);
            for dst in row {
                *dst = src.next().map_or(face, |p| pixel([p[0], p[1], p[2], 0xFF]));
            }
        }
        if let Err(e) = buffer.present() {
            log::warn!("cannot show the dialog: {e}");
        }
    }
}

/// softbuffer's 0x00RRGGBB.
fn pixel(c: [u8; 4]) -> u32 {
    (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2])
}

fn dimension_u32(v: u32) -> NonZeroU32 {
    NonZeroU32::new(v).unwrap_or(NonZeroU32::MIN)
}
