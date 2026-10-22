//! The Jump to file dialog (J): a search box over the playlist, drawn like the Windows 2000
//! dialog Winamp 2.x showed. Typing narrows the list to the entries whose title or file name
//! (for a stream, its URL) holds every word typed, ignoring case; Enter, a double-click or
//! the Jump button plays the highlighted entry and closes the dialog.
//!
//! This is the dialog's state, layout, input and drawing; `jump_window` gives it an OS
//! window. Geometry is in physical pixels: Windows' sizes at 96 DPI times the dialog's scale,
//! with the text drawn at that size, as for the menus.

use std::time::Instant;

use crate::audio::Location;
use crate::canvas::{Image, Rect};
use crate::classic::{self, FACE, TEXT, line, px};
use crate::font::TtfFont;
use crate::list_box::{self, ListBox};
use crate::menu::Measure;
use crate::text_box::{Edit, TextBox};
use crate::ui::{Track, push_fmt};

/// The window's title.
pub const TITLE: &str = "Jump to file";
/// Two clicks on the same row closer than this count as a double-click.
const DOUBLE_CLICK_MS: u128 = 400;

/// Sizes at 96 DPI, which `px` scales.
mod metric {
    pub const W: i32 = 340;
    pub const H: i32 = 300;
    pub const MARGIN: i32 = 8;
    pub const GAP: i32 = 6;
    pub const EDIT_H: i32 = 21;
    pub const BUTTON_W: i32 = 75;
    pub const BUTTON_H: i32 = 23;
}

const LABEL: &str = "Search:";
const JUMP: &str = "Jump";
const CLOSE: &str = "Close";

/// What a key does in the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// Move the highlight in the list.
    Up,
    Down,
    PageUp,
    PageDown,
    /// Move the caret in the search box.
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Escape,
}

/// What the window layer must do after an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    None,
    Redraw,
    /// Play playlist entry `n` and close the dialog.
    Jump(usize),
    Close,
}

/// A part of the dialog the pointer can press.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Edit,
    /// The list, rows or scroll bar.
    List,
    JumpButton,
    CloseButton,
}

/// Where everything is, in the dialog's pixels.
#[derive(Clone, Copy, Debug)]
struct Layout {
    size: (i32, i32),
    /// One-pixel lines at this scale.
    t: i32,
    label_x: i32,
    edit: Rect,
    /// The list box, border included.
    list: Rect,
    jump: Rect,
    close: Rect,
}

impl Layout {
    fn new(scale: f64, m: &dyn Measure) -> Layout {
        let p = |v| px(v, scale);
        let t = line(scale);
        let (w, h) = (p(metric::W), p(metric::H));
        let margin = p(metric::MARGIN);
        let label_w = m.text_width(LABEL);
        let edit_x = margin + label_w + p(metric::GAP);
        let edit = Rect::new(edit_x, margin, w - margin - edit_x, p(metric::EDIT_H));
        let button = (p(metric::BUTTON_W), p(metric::BUTTON_H));
        let buttons_y = h - margin - button.1;
        let close = Rect::new(w - margin - button.0, buttons_y, button.0, button.1);
        let jump = close.offset(-(button.0 + p(metric::GAP)), 0);
        let list_y = edit.bottom() + p(metric::GAP);
        let list = Rect::new(
            margin,
            list_y,
            w - 2 * margin,
            buttons_y - p(metric::GAP) - list_y,
        );
        Layout {
            size: (w, h),
            t,
            label_x: margin,
            edit,
            list,
            jump,
            close,
        }
    }
}

/// The dialog.
#[derive(Debug)]
pub struct Jump {
    scale: f64,
    layout: Layout,
    /// The search box.
    search: TextBox,
    /// The playlist entries that match, in playlist order.
    matches: Vec<usize>,
    /// One row per match.
    list: ListBox,
    /// The playlist entry to highlight once the playlist is matched the first time.
    start: Option<usize>,
    /// The playlist version `matches` was made for.
    version: Option<u64>,
    pressed: Option<Part>,
    /// The last pointer position: a held button shows pushed in only while under it.
    pointer: (i32, i32),
    last_click: Option<(usize, Instant)>,
}

impl Jump {
    /// A dialog at `scale` times Windows' 96 DPI sizes, measuring text with `m`, highlighting
    /// playlist entry `current` once it has the playlist (see [`Jump::refresh`]).
    #[must_use]
    pub fn new(scale: f64, m: &dyn Measure, current: Option<usize>, now: Instant) -> Jump {
        let layout = Layout::new(scale, m);
        Jump {
            scale,
            layout,
            search: TextBox::new(now),
            matches: Vec::new(),
            list: ListBox::new(layout.list, scale),
            start: current,
            version: None,
            pressed: None,
            pointer: (-1, -1),
            last_click: None,
        }
    }

    /// The window's size in pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        self.layout.size
    }

    #[must_use]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Lay the dialog out again at another scale, keeping what was typed.
    pub fn rescale(&mut self, scale: f64, m: &dyn Measure) {
        self.scale = scale;
        self.layout = Layout::new(scale, m);
        self.list.relayout(self.layout.list, scale);
    }

    #[must_use]
    pub fn query(&self) -> &str {
        self.search.text()
    }

    /// The playlist entries listed, in order.
    #[must_use]
    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    /// The highlighted playlist entry.
    #[must_use]
    pub fn selected_entry(&self) -> Option<usize> {
        self.matches.get(self.list.selected()?).copied()
    }

    /// Match the playlist again if it has changed since (`version`, from
    /// [`crate::ui::Ui::playlist_version`]). True if it had.
    pub fn refresh(&mut self, tracks: &[Track], version: u64) -> bool {
        if self.version == Some(version) {
            return false;
        }
        let keep = if self.version.is_none() {
            self.start
        } else {
            self.selected_entry()
        };
        self.version = Some(version);
        self.filter(tracks, keep);
        true
    }

    /// List the entries matching the query, highlighting `keep` if it is among them, else the
    /// first.
    fn filter(&mut self, tracks: &[Track], keep: Option<usize>) {
        let words: Vec<String> = self
            .search
            .text()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut haystack = String::new();
        self.matches.clear();
        for (i, t) in tracks.iter().enumerate() {
            haystack.clear();
            haystack.push_str(&t.title.to_lowercase());
            haystack.push('\n');
            match &t.location {
                Location::File(_) => haystack.push_str(&t.location.name().to_lowercase()),
                // A stream's whole address, so that its host finds it.
                Location::Url(url) => haystack.push_str(&url.to_lowercase()),
            }
            if words.iter().all(|w| haystack.contains(w.as_str())) {
                self.matches.push(i);
            }
        }
        let selected = keep
            .and_then(|k| self.matches.iter().position(|&m| m == k))
            .or_else(|| (!self.matches.is_empty()).then_some(0));
        self.list.reset(self.matches.len(), selected);
    }

    /// The text changed: match again.
    fn edited(&mut self, tracks: &[Track]) {
        let keep = self.selected_entry();
        self.filter(tracks, keep);
    }

    /// Typed or pasted text (typed text comes from the keyboard's layout, so already shifted
    /// or composed).
    pub fn text(&mut self, text: &str, tracks: &[Track], now: Instant) -> Outcome {
        if !self.search.insert(text, now) {
            return Outcome::None;
        }
        self.edited(tracks);
        Outcome::Redraw
    }

    /// Select everything in the search box (Ctrl+A), so that typing replaces it.
    pub fn select_all(&mut self, now: Instant) -> Outcome {
        self.search.select_all(now);
        Outcome::Redraw
    }

    /// A key that is not text.
    pub fn key(&mut self, key: Key, tracks: &[Track], now: Instant) -> Outcome {
        let edit = match key {
            Key::Up => {
                self.list.move_selection(-1);
                return Outcome::Redraw;
            }
            Key::Down => {
                self.list.move_selection(1);
                return Outcome::Redraw;
            }
            Key::PageUp => {
                self.list.move_selection(-self.list.page());
                return Outcome::Redraw;
            }
            Key::PageDown => {
                self.list.move_selection(self.list.page());
                return Outcome::Redraw;
            }
            Key::Enter => return self.selected_entry().map_or(Outcome::None, Outcome::Jump),
            Key::Escape => return Outcome::Close,
            Key::Left => Edit::Left,
            Key::Right => Edit::Right,
            Key::Home => Edit::Home,
            Key::End => Edit::End,
            Key::Backspace => Edit::Backspace,
            Key::Delete => Edit::Delete,
        };
        let had = (self.search.text().len(), self.search.is_all_selected());
        if self.search.edit(edit, now) {
            self.edited(tracks);
        } else if matches!(edit, Edit::Backspace | Edit::Delete)
            && had == (self.search.text().len(), false)
        {
            // Nothing to delete.
            return Outcome::None;
        }
        Outcome::Redraw
    }

    /// The mouse wheel turned `notches`, positive away from the user.
    pub fn wheel(&mut self, notches: i32) -> Outcome {
        self.list.wheel(notches);
        Outcome::Redraw
    }

    fn part_at(&self, x: i32, y: i32) -> Option<Part> {
        let l = &self.layout;
        if l.edit.contains(x, y) {
            return Some(Part::Edit);
        }
        if l.jump.contains(x, y) {
            return Some(Part::JumpButton);
        }
        if l.close.contains(x, y) {
            return Some(Part::CloseButton);
        }
        self.list.part_at(x, y).map(|_| Part::List)
    }

    /// The pointer pressed at (`x`, `y`).
    pub fn pointer_down(&mut self, x: i32, y: i32, now: Instant, m: &dyn Measure) -> Outcome {
        self.pointer = (x, y);
        let Some(part) = self.part_at(x, y) else {
            return Outcome::None;
        };
        match part {
            Part::Edit => self.search.click(x, self.layout.edit, self.scale, m, now),
            Part::List => {
                if let Some(list_box::Part::Row(i)) = self.list.pointer_down(x, y) {
                    let double = matches!(self.last_click,
                        Some((j, t)) if j == i && now.saturating_duration_since(t).as_millis() < DOUBLE_CLICK_MS);
                    if double {
                        self.last_click = None;
                        self.list.pointer_up();
                        return self.selected_entry().map_or(Outcome::None, Outcome::Jump);
                    }
                    self.last_click = Some((i, now));
                }
            }
            Part::JumpButton | Part::CloseButton => {}
        }
        self.pressed = Some(part);
        Outcome::Redraw
    }

    /// Whether `part` is held with the pointer over it, so that letting go would press it.
    fn pushed(&self, part: Part) -> bool {
        self.pressed == Some(part) && self.part_at(self.pointer.0, self.pointer.1) == Some(part)
    }

    /// The pointer moved to (`x`, `y`): drags the thumb, and a held button pops out while the
    /// pointer is off it.
    pub fn pointer_moved(&mut self, x: i32, y: i32) -> Outcome {
        let held = self
            .pressed
            .filter(|p| matches!(p, Part::JumpButton | Part::CloseButton));
        if let Some(button) = held {
            let was = self.pushed(button);
            self.pointer = (x, y);
            return if self.pushed(button) == was {
                Outcome::None
            } else {
                Outcome::Redraw
            };
        }
        self.pointer = (x, y);
        if self.pressed == Some(Part::List) && self.list.pointer_moved(y) {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    /// The pointer let go at (`x`, `y`): a button pressed and let go over runs.
    pub fn pointer_up(&mut self, x: i32, y: i32) -> Outcome {
        let Some(part) = self.pressed.take() else {
            return Outcome::None;
        };
        let over = self.part_at(x, y) == Some(part);
        self.list.pointer_up();
        match part {
            Part::JumpButton if over => {
                self.selected_entry().map_or(Outcome::Redraw, Outcome::Jump)
            }
            Part::CloseButton if over => Outcome::Close,
            _ => Outcome::Redraw,
        }
    }

    /// Whether the caret shows at `now`.
    #[must_use]
    pub fn caret_on(&self, now: Instant) -> bool {
        self.search.caret_on(now)
    }

    /// Draw the dialog into `c`, which must be [`Jump::size`]. `font` must be the menu font at
    /// this scale ([`crate::menu::font_px`]).
    pub fn draw(&self, c: &mut Image, font: &TtfFont, tracks: &[Track], now: Instant) {
        let l = &self.layout;
        let t = l.t;
        let lh = font.line_height();
        c.clear(FACE);
        let all = c.bounds();
        let centred = |r: Rect| r.y + (r.h - lh) / 2;

        font.draw(c, LABEL, l.label_x, centred(l.edit), TEXT, all);

        self.search.draw(c, font, l.edit, t, self.scale, now);

        // The list; its scroll bar is greyed while everything fits.
        self.list.draw(c, font, |row, label| {
            let entry = self.matches[row];
            let Some(track) = tracks.get(entry) else {
                return false;
            };
            push_fmt(label, format_args!("{}. {}", entry + 1, track.title));
            true
        });

        // The buttons; Jump is the default, which Enter presses.
        for (part, r, text) in [
            (Part::JumpButton, l.jump, JUMP),
            (Part::CloseButton, l.close, CLOSE),
        ] {
            let default = part == Part::JumpButton;
            classic::push_button(c, font, r, t, text, default, self.pushed(part));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    /// Every character 6 pixels wide.
    struct Fixed;

    impl Measure for Fixed {
        fn text_width(&self, text: &str) -> i32 {
            i32::try_from(text.chars().count()).unwrap() * 6
        }
    }

    fn tracks() -> Vec<Track> {
        [
            ("DJ Mike Llama - Llama Whippin' Intro", "intro.mp3"),
            ("Nullsoft - Winamp Theme", "theme.mp3"),
            ("Justin F. - Rolling Llama", "rolling.flac"),
            ("Unknown - Track 07", "llama-07.ogg"),
        ]
        .into_iter()
        .map(|(title, file)| Track {
            title: title.into(),
            seconds: None,
            location: Location::File(PathBuf::from("/music").join(file)),
        })
        .collect()
    }

    fn open(current: Option<usize>) -> (Jump, Vec<Track>, Instant) {
        let now = Instant::now();
        let list = tracks();
        let mut jump = Jump::new(1.0, &Fixed, current, now);
        assert!(jump.refresh(&list, 1));
        (jump, list, now)
    }

    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn it_opens_on_the_current_entry_with_everything_listed() {
        let (jump, _, _) = open(Some(2));
        assert_eq!(jump.matches(), [0, 1, 2, 3]);
        assert_eq!(jump.selected_entry(), Some(2));
        let (jump, _, _) = open(None);
        assert_eq!(jump.selected_entry(), Some(0));
    }

    #[test]
    fn typing_narrows_the_list_by_every_word_in_title_or_file_name() {
        let (mut jump, list, now) = open(Some(1));
        assert_eq!(jump.text("LLAMA", &list, now), Outcome::Redraw);
        // Entry 3's title has no llama, but its file name does.
        assert_eq!(jump.matches(), [0, 2, 3]);
        assert_eq!(jump.selected_entry(), Some(0), "entry 1 no longer matches");
        let _ = jump.text(" roll", &list, now);
        assert_eq!(jump.matches(), [2]);
        assert_eq!(jump.query(), "LLAMA roll");
        for _ in 0..5 {
            let _ = jump.key(Key::Backspace, &list, now);
        }
        assert_eq!(jump.query(), "LLAMA");
        assert_eq!(jump.matches(), [0, 2, 3]);
        assert_eq!(
            jump.selected_entry(),
            Some(2),
            "the highlight stays where it was"
        );
        assert_eq!(
            jump.text("\u{8}\r", &list, now),
            Outcome::None,
            "controls are not text"
        );
    }

    #[test]
    fn streams_are_found_by_their_address() {
        let now = Instant::now();
        let mut list = tracks();
        list.push(Track {
            title: "Radio Llama".into(),
            seconds: None,
            location: Location::Url("https://ice.radio.example/llama-128".into()),
        });
        let mut jump = Jump::new(1.0, &Fixed, None, now);
        jump.refresh(&list, 1);
        let _ = jump.text("radio.example", &list, now);
        assert_eq!(jump.matches(), [4]);
    }

    #[test]
    fn the_caret_moves_and_deletes_by_character() {
        let (mut jump, list, now) = open(None);
        let _ = jump.text("añb", &list, now);
        let _ = jump.key(Key::Left, &list, now);
        let _ = jump.key(Key::Left, &list, now);
        let _ = jump.key(Key::Delete, &list, now);
        assert_eq!(jump.query(), "ab");
        let _ = jump.key(Key::Home, &list, now);
        assert_eq!(jump.key(Key::Backspace, &list, now), Outcome::None);
        let _ = jump.text("x", &list, now);
        let _ = jump.key(Key::End, &list, now);
        let _ = jump.key(Key::Right, &list, now);
        let _ = jump.text("!", &list, now);
        assert_eq!(jump.query(), "xab!");
    }

    #[test]
    fn keys_move_the_highlight_and_enter_jumps() {
        let (mut jump, list, now) = open(Some(0));
        let _ = jump.key(Key::Down, &list, now);
        let _ = jump.key(Key::Down, &list, now);
        assert_eq!(jump.key(Key::Enter, &list, now), Outcome::Jump(2));
        let _ = jump.key(Key::PageDown, &list, now);
        assert_eq!(jump.selected_entry(), Some(3), "stops at the last");
        let _ = jump.key(Key::PageUp, &list, now);
        assert_eq!(jump.selected_entry(), Some(0));
        assert_eq!(jump.key(Key::Escape, &list, now), Outcome::Close);
        let _ = jump.text("nothing like it", &list, now);
        assert_eq!(jump.key(Key::Enter, &list, now), Outcome::None);
    }

    #[test]
    fn clicks_pick_rows_and_buttons() {
        let (mut jump, _, now) = open(None);
        let rows = jump.list.rows();
        let row = |i: i32| (rows.x + 5, rows.y + i * jump.list.row_h() + 2);
        let (x, y) = row(1);
        let _ = jump.pointer_down(x, y, now, &Fixed);
        let _ = jump.pointer_up(x, y);
        assert_eq!(jump.selected_entry(), Some(1));
        let later = now + Duration::from_millis(100);
        assert_eq!(jump.pointer_down(x, y, later, &Fixed), Outcome::Jump(1));
        let _ = jump.pointer_up(x, y);

        let (bx, by) = centre(jump.layout.jump);
        let _ = jump.pointer_down(bx, by, now, &Fixed);
        assert_eq!(jump.pointer_up(bx, by), Outcome::Jump(1));
        // Letting go elsewhere cancels.
        let _ = jump.pointer_down(bx, by, now, &Fixed);
        assert_eq!(jump.pointer_up(0, 0), Outcome::Redraw);
        let (cx, cy) = centre(jump.layout.close);
        let _ = jump.pointer_down(cx, cy, now, &Fixed);
        assert_eq!(jump.pointer_up(cx, cy), Outcome::Close);
    }

    #[test]
    fn a_click_in_the_search_box_places_the_caret() {
        let (mut jump, list, now) = open(None);
        let _ = jump.text("abcd", &list, now);
        let x = TextBox::text_x(jump.layout.edit, 1.0) + 2 * 6 + 1; // just after "ab"
        let y = centre(jump.layout.edit).1;
        let _ = jump.pointer_down(x, y, now, &Fixed);
        let _ = jump.text("-", &list, now);
        assert_eq!(jump.query(), "ab-cd");
    }

    #[test]
    fn long_lists_scroll_by_wheel_arrows_and_thumb() {
        let now = Instant::now();
        let list: Vec<Track> = (0..100)
            .map(|i| Track::from_path(PathBuf::from(format!("/m/{i}.mp3"))))
            .collect();
        let mut jump = Jump::new(1.0, &Fixed, Some(0), now);
        jump.refresh(&list, 7);
        assert!(!jump.refresh(&list, 7), "unchanged");
        let shown = jump.list.visible_rows();
        assert!(shown > 5 && shown < 100);
        let _ = jump.wheel(-2);
        assert_eq!(jump.list.scroll(), 6);
        // The scroll bar's up arrow, at the list's top right.
        let r = jump.list.rect();
        let (ax, ay) = (r.right() - 6, r.y + 6);
        let _ = jump.pointer_down(ax, ay, now, &Fixed);
        let _ = jump.pointer_up(ax, ay);
        assert_eq!(jump.list.scroll(), 5);
        // The thumb, near the top, dragged far down.
        let (tx, ty) = (ax, r.y + 40);
        let _ = jump.pointer_down(tx, ty, now, &Fixed);
        assert_eq!(jump.pointer_moved(tx, r.bottom() + 50), Outcome::Redraw);
        assert_eq!(jump.list.scroll(), jump.list.max_scroll());
        let _ = jump.pointer_up(tx, r.bottom() + 50);
        assert_eq!(jump.pointer_moved(tx, r.y), Outcome::None, "let go");
    }

    #[test]
    fn a_changed_playlist_is_matched_again() {
        let (mut jump, mut list, _) = open(Some(2));
        list.remove(0);
        assert!(jump.refresh(&list, 2));
        assert_eq!(jump.matches(), [0, 1, 2]);
        // The highlight keeps its index (the dialog does not follow entries as they move).
        assert_eq!(jump.selected_entry(), Some(2));
        list.clear();
        assert!(jump.refresh(&list, 3));
        assert_eq!(jump.selected_entry(), None);
    }

    #[test]
    fn a_held_button_pops_out_when_the_pointer_leaves_it() {
        let (mut jump, _, now) = open(None);
        let (bx, by) = centre(jump.layout.close);
        let _ = jump.pointer_down(bx, by, now, &Fixed);
        assert!(jump.pushed(Part::CloseButton));
        assert_eq!(jump.pointer_moved(0, 0), Outcome::Redraw);
        assert!(!jump.pushed(Part::CloseButton));
        assert_eq!(jump.pointer_moved(1, 1), Outcome::None);
        assert_eq!(jump.pointer_moved(bx, by), Outcome::Redraw);
    }

    #[test]
    fn it_draws_its_controls() {
        let font = TtfFont::load_menu(crate::menu::font_px(1.0));
        let (mut jump, list, now) = open(Some(1));
        jump.rescale(1.0, &font);
        let (w, h) = jump.size();
        let mut c = Image::new(w, h);
        jump.draw(&mut c, &font, &list, now);
        let l = jump.layout;
        assert_eq!(c.get(l.edit.x + 10, l.edit.y), classic::SHADOW);
        assert_eq!(
            c.get(l.jump.x, l.jump.y),
            classic::FRAME,
            "the default button's frame"
        );
        let rows = jump.list.rows();
        let y = rows.y + jump.list.row_h() + 1;
        assert_eq!(c.get(rows.right() - 2, y), classic::SELECTION);
    }
}
