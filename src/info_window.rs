//! The File info dialog's OS window (see [`crate::dialog_window`]). The file is read on a
//! thread of its own, so a slow disk never holds up the interface: the dialog opens at once and
//! fills in when the file has been read. What it shows about a stream comes from the app,
//! which hears it from the audio engine as the stream plays.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::Result;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key as WinitKey, NamedKey};
use winit::window::WindowId;

use crate::audio::{self, Location};
use crate::canvas::Image;
use crate::dialog_window::{DialogWindow, window_size};
use crate::font::TtfFont;
use crate::info::{self, Contents, Info, Key, Outcome, StreamDetails};
use crate::menu;

/// The dialog and its window.
pub(crate) struct InfoWindow {
    window: DialogWindow,
    canvas: Image,
    font: TtfFont,
    info: Info,
    /// NeoAmp's zoom; the dialog's scale is this times the desktop's.
    zoom: f64,
    cursor: PhysicalPosition<f64>,
    focused: bool,
    /// Where the file's details arrive, while it is being read.
    reading: Option<Receiver<Contents>>,
}

impl std::fmt::Debug for InfoWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InfoWindow")
            .field("info", &self.info)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

/// Read the file at `path` on a thread of its own; its details arrive on the receiver.
fn read(path: PathBuf) -> Receiver<Contents> {
    let (tx, rx) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("file info".into())
        .spawn(move || {
            let contents = match audio::details(&path) {
                Ok(d) => Contents::Read(Box::new(d)),
                Err(e) => Contents::Failed(format!("{e:#}")),
            };
            // The receiver is gone if the dialog closed or moved on to another file.
            let _ = tx.send(contents);
        });
    if let Err(e) = spawned {
        log::error!("cannot start reading the file: {e}");
    }
    rx
}

/// What the dialog shows first about `location`, and where a file's details will arrive: a
/// file is read on a thread of its own, and a stream shows `stream` (nothing yet, if `None`).
fn start(
    location: &Location,
    stream: Option<StreamDetails>,
) -> (Contents, Option<Receiver<Contents>>) {
    match location {
        Location::File(path) => (Contents::Reading, Some(read(path.clone()))),
        Location::Url(_) => (Contents::Stream(Box::new(stream.unwrap_or_default())), None),
    }
}

impl InfoWindow {
    /// Open the dialog about a file or a stream (showing `stream` about a stream), at `zoom`
    /// on a desktop scaled by `scale_factor`.
    ///
    /// # Errors
    /// Fails if no font is found or the window cannot be created.
    pub(crate) fn open(
        el: &ActiveEventLoop,
        zoom: f64,
        scale_factor: f64,
        on_top: bool,
        location: Location,
        stream: Option<StreamDetails>,
    ) -> Result<InfoWindow> {
        let scale = zoom * scale_factor;
        let font = TtfFont::load_menu(menu::font_px(scale));
        let (contents, reading) = start(&location, stream);
        let mut info = Info::new(scale, &font, location);
        info.set_contents(contents);
        let (w, h) = info.size();
        let window = DialogWindow::open(el, info::TITLE, (w, h), on_top)?;
        Ok(InfoWindow {
            window,
            canvas: Image::new(w, h),
            font,
            info,
            zoom,
            cursor: PhysicalPosition::new(-1.0, -1.0),
            focused: false,
            reading,
        })
    }

    pub(crate) fn id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn has_focus(&self) -> bool {
        self.focused
    }

    /// Show another file or stream instead (showing `stream` about a stream), and bring the
    /// dialog to the front with the keyboard focus.
    pub(crate) fn show(&mut self, location: Location, stream: Option<StreamDetails>) {
        if &location != self.info.location() {
            let (contents, reading) = start(&location, stream);
            self.reading = reading;
            self.info.show(location);
            self.info.set_contents(contents);
            self.redraw();
        }
        self.window.window().focus_window();
    }

    /// The stream at `url` has told more about itself: show it, if the dialog is about it.
    pub(crate) fn update_stream(&mut self, url: &str, stream: StreamDetails) {
        if self.info.location().url() != Some(url) {
            return;
        }
        let contents = Contents::Stream(Box::new(stream));
        if self.info.contents() != &contents {
            self.info.set_contents(contents);
            self.redraw();
        }
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
                self.info.pointer_moved(x, y)
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.pointer();
                match state {
                    ElementState::Pressed => self.info.pointer_down(x, y),
                    ElementState::Released => self.info.pointer_up(x, y),
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let key = match &event.logical_key {
                    WinitKey::Named(NamedKey::Enter) => Some(Key::Enter),
                    WinitKey::Named(NamedKey::Escape) => Some(Key::Escape),
                    WinitKey::Named(NamedKey::Space) => Some(Key::Space),
                    _ => None,
                };
                key.map_or(Outcome::None, |k| self.info.key(k))
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
                self.info.rescale(scale, &self.font);
                let (w, h) = self.info.size();
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

    /// Call every frame: show the file's details once they have been read.
    pub(crate) fn tick(&mut self) {
        let Some(rx) = &self.reading else {
            return;
        };
        let contents = match rx.try_recv() {
            Ok(contents) => contents,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Contents::Failed("the file could not be read".to_owned())
            }
        };
        self.reading = None;
        self.info.set_contents(contents);
        self.redraw();
    }

    fn redraw(&mut self) {
        self.info.draw(&mut self.canvas, &self.font);
        self.window.present(&self.canvas);
    }
}
