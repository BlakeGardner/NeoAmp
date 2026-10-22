//! The Open URL dialog's OS window (see [`crate::dialog_window`]). It opens with the address
//! on the clipboard, if there is one, ready to open or to type over, and Ctrl+V (Cmd+V on
//! macOS) pastes.

use std::time::Instant;

use anyhow::Result;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::WindowId;

use crate::app::{EditShortcut, edit_shortcut};
use crate::canvas::Image;
use crate::clipboard;
use crate::dialog_window::{DialogWindow, window_size};
use crate::font::TtfFont;
use crate::menu;
use crate::text_box::Edit;
use crate::url_dialog::{Key, Outcome, Purpose, UrlDialog};

/// The dialog and its window.
pub(crate) struct UrlWindow {
    window: DialogWindow,
    canvas: Image,
    font: TtfFont,
    dialog: UrlDialog,
    /// NeoAmp's zoom; the dialog's scale is this times the desktop's.
    zoom: f64,
    cursor: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    focused: bool,
    /// Whether the caret showed in the last frame drawn.
    caret_drawn: bool,
}

impl std::fmt::Debug for UrlWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UrlWindow")
            .field("dialog", &self.dialog)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

impl UrlWindow {
    /// Open the dialog for `purpose` at `zoom` on a desktop scaled by `scale_factor`, holding
    /// the web address on the clipboard if there is one.
    ///
    /// # Errors
    /// Fails if no font is found or the window cannot be created.
    pub(crate) fn open(
        el: &ActiveEventLoop,
        purpose: Purpose,
        zoom: f64,
        scale_factor: f64,
        on_top: bool,
        now: Instant,
    ) -> Result<UrlWindow> {
        let scale = zoom * scale_factor;
        let font = TtfFont::load_menu(menu::font_px(scale));
        let start = clipboard::web_url();
        let dialog = UrlDialog::new(purpose, scale, start.as_deref(), now);
        let (w, h) = dialog.size();
        let window = DialogWindow::open(el, purpose.title(), (w, h), on_top, None)?;
        Ok(UrlWindow {
            window,
            canvas: Image::new(w, h),
            font,
            dialog,
            zoom,
            cursor: PhysicalPosition::new(-1.0, -1.0),
            modifiers: ModifiersState::empty(),
            focused: false,
            caret_drawn: false,
        })
    }

    pub(crate) fn id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn has_focus(&self) -> bool {
        self.focused
    }

    pub(crate) fn purpose(&self) -> Purpose {
        self.dialog.purpose()
    }

    /// Bring the dialog to the front with the keyboard focus.
    pub(crate) fn raise(&self) {
        self.window.window().focus_window();
    }

    fn pointer(&self) -> (i32, i32) {
        // Window coordinates are far inside `i32`; the casts only drop the fraction.
        (self.cursor.x.floor() as i32, self.cursor.y.floor() as i32)
    }

    /// Handle an event for the dialog's window.
    pub(crate) fn window_event(&mut self, event: WindowEvent, now: Instant) -> Outcome {
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
                self.dialog.pointer_moved(x, y)
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.pointer();
                match state {
                    ElementState::Pressed => self.dialog.pointer_down(x, y, now, &self.font),
                    ElementState::Released => self.dialog.pointer_up(x, y),
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let key = match &event.logical_key {
                    WinitKey::Named(NamedKey::ArrowLeft) => Some(Key::Edit(Edit::Left)),
                    WinitKey::Named(NamedKey::ArrowRight) => Some(Key::Edit(Edit::Right)),
                    WinitKey::Named(NamedKey::Home) => Some(Key::Edit(Edit::Home)),
                    WinitKey::Named(NamedKey::End) => Some(Key::Edit(Edit::End)),
                    WinitKey::Named(NamedKey::Backspace) => Some(Key::Edit(Edit::Backspace)),
                    WinitKey::Named(NamedKey::Delete) => Some(Key::Edit(Edit::Delete)),
                    WinitKey::Named(NamedKey::Enter) => Some(Key::Enter),
                    WinitKey::Named(NamedKey::Escape) => Some(Key::Escape),
                    _ => None,
                };
                let shortcut = self.modifiers.control_key()
                    || self.modifiers.alt_key()
                    || self.modifiers.super_key();
                match (edit_shortcut(&event, self.modifiers), key, &event.text) {
                    (Some(EditShortcut::Paste), ..) => match clipboard::text() {
                        // A URL copied from a page often ends in a line break.
                        Some(text) => self.dialog.text_input(text.trim(), now),
                        None => Outcome::None,
                    },
                    (Some(EditShortcut::SelectAll), ..) => self.dialog.select_all(now),
                    (None, Some(key), _) => self.dialog.key(key, now),
                    (None, None, Some(text)) if !shortcut => self.dialog.text_input(text, now),
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
                self.dialog.rescale(scale);
                let (w, h) = self.dialog.size();
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
            self.redraw(now);
        }
        outcome
    }

    /// Call every frame: blink the caret.
    pub(crate) fn tick(&mut self, now: Instant) {
        if self.dialog.caret_on(now) != self.caret_drawn {
            self.redraw(now);
        }
    }

    fn redraw(&mut self, now: Instant) {
        self.dialog.draw(&mut self.canvas, &self.font, now);
        self.caret_drawn = self.dialog.caret_on(now);
        self.window.present(&self.canvas);
    }
}
