//! Windows 2000's "Windows Classic" look, which Winamp 2.x's menus and dialogs had whatever skin
//! was loaded: its colours, its bevelled edges and the small glyphs drawn in them.
//!
//! Sizes are Windows' at 96 DPI; [`px`] scales them, and [`line`] gives the width of a
//! one-pixel line, which Windows keeps thin when it scales.

use crate::canvas::{Image, Rect};

/// Buttons, menus and dialogs.
pub const FACE: [u8; 4] = [0xD4, 0xD0, 0xC8, 0xFF];
/// The lit edge of a bevel.
pub const HILIGHT: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
pub const SHADOW: [u8; 4] = [0x80, 0x80, 0x80, 0xFF];
pub const DARK_SHADOW: [u8; 4] = [0x40, 0x40, 0x40, 0xFF];
/// Edit boxes and lists.
pub const WINDOW: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
pub const TEXT: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
pub const GRAY_TEXT: [u8; 4] = [0x80, 0x80, 0x80, 0xFF];
pub const SELECTION: [u8; 4] = [0x0A, 0x24, 0x6A, 0xFF];
pub const SELECTION_TEXT: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
/// The frame around the default button.
pub const FRAME: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];

/// `v` pixels at 96 DPI, at `scale`.
#[must_use]
pub fn px(v: i32, scale: f64) -> i32 {
    // Control metrics are small and scales at most a few times 1, so the cast cannot truncate.
    (f64::from(v) * scale).round() as i32
}

/// The width of a one-pixel line at `scale`.
#[must_use]
pub fn line(scale: f64) -> i32 {
    // At least 1 and small, so the cast cannot truncate.
    scale.floor().max(1.0) as i32
}

/// Two-line bevel around the inside of `r`, `t` pixels per line: `outer` and `inner` colours
/// for the top and left edges, then for the bottom and right.
fn bevel(c: &mut Image, r: Rect, t: i32, top_left: [[u8; 4]; 2], bottom_right: [[u8; 4]; 2]) {
    for (i, (tl, br)) in (0..).zip(top_left.into_iter().zip(bottom_right)) {
        let r = Rect::new(r.x + i * t, r.y + i * t, r.w - 2 * i * t, r.h - 2 * i * t);
        c.fill_rect(Rect::new(r.x, r.bottom() - t, r.w, t), br);
        c.fill_rect(Rect::new(r.right() - t, r.y, t, r.h), br);
        c.fill_rect(Rect::new(r.x, r.y, r.w - t, t), tl);
        c.fill_rect(Rect::new(r.x, r.y, t, r.h - t), tl);
    }
}

/// The raised edge of a menu, a scroll bar's arrows and thumb.
pub fn raised(c: &mut Image, r: Rect, t: i32) {
    bevel(c, r, t, [FACE, HILIGHT], [DARK_SHADOW, SHADOW]);
}

/// The raised edge of a push button.
pub fn button(c: &mut Image, r: Rect, t: i32) {
    bevel(c, r, t, [HILIGHT, FACE], [DARK_SHADOW, SHADOW]);
}

/// The sunken edge of an edit box or a list.
pub fn sunken(c: &mut Image, r: Rect, t: i32) {
    bevel(c, r, t, [SHADOW, DARK_SHADOW], [HILIGHT, FACE]);
}

/// A one-line frame around the inside of `r`.
pub fn frame(c: &mut Image, r: Rect, t: i32, color: [u8; 4]) {
    c.fill_rect(Rect::new(r.x, r.y, r.w, t), color);
    c.fill_rect(Rect::new(r.x, r.bottom() - t, r.w, t), color);
    c.fill_rect(Rect::new(r.x, r.y, t, r.h), color);
    c.fill_rect(Rect::new(r.right() - t, r.y, t, r.h), color);
}

/// Fill `r` with a checkerboard of `a` and `b` in squares of `t` pixels: a scroll bar's track.
pub fn dither(c: &mut Image, r: Rect, t: i32, a: [u8; 4], b: [u8; 4]) {
    let t = t.max(1);
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            let odd = ((x / t) + (y / t)) % 2 != 0;
            c.put(x, y, if odd { b } else { a });
        }
    }
}

/// Draw a `#`-and-`.` pattern with each `#` a `k`-pixel square.
pub fn draw_glyph(c: &mut Image, pattern: &[&str], x: i32, y: i32, k: i32, color: [u8; 4]) {
    for (row, line) in (0..).zip(pattern) {
        for (col, b) in (0..).zip(line.bytes()) {
            if b == b'#' {
                c.fill_rect(Rect::new(x + col * k, y + row * k, k, k), color);
            }
        }
    }
}

/// The size of a glyph pattern drawn with `k`-pixel squares.
#[must_use]
pub fn glyph_size(pattern: &[&str], k: i32) -> (i32, i32) {
    let w = pattern.first().map_or(0, |row| row.len());
    (
        i32::try_from(w).unwrap_or(0) * k,
        i32::try_from(pattern.len()).unwrap_or(0) * k,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sunken_edges_are_dark_above_and_light_below() {
        let mut c = Image::new(10, 10);
        c.clear(WINDOW);
        sunken(&mut c, Rect::new(0, 0, 10, 10), 1);
        assert_eq!(c.get(5, 0), SHADOW);
        assert_eq!(c.get(5, 1), DARK_SHADOW);
        assert_eq!(c.get(5, 9), HILIGHT);
        assert_eq!(c.get(5, 8), FACE);
        assert_eq!(
            c.get(9, 0),
            HILIGHT,
            "the bottom-right edge wins the corner"
        );
        assert_eq!(c.get(5, 5), WINDOW);
    }

    #[test]
    fn edges_scale_by_whole_lines() {
        let mut c = Image::new(12, 12);
        c.clear(FACE);
        button(&mut c, Rect::new(0, 0, 12, 12), 2);
        assert_eq!(c.get(5, 1), HILIGHT);
        assert_eq!(c.get(5, 11), DARK_SHADOW);
        assert_eq!(c.get(5, 9), SHADOW);
        assert_eq!(glyph_size(&["##.", "#.."], 3), (9, 6));
    }
}
