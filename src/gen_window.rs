//! Drawing of Winamp's generic window: the gen.bmp frame that plug-ins such as MilkDrop
//! showed in, with a title spelled in gen.bmp's own letters. The layout follows Webamp's
//! `GenWindow` and its stylesheet.

use crate::canvas::{Canvas, Rect};
use crate::skin::Skin;
use crate::sprites::{Sheet, Sprite, generic as g};

/// Where a point in a generic window falls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GenPart {
    /// The title bar, sides and bottom, which move the window.
    Frame,
    Close,
    /// The grip in the bottom-right corner.
    Resize,
    /// The window's contents.
    Inside,
}

/// The contents' rectangle in a window `size` skin pixels big.
#[must_use]
pub fn inner(size: (i32, i32)) -> Rect {
    let (w, h) = size;
    Rect::new(
        g::LEFT_W,
        g::TOP_H,
        (w - g::LEFT_W - g::RIGHT_W).max(0),
        (h - g::TOP_H - g::BOTTOM_H).max(0),
    )
}

/// What is at `(x, y)` in a window `size` big.
#[must_use]
pub fn hit(size: (i32, i32), x: i32, y: i32) -> Option<GenPart> {
    let (w, h) = size;
    if !Rect::new(0, 0, w, h).contains(x, y) {
        return None;
    }
    let close = Rect::new(
        w - g::CLOSE_FROM_RIGHT,
        g::CLOSE_Y,
        g::CLOSE_SIZE,
        g::CLOSE_SIZE,
    );
    Some(if close.contains(x, y) {
        GenPart::Close
    } else if x >= w - g::RESIZE_SIZE && y >= h - g::RESIZE_SIZE {
        GenPart::Resize
    } else if inner(size).contains(x, y) {
        GenPart::Inside
    } else {
        GenPart::Frame
    })
}

/// The letters A to Z in gen.bmp's row at `y`: each one's left edge and width, found by
/// looking for the background colour between them, as Webamp does.
fn letters(skin: &Skin, y: i32) -> [(i32, i32); 26] {
    let sheet = skin.sheet(Sheet::Gen);
    let width = sheet.size().0;
    let background = sheet.get(0, y);
    let mut x = 1;
    std::array::from_fn(|_| {
        let mut next = x;
        while next < width && sheet.get(next, y) != background {
            next += 1;
        }
        let letter = (x, next - x);
        x = next + 1;
        letter
    })
}

/// Draw `title` in gen.bmp's letters at `(x, y)`; returns its width. Letters it lacks are
/// left out, and spaces are gaps.
fn draw_title(
    mut c: Option<&mut Canvas>,
    skin: &Skin,
    title: &str,
    active: bool,
    x: i32,
    y: i32,
) -> i32 {
    let row = if active {
        g::LETTERS_Y_ACTIVE
    } else {
        g::LETTERS_Y
    };
    let glyphs = letters(skin, row);
    let mut pen = x;
    for ch in title.chars() {
        let ch = ch.to_ascii_uppercase();
        if ch == ' ' {
            pen += g::SPACE_W;
            continue;
        }
        let Some(i) = ch
            .is_ascii_uppercase()
            .then(|| usize::from(ch as u8 - b'A'))
        else {
            continue;
        };
        let (sx, w) = glyphs[i];
        if let Some(c) = c.as_deref_mut() {
            let sprite = Sprite {
                sheet: Sheet::Gen,
                x: sx,
                y: row,
                w,
                h: g::LETTER_H,
            };
            skin.blit(c, sprite, pen, y);
        }
        pen += w;
    }
    pen - x
}

/// Draw the frame of a generic window over `r`, titled `title`. The contents are left alone.
pub fn draw(c: &mut Canvas, skin: &Skin, r: Rect, title: &str, active: bool, close_pressed: bool) {
    let (ox, oy, w, h) = (r.x, r.y, r.w, r.h);
    let sheet = skin.sheet_at(Sheet::Gen, c.density());

    // Title bar: the centre fill under everything, then corners, fills, ends and the title,
    // the spare width shared between the fills on either side.
    c.tile_h(
        sheet,
        g::top_center_fill(active).rect(),
        Rect::new(ox, oy, w, g::TOP_H),
    );
    let title_w = draw_title(None, skin, title, active, 0, 0);
    let middle = g::TITLE_PAD_LEFT + title_w + g::TITLE_PAD_RIGHT;
    let spare = (w - 4 * 25 - middle).max(0);
    let left_fill = spare / 2;
    let right_fill = spare - left_fill;
    let fill = g::top_fill(active).rect();
    c.tile_h(sheet, fill, Rect::new(ox + 25, oy, left_fill, g::TOP_H));
    let left_end_x = ox + 25 + left_fill;
    skin.blit(c, g::top_left_end(active), left_end_x, oy);
    let title_x = left_end_x + 25;
    draw_title(
        Some(c),
        skin,
        title,
        active,
        title_x + g::TITLE_PAD_LEFT,
        oy + g::TITLE_Y,
    );
    let right_end_x = title_x + middle;
    skin.blit(c, g::top_right_end(active), right_end_x, oy);
    // The right fill's tiles line up with the right edge, as Webamp's background-position does.
    let right_fill_x = right_end_x + 25;
    let tile = fill.w;
    let first = right_fill % tile;
    if first > 0 {
        c.blit_clipped(
            sheet,
            Rect::new(fill.x + tile - first, fill.y, first, fill.h),
            right_fill_x,
            oy,
            Rect::new(right_fill_x, oy, first, g::TOP_H),
        );
    }
    c.tile_h(
        sheet,
        fill,
        Rect::new(right_fill_x + first, oy, right_fill - first, g::TOP_H),
    );
    skin.blit(c, g::top_left(active), ox, oy);
    skin.blit(c, g::top_right(active), ox + w - 25, oy);
    if close_pressed {
        skin.blit(
            c,
            g::CLOSE_PRESSED,
            ox + w - g::CLOSE_FROM_RIGHT,
            oy + g::CLOSE_Y,
        );
    }

    // Sides: tiles down from the top, with their bottom pieces at the foot.
    let side_y = oy + g::TOP_H;
    let side_h = h - g::TOP_H - g::BOTTOM_H;
    c.tile_v(
        sheet,
        g::MIDDLE_LEFT.rect(),
        Rect::new(ox, side_y, g::LEFT_W, side_h),
    );
    c.tile_v(
        sheet,
        g::MIDDLE_RIGHT.rect(),
        Rect::new(ox + w - g::RIGHT_W, side_y, g::RIGHT_W, side_h),
    );
    let clip = Rect::new(ox, side_y, w, side_h.max(0));
    let bottom_piece = g::MIDDLE_LEFT_BOTTOM.h;
    skin.blit_clipped(
        c,
        g::MIDDLE_LEFT_BOTTOM,
        ox,
        side_y + side_h - bottom_piece,
        clip,
    );
    skin.blit_clipped(
        c,
        g::MIDDLE_RIGHT_BOTTOM,
        ox + w - g::RIGHT_W,
        side_y + side_h - bottom_piece,
        clip,
    );

    // Bottom: the fill, with the two ends over it.
    let by = oy + h - g::BOTTOM_H;
    c.tile_h(
        sheet,
        g::BOTTOM_FILL.rect(),
        Rect::new(ox, by, w, g::BOTTOM_H),
    );
    skin.blit(c, g::BOTTOM_LEFT, ox, by);
    skin.blit(c, g::BOTTOM_RIGHT, ox + w - g::BOTTOM_RIGHT.w, by);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::canvas::Density;

    fn base() -> Skin {
        Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap()
    }

    #[test]
    fn parts_of_the_window() {
        let size = (275, 116);
        assert_eq!(hit(size, 268, 6), Some(GenPart::Close));
        assert_eq!(hit(size, 270, 110), Some(GenPart::Resize));
        assert_eq!(hit(size, 100, 5), Some(GenPart::Frame));
        assert_eq!(hit(size, 3, 60), Some(GenPart::Frame));
        assert_eq!(hit(size, 100, 60), Some(GenPart::Inside));
        assert_eq!(hit(size, 300, 60), None);
        assert_eq!(inner(size), Rect::new(11, 20, 256, 82));
    }

    #[test]
    fn the_base_skin_has_every_letter() {
        let skin = base();
        for row in [g::LETTERS_Y_ACTIVE, g::LETTERS_Y] {
            let l = letters(&skin, row);
            // Z runs on to the sheet's edge, as it does in Webamp.
            assert!(l[..25].iter().all(|(_, w)| (3..=8).contains(w)), "{l:?}");
            // Left to right, without overlapping.
            assert!(l.windows(2).all(|p| p[1].0 > p[0].0 + p[0].1 - 1));
        }
        let w = draw_title(None, &skin, "MILKDROP", true, 0, 0);
        assert!((30..70).contains(&w), "{w}");
    }

    #[test]
    fn the_frame_fills_the_window_at_any_size() {
        let skin = base();
        for size in [(275, 116), (300, 145), (500, 290)] {
            let mut c = Canvas::new(size.0, size.1, Density::ONE);
            c.fill_rect(Rect::new(0, 0, size.0, size.1), [255, 0, 255, 255]);
            draw(
                &mut c,
                &skin,
                Rect::new(0, 0, size.0, size.1),
                "MilkDrop",
                true,
                false,
            );
            let r = inner(size);
            // Every pixel of the frame is drawn; the inside is untouched.
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let magenta = c.get(x, y) == [255, 0, 255, 255];
                    assert_eq!(magenta, r.contains(x, y), "{size:?} at {x},{y}");
                }
            }
        }
    }
}
