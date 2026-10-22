//! A list box with a vertical scroll bar, drawn like Windows 2000's, for the dialogs: one line
//! of text a row, one row highlighted, scrolled by the wheel, the scroll bar's arrows, its page
//! areas and its thumb.
//!
//! Geometry is in physical pixels: Windows' sizes at 96 DPI times the dialog's scale.

use crate::canvas::{Image, Rect};
use crate::classic::{self, FACE, GRAY_TEXT, SELECTION, SELECTION_TEXT, TEXT, WINDOW, line, px};
use crate::font::TtfFont;

/// Sizes at 96 DPI, which `px` scales.
mod metric {
    pub const ROW_H: i32 = 14;
    pub const SCROLL_W: i32 = 16;
    pub const MIN_THUMB: i32 = 8;
}

/// The scroll bar's arrows.
const UP: [&str; 4] = ["...#...", "..###..", ".#####.", "#######"];
const DOWN: [&str; 4] = ["#######", ".#####.", "..###..", "...#..."];

/// A part of the list the pointer can press.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Row(usize),
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    Thumb,
}

/// The list, its rows and where it is scrolled to.
#[derive(Clone, Debug)]
pub struct ListBox {
    scale: f64,
    /// One-pixel lines at this scale.
    t: i32,
    /// The list box, border included.
    list: Rect,
    /// The rows, inside the border and left of the scroll bar.
    rows: Rect,
    scroll_bar: Rect,
    row_h: i32,
    /// How many rows there are.
    len: usize,
    selected: Option<usize>,
    /// The first row shown.
    scroll: usize,
    /// The part pressed and not yet let go.
    held: Option<Part>,
    /// While the thumb is dragged: the pointer's offset from the thumb's top.
    thumb_grab: i32,
}

impl ListBox {
    /// An empty list filling `list`, border included, at `scale` times Windows' sizes.
    #[must_use]
    pub fn new(list: Rect, scale: f64) -> ListBox {
        let mut b = ListBox {
            scale,
            t: 1,
            list,
            rows: list,
            scroll_bar: list,
            row_h: 1,
            len: 0,
            selected: None,
            scroll: 0,
            held: None,
            thumb_grab: 0,
        };
        b.relayout(list, scale);
        b
    }

    /// Fill `list` at `scale` instead, keeping the rows and the highlight.
    pub fn relayout(&mut self, list: Rect, scale: f64) {
        let t = line(scale);
        let border = 2 * t;
        let scroll_w = px(metric::SCROLL_W, scale);
        let inner = Rect::new(
            list.x + border,
            list.y + border,
            list.w - 2 * border,
            list.h - 2 * border,
        );
        self.scale = scale;
        self.t = t;
        self.list = list;
        self.scroll_bar = Rect::new(inner.right() - scroll_w, inner.y, scroll_w, inner.h);
        self.rows = Rect::new(inner.x, inner.y, inner.w - scroll_w, inner.h);
        self.row_h = px(metric::ROW_H, scale).max(1);
        self.keep_selection_visible();
    }

    /// The list box, border included.
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.list
    }

    /// Where the rows are drawn.
    #[must_use]
    pub fn rows(&self) -> Rect {
        self.rows
    }

    #[must_use]
    pub fn row_h(&self) -> i32 {
        self.row_h
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The first row shown.
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Show `len` rows from the top, highlighting `selected` if it is one of them.
    pub fn reset(&mut self, len: usize, selected: Option<usize>) {
        self.len = len;
        self.selected = selected.filter(|&s| s < len);
        self.scroll = 0;
        self.keep_selection_visible();
    }

    /// There are `len` rows now, where the first ones stay as they were (more have been
    /// added at the end, say). The highlight and the scrolling stay put.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.selected = self.selected.filter(|&s| s < len);
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// Highlight `row`, scrolling it into view.
    pub fn select(&mut self, row: Option<usize>) {
        self.selected = row.filter(|&r| r < self.len);
        self.keep_selection_visible();
    }

    /// How many whole rows fit.
    #[must_use]
    pub fn visible_rows(&self) -> usize {
        usize::try_from(self.rows.h / self.row_h).unwrap_or(0)
    }

    #[must_use]
    pub fn max_scroll(&self) -> usize {
        self.len.saturating_sub(self.visible_rows())
    }

    fn keep_selection_visible(&mut self) {
        let rows = self.visible_rows().max(1);
        if let Some(s) = self.selected {
            if s < self.scroll {
                self.scroll = s;
            } else if s >= self.scroll + rows {
                self.scroll = s + 1 - rows;
            }
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// Highlight the row `delta` rows from the highlighted one, staying in the list; with
    /// none highlighted, the first.
    pub fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.len.checked_sub(1) else {
            return;
        };
        let to = match self.selected {
            Some(s) => s.saturating_add_signed(delta).min(last),
            None => 0,
        };
        self.selected = Some(to);
        self.keep_selection_visible();
    }

    fn scroll_by(&mut self, rows: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(rows)
            .min(self.max_scroll());
    }

    /// A page of rows, for Page Up and Page Down.
    #[must_use]
    pub fn page(&self) -> isize {
        isize::try_from(self.visible_rows().max(1)).unwrap_or(1)
    }

    /// The mouse wheel turned `notches`, positive away from the user.
    pub fn wheel(&mut self, notches: i32) {
        // Three rows a notch, as Windows' lists scroll.
        let rows = isize::try_from(notches).unwrap_or(0).saturating_mul(-3);
        self.scroll_by(rows);
    }

    fn arrow_up(&self) -> Rect {
        let s = self.scroll_bar;
        Rect::new(s.x, s.y, s.w, s.w.min(s.h / 2))
    }

    fn arrow_down(&self) -> Rect {
        let up = self.arrow_up();
        Rect::new(up.x, self.scroll_bar.bottom() - up.h, up.w, up.h)
    }

    /// The scroll bar between its arrows.
    fn track(&self) -> Rect {
        let (up, down) = (self.arrow_up(), self.arrow_down());
        Rect::new(up.x, up.bottom(), up.w, down.y - up.bottom())
    }

    /// The scroll bar's thumb, if the list is long enough to scroll.
    fn thumb(&self) -> Option<Rect> {
        let total = self.len;
        let shown = self.visible_rows();
        if total <= shown {
            return None;
        }
        let track = self.track();
        let ratio = |n: usize, of: usize| {
            // Row counts far below 2^53 convert exactly.
            (n as f64 / of.max(1) as f64).clamp(0.0, 1.0)
        };
        let min_h = px(metric::MIN_THUMB, self.scale);
        // Within the track's height, so the casts cannot truncate.
        let h = ((f64::from(track.h) * ratio(shown, total)).round() as i32)
            .max(min_h)
            .min(track.h);
        let y = track.y
            + (f64::from(track.h - h) * ratio(self.scroll, self.max_scroll())).round() as i32;
        Some(Rect::new(track.x, y, track.w, h))
    }

    /// The part of the list at (`x`, `y`), if any: a row that is there, or a part of the
    /// scroll bar.
    #[must_use]
    pub fn part_at(&self, x: i32, y: i32) -> Option<Part> {
        if self.rows.contains(x, y) {
            let row = usize::try_from((y - self.rows.y) / self.row_h).ok()?;
            let i = self.scroll + row;
            return (i < self.len).then_some(Part::Row(i));
        }
        if self.scroll_bar.contains(x, y) {
            if self.arrow_up().contains(x, y) {
                return Some(Part::ArrowUp);
            }
            if self.arrow_down().contains(x, y) {
                return Some(Part::ArrowDown);
            }
            let thumb = self.thumb()?;
            return Some(if y < thumb.y {
                Part::PageUp
            } else if y >= thumb.bottom() {
                Part::PageDown
            } else {
                Part::Thumb
            });
        }
        None
    }

    /// The pointer pressed at (`x`, `y`): a row is highlighted, the scroll bar scrolls. The
    /// part pressed, if any, stays held until [`ListBox::pointer_up`].
    pub fn pointer_down(&mut self, x: i32, y: i32) -> Option<Part> {
        let part = self.part_at(x, y)?;
        match part {
            Part::Row(i) => self.selected = Some(i),
            Part::ArrowUp => self.scroll_by(-1),
            Part::ArrowDown => self.scroll_by(1),
            Part::PageUp => self.scroll_by(-self.page()),
            Part::PageDown => self.scroll_by(self.page()),
            Part::Thumb => self.thumb_grab = self.thumb().map_or(0, |t| y - t.y),
        }
        self.held = Some(part);
        Some(part)
    }

    /// The pointer moved to `y` with a part held: a held thumb is dragged. True if the list
    /// scrolled.
    pub fn pointer_moved(&mut self, y: i32) -> bool {
        if self.held != Some(Part::Thumb) {
            return false;
        }
        let Some(thumb) = self.thumb() else {
            return false;
        };
        let track = self.track();
        let span = (track.h - thumb.h).max(1);
        let f = (f64::from(y - self.thumb_grab - track.y) / f64::from(span)).clamp(0.0, 1.0);
        // Row counts are far below 2^53, so the conversions are exact.
        let scroll = (f * self.max_scroll() as f64).round() as usize;
        if scroll == self.scroll {
            return false;
        }
        self.scroll = scroll;
        true
    }

    /// The pointer let go. True if a part was held.
    pub fn pointer_up(&mut self) -> bool {
        self.held.take().is_some()
    }

    /// Draw the list into `c`: its sunken box, the rows `label` writes (it returns false for
    /// a row to leave blank) and the scroll bar, greyed while everything fits. `font` must be
    /// the menu font at this scale.
    pub fn draw(
        &self,
        c: &mut Image,
        font: &TtfFont,
        mut label: impl FnMut(usize, &mut String) -> bool,
    ) {
        let t = self.t;
        let lh = font.line_height();
        let centred = |r: Rect| r.y + (r.h - lh) / 2;
        c.fill_rect(self.list, WINDOW);
        classic::sunken(c, self.list, t);
        let mut text = String::with_capacity(96);
        let shown = self.visible_rows() + 1;
        for row in (self.scroll..self.len).take(shown) {
            text.clear();
            if !label(row, &mut text) {
                continue;
            }
            let offset = i32::try_from(row - self.scroll).unwrap_or(0);
            let r = Rect::new(
                self.rows.x,
                self.rows.y + offset * self.row_h,
                self.rows.w,
                self.row_h,
            );
            let selected = self.selected == Some(row);
            if selected {
                c.fill_rect_clipped(r, SELECTION, self.rows);
            }
            let ink = if selected { SELECTION_TEXT } else { TEXT };
            let clip = r.intersect(&self.rows);
            font.draw(c, &text, r.x + px(2, self.scale), centred(r), ink, clip);
        }

        let thumb = self.thumb();
        let track = self.track();
        let held = |part: Part| self.held == Some(part);
        classic::dither(c, track, t, FACE, classic::HILIGHT);
        // A held page area darkens, as Windows' did.
        let dark = |c: &mut Image, part: Part, r: Rect| {
            if held(part) {
                c.fill_rect(r, classic::DARK_SHADOW);
            }
        };
        if let Some(th) = thumb {
            dark(
                c,
                Part::PageUp,
                Rect::new(track.x, track.y, track.w, th.y - track.y),
            );
            dark(
                c,
                Part::PageDown,
                Rect::new(track.x, th.bottom(), track.w, track.bottom() - th.bottom()),
            );
            c.fill_rect(th, FACE);
            classic::raised(c, th, t);
        }
        // Glyph pixels: whole squares, a few at most, so the cast cannot truncate.
        let k = (self.scale.round() as i32).max(1);
        for (part, r, glyph) in [
            (Part::ArrowUp, self.arrow_up(), &UP),
            (Part::ArrowDown, self.arrow_down(), &DOWN),
        ] {
            c.fill_rect(r, FACE);
            let down = held(part);
            if down {
                classic::frame(c, r, t, classic::SHADOW);
            } else {
                classic::raised(c, r, t);
            }
            let (gw, gh) = classic::glyph_size(glyph, k);
            let shift = if down { t } else { 0 };
            let (gx, gy) = (r.x + (r.w - gw) / 2 + shift, r.y + (r.h - gh) / 2 + shift);
            if thumb.is_some() {
                classic::draw_glyph(c, glyph, gx, gy, k, TEXT);
            } else {
                classic::draw_glyph(c, glyph, gx + t, gy + t, k, classic::HILIGHT);
                classic::draw_glyph(c, glyph, gx, gy, k, GRAY_TEXT);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn list(len: usize) -> ListBox {
        let mut b = ListBox::new(Rect::new(0, 0, 200, 150), 1.0);
        b.reset(len, None);
        b
    }

    #[test]
    fn keys_move_the_highlight_within_the_list_and_keep_it_in_view() {
        let mut b = list(100);
        assert_eq!(b.selected(), None);
        b.move_selection(1);
        assert_eq!(b.selected(), Some(0), "the first, from none");
        b.move_selection(b.page() * 3);
        let s = b.selected().unwrap();
        assert!(b.scroll() <= s && s < b.scroll() + b.visible_rows());
        b.move_selection(1000);
        assert_eq!(b.selected(), Some(99));
        assert_eq!(b.scroll(), b.max_scroll());
        b.move_selection(-1000);
        assert_eq!((b.selected(), b.scroll()), (Some(0), 0));
        let mut empty = list(0);
        empty.move_selection(1);
        assert_eq!(empty.selected(), None);
    }

    #[test]
    fn rows_added_at_the_end_keep_the_highlight_and_scrolling() {
        let mut b = list(20);
        b.select(Some(15));
        let scroll = b.scroll();
        b.set_len(70);
        assert_eq!((b.selected(), b.scroll()), (Some(15), scroll));
        b.set_len(10);
        assert_eq!(b.selected(), None, "gone");
        b.reset(5, Some(3));
        assert_eq!((b.selected(), b.scroll()), (Some(3), 0));
    }

    #[test]
    fn it_scrolls_by_wheel_arrows_page_areas_and_thumb() {
        let mut b = list(100);
        let shown = b.visible_rows();
        assert!(shown > 5 && shown < 100);
        b.wheel(-2);
        assert_eq!(b.scroll(), 6);
        let (ax, ay) = centre(b.arrow_up());
        assert_eq!(b.pointer_down(ax, ay), Some(Part::ArrowUp));
        assert!(b.pointer_up());
        assert_eq!(b.scroll(), 5);
        let thumb = b.thumb().unwrap();
        assert_eq!(
            b.pointer_down(thumb.x + 2, thumb.bottom() + 2),
            Some(Part::PageDown)
        );
        b.pointer_up();
        assert_eq!(b.scroll(), 5 + shown);
        let thumb = b.thumb().unwrap();
        b.pointer_down(thumb.x + 2, thumb.y + 1);
        assert!(b.pointer_moved(b.track().bottom() + 50));
        assert_eq!(b.scroll(), b.max_scroll());
        assert!(!b.pointer_moved(b.track().bottom() + 60), "already there");
        b.pointer_up();
        assert!(!b.pointer_up(), "nothing held");
        // A short list has no thumb, and its empty rows are nothing to press.
        let mut short = list(3);
        assert!(short.thumb().is_none());
        let rows = short.rows();
        assert_eq!(short.pointer_down(rows.x + 2, rows.bottom() - 2), None);
        assert_eq!(
            short.pointer_down(rows.x + 2, rows.y + short.row_h() + 1),
            Some(Part::Row(1))
        );
        assert_eq!(short.selected(), Some(1));
    }

    #[test]
    fn it_draws_rows_the_highlight_and_a_greyed_scroll_bar_when_all_fits() {
        let font = TtfFont::load_menu(crate::menu::font_px(1.0));
        let mut b = list(3);
        b.select(Some(1));
        let mut c = Image::new(200, 150);
        b.draw(&mut c, &font, |i, s| {
            s.push_str(["a", "b", "c"][i]);
            i != 2
        });
        let r = b.rows();
        assert_eq!(c.get(r.right() - 2, r.y + b.row_h() + 1), SELECTION);
        assert_eq!(c.get(r.right() - 2, r.y + 1), WINDOW);
        assert_eq!(c.get(b.rect().x, b.rect().y), classic::SHADOW, "sunken");
        let (ax, ay) = centre(b.arrow_up());
        let greyed = (ay - 6..ay + 6).any(|y| c.get(ax, y) == GRAY_TEXT);
        assert!(greyed, "nothing to scroll");
    }
}
