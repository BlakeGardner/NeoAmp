//! The Jump to file dialog's OS window (see [`crate::dialog_window`]).

use std::time::Instant;

use anyhow::Result;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::WindowId;

use crate::app::{EditShortcut, edit_shortcut};
use crate::canvas::Image;
use crate::clipboard;
use crate::dialog_window::{DialogWindow, window_size};
use crate::font::TtfFont;
use crate::jump::{self, Jump, Key, Outcome};
use crate::menu;
use crate::ui::Track;

/// The dialog and its window.
pub(crate) struct JumpWindow {
    window: DialogWindow,
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
        let font = TtfFont::load_menu(menu::font_px(scale));
        let jump = Jump::new(scale, &font, current, now);
        let (w, h) = jump.size();
        let window = DialogWindow::open(el, jump::TITLE, (w, h), on_top)?;
        Ok(JumpWindow {
            window,
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
        self.window.id()
    }

    pub(crate) fn has_focus(&self) -> bool {
        self.focused
    }

    /// Bring the dialog to the front with the keyboard focus.
    pub(crate) fn raise(&self) {
        self.window.window().focus_window();
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
                match (edit_shortcut(&event, self.modifiers), key, &event.text) {
                    (Some(EditShortcut::Paste), ..) => match clipboard::text() {
                        Some(text) => self.jump.text(text.trim(), tracks, now),
                        None => Outcome::None,
                    },
                    (Some(EditShortcut::SelectAll), ..) => self.jump.select_all(now),
                    (None, Some(key), _) => self.jump.key(key, tracks, now),
                    (None, None, Some(text)) if !shortcut => self.jump.text(text, tracks, now),
                    _ => Outcome::None,
                }
            }
            WindowEvent::Resized(size) => {
                self.window.resized(size);
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
                if let Err(e) = inner_size_writer.request_inner_size(window_size((w, h))) {
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
        self.window.present(&self.canvas);
    }
}
