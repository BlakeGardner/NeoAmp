//! The Open URL dialog (Ctrl+L, Play URL... in the main menu) and its twin Add URL (the
//! playlist's ADD > URL): a box for the address of an internet radio stream, a playlist
//! leading to one, or a file on the web, drawn like the Windows 2000 dialogs Winamp 2.x
//! showed. Open replaces the playlist and plays; Add adds to the end of the playlist.
//!
//! This is the dialog's state, layout, input and drawing; `url_window` gives it an OS window
//! and the clipboard. Geometry is in physical pixels, as in [`crate::jump`].

use std::path::Path;
use std::time::Instant;

use crate::audio::{Location, is_web_url};
use crate::canvas::{Image, Rect};
use crate::classic::{self, FACE, TEXT, line, px};
use crate::font::TtfFont;
use crate::menu::Measure;
use crate::playlist_file;
use crate::text_box::{Edit, TextBox};

/// Sizes at 96 DPI, which `px` scales.
mod metric {
    pub const W: i32 = 380;
    pub const MARGIN: i32 = 8;
    pub const GAP: i32 = 6;
    /// The prompt's line.
    pub const LABEL_H: i32 = 14;
    pub const EDIT_H: i32 = 21;
    pub const BUTTON_W: i32 = 75;
    pub const BUTTON_H: i32 = 23;
}

const PROMPT: &str = "Enter the URL of a stream or of a file on the web:";
const CANCEL: &str = "Cancel";

/// What the address is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    /// Replace the playlist with it and play it.
    Play,
    /// Add it to the end of the playlist.
    Add,
}

impl Purpose {
    /// The window's title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Purpose::Play => "Open URL",
            Purpose::Add => "Add URL",
        }
    }

    fn button(self) -> &'static str {
        match self {
            Purpose::Play => "Open",
            Purpose::Add => "Add",
        }
    }
}

/// What a key does in the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Edit(Edit),
    Enter,
    Escape,
}

/// What the window layer must do after an input.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    None,
    Redraw,
    /// Open (or add) this and close the dialog.
    Open(Location),
    Close,
}

/// A part of the dialog the pointer can press.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Edit,
    OpenButton,
    CancelButton,
}

/// Where everything is, in the dialog's pixels.
#[derive(Clone, Copy, Debug)]
struct Layout {
    size: (i32, i32),
    /// One-pixel lines at this scale.
    t: i32,
    prompt: (i32, i32),
    edit: Rect,
    open: Rect,
    cancel: Rect,
}

impl Layout {
    fn new(scale: f64) -> Layout {
        let p = |v| px(v, scale);
        let w = p(metric::W);
        let margin = p(metric::MARGIN);
        let edit_y = margin + p(metric::LABEL_H) + p(metric::GAP) / 2;
        let edit = Rect::new(margin, edit_y, w - 2 * margin, p(metric::EDIT_H));
        let button = (p(metric::BUTTON_W), p(metric::BUTTON_H));
        let buttons_y = edit.bottom() + p(metric::GAP) + p(metric::GAP);
        let cancel = Rect::new(w - margin - button.0, buttons_y, button.0, button.1);
        let open = cancel.offset(-(button.0 + p(metric::GAP)), 0);
        Layout {
            size: (w, cancel.bottom() + margin),
            t: line(scale),
            prompt: (margin, margin),
            edit,
            open,
            cancel,
        }
    }
}

/// What was typed, as a playlist entry: an `http://` or `https://` URL as it is, a
/// `file://` URL or an absolute path as a file, any other URL as it is (it fails when played,
/// saying why), and anything else as a web address without its `http://` (`radio.example:8000`).
/// `None` for nothing but spaces.
#[must_use]
pub fn location(text: &str) -> Option<Location> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if is_web_url(text) {
        return Some(Location::Url(text.to_owned()));
    }
    let file_url = text
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"));
    if file_url {
        return playlist_file::entry_location(text.as_bytes(), Path::new(""));
    }
    if Path::new(text).is_absolute() {
        return Some(Location::File(text.into()));
    }
    if text.contains("://") {
        return Some(Location::Url(text.to_owned()));
    }
    Some(Location::Url(format!("http://{text}")))
}

/// The dialog.
#[derive(Debug)]
pub struct UrlDialog {
    purpose: Purpose,
    scale: f64,
    layout: Layout,
    url: TextBox,
    pressed: Option<Part>,
    /// The last pointer position: a held button shows pushed in only while under it.
    pointer: (i32, i32),
}

impl UrlDialog {
    /// A dialog at `scale` times Windows' 96 DPI sizes, its box holding `start` (selected, so
    /// that typing replaces it), or empty.
    #[must_use]
    pub fn new(purpose: Purpose, scale: f64, start: Option<&str>, now: Instant) -> UrlDialog {
        let mut url = TextBox::new(now);
        if let Some(text) = start {
            let _ = url.insert(text, now);
            url.select_all(now);
        }
        UrlDialog {
            purpose,
            scale,
            layout: Layout::new(scale),
            url,
            pressed: None,
            pointer: (-1, -1),
        }
    }

    #[must_use]
    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    /// The window's size in pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        self.layout.size
    }

    /// Lay the dialog out again at another scale, keeping what was typed.
    pub fn rescale(&mut self, scale: f64) {
        self.scale = scale;
        self.layout = Layout::new(scale);
    }

    /// What is in the box.
    #[must_use]
    pub fn text(&self) -> &str {
        self.url.text()
    }

    fn submit(&self) -> Outcome {
        location(self.url.text()).map_or(Outcome::None, Outcome::Open)
    }

    /// Typed or pasted text.
    pub fn text_input(&mut self, text: &str, now: Instant) -> Outcome {
        if self.url.insert(text, now) {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    /// Select all of the box (Ctrl+A).
    pub fn select_all(&mut self, now: Instant) -> Outcome {
        self.url.select_all(now);
        Outcome::Redraw
    }

    /// A key that is not text.
    pub fn key(&mut self, key: Key, now: Instant) -> Outcome {
        match key {
            Key::Edit(edit) => {
                let _ = self.url.edit(edit, now);
                Outcome::Redraw
            }
            Key::Enter => self.submit(),
            Key::Escape => Outcome::Close,
        }
    }

    fn part_at(&self, x: i32, y: i32) -> Option<Part> {
        let l = &self.layout;
        [
            (l.edit, Part::Edit),
            (l.open, Part::OpenButton),
            (l.cancel, Part::CancelButton),
        ]
        .into_iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|(_, part)| part)
    }

    /// Whether `part` is held with the pointer over it, so that letting go would press it.
    fn pushed(&self, part: Part) -> bool {
        self.pressed == Some(part) && self.part_at(self.pointer.0, self.pointer.1) == Some(part)
    }

    /// The pointer pressed at (`x`, `y`).
    pub fn pointer_down(&mut self, x: i32, y: i32, now: Instant, m: &dyn Measure) -> Outcome {
        self.pointer = (x, y);
        let Some(part) = self.part_at(x, y) else {
            return Outcome::None;
        };
        if part == Part::Edit {
            self.url.click(x, self.layout.edit, self.scale, m, now);
        }
        self.pressed = Some(part);
        Outcome::Redraw
    }

    /// The pointer moved to (`x`, `y`): a held button pops out while the pointer is off it.
    pub fn pointer_moved(&mut self, x: i32, y: i32) -> Outcome {
        let held = self.pressed.filter(|p| *p != Part::Edit);
        let was = held.is_some_and(|p| self.pushed(p));
        self.pointer = (x, y);
        match held {
            Some(button) if self.pushed(button) != was => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// The pointer let go at (`x`, `y`): a button pressed and let go over runs.
    pub fn pointer_up(&mut self, x: i32, y: i32) -> Outcome {
        self.pointer = (x, y);
        let Some(part) = self.pressed.take() else {
            return Outcome::None;
        };
        let over = self.part_at(x, y) == Some(part);
        match part {
            Part::OpenButton if over => match self.submit() {
                Outcome::None => Outcome::Redraw,
                open => open,
            },
            Part::CancelButton if over => Outcome::Close,
            _ => Outcome::Redraw,
        }
    }

    /// Whether the caret shows at `now`.
    #[must_use]
    pub fn caret_on(&self, now: Instant) -> bool {
        !self.url.is_all_selected() && self.url.caret_on(now)
    }

    /// Draw the dialog into `c`, which must be [`UrlDialog::size`]. `font` must be the menu
    /// font at this scale ([`crate::menu::font_px`]).
    pub fn draw(&self, c: &mut Image, font: &TtfFont, now: Instant) {
        let l = &self.layout;
        c.clear(FACE);
        let all = c.bounds();
        font.draw(c, PROMPT, l.prompt.0, l.prompt.1, TEXT, all);
        self.url.draw(c, font, l.edit, l.t, self.scale, now);
        // Open is the default button, which Enter presses.
        for (part, r, text, default) in [
            (Part::OpenButton, l.open, self.purpose.button(), true),
            (Part::CancelButton, l.cancel, CANCEL, false),
        ] {
            classic::push_button(c, font, r, l.t, text, default, self.pushed(part));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Every character 6 pixels wide.
    struct Fixed;

    impl Measure for Fixed {
        fn text_width(&self, text: &str) -> i32 {
            i32::try_from(text.chars().count()).unwrap() * 6
        }
    }

    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn url(u: &str) -> Option<Location> {
        Some(Location::Url(u.into()))
    }

    #[test]
    fn what_is_typed_becomes_a_stream_or_a_file() {
        assert_eq!(
            location("  https://radio.example/live\n"),
            url("https://radio.example/live")
        );
        assert_eq!(
            location("radio.example:8000/;"),
            url("http://radio.example:8000/;")
        );
        assert_eq!(location("mms://old.example/x"), url("mms://old.example/x"));
        assert_eq!(location("   "), None);
        assert_eq!(
            location("file:///music/a%20b.mp3"),
            Some(Location::File(Path::new("/music/a b.mp3").into()))
        );
        let absolute = std::env::temp_dir().join("song.mp3");
        assert_eq!(
            location(&absolute.to_string_lossy()),
            Some(Location::File(absolute))
        );
    }

    #[test]
    fn a_url_from_the_clipboard_is_selected_and_typing_replaces_it() {
        let now = Instant::now();
        let mut d = UrlDialog::new(Purpose::Play, 1.0, Some("http://old.example/"), now);
        assert!(!d.caret_on(now), "selected text hides the caret");
        assert_eq!(
            d.key(Key::Enter, now),
            Outcome::Open(Location::Url("http://old.example/".into()))
        );
        let _ = d.text_input("new.example", now);
        assert_eq!(d.text(), "new.example");
        assert_eq!(
            d.key(Key::Enter, now),
            Outcome::Open(Location::Url("http://new.example".into()))
        );
        let _ = d.select_all(now);
        let _ = d.key(Key::Edit(Edit::Delete), now);
        assert_eq!(d.key(Key::Enter, now), Outcome::None, "nothing to open");
        assert_eq!(d.key(Key::Escape, now), Outcome::Close);
    }

    #[test]
    fn the_buttons_open_and_cancel() {
        let now = Instant::now();
        let mut d = UrlDialog::new(Purpose::Add, 1.5, None, now);
        let (ox, oy) = centre(d.layout.open);
        let _ = d.pointer_down(ox, oy, now, &Fixed);
        assert_eq!(d.pointer_up(ox, oy), Outcome::Redraw, "nothing typed");
        let _ = d.text_input("https://radio.example/", now);
        let _ = d.pointer_down(ox, oy, now, &Fixed);
        assert!(d.pushed(Part::OpenButton));
        assert_eq!(
            d.pointer_moved(0, 0),
            Outcome::Redraw,
            "pops out off the button"
        );
        assert_eq!(d.pointer_up(0, 0), Outcome::Redraw, "let go elsewhere");
        let _ = d.pointer_down(ox, oy, now, &Fixed);
        assert_eq!(
            d.pointer_up(ox, oy),
            Outcome::Open(Location::Url("https://radio.example/".into()))
        );
        let (cx, cy) = centre(d.layout.cancel);
        let _ = d.pointer_down(cx, cy, now, &Fixed);
        assert_eq!(d.pointer_up(cx, cy), Outcome::Close);
        assert_eq!(d.purpose().title(), "Add URL");
    }

    #[test]
    fn the_layout_fits_in_the_window() {
        for scale in [1.0, 1.5, 3.0] {
            let d = UrlDialog::new(Purpose::Play, scale, None, Instant::now());
            let l = d.layout;
            let window = Rect::new(0, 0, l.size.0, l.size.1);
            for r in [l.edit, l.open, l.cancel] {
                assert_eq!(window.intersect(&r), r, "{r:?} at {scale}");
            }
            assert!(l.open.right() < l.cancel.x);
            assert!(l.edit.bottom() < l.open.y);
        }
    }

    #[test]
    fn it_draws_its_box_and_buttons() {
        let Some(font) = TtfFont::load_menu(crate::menu::font_px(1.0)) else {
            return; // no font on this system
        };
        let now = Instant::now();
        let d = UrlDialog::new(Purpose::Play, 1.0, Some("http://radio.example/"), now);
        let (w, h) = d.size();
        let mut c = Image::new(w, h);
        d.draw(&mut c, &font, now);
        let l = d.layout;
        assert_eq!(c.get(l.edit.x + 10, l.edit.y), classic::SHADOW);
        assert_eq!(
            c.get(l.open.x, l.open.y),
            classic::FRAME,
            "the default button"
        );
        let highlighted = (l.edit.y..l.edit.bottom())
            .any(|y| (l.edit.x..l.edit.right()).any(|x| c.get(x, y) == classic::SELECTION));
        assert!(highlighted, "the URL starts out selected");
    }
}
