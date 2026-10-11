//! Text rendering: the skin's 5x6 bitmap font (text.bmp) and a TrueType font for the playlist.

use crate::canvas::{Image, Rect};
use crate::skin::Skin;
use crate::sprites::font as f;

/// Map a character to its (column, row) in text.bmp. Unknown characters map to a space.
pub fn glyph_pos(ch: char) -> (i32, i32) {
    let ch = ch.to_ascii_uppercase();
    match ch {
        'A'..='Z' => ((ch as u8 - b'A') as i32, 0),
        '"' => (26, 0),
        '@' => (27, 0),
        '0'..='9' => ((ch as u8 - b'0') as i32, 1),
        '…' => (10, 1),
        '.' => (11, 1),
        ':' => (12, 1),
        '(' => (13, 1),
        ')' => (14, 1),
        '-' => (15, 1),
        '\'' | '`' => (16, 1),
        '!' => (17, 1),
        '_' => (18, 1),
        '+' => (19, 1),
        '\\' => (20, 1),
        '/' => (21, 1),
        '[' => (22, 1),
        ']' => (23, 1),
        '^' => (24, 1),
        '&' => (25, 1),
        '%' => (26, 1),
        ',' => (27, 1),
        '=' => (28, 1),
        '$' => (29, 1),
        '#' => (30, 1),
        'Å' | 'å' => (0, 2),
        'Ö' | 'ö' => (1, 2),
        'Ä' | 'ä' => (2, 2),
        '?' => (3, 2),
        '*' => (4, 2),
        _ => (30, 0),
    }
}

/// Draw `text` with the skin bitmap font at (x, y), clipped to `clip`.
pub fn draw_bitmap_text(c: &mut Image, skin: &Skin, text: &str, x: i32, y: i32, clip: Rect) {
    let mut px = x;
    for ch in text.chars() {
        if px >= clip.right() {
            break;
        }
        let (col, row) = glyph_pos(ch);
        skin.blit_clipped(c, f::glyph(col, row), px, y, clip);
        px += f::GLYPH_W;
    }
}

pub fn bitmap_text_width(text: &str) -> i32 {
    text.chars().count() as i32 * f::GLYPH_W
}

/// A small TrueType font for the playlist, rendered antialiased with half-pixel letter spacing
/// (the same metrics Webamp uses for its pixel-matched reproduction).
pub struct TtfFont {
    font: fontdue::Font,
    px: f32,
    ascent: i32,
    letter_spacing: f32,
}

const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/msttcorefonts/Arial.ttf",
    "/usr/share/fonts/truetype/msttcorefonts/arial.ttf",
    "/usr/share/fonts/TTF/arial.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
];

impl TtfFont {
    /// The playlist font as Winamp rendered it: a 9px Arial-like face.
    pub const DEFAULT_PX: f32 = 9.0;

    pub fn load_system(px: f32) -> Option<TtfFont> {
        for path in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path)
                && let Some(font) = Self::from_bytes(bytes, px)
            {
                log::info!("playlist font: {path}");
                return Some(font);
            }
        }
        log::warn!("no TrueType font found; playlist falls back to the bitmap font");
        None
    }

    pub fn from_bytes(bytes: Vec<u8>, px: f32) -> Option<TtfFont> {
        let settings = fontdue::FontSettings { scale: px, ..Default::default() };
        let font = fontdue::Font::from_bytes(bytes, settings).ok()?;
        let ascent = font
            .horizontal_line_metrics(px)
            .map(|m| m.ascent.round() as i32)
            .unwrap_or(px.round() as i32);
        Some(TtfFont { font, px, ascent, letter_spacing: 0.5 })
    }

    fn advance(&self, ch: char) -> f32 {
        self.font.metrics(ch, self.px).advance_width + self.letter_spacing
    }

    pub fn width(&self, text: &str) -> i32 {
        text.chars().map(|ch| self.advance(ch)).sum::<f32>().round() as i32
    }

    /// Draw `text` with its top at `top`, returning the advance in pixels.
    pub fn draw(&self, c: &mut Image, text: &str, x: i32, top: i32, color: [u8; 4], clip: Rect) -> i32 {
        let baseline = top + self.ascent;
        let mut pen = x as f32;
        for ch in text.chars() {
            let (m, bitmap) = self.font.rasterize(ch, self.px);
            let gx = pen.round() as i32 + m.xmin;
            let gy = baseline - m.ymin - m.height as i32;
            for yy in 0..m.height {
                for xx in 0..m.width {
                    let cov = bitmap[yy * m.width + xx];
                    if cov == 0 {
                        continue;
                    }
                    let px = gx + xx as i32;
                    let py = gy + yy as i32;
                    if clip.contains(px, py) {
                        c.put(px, py, blend(c.get(px, py), color, cov));
                    }
                }
            }
            pen += self.advance(ch);
        }
        pen.round() as i32 - x
    }
}

fn blend(dst: [u8; 4], src: [u8; 4], cov: u8) -> [u8; 4] {
    let a = cov as u32;
    let mix = |d: u8, s: u8| ((d as u32 * (255 - a) + s as u32 * a) / 255) as u8;
    [mix(dst[0], src[0]), mix(dst[1], src[1]), mix(dst[2], src[2]), 255]
}
