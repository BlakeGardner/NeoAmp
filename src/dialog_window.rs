//! The OS window of a dialog drawn like Windows 2000's (Jump to file, File info). Unlike the
//! menus, a dialog is an ordinary window with the system's title bar, as Winamp's dialogs were,
//! so the window manager places it, moves it and gives it the keyboard. It is drawn in software
//! with softbuffer, like the menus.

use std::num::NonZeroU32;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use softbuffer::{Context, Surface};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::window::{Window, WindowId, WindowLevel};

use crate::canvas::{Image, Rect};
use crate::classic::FACE;

/// A dialog's window and the surface it is drawn on.
pub(crate) struct DialogWindow {
    surface: Surface<OwnedDisplayHandle, Arc<Window>>,
}

impl std::fmt::Debug for DialogWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogWindow")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// A window dimension from a canvas one: at least a pixel.
pub(crate) fn dimension(v: i32) -> NonZeroU32 {
    NonZeroU32::new(v.max(1).unsigned_abs()).unwrap_or(NonZeroU32::MIN)
}

fn dimension_u32(v: u32) -> NonZeroU32 {
    NonZeroU32::new(v).unwrap_or(NonZeroU32::MIN)
}

/// `(w, h)` as a window size.
pub(crate) fn window_size((w, h): (i32, i32)) -> PhysicalSize<u32> {
    PhysicalSize::new(dimension(w).get(), dimension(h).get())
}

/// Where a dialog `size` pixels inside goes beside `stack`, the windows it belongs with, on
/// `screen`, `gap` pixels away: to the right, or else to the left, with the top edges level.
/// `None` when it fits on neither side.
pub(crate) fn beside(
    stack: Rect,
    screen: Rect,
    (w, _): (i32, i32),
    gap: i32,
) -> Option<PhysicalPosition<i32>> {
    let right = stack.right() + gap;
    let left = stack.x - gap - w;
    let x = if right + w <= screen.right() {
        right
    } else if left >= screen.x {
        left
    } else {
        return None;
    };
    Some(PhysicalPosition::new(x, stack.y.max(screen.y)))
}

impl DialogWindow {
    /// Open a window titled `title`, `size` pixels inside, above other applications' windows
    /// if `on_top`, with its top-left corner `at` that position on the desktop, or wherever
    /// the window manager puts it.
    ///
    /// # Errors
    /// Fails if the window or its surface cannot be created.
    pub(crate) fn open(
        el: &ActiveEventLoop,
        title: &str,
        size: (i32, i32),
        on_top: bool,
        at: Option<PhysicalPosition<i32>>,
    ) -> Result<DialogWindow> {
        let level = if on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_resizable(false)
            .with_window_level(level)
            .with_inner_size(window_size(size));
        if let Some(at) = at {
            attrs = attrs.with_position(at);
        }
        let attrs = crate::window_icon::with_icon(attrs, el);
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
        Ok(DialogWindow { surface })
    }

    pub(crate) fn id(&self) -> WindowId {
        self.surface.window().id()
    }

    pub(crate) fn window(&self) -> &Window {
        self.surface.window()
    }

    /// The window was resized to `size`.
    pub(crate) fn resized(&mut self, size: PhysicalSize<u32>) {
        if let Err(e) = self
            .surface
            .resize(dimension_u32(size.width), dimension_u32(size.height))
        {
            log::warn!("cannot resize the dialog: {e}");
        }
    }

    /// Show `canvas` in the window.
    pub(crate) fn present(&mut self, canvas: &Image) {
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
        let cw = usize::try_from(canvas.w).unwrap_or(0);
        for (y, row) in buffer.chunks_mut(bw.max(1)).enumerate() {
            let src = canvas.data.get(y * cw * 4..(y + 1) * cw * 4).unwrap_or(&[]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_goes_right_of_the_windows_or_else_left_or_where_it_is_put() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let at = |stack, w| beside(stack, screen, (w, 420), 8).map(|p| (p.x, p.y));
        assert_eq!(at(Rect::new(64, 64, 275, 348), 600), Some((347, 64)));
        assert_eq!(at(Rect::new(1500, 64, 275, 348), 600), Some((892, 64)));
        assert_eq!(
            at(Rect::new(500, 64, 900, 348), 600),
            None,
            "no room either side"
        );
        // A stack poking above the screen does not take the dialog with it.
        assert_eq!(at(Rect::new(64, -40, 275, 348), 600), Some((347, 0)));
    }
}
