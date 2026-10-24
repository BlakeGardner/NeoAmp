//! The OS window of the dialog that tells of a new release (see [`crate::dialog_window`]).

use anyhow::Result;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key as WinitKey, NamedKey};
use winit::window::WindowId;

use crate::canvas::Image;
use crate::dialog_window::{DialogWindow, window_size};
use crate::font::TtfFont;
use crate::menu;
use crate::update::Version;
use crate::update_dialog::{Key, Outcome, TITLE, UpdateDialog};

/// The dialog and its window.
pub(crate) struct UpdateWindow {
    window: DialogWindow,
    canvas: Image,
    font: TtfFont,
    dialog: UpdateDialog,
    /// NeoAmp's zoom; the dialog's scale is this times the desktop's.
    zoom: f64,
    cursor: PhysicalPosition<f64>,
    focused: bool,
}

impl std::fmt::Debug for UpdateWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateWindow")
            .field("dialog", &self.dialog)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

impl UpdateWindow {
    /// Open the dialog telling of NeoAmp `version`, at `zoom` on a desktop scaled by
    /// `scale_factor`.
    ///
    /// # Errors
    /// Fails if no font is found or the window cannot be created.
    pub(crate) fn open(
        el: &ActiveEventLoop,
        version: Version,
        zoom: f64,
        scale_factor: f64,
        on_top: bool,
    ) -> Result<UpdateWindow> {
        let scale = zoom * scale_factor;
        let font = TtfFont::load_menu(menu::font_px(scale));
        let dialog = UpdateDialog::new(version, scale, &font);
        let (w, h) = dialog.size();
        let window = DialogWindow::open(el, TITLE, (w, h), on_top, None)?;
        Ok(UpdateWindow {
            window,
            canvas: Image::new(w, h),
            font,
            dialog,
            zoom,
            cursor: PhysicalPosition::new(-1.0, -1.0),
            focused: false,
        })
    }

    pub(crate) fn id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn has_focus(&self) -> bool {
        self.focused
    }

    /// The release the dialog tells of.
    pub(crate) fn version(&self) -> Version {
        self.dialog.version()
    }

    fn pointer(&self) -> (i32, i32) {
        // Window coordinates are far inside `i32`; the casts only drop the fraction.
        (self.cursor.x.floor() as i32, self.cursor.y.floor() as i32)
    }

    /// Handle an event for the dialog's window.
    pub(crate) fn window_event(&mut self, event: WindowEvent) -> Outcome {
        let outcome = match event {
            WindowEvent::CloseRequested => Outcome::Close,
            WindowEvent::Focused(f) => {
                self.focused = f;
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
                    ElementState::Pressed => self.dialog.pointer_down(x, y),
                    ElementState::Released => self.dialog.pointer_up(x, y),
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match &event.logical_key {
                    WinitKey::Named(NamedKey::Enter) => self.dialog.key(Key::Enter),
                    WinitKey::Named(NamedKey::Escape) => self.dialog.key(Key::Escape),
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
                self.dialog.rescale(scale, &self.font);
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
            self.redraw();
        }
        outcome
    }

    fn redraw(&mut self) {
        self.dialog.draw(&mut self.canvas, &self.font);
        self.window.present(&self.canvas);
    }
}
