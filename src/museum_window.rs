//! The Skin Museum dialog's OS window (see [`crate::dialog_window`]), and the threads that fetch
//! what the dialog asks for ([`crate::museum_dialog::Request`]), so that the network never holds
//! up the interface. A skin to try on is fetched and read on its thread too, and handed to the
//! app to wear.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use anyhow::{Result, bail};
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::WindowId;

use crate::app::{EditShortcut, edit_shortcut};
use crate::canvas::{Image, Rect};
use crate::clipboard;
use crate::dialog_window::{DialogWindow, beside, window_size};
use crate::font::TtfFont;
use crate::menu;
use crate::museum::{self, Cache, Listing, MuseumSkin, Page};
use crate::museum_dialog::{self, Arrival, Key, Listings, Museum, Outcome, Request};
use crate::skin::Skin;

/// Largest screenshot side accepted, in pixels; the museum's are 275 x 348.
const MAX_SCREENSHOT_SIDE: u32 = 1024;
/// From NeoAmp's windows to the dialog beside them, at 96 DPI.
const GAP: f64 = 8.0;

/// A skin from the museum, being tried on.
#[derive(Debug)]
pub(crate) struct Tried {
    /// The museum's identifier for it.
    pub md5: String,
    pub skin: Box<Skin>,
    /// Where its file is kept in the cache.
    pub file: PathBuf,
    /// The file name to install it as.
    pub file_name: String,
}

/// What the app must do after an event or a tick.
#[derive(Debug)]
pub(crate) enum Event {
    None,
    /// Wear `tried`, to try it on; with `install`, keep it too.
    Wear {
        tried: Tried,
        install: bool,
    },
    /// Keep the skin being tried on, the one with this MD5.
    Install(String),
    Close,
}

/// What a fetch brought back.
enum Done {
    Page {
        listing: Listing,
        offset: usize,
        page: Result<Page, String>,
    },
    Screenshot {
        md5: String,
        picture: Result<Image, String>,
    },
    Skin {
        skin: MuseumSkin,
        wsz: Result<(Box<Skin>, PathBuf), String>,
    },
}

impl Done {
    /// What comes back for `request` when it cannot be fetched at all.
    fn failed(request: Request, why: &str) -> Done {
        match request {
            Request::Page { listing, offset } => Done::Page {
                listing,
                offset,
                page: Err(why.to_owned()),
            },
            Request::Screenshot(skin) => Done::Screenshot {
                md5: skin.md5,
                picture: Err(why.to_owned()),
            },
            Request::Skin(skin) => Done::Skin {
                skin,
                wsz: Err(why.to_owned()),
            },
        }
    }
}

/// Fetch what `request` asks for, giving up when `cancel` says so.
fn fetch(request: Request, cache: &Cache, cancel: &dyn Fn() -> bool) -> Done {
    let why = |e: anyhow::Error| format!("{e:#}");
    match request {
        Request::Page { listing, offset } => {
            let page = museum::fetch_page(&listing, offset, cancel).map_err(why);
            Done::Page {
                listing,
                offset,
                page,
            }
        }
        Request::Screenshot(skin) => {
            let picture = museum::screenshot(&skin, cache, cancel)
                .and_then(|png| decode_screenshot(&png))
                .map_err(why);
            Done::Screenshot {
                md5: skin.md5,
                picture,
            }
        }
        Request::Skin(skin) => {
            let wsz = museum::download(&skin, cache, cancel)
                .and_then(|(bytes, file)| {
                    Ok((Box::new(Skin::from_wsz(&bytes, skin.name())?), file))
                })
                .map_err(why);
            Done::Skin { skin, wsz }
        }
    }
}

/// A screenshot's pixels, from its PNG file.
fn decode_screenshot(png: &[u8]) -> Result<Image> {
    let reader = || {
        let mut r = image::ImageReader::new(std::io::Cursor::new(png));
        r.set_format(image::ImageFormat::Png);
        r
    };
    // The size first, so that a small file claiming a huge picture is never decoded.
    let (w, h) = reader().into_dimensions()?;
    if w == 0 || h == 0 || w > MAX_SCREENSHOT_SIDE || h > MAX_SCREENSHOT_SIDE {
        bail!("a screenshot of {w}x{h}");
    }
    let picture = reader().decode()?.into_rgba8();
    let (w, h) = (
        i32::try_from(picture.width())?,
        i32::try_from(picture.height())?,
    );
    Image::from_rgba(w, h, picture.into_raw()).ok_or_else(|| anyhow::anyhow!("a broken screenshot"))
}

/// The dialog and its window.
pub(crate) struct MuseumWindow {
    window: DialogWindow,
    canvas: Image,
    font: TtfFont,
    museum: Museum,
    /// NeoAmp's zoom; the dialog's scale is this times the desktop's.
    zoom: f64,
    cursor: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    focused: bool,
    /// Whether the caret showed in the last frame drawn.
    caret_drawn: bool,
    /// Wheel travel not yet turned into whole notches.
    wheel_rest: f64,
    cache: Cache,
    /// Where fetches send what they brought back.
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
    /// Set when the window closes, so that fetches still running give up.
    closed: Arc<AtomicBool>,
}

impl std::fmt::Debug for MuseumWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MuseumWindow")
            .field("museum", &self.museum)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

impl Drop for MuseumWindow {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

impl MuseumWindow {
    /// Open the dialog at `zoom` on a desktop scaled by `scale_factor`, beside `stack` (NeoAmp's
    /// windows, on `screen`) if it is known and there is room, listing what `listings` holds
    /// and keeping what it fetches in `cache`.
    ///
    /// # Errors
    /// Fails if no font is found or the window cannot be created.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open(
        el: &ActiveEventLoop,
        zoom: f64,
        scale_factor: f64,
        on_top: bool,
        stack: Option<(Rect, Rect)>,
        listings: Listings,
        cache: Cache,
        now: Instant,
    ) -> Result<MuseumWindow> {
        let scale = zoom * scale_factor;
        let font = TtfFont::load_menu(menu::font_px(scale));
        let museum = Museum::new(scale, &font, listings, now);
        let (w, h) = museum.size();
        // The gap is small and the scale a few times 1 at most, so the cast cannot truncate.
        let gap = (GAP * scale).round() as i32;
        let at = stack.and_then(|(stack, screen)| beside(stack, screen, (w, h), gap));
        let window = DialogWindow::open(el, museum_dialog::TITLE, (w, h), on_top, at)?;
        let (done_tx, done_rx) = mpsc::channel();
        log::debug!(
            "the Skin Museum keeps its downloads in {}",
            cache.dir().display()
        );
        Ok(MuseumWindow {
            window,
            canvas: Image::new(w, h),
            font,
            museum,
            zoom,
            cursor: PhysicalPosition::new(-1.0, -1.0),
            modifiers: ModifiersState::empty(),
            focused: false,
            caret_drawn: false,
            wheel_rest: 0.0,
            cache,
            done_tx,
            done_rx,
            closed: Arc::new(AtomicBool::new(false)),
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

    /// What has been listed, to keep for the next time the dialog opens.
    pub(crate) fn take_listings(&mut self) -> Listings {
        self.museum.take_listings()
    }

    /// NeoAmp no longer wears the skin being tried on.
    pub(crate) fn dropped(&mut self, now: Instant) {
        self.museum.dropped();
        self.redraw(now);
    }

    fn pointer(&self) -> (i32, i32) {
        // Window coordinates are far inside `i32`; the casts only drop the fraction.
        (self.cursor.x.floor() as i32, self.cursor.y.floor() as i32)
    }

    /// What the dialog's outcome asks of the app.
    fn event(&self, outcome: Outcome) -> Event {
        match outcome {
            Outcome::Install => self
                .museum
                .highlighted()
                .map_or(Event::None, |s| Event::Install(s.md5.clone())),
            Outcome::Close => Event::Close,
            Outcome::None | Outcome::Redraw => Event::None,
        }
    }

    /// Handle an event for the dialog's window.
    pub(crate) fn window_event(&mut self, event: WindowEvent, now: Instant) -> Event {
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
                self.museum.pointer_moved(x, y)
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.pointer();
                match state {
                    ElementState::Pressed => self.museum.pointer_down(x, y, now, &self.font),
                    ElementState::Released => self.museum.pointer_up(x, y),
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel_rest += match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    // A notch is about three rows.
                    MouseScrollDelta::PixelDelta(p) => p.y / (42.0 * self.museum.scale()),
                };
                let notches = self.wheel_rest.trunc();
                self.wheel_rest -= notches;
                // Whole notches from one event: a handful at most.
                let notches = notches as i32;
                if notches == 0 {
                    Outcome::None
                } else {
                    self.museum.wheel(notches)
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
                        Some(text) => self.museum.text(text.trim(), now),
                        None => Outcome::None,
                    },
                    (Some(EditShortcut::SelectAll), ..) => self.museum.select_all(now),
                    (None, Some(key), _) => self.museum.key(key, now),
                    (None, None, Some(text)) if !shortcut => self.museum.text(text, now),
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
                self.museum.rescale(scale, &self.font);
                let (w, h) = self.museum.size();
                self.canvas = Image::new(w, h);
                if let Err(e) = inner_size_writer.request_inner_size(window_size((w, h))) {
                    log::warn!("cannot resize the dialog for the new scale: {e}");
                }
                Outcome::Redraw
            }
            WindowEvent::RedrawRequested => Outcome::Redraw,
            _ => Outcome::None,
        };
        if outcome != Outcome::None {
            self.redraw(now);
        }
        self.event(outcome)
    }

    /// Call every frame: take in what fetches brought back, start the fetches the dialog wants
    /// now, list a search once typing pauses, and blink the caret.
    pub(crate) fn tick(&mut self, now: Instant) -> Event {
        let mut redraw = self.museum.tick(now);
        let mut event = Event::None;
        while let Ok(done) = self.done_rx.try_recv() {
            match done {
                Done::Page {
                    listing,
                    offset,
                    page,
                } => redraw |= self.museum.page_arrived(&listing, offset, page),
                Done::Screenshot { md5, picture } => {
                    redraw |= self.museum.screenshot_arrived(&md5, picture);
                }
                Done::Skin { skin, wsz } => {
                    redraw = true;
                    let (read, wsz) = match wsz {
                        Ok(wsz) => (Ok(()), Some(wsz)),
                        Err(why) => (Err(why), None),
                    };
                    let arrival = self.museum.skin_arrived(&skin.md5, read);
                    if let (Arrival::Wear | Arrival::Install, Some((worn, file))) = (arrival, wsz) {
                        log::info!("trying on {} from the Skin Museum", skin.name());
                        event = Event::Wear {
                            tried: Tried {
                                file_name: skin.file_name(),
                                md5: skin.md5,
                                skin: worn,
                                file,
                            },
                            install: arrival == Arrival::Install,
                        };
                    }
                }
            }
        }
        while let Some(request) = self.museum.next_request(now) {
            self.start(request);
            redraw = true;
        }
        if redraw || self.museum.caret_on(now) != self.caret_drawn {
            self.redraw(now);
        }
        event
    }

    /// Fetch what `request` asks for on a thread of its own.
    fn start(&self, request: Request) {
        log::debug!("the Skin Museum fetches {request:?}");
        let tx = self.done_tx.clone();
        let closed = Arc::clone(&self.closed);
        let cache = self.cache.clone();
        let asked = request.clone();
        let spawned = thread::Builder::new()
            .name("skin museum".into())
            .spawn(move || {
                let cancel = || closed.load(Ordering::Relaxed);
                // The receiver is gone if the dialog has closed.
                let _ = tx.send(fetch(request, &cache, &cancel));
            });
        if let Err(e) = spawned {
            log::error!("cannot start fetching from the Skin Museum: {e}");
            let _ = self.done_tx.send(Done::failed(asked, &e.to_string()));
        }
    }

    fn redraw(&mut self, now: Instant) {
        self.museum.draw(&mut self.canvas, &self.font, now);
        self.caret_drawn = self.museum.caret_on(now);
        self.window.present(&self.canvas);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_are_decoded_and_oversized_ones_refused() {
        let encode = |w: u32, h: u32| {
            let mut png = std::io::Cursor::new(Vec::new());
            image::RgbaImage::new(w, h)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            png.into_inner()
        };
        let shot = decode_screenshot(&encode(275, 348)).unwrap();
        assert_eq!((shot.w, shot.h), (275, 348));
        assert!(decode_screenshot(&encode(2000, 10)).is_err());
        assert!(decode_screenshot(b"not a png").is_err());
    }

    #[test]
    fn what_cannot_be_fetched_comes_back_failed() {
        let listing = Listing::Search("x".into());
        let request = Request::Page {
            listing: listing.clone(),
            offset: 50,
        };
        assert!(matches!(
            Done::failed(request, "no threads"),
            Done::Page { listing: l, offset: 50, page: Err(why) } if l == listing && why == "no threads"
        ));
    }
}
