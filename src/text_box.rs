//! A one-line edit box drawn like Windows 2000's, for the dialogs: typed and pasted text, a
//! blinking caret, the arrow keys, Home and End, Backspace and Delete. Selection is all or
//! nothing (Ctrl+A, or text filled in for the user), and typing over it replaces it.
//!
//! Geometry is in physical pixels, as in [`crate::jump`]: the box is given as a rectangle and
//! a scale, and text is measured with the font it is drawn in.

use std::time::Instant;

use crate::canvas::{Image, Rect};
use crate::classic::{self, SELECTION, SELECTION_TEXT, TEXT, WINDOW, px};
use crate::font::TtfFont;
use crate::menu::Measure;

/// The caret is shown for this long, then hidden for as long (Windows' default).
const CARET_BLINK_MS: u128 = 530;
/// From the box's edge to its text, at 96 DPI.
const TEXT_PAD: i32 = 4;

/// A key that edits, or moves the caret.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
}

/// The edit box's text and caret.
#[derive(Clone, Debug)]
pub struct TextBox {
    text: String,
    /// The caret's byte offset in `text`.
    caret: usize,
    /// All the text is selected.
    all: bool,
    /// The caret blinks from here, so it shows at once after typing.
    blink_from: Instant,
}

impl TextBox {
    /// An empty box.
    #[must_use]
    pub fn new(now: Instant) -> TextBox {
        TextBox {
            text: String::new(),
            caret: 0,
            all: false,
            blink_from: now,
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether all the text is selected, so that typing replaces it.
    #[must_use]
    pub fn is_all_selected(&self) -> bool {
        self.all
    }

    /// Select all the text (Ctrl+A).
    pub fn select_all(&mut self, now: Instant) {
        self.all = !self.text.is_empty();
        self.caret = self.text.len();
        self.blink_from = now;
    }

    /// Type or paste `text` at the caret, in place of the selection if all is selected.
    /// Control characters, line breaks among them, are left out. Returns whether the text
    /// changed.
    pub fn insert(&mut self, text: &str, now: Instant) -> bool {
        let typed: String = text.chars().filter(|c| !c.is_control()).collect();
        if typed.is_empty() {
            return false;
        }
        if std::mem::take(&mut self.all) {
            self.text.clear();
            self.caret = 0;
        }
        self.text.insert_str(self.caret, &typed);
        self.caret += typed.len();
        self.blink_from = now;
        true
    }

    /// Move the caret or delete. Returns whether the text changed.
    pub fn edit(&mut self, key: Edit, now: Instant) -> bool {
        self.blink_from = now;
        if std::mem::take(&mut self.all) {
            // The selection goes; moving lands at its start or end, as in Windows.
            match key {
                Edit::Backspace | Edit::Delete => {
                    self.text.clear();
                    self.caret = 0;
                    return true;
                }
                Edit::Left | Edit::Home => self.caret = 0,
                Edit::Right | Edit::End => self.caret = self.text.len(),
            }
            return false;
        }
        let t = &self.text;
        let before = t[..self.caret]
            .chars()
            .next_back()
            .map_or(0, char::len_utf8);
        let after = t[self.caret..].chars().next().map_or(0, char::len_utf8);
        let (from, to) = match key {
            Edit::Left => {
                self.caret -= before;
                return false;
            }
            Edit::Right => {
                self.caret += after;
                return false;
            }
            Edit::Home => {
                self.caret = 0;
                return false;
            }
            Edit::End => {
                self.caret = self.text.len();
                return false;
            }
            Edit::Backspace => (self.caret - before, self.caret),
            Edit::Delete => (self.caret, self.caret + after),
        };
        if from == to {
            return false;
        }
        self.text.replace_range(from..to, "");
        self.caret = from;
        true
    }

    /// Whether the caret shows at `now`.
    #[must_use]
    pub fn caret_on(&self, now: Instant) -> bool {
        let ms = now.saturating_duration_since(self.blink_from).as_millis();
        (ms / CARET_BLINK_MS).is_multiple_of(2)
    }

    /// Where the text starts in box `r` at `scale`, before scrolling.
    #[must_use]
    pub fn text_x(r: Rect, scale: f64) -> i32 {
        r.x + px(TEXT_PAD, scale)
    }

    /// How far the text is shifted left to keep the caret in box `r`.
    fn shift(&self, r: Rect, scale: f64, m: &dyn Measure) -> i32 {
        let room = r.w - 2 * px(TEXT_PAD, scale);
        (m.text_width(&self.text[..self.caret]) - room).max(0)
    }

    /// A click at `x` in box `r`: the caret goes to the nearest place between characters.
    pub fn click(&mut self, x: i32, r: Rect, scale: f64, m: &dyn Measure, now: Instant) {
        let start = TextBox::text_x(r, scale) - self.shift(r, scale, m);
        let mut best = (0, (x - start).abs());
        for (at, ch) in self.text.char_indices() {
            let end = at + ch.len_utf8();
            let d = (x - start - m.text_width(&self.text[..end])).abs();
            if d < best.1 {
                best = (end, d);
            }
        }
        self.caret = best.0;
        self.all = false;
        self.blink_from = now;
    }

    /// Draw the box in `r`, `t` pixels to a line, at `scale`, with `font` (the menu font at
    /// this scale).
    pub fn draw(&self, c: &mut Image, font: &TtfFont, r: Rect, t: i32, scale: f64, now: Instant) {
        c.fill_rect(r, WINDOW);
        classic::sunken(c, r, t);
        let inner = Rect::new(r.x + 2 * t, r.y + 2 * t, r.w - 4 * t, r.h - 4 * t);
        let lh = font.line_height();
        let y = r.y + (r.h - lh) / 2;
        let x = TextBox::text_x(r, scale) - self.shift(r, scale, font);
        if self.all {
            let selected = Rect::new(x, y, font.width(&self.text), lh);
            c.fill_rect_clipped(selected, SELECTION, inner);
            font.draw(c, &self.text, x, y, SELECTION_TEXT, inner);
            return;
        }
        font.draw(c, &self.text, x, y, TEXT, inner);
        if self.caret_on(now) {
            let cx = x + font.width(&self.text[..self.caret]);
            c.fill_rect_clipped(Rect::new(cx, y, t, lh), TEXT, inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 6 pixels wide.
    struct Fixed;

    impl Measure for Fixed {
        fn text_width(&self, text: &str) -> i32 {
            i32::try_from(text.chars().count()).unwrap_or(0) * 6
        }
    }

    #[test]
    fn the_caret_moves_and_deletes_by_character() {
        let now = Instant::now();
        let mut b = TextBox::new(now);
        assert!(b.insert("añb", now));
        assert!(!b.edit(Edit::Left, now));
        assert!(!b.edit(Edit::Left, now));
        assert!(b.edit(Edit::Delete, now));
        assert_eq!(b.text(), "ab");
        assert!(!b.edit(Edit::Home, now));
        assert!(!b.edit(Edit::Backspace, now), "nothing before the caret");
        assert!(b.insert("x", now));
        assert!(!b.edit(Edit::End, now));
        assert!(!b.edit(Edit::Right, now));
        assert!(b.insert("!\r\n", now));
        assert_eq!(b.text(), "xab!", "line breaks are left out");
        assert!(!b.insert("\u{8}", now));
    }

    #[test]
    fn a_selection_is_replaced_by_typing_and_cleared_by_deleting() {
        let now = Instant::now();
        let mut b = TextBox::new(now);
        b.select_all(now);
        assert!(!b.is_all_selected(), "nothing to select");
        let _ = b.insert("http://old.example/", now);
        b.select_all(now);
        assert!(b.insert("https://new.example/", now));
        assert_eq!(b.text(), "https://new.example/");
        b.select_all(now);
        assert!(!b.edit(Edit::Home, now));
        assert!(!b.is_all_selected());
        assert!(b.insert("x", now));
        assert!(b.text().starts_with("xhttps"));
        b.select_all(now);
        assert!(b.edit(Edit::Backspace, now));
        assert_eq!(b.text(), "");
    }

    #[test]
    fn a_click_places_the_caret_and_drops_the_selection() {
        let now = Instant::now();
        let mut b = TextBox::new(now);
        let _ = b.insert("abcd", now);
        b.select_all(now);
        let r = Rect::new(10, 0, 200, 20);
        b.click(TextBox::text_x(r, 1.0) + 2 * 6 + 1, r, 1.0, &Fixed, now);
        assert!(!b.is_all_selected());
        let _ = b.insert("-", now);
        assert_eq!(b.text(), "ab-cd");
    }
}
