//! Popup menus, drawn the way Windows 2000 drew its own. Winamp 2.x used the system's menus
//! rather than the skin, so NeoAmp copies their classic grey look whatever skin is loaded.
//!
//! [`Item`]s describe a menu. [`Menus`] is what is on screen: a menu and the submenus opened
//! from it, one level each, with Win32's handling of the pointer and keyboard. A submenu
//! opens when the pointer rests on its item (or at once on a click), a command runs when the
//! button is let go over it, and the arrow keys, Enter, Escape and mnemonic letters work.
//!
//! All geometry is in physical screen pixels: sizes are Windows' at 96 DPI times the menu's
//! scale, and the text is drawn at that size rather than enlarged afterwards, so it stays
//! sharp. The window layer shows each level in its own borderless OS window, since a menu
//! usually reaches past the NeoAmp window it came from.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use crate::canvas::{Image, Rect};
use crate::classic::{
    FACE, GRAY_TEXT, HILIGHT, SELECTION, SELECTION_TEXT, SHADOW, TEXT, draw_glyph, line, px, raised,
};
use crate::font::TtfFont;

/// A submenu opens, or gives way to another, once the pointer has rested this long on its
/// item. Windows' default is 400 ms.
pub const HOVER_DELAY: Duration = Duration::from_millis(300);

/// Sizes at 96 DPI, which `px` scales.
mod metric {
    /// The 3D frame (two pixels) plus one pixel of padding, on every side.
    pub const EDGE: i32 = 3;
    pub const ITEM_H: i32 = 17;
    pub const SEPARATOR_H: i32 = 9;
    /// From an item's left edge to its text; the check mark sits in this column.
    pub const TEXT_X: i32 = 17;
    /// Between the longest label and the shortcut column.
    pub const SHORTCUT_GAP: i32 = 16;
    /// From the end of the text to an item's right edge; the submenu arrow sits in this column.
    pub const RIGHT_COLUMN: i32 = 17;
    /// From an item's right edge to the right of the submenu arrow.
    pub const ARROW_RIGHT: i32 = 8;
    /// Submenus overlap their parent by this much.
    pub const OVERLAP: i32 = 3;
    /// Tahoma 8 pt.
    pub const FONT_PX: f32 = 11.0;
}

/// Marlett's check mark, 7x7, one string per row.
const CHECK: [&str; 7] = [
    "......#", ".....##", "#...###", "##.###.", "#####..", ".###...", "..#....",
];
/// The radio-item bullet.
const BULLET: [&str; 5] = [".###.", "#####", "#####", "#####", ".###."];
/// The submenu arrow.
const ARROW: [&str; 7] = ["#...", "##..", "###.", "####", "###.", "##..", "#..."];

/// What an item shows in its left column when it is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    None,
    /// An option that is on.
    Check,
    /// The chosen one of several options.
    Bullet,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Kind<C> {
    Command(C),
    Submenu(Vec<Item<C>>),
    Separator,
}

/// One line of a menu. Labels mark their mnemonic with `&` as in Win32 (`&&` is a literal
/// `&`; see [`escape`]); without one, the first letter is the mnemonic.
#[derive(Clone, PartialEq, Debug)]
pub struct Item<C> {
    label: Cow<'static, str>,
    shortcut: Cow<'static, str>,
    kind: Kind<C>,
    mark: Mark,
    enabled: bool,
}

impl<C> Item<C> {
    /// An item that runs `command`.
    #[must_use]
    pub fn new(label: impl Into<Cow<'static, str>>, command: C) -> Item<C> {
        Item {
            label: label.into(),
            shortcut: Cow::Borrowed(""),
            kind: Kind::Command(command),
            mark: Mark::None,
            enabled: true,
        }
    }

    /// An item that opens `items`.
    #[must_use]
    pub fn submenu(label: impl Into<Cow<'static, str>>, items: Vec<Item<C>>) -> Item<C> {
        Item {
            label: label.into(),
            shortcut: Cow::Borrowed(""),
            kind: Kind::Submenu(items),
            mark: Mark::None,
            enabled: true,
        }
    }

    #[must_use]
    pub fn separator() -> Item<C> {
        Item {
            label: Cow::Borrowed(""),
            shortcut: Cow::Borrowed(""),
            kind: Kind::Separator,
            mark: Mark::None,
            enabled: false,
        }
    }

    /// Show the key that does the same, e.g. `"Ctrl+D"` (see [`crate::shortcut`]).
    #[must_use]
    pub fn shortcut(mut self, keys: impl Into<Cow<'static, str>>) -> Item<C> {
        self.shortcut = keys.into();
        self
    }

    /// A check mark when `on`.
    #[must_use]
    pub fn checked(mut self, on: bool) -> Item<C> {
        self.mark = if on { Mark::Check } else { Mark::None };
        self
    }

    /// A bullet when `on`: the chosen one of a group.
    #[must_use]
    pub fn chosen(mut self, on: bool) -> Item<C> {
        self.mark = if on { Mark::Bullet } else { Mark::None };
        self
    }

    /// Greyed out, and does nothing, unless `on`.
    #[must_use]
    pub fn enabled(mut self, on: bool) -> Item<C> {
        self.enabled = on;
        self
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn kind(&self) -> &Kind<C> {
        &self.kind
    }

    #[must_use]
    pub fn mark(&self) -> Mark {
        self.mark
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn is_separator(&self) -> bool {
        matches!(self.kind, Kind::Separator)
    }

    fn is_submenu(&self) -> bool {
        matches!(self.kind, Kind::Submenu(_))
    }

    /// The lowercase letter that picks this item from the keyboard.
    fn mnemonic(&self) -> Option<char> {
        let (text, at) = parse_label(&self.label);
        let ch = match at {
            Some(i) => text[i..].chars().next(),
            None => text.chars().next(),
        };
        ch.and_then(|c| c.to_lowercase().next())
    }
}

/// `text` with every `&` doubled, so that it shows as written in a label.
#[must_use]
pub fn escape(text: &str) -> Cow<'static, str> {
    Cow::Owned(text.replace('&', "&&"))
}

/// A label as shown: `&&` made `&` and the mnemonic's `&` dropped, with the byte position of
/// the mnemonic in the result.
fn parse_label(label: &str) -> (String, Option<usize>) {
    let mut text = String::with_capacity(label.len());
    let mut mnemonic = None;
    let mut chars = label.chars();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            text.push(ch);
            continue;
        }
        match chars.next() {
            Some('&') => text.push('&'),
            Some(next) => {
                mnemonic.get_or_insert(text.len());
                text.push(next);
            }
            None => {}
        }
    }
    (text, mnemonic)
}

/// How wide text is, in physical pixels at the menu's size.
pub trait Measure {
    fn text_width(&self, text: &str) -> i32;
}

impl Measure for TtfFont {
    fn text_width(&self, text: &str) -> i32 {
        self.width(text)
    }
}

/// The menu font at `scale`.
#[must_use]
pub fn font_px(scale: f64) -> f32 {
    // A font size of a few dozen pixels at most; f32 is plenty.
    (f64::from(metric::FONT_PX) * scale) as f32
}

/// Where a menu opens, in screen pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// With its top-left corner here (at the pointer, for a right-click).
    Point(i32, i32),
    /// Dropping down from this button, or rising above it when there is no room below.
    Below(Rect),
}

/// What a key does in a menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    /// A letter or digit: the item with that mnemonic.
    Char(char),
}

/// What the window layer must do after an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome<C> {
    /// Nothing changed.
    None,
    /// Redraw, and open or close windows to match [`Menus::levels`].
    Changed,
    /// Close every menu and run this.
    Chosen(C),
    /// Close every menu.
    Close,
}

/// One menu on screen.
#[derive(Debug)]
struct Level<C> {
    items: Vec<Item<C>>,
    /// Where it is on the screen.
    rect: Rect,
    /// Each item's area, relative to `rect`.
    rows: Vec<Rect>,
    /// Shortcuts start this far from an item's left edge.
    shortcut_x: i32,
    /// The highlighted item.
    hot: Option<usize>,
    /// The item whose submenu is the next level, if one is open.
    opened: Option<usize>,
}

impl<C> Level<C> {
    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.contains(x, y))
            .filter(|&i| self.items.get(i).is_some_and(|it| !it.is_separator()))
    }

    /// The next item in direction `step` (1 or -1) from the highlighted one, wrapping, that is
    /// not a separator.
    fn step(&self, step: isize) -> Option<usize> {
        let n = self.items.len();
        let start = match self.hot {
            Some(i) => i,
            // Down from nothing lands on the first item, up on the last.
            None if step > 0 => n.checked_sub(1)?,
            None => 0,
        };
        (1..=n)
            .map(|k| (start as isize + step * k as isize).rem_euclid(n as isize) as usize)
            .find(|&i| !self.items[i].is_separator())
    }

    fn first(&self) -> Option<usize> {
        self.items.iter().position(|it| !it.is_separator())
    }
}

/// Item areas and the menu's size, wrapping into more columns when one would be taller than
/// `max_h`, as Windows does.
fn layout<C>(
    items: &[Item<C>],
    scale: f64,
    max_h: i32,
    m: &dyn Measure,
) -> (i32, i32, Vec<Rect>, i32) {
    let edge = px(metric::EDGE, scale);
    let label_w = items
        .iter()
        .map(|it| m.text_width(&parse_label(&it.label).0))
        .max()
        .unwrap_or(0);
    let shortcut_w = items
        .iter()
        .map(|it| m.text_width(&it.shortcut))
        .max()
        .unwrap_or(0);
    let text_x = px(metric::TEXT_X, scale);
    let shortcut_x = text_x + label_w + px(metric::SHORTCUT_GAP, scale);
    let col_w = if shortcut_w > 0 {
        shortcut_x + shortcut_w
    } else {
        text_x + label_w
    } + px(metric::RIGHT_COLUMN, scale);
    let limit = (max_h - edge).max(edge + px(metric::ITEM_H, scale));
    let mut rows = Vec::with_capacity(items.len());
    let (mut x, mut y, mut bottom) = (edge, edge, edge);
    for it in items {
        let h = px(
            if it.is_separator() {
                metric::SEPARATOR_H
            } else {
                metric::ITEM_H
            },
            scale,
        );
        if y + h > limit && y > edge {
            x += col_w;
            y = edge;
        }
        rows.push(Rect::new(x, y, col_w, h));
        y += h;
        bottom = bottom.max(y);
    }
    (x + col_w + edge, bottom + edge, rows, shortcut_x)
}

/// `rect` moved inside `screen` where it can be, keeping its size.
fn keep_inside(rect: Rect, screen: Rect) -> Rect {
    let x = rect.x.min(screen.right() - rect.w).max(screen.x);
    let y = rect.y.min(screen.bottom() - rect.h).max(screen.y);
    Rect::new(x, y, rect.w, rect.h)
}

/// The menus on screen: a menu and the submenus opened from it.
#[derive(Debug)]
pub struct Menus<C> {
    levels: Vec<Level<C>>,
    scale: f64,
    /// The monitor the menu opened on; menus stay on it.
    screen: Rect,
    /// Underline mnemonics. Windows 2000 shows them once the keyboard is used.
    cues: bool,
    /// A submenu to open, or close, once the pointer has rested: the level, the item under
    /// the pointer (`None` closes any submenu of that level) and since when.
    pending: Option<(usize, Option<usize>, Instant)>,
}

impl<C: Copy> Menus<C> {
    /// Show `items` at `anchor`, on `screen`, at `scale` times Windows' 96 DPI sizes. A menu
    /// opened from the keyboard starts with its first item highlighted and mnemonics shown.
    #[must_use]
    pub fn open(
        items: Vec<Item<C>>,
        anchor: Anchor,
        screen: Rect,
        scale: f64,
        keyboard: bool,
        m: &dyn Measure,
    ) -> Menus<C> {
        let (w, h, rows, shortcut_x) = layout(&items, scale, screen.h, m);
        let rect = match anchor {
            Anchor::Point(x, y) => {
                // Flip to the other side of the point when there is no room, as Win32 does.
                let x = if x + w > screen.right() { x - w } else { x };
                let y = if y + h > screen.bottom() { y - h } else { y };
                Rect::new(x, y, w, h)
            }
            Anchor::Below(button) => {
                let y = if button.bottom() + h > screen.bottom() {
                    button.y - h
                } else {
                    button.bottom()
                };
                Rect::new(button.x, y, w, h)
            }
        };
        let mut level = Level {
            items,
            rect: keep_inside(rect, screen),
            rows,
            shortcut_x,
            hot: None,
            opened: None,
        };
        if keyboard {
            level.hot = level.first();
        }
        Menus {
            levels: vec![level],
            scale,
            screen,
            cues: keyboard,
            pending: None,
        }
    }

    /// How many menus are showing: the first one and its open submenus.
    #[must_use]
    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// Where menu `level` is on the screen.
    #[must_use]
    pub fn rect(&self, level: usize) -> Option<Rect> {
        self.levels.get(level).map(|l| l.rect)
    }

    #[must_use]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Close the submenus below `level`.
    fn close_below(&mut self, level: usize) {
        self.levels.truncate(level + 1);
        if let Some(l) = self.levels.get_mut(level) {
            l.opened = None;
        }
    }

    /// Open the submenu of item `item` of `level`, replacing any open below it. `select`
    /// highlights its first item (when opened from the keyboard).
    fn open_submenu(&mut self, level: usize, item: usize, select: bool, m: &dyn Measure) -> bool {
        let Some(parent) = self.levels.get(level) else {
            return false;
        };
        let Some(Item {
            kind: Kind::Submenu(items),
            enabled: true,
            ..
        }) = parent.items.get(item)
        else {
            return false;
        };
        if parent.opened == Some(item) && self.levels.len() > level + 1 {
            return false;
        }
        let row = parent.rows[item].offset(parent.rect.x, parent.rect.y);
        let parent_rect = parent.rect;
        let items = items.clone();
        let (w, h, rows, shortcut_x) = layout(&items, self.scale, self.screen.h, m);
        let (edge, overlap) = (
            px(metric::EDGE, self.scale),
            px(metric::OVERLAP, self.scale),
        );
        // To the right of the parent, its first item level with the one that opened it; to the
        // left when there is no room on the right.
        let mut x = parent_rect.right() - overlap;
        if x + w > self.screen.right() {
            x = parent_rect.x - w + overlap;
        }
        let rect = keep_inside(Rect::new(x, row.y - edge, w, h), self.screen);
        self.close_below(level);
        let mut sub = Level {
            items,
            rect,
            rows,
            shortcut_x,
            hot: None,
            opened: None,
        };
        if select {
            sub.hot = sub.first();
        }
        let parent = &mut self.levels[level];
        parent.opened = Some(item);
        parent.hot = Some(item);
        self.levels.push(sub);
        true
    }

    /// Activate item `item` of `level`: a command is chosen, a submenu opens.
    fn activate(&mut self, level: usize, item: usize, select: bool, m: &dyn Measure) -> Outcome<C> {
        let Some(it) = self.levels.get(level).and_then(|l| l.items.get(item)) else {
            return Outcome::None;
        };
        match (&it.kind, it.enabled) {
            (Kind::Command(c), true) => Outcome::Chosen(*c),
            (Kind::Submenu(_), true) => {
                self.pending = None;
                self.open_submenu(level, item, select, m);
                Outcome::Changed
            }
            _ => Outcome::None,
        }
    }

    /// The pointer moved to (`x`, `y`) in menu `level`'s window.
    pub fn pointer_moved(&mut self, level: usize, x: i32, y: i32, now: Instant) -> Outcome<C> {
        let Some(l) = self.levels.get(level) else {
            return Outcome::None;
        };
        let item = l.item_at(x, y);
        let mut changed = false;
        // Back on a submenu: the items that opened it light up again and stay open.
        for parent in &mut self.levels[..level] {
            if parent.hot != parent.opened {
                parent.hot = parent.opened;
                changed = true;
            }
        }
        if self.pending.is_some_and(|(p, _, _)| p < level) {
            self.pending = None;
        }
        let l = &mut self.levels[level];
        if l.hot != item {
            l.hot = item;
            changed = true;
            // Let the pointer rest before a submenu opens or the open one closes.
            let wanted = item.filter(|&i| l.items[i].is_submenu() && l.items[i].enabled);
            self.pending = (wanted != l.opened).then_some((level, wanted, now));
        }
        if changed {
            Outcome::Changed
        } else {
            Outcome::None
        }
    }

    /// The pointer left menu `level`'s window.
    pub fn pointer_left(&mut self, level: usize) -> Outcome<C> {
        let Some(l) = self.levels.get_mut(level) else {
            return Outcome::None;
        };
        // An item whose submenu is open stays lit.
        let hot = l.opened;
        if self.pending.is_some_and(|(p, _, _)| p == level) {
            self.pending = None;
        }
        if l.hot == hot {
            Outcome::None
        } else {
            l.hot = hot;
            Outcome::Changed
        }
    }

    /// A button went down at (`x`, `y`) in menu `level`'s window: a submenu item opens at once.
    pub fn pressed(&mut self, level: usize, x: i32, y: i32, m: &dyn Measure) -> Outcome<C> {
        match self.levels.get(level).and_then(|l| l.item_at(x, y)) {
            Some(i) if self.levels[level].items[i].is_submenu() => {
                self.activate(level, i, false, m)
            }
            _ => Outcome::None,
        }
    }

    /// A button was let go at (`x`, `y`) in menu `level`'s window: a command item is chosen.
    pub fn released(&mut self, level: usize, x: i32, y: i32, m: &dyn Measure) -> Outcome<C> {
        match self.levels.get(level).and_then(|l| l.item_at(x, y)) {
            Some(i) => self.activate(level, i, false, m),
            None => Outcome::None,
        }
    }

    /// Open or close the submenu the pointer has rested on.
    pub fn tick(&mut self, now: Instant, m: &dyn Measure) -> Outcome<C> {
        let Some((level, item, since)) = self.pending else {
            return Outcome::None;
        };
        if now.saturating_duration_since(since) < HOVER_DELAY {
            return Outcome::None;
        }
        self.pending = None;
        match item {
            Some(i) => {
                self.open_submenu(level, i, false, m);
            }
            None => self.close_below(level),
        }
        Outcome::Changed
    }

    /// A key was pressed while the menus have the keyboard. Keys act on the last submenu.
    pub fn key(&mut self, key: Key, m: &dyn Measure) -> Outcome<C> {
        self.cues = true;
        self.pending = None;
        let level = self.levels.len() - 1;
        let l = &mut self.levels[level];
        match key {
            Key::Up | Key::Down => {
                l.hot = l.step(if key == Key::Down { 1 } else { -1 });
                Outcome::Changed
            }
            Key::Right => match l.hot {
                Some(i) if l.items[i].is_submenu() => self.activate(level, i, true, m),
                _ => Outcome::Changed,
            },
            Key::Left | Key::Escape if level > 0 => {
                self.close_below(level - 1);
                Outcome::Changed
            }
            Key::Left => Outcome::Changed,
            Key::Escape => Outcome::Close,
            Key::Enter => match l.hot {
                Some(i) => self.activate(level, i, true, m),
                None => Outcome::Changed,
            },
            Key::Char(ch) => {
                let ch = ch.to_lowercase().next().unwrap_or(ch);
                let matches: Vec<usize> = (0..l.items.len())
                    .filter(|&i| l.items[i].mnemonic() == Some(ch))
                    .collect();
                match matches.as_slice() {
                    [] => Outcome::Changed,
                    [only] => {
                        l.hot = Some(*only);
                        let only = *only;
                        match self.activate(level, only, true, m) {
                            Outcome::None => Outcome::Changed,
                            other => other,
                        }
                    }
                    // Several share the letter: each press moves to the next of them.
                    several => {
                        let after = l.hot.unwrap_or(usize::MAX);
                        l.hot = several
                            .iter()
                            .copied()
                            .find(|&i| i > after || after == usize::MAX)
                            .or_else(|| several.first().copied());
                        Outcome::Changed
                    }
                }
            }
        }
    }

    /// Draw menu `level` into `c`, which must be the size of its [`Menus::rect`]. `font` must
    /// be the menu font at [`font_px`] for this scale.
    pub fn draw(&self, level: usize, c: &mut Image, font: &TtfFont) {
        let Some(l) = self.levels.get(level) else {
            return;
        };
        let s = self.scale;
        let t = line(s);
        let (w, h) = (c.w, c.h);
        c.clear(FACE);
        raised(c, Rect::new(0, 0, w, h), t);

        let glyph_px = s.round().max(1.0) as i32;
        for (i, (it, row)) in l.items.iter().zip(&l.rows).enumerate() {
            if it.is_separator() {
                let y = row.y + (row.h - 2 * t) / 2;
                c.fill_rect(Rect::new(row.x, y, row.w, t), SHADOW);
                c.fill_rect(Rect::new(row.x, y + t, row.w, t), HILIGHT);
                continue;
            }
            let hot = l.hot == Some(i);
            if hot {
                c.fill_rect(*row, SELECTION);
            }
            let ink = match (hot, it.enabled) {
                (true, true) => SELECTION_TEXT,
                (false, true) => TEXT,
                (_, false) => GRAY_TEXT,
            };
            // Disabled text is etched: a white copy one pixel down and right, unless lit.
            let etched = !it.enabled && !hot;
            let column = |pattern: &[&str], x: i32| {
                let gh = pattern.len() as i32 * glyph_px;
                (x, row.y + (row.h - gh) / 2)
            };
            let mark = match it.mark {
                Mark::None => None,
                Mark::Check => Some(&CHECK[..]),
                Mark::Bullet => Some(&BULLET[..]),
            };
            if let Some(pattern) = mark {
                let gw = pattern[0].len() as i32 * glyph_px;
                let (x, y) = column(pattern, row.x + (px(metric::TEXT_X, s) - gw) / 2);
                if etched {
                    draw_glyph(c, pattern, x + t, y + t, glyph_px, HILIGHT);
                }
                draw_glyph(c, pattern, x, y, glyph_px, ink);
            }
            if it.is_submenu() {
                let gw = ARROW[0].len() as i32 * glyph_px;
                let (x, y) = column(&ARROW[..], row.right() - px(metric::ARROW_RIGHT, s) - gw);
                if etched {
                    draw_glyph(c, &ARROW, x + t, y + t, glyph_px, HILIGHT);
                }
                draw_glyph(c, &ARROW, x, y, glyph_px, ink);
            }
            let (text, mnemonic) = parse_label(&it.label);
            let top = row.y + (row.h - font.line_height()) / 2;
            let text_x = row.x + px(metric::TEXT_X, s);
            let shortcut_x = row.x + l.shortcut_x;
            let clip = Rect::new(0, 0, w, h);
            let underline = mnemonic.filter(|_| self.cues).map(|at| {
                let x0 = text_x + font.width(&text[..at]);
                let ch_end = text[at..]
                    .chars()
                    .next()
                    .map_or(at, |ch| at + ch.len_utf8());
                let x1 = text_x + font.width(&text[..ch_end]);
                Rect::new(x0, top + font.ascent() + t, (x1 - x0).max(t), t)
            });
            let passes: &[(i32, [u8; 4])] = if etched {
                &[(1, HILIGHT), (0, ink)]
            } else {
                &[(0, ink)]
            };
            for &(d, color) in passes {
                let d = d * t;
                font.draw(c, &text, text_x + d, top + d, color, clip);
                if !it.shortcut.is_empty() {
                    font.draw(c, &it.shortcut, shortcut_x + d, top + d, color, clip);
                }
                if let Some(u) = underline {
                    c.fill_rect(u.offset(d, d), color);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::classic::DARK_SHADOW;

    /// Every character 6 pixels wide.
    struct Fixed;

    impl Measure for Fixed {
        fn text_width(&self, text: &str) -> i32 {
            i32::try_from(text.chars().count()).unwrap() * 6
        }
    }

    const SCREEN: Rect = Rect::new(0, 0, 1000, 800);

    fn sample() -> Vec<Item<u8>> {
        vec![
            Item::new("&Open...", 1).shortcut("L"),
            Item::separator(),
            Item::submenu(
                "&More",
                vec![
                    Item::new("&Alpha", 10),
                    Item::new("&Beta", 11).checked(true),
                ],
            ),
            Item::new("&Grey", 3).enabled(false),
            Item::new("E&xit", 4),
        ]
    }

    fn open_at(x: i32, y: i32) -> Menus<u8> {
        Menus::open(sample(), Anchor::Point(x, y), SCREEN, 1.0, false, &Fixed)
    }

    /// The centre of item `i` of `level`, in that level's coordinates.
    fn centre(m: &Menus<u8>, level: usize, i: usize) -> (i32, i32) {
        let r = m.levels[level].rows[i];
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn labels_lose_their_ampersands_and_keep_the_mnemonic() {
        assert_eq!(parse_label("E&xit"), ("Exit".into(), Some(1)));
        assert_eq!(
            parse_label("Bass && Treble"),
            ("Bass & Treble".into(), None)
        );
        assert_eq!(parse_label("&Ünïcode"), ("Ünïcode".into(), Some(0)));
        assert_eq!(parse_label("trailing&"), ("trailing".into(), None));
        assert_eq!(Item::new("E&xit", 0).mnemonic(), Some('x'));
        assert_eq!(Item::new("Rock", 0).mnemonic(), Some('r'), "first letter");
        assert_eq!(escape("A & B"), "A && B");
    }

    #[test]
    fn the_layout_stacks_items_and_sizes_the_menu_to_the_longest_text() {
        let m = open_at(10, 20);
        let l = &m.levels[0];
        assert_eq!(l.rows[0], Rect::new(3, 3, l.rows[0].w, 17));
        assert_eq!(l.rows[1].h, 9, "separator");
        assert_eq!(l.rows[2].y, 3 + 17 + 9);
        // "Open..." is the longest label (7 chars), "L" the only shortcut.
        assert_eq!(l.shortcut_x, 17 + 7 * 6 + 16);
        assert_eq!(
            l.rect,
            Rect::new(10, 20, l.shortcut_x + 6 + 17 + 6, 3 + 4 * 17 + 9 + 3)
        );
    }

    #[test]
    fn menus_flip_away_from_the_screen_edges() {
        let m = open_at(990, 790);
        let r = m.rect(0).unwrap();
        assert_eq!((r.right(), r.bottom()), (990, 790));
        let below = Menus::open(
            sample(),
            Anchor::Below(Rect::new(100, 780, 20, 10)),
            SCREEN,
            1.0,
            false,
            &Fixed,
        );
        assert_eq!(
            below.rect(0).unwrap().bottom(),
            780,
            "rises above the button"
        );
    }

    #[test]
    fn scaled_menus_grow_but_keep_thin_lines() {
        let m = Menus::open(sample(), Anchor::Point(0, 0), SCREEN, 1.5, false, &Fixed);
        assert_eq!(m.levels[0].rows[0].h, 26);
        assert_eq!(line(1.5), 1);
        assert_eq!(line(2.0), 2);
    }

    #[test]
    fn resting_on_a_submenu_opens_it_after_the_delay() {
        let mut m = open_at(10, 10);
        let t0 = Instant::now();
        let (x, y) = centre(&m, 0, 2);
        assert_eq!(m.pointer_moved(0, x, y, t0), Outcome::Changed);
        assert_eq!(m.tick(t0 + HOVER_DELAY / 2, &Fixed), Outcome::None);
        assert_eq!(m.tick(t0 + HOVER_DELAY, &Fixed), Outcome::Changed);
        assert_eq!(m.levels(), 2);
        // The submenu's first item lines up with the item that opened it.
        let (parent, sub) = (m.rect(0).unwrap(), m.rect(1).unwrap());
        assert_eq!(sub.x, parent.right() - 3);
        assert_eq!(sub.y + 3, parent.y + m.levels[0].rows[2].y);
        // Choosing in the submenu.
        let (x, y) = centre(&m, 1, 0);
        assert_eq!(m.released(1, x, y, &Fixed), Outcome::Chosen(10));
    }

    #[test]
    fn moving_off_a_submenu_item_closes_it_unless_the_pointer_reaches_it() {
        let mut m = open_at(10, 10);
        let t0 = Instant::now();
        let (x, y) = centre(&m, 0, 2);
        let _ = m.pressed(0, x, y, &Fixed);
        assert_eq!(m.levels(), 2, "a click opens it at once");
        // Crossing another item on the way to the submenu does not close it...
        let (ox, oy) = centre(&m, 0, 4);
        let _ = m.pointer_moved(0, ox, oy, t0);
        let _ = m.pointer_moved(1, 5, 5, t0 + HOVER_DELAY / 3);
        assert_eq!(m.tick(t0 + HOVER_DELAY * 2, &Fixed), Outcome::None);
        assert_eq!((m.levels(), m.levels[0].hot), (2, Some(2)));
        // ...but resting on it does.
        let _ = m.pointer_moved(0, ox, oy, t0);
        assert_eq!(m.tick(t0 + HOVER_DELAY, &Fixed), Outcome::Changed);
        assert_eq!((m.levels(), m.levels[0].hot), (1, Some(4)));
    }

    #[test]
    fn only_enabled_commands_can_be_chosen() {
        let mut m = open_at(10, 10);
        let (x, y) = centre(&m, 0, 3);
        assert_eq!(m.released(0, x, y, &Fixed), Outcome::None, "disabled");
        let (x, y) = centre(&m, 0, 1);
        assert_eq!(m.released(0, x, y, &Fixed), Outcome::None, "separator");
        assert_eq!(m.released(0, -5, -5, &Fixed), Outcome::None, "outside");
        let (x, y) = centre(&m, 0, 0);
        assert_eq!(m.released(0, x, y, &Fixed), Outcome::Chosen(1));
    }

    #[test]
    fn arrow_keys_skip_separators_and_wrap() {
        let mut m = open_at(10, 10);
        let hot = |m: &Menus<u8>| m.levels.last().unwrap().hot;
        let _ = m.key(Key::Down, &Fixed);
        assert_eq!(hot(&m), Some(0));
        let _ = m.key(Key::Down, &Fixed);
        assert_eq!(hot(&m), Some(2), "past the separator");
        let _ = m.key(Key::Up, &Fixed);
        let _ = m.key(Key::Up, &Fixed);
        assert_eq!(hot(&m), Some(4), "wraps to the bottom");
        assert!(m.cues, "the keyboard shows mnemonics");
    }

    #[test]
    fn right_left_enter_and_escape_walk_the_submenus() {
        let mut m = open_at(10, 10);
        let _ = m.key(Key::Char('m'), &Fixed);
        assert_eq!(m.levels(), 2, "a mnemonic opens its submenu");
        assert_eq!(m.levels[1].hot, Some(0), "with its first item lit");
        assert_eq!(m.key(Key::Left, &Fixed), Outcome::Changed);
        assert_eq!(m.levels(), 1);
        let _ = m.key(Key::Right, &Fixed);
        assert_eq!(m.levels(), 2);
        let _ = m.key(Key::Down, &Fixed);
        assert_eq!(m.key(Key::Enter, &Fixed), Outcome::Chosen(11));
        let _ = m.key(Key::Escape, &Fixed);
        assert_eq!(m.levels(), 1);
        assert_eq!(m.key(Key::Escape, &Fixed), Outcome::Close);
        assert_eq!(m.key(Key::Char('X'), &Fixed), Outcome::Chosen(4));
    }

    #[test]
    fn shared_mnemonics_cycle_instead_of_choosing() {
        let items = vec![
            Item::new("Rock", 1),
            Item::new("Reggae", 2),
            Item::new("Pop", 3),
        ];
        let mut m = Menus::open(items, Anchor::Point(0, 0), SCREEN, 1.0, false, &Fixed);
        assert_eq!(m.key(Key::Char('r'), &Fixed), Outcome::Changed);
        assert_eq!(m.levels[0].hot, Some(0));
        let _ = m.key(Key::Char('r'), &Fixed);
        assert_eq!(m.levels[0].hot, Some(1));
        let _ = m.key(Key::Char('r'), &Fixed);
        assert_eq!(m.levels[0].hot, Some(0));
        assert_eq!(m.key(Key::Char('p'), &Fixed), Outcome::Chosen(3));
    }

    #[test]
    fn long_menus_wrap_into_columns() {
        let items: Vec<Item<u8>> = (0..20).map(|i| Item::new("x", i)).collect();
        let screen = Rect::new(0, 0, 1000, 3 + 10 * 17 + 3);
        let m = Menus::open(items, Anchor::Point(0, 0), screen, 1.0, false, &Fixed);
        let l = &m.levels[0];
        assert_eq!(l.rows[10].y, 3);
        assert!(l.rows[10].x > l.rows[9].x);
        assert!(m.rect(0).unwrap().h <= screen.h);
    }

    #[test]
    fn submenus_open_to_the_left_when_the_right_is_full() {
        let mut m = Menus::open(sample(), Anchor::Point(960, 10), SCREEN, 1.0, true, &Fixed);
        let _ = m.key(Key::Char('m'), &Fixed);
        let (parent, sub) = (m.rect(0).unwrap(), m.rect(1).unwrap());
        assert_eq!(sub.right(), parent.x + 3);
    }

    #[test]
    fn leaving_a_menu_keeps_only_the_open_submenus_item_lit() {
        let mut m = open_at(10, 10);
        let (x, y) = centre(&m, 0, 4);
        let _ = m.pointer_moved(0, x, y, Instant::now());
        assert_eq!(m.pointer_left(0), Outcome::Changed);
        assert_eq!(m.levels[0].hot, None);
        let (x, y) = centre(&m, 0, 2);
        let _ = m.pressed(0, x, y, &Fixed);
        assert_eq!(m.pointer_left(0), Outcome::None);
        assert_eq!(m.levels[0].hot, Some(2));
    }

    #[test]
    fn drawing_lights_the_hot_item_and_marks_checked_ones() {
        let font = TtfFont::load_menu(font_px(1.0));
        let mut m = Menus::open(sample(), Anchor::Point(0, 0), SCREEN, 1.0, true, &font);
        let _ = m.key(Key::Char('m'), &font);
        let r = m.rect(1).unwrap();
        let mut c = Image::new(r.w, r.h);
        m.draw(1, &mut c, &font);
        assert_eq!(c.get(0, r.h - 1), DARK_SHADOW);
        assert_eq!(c.get(1, 1), HILIGHT);
        let lit = m.levels[1].rows[0];
        assert_eq!(
            c.get(lit.x + 1, lit.y + 1),
            SELECTION,
            "first item highlighted"
        );
        // Beta is checked: its column has black pixels.
        let beta = m.levels[1].rows[1];
        let marked =
            (beta.y..beta.bottom()).any(|y| (beta.x..beta.x + 17).any(|x| c.get(x, y) == TEXT));
        assert!(marked);
    }
}
