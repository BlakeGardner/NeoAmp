//! Text rendering: the skin's 5x6 bitmap font (text.bmp) and TrueType fonts for the playlist
//! and the menus.

use std::path::{Path, PathBuf};

use crate::canvas::{Image, Rect};
use crate::shortcut::Style;
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

/// Draw a sequence of characters with the skin bitmap font at (x, y), clipped to `clip`.
pub fn draw_bitmap_chars<I>(c: &mut Image, skin: &Skin, chars: I, x: i32, y: i32, clip: Rect)
where
    I: IntoIterator<Item = char>,
{
    let mut px = x;
    for ch in chars {
        if px >= clip.right() {
            break;
        }
        let (col, row) = glyph_pos(ch);
        skin.blit_clipped(c, f::glyph(col, row), px, y, clip);
        px += f::GLYPH_W;
    }
}

/// Draw `text` with the skin bitmap font at (x, y), clipped to `clip`.
pub fn draw_bitmap_text(c: &mut Image, skin: &Skin, text: &str, x: i32, y: i32, clip: Rect) {
    draw_bitmap_chars(c, skin, text.chars(), x, y, clip);
}

#[must_use]
pub fn bitmap_text_width(text: &str) -> i32 {
    i32::try_from(text.chars().count())
        .unwrap_or(i32::MAX / 2)
        .saturating_mul(f::GLYPH_W)
}

/// A small TrueType font, rendered antialiased. The playlist's has half-pixel letter spacing
/// (the same metrics Webamp uses for its pixel-matched reproduction); the menus' has none.
#[derive(Clone)]
pub struct TtfFont {
    font: fontdue::Font,
    /// Draws the characters `font` has no glyph for (the menus' key symbols on macOS).
    fallback: Option<fontdue::Font>,
    px: f32,
    ascent: i32,
    descent: i32,
    letter_spacing: f32,
}

/// Arial and its look-alikes, for the playlist (and the menus when there is no Tahoma).
/// Windows' own folder is added by [`windows_font`].
const SANS: &[&str] = &[
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
];

/// Tahoma, the menu font of Windows 2000 and XP, where Winamp 2.x ran. macOS ships it, and so
/// does Wine (its own lookalike).
const TAHOMA: &[&str] = &[
    "/usr/share/wine/fonts/tahoma.ttf",
    "/usr/share/fonts/truetype/msttcorefonts/tahoma.ttf",
    "/usr/share/fonts/truetype/msttcorefonts/Tahoma.ttf",
    "/usr/share/fonts/TTF/tahoma.ttf",
    "/System/Library/Fonts/Supplemental/Tahoma.ttf",
];

/// Fonts with the key symbols macOS writes shortcuts with (⌘ ⌥ ⇧), which Tahoma and Arial
/// lack. Windows' own folder is added by [`windows_font`] (Segoe UI Symbol).
const SYMBOLS: &[&str] = &[
    "/System/Library/Fonts/LucidaGrande.ttc",
    "/System/Library/Fonts/Apple Symbols.ttf",
    "/System/Library/Fonts/Menlo.ttc",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
];

/// `file` in Windows' font folder (`%WINDIR%\Fonts`). `None` elsewhere.
fn windows_font(file: &str) -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("WINDIR").map(|dir| Path::new(&dir).join("Fonts").join(file))
    } else {
        None
    }
}

/// Where to look for a font: Windows' font folder first, then `paths`.
fn candidates(windows_file: &str, paths: &[&str]) -> Vec<PathBuf> {
    windows_font(windows_file)
        .into_iter()
        .chain(paths.iter().map(PathBuf::from))
        .collect()
}

impl std::fmt::Debug for TtfFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtfFont")
            .field("px", &self.px)
            .field("ascent", &self.ascent)
            .field("descent", &self.descent)
            .field("letter_spacing", &self.letter_spacing)
            .field("fallback", &self.fallback.is_some())
            .finish_non_exhaustive()
    }
}

impl TtfFont {
    /// The playlist font as Winamp rendered it: a 9px Arial-like face.
    pub const DEFAULT_PX: f32 = 9.0;
    /// The playlist's letter spacing.
    const PLAYLIST_SPACING: f32 = 0.5;

    /// The playlist font: the first Arial-like font found.
    #[must_use]
    pub fn load_system(px: f32) -> Option<TtfFont> {
        let font = Self::load_first(&candidates("arial.ttf", SANS), px, Self::PLAYLIST_SPACING);
        if font.is_none() {
            log::warn!("no TrueType font found; playlist falls back to the bitmap font");
        }
        font
    }

    /// The menu font: Tahoma if the system has it, else the playlist's face, without letter
    /// spacing, with a fallback for the symbols this platform's shortcuts are written with.
    #[must_use]
    pub fn load_menu(px: f32) -> Option<TtfFont> {
        let mut paths = candidates("tahoma.ttf", TAHOMA);
        paths.extend(candidates("arial.ttf", SANS));
        let font = Self::load_first(&paths, px, 0.0);
        if font.is_none() {
            log::warn!("no TrueType font found; menus cannot be shown");
        }
        font.map(|font| font.with_symbols(Style::CURRENT.symbols()))
    }

    /// This font, drawing those of `symbols` it lacks with the first font in [`SYMBOLS`] that
    /// has them all.
    #[must_use]
    pub fn with_symbols(self, symbols: &[char]) -> TtfFont {
        let lacking = || {
            symbols
                .iter()
                .copied()
                .filter(|&ch| !self.font.has_glyph(ch))
        };
        if lacking().next().is_none() {
            return self;
        }
        let fallback = candidates("seguisym.ttf", SYMBOLS).iter().find_map(|path| {
            let font = Self::parse(std::fs::read(path).ok()?, self.px)?;
            lacking().all(|ch| font.has_glyph(ch)).then(|| {
                log::info!("symbol font: {}", path.display());
                font
            })
        });
        if fallback.is_none() {
            let missing: String = lacking().collect();
            log::warn!("no font found with {missing}; the menus show boxes for them");
        }
        TtfFont { fallback, ..self }
    }

    fn load_first(paths: &[PathBuf], px: f32, letter_spacing: f32) -> Option<TtfFont> {
        paths.iter().find_map(|path| {
            let font = Self::from_bytes(std::fs::read(path).ok()?, px)?;
            log::info!("font: {}", path.display());
            Some(TtfFont {
                letter_spacing,
                ..font
            })
        })
    }

    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>, px: f32) -> Option<TtfFont> {
        let font = Self::parse(bytes, px)?;
        Some(Self::sized(font, px, Self::PLAYLIST_SPACING))
    }

    /// A font file (the first face of a collection), made ready for `px`.
    fn parse(bytes: Vec<u8>, px: f32) -> Option<fontdue::Font> {
        let settings = fontdue::FontSettings {
            scale: px,
            ..Default::default()
        };
        fontdue::Font::from_bytes(bytes, settings).ok()
    }

    fn sized(font: fontdue::Font, px: f32, letter_spacing: f32) -> TtfFont {
        let metrics = font.horizontal_line_metrics(px);
        let ascent = metrics.map_or(px.round() as i32, |m| m.ascent.round() as i32);
        // fontdue reports the descent as a negative number.
        let descent = metrics.map_or(0, |m| (-m.descent).round() as i32);
        TtfFont {
            font,
            fallback: None,
            px,
            ascent,
            descent,
            letter_spacing,
        }
    }

    /// The same face at another size.
    #[must_use]
    pub fn resized(&self, px: f32) -> TtfFont {
        TtfFont {
            fallback: self.fallback.clone(),
            ..Self::sized(self.font.clone(), px, self.letter_spacing)
        }
    }

    /// The face that draws `ch`.
    fn face(&self, ch: char) -> &fontdue::Font {
        match &self.fallback {
            Some(fallback) if !self.font.has_glyph(ch) && fallback.has_glyph(ch) => fallback,
            _ => &self.font,
        }
    }

    /// The size in pixels (the em height).
    #[must_use]
    pub fn px(&self) -> f32 {
        self.px
    }

    /// Pixels from the top of a line of text to its baseline.
    #[must_use]
    pub fn ascent(&self) -> i32 {
        self.ascent
    }

    /// Pixels from the top of a line to the bottom of its lowest descender.
    #[must_use]
    pub fn line_height(&self) -> i32 {
        self.ascent + self.descent
    }

    fn advance(&self, ch: char) -> f32 {
        self.face(ch).metrics(ch, self.px).advance_width + self.letter_spacing
    }

    #[must_use]
    pub fn width(&self, text: &str) -> i32 {
        text.chars().map(|ch| self.advance(ch)).sum::<f32>().round() as i32
    }

    /// Draw `text` with its top at `top`, returning the advance in pixels.
    pub fn draw(
        &self,
        c: &mut Image,
        text: &str,
        x: i32,
        top: i32,
        color: [u8; 4],
        clip: Rect,
    ) -> i32 {
        let baseline = top + self.ascent;
        let mut pen = x as f32;
        for ch in text.chars() {
            let (m, bitmap) = self.face(ch).rasterize(ch, self.px);
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
    [
        mix(dst[0], src[0]),
        mix(dst[1], src[1]),
        mix(dst[2], src[2]),
        255,
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn glyph_lookup_is_case_insensitive_and_falls_back_to_space() {
        assert_eq!(glyph_pos('a'), glyph_pos('A'));
        assert_eq!(glyph_pos('A'), (0, 0));
        assert_eq!(glyph_pos('0'), (0, 1));
        assert_eq!(glyph_pos(':'), (12, 1));
        assert_eq!(glyph_pos('?'), (3, 2));
        assert_eq!(glyph_pos('€'), glyph_pos(' '));
    }

    #[test]
    fn bitmap_width_is_five_pixels_per_character() {
        assert_eq!(bitmap_text_width("abc"), 15);
        assert_eq!(bitmap_text_width(""), 0);
    }

    #[test]
    fn a_symbol_font_draws_the_mac_key_symbols_the_menu_font_lacks() {
        let Some(font) = TtfFont::load_menu(12.0) else {
            return; // no system font to draw with
        };
        let font = font.with_symbols(Style::Mac.symbols());
        for &ch in Style::Mac.symbols() {
            // Only a system with no font that has them all leaves them as boxes.
            assert!(
                font.face(ch).has_glyph(ch) || font.fallback.is_none(),
                "{ch}"
            );
        }
        assert!(
            std::ptr::eq(font.face('A'), &font.font),
            "letters stay in the menu font"
        );
        let resized = font.resized(18.0);
        assert_eq!(resized.fallback.is_some(), font.fallback.is_some());
    }

    #[test]
    fn blend_is_a_linear_mix() {
        assert_eq!(
            blend([0, 0, 0, 255], [255, 255, 255, 255], 0),
            [0, 0, 0, 255]
        );
        assert_eq!(
            blend([0, 0, 0, 255], [255, 255, 255, 255], 255),
            [255, 255, 255, 255]
        );
        assert_eq!(blend([0, 0, 0, 255], [200, 100, 0, 255], 51)[0], 40);
    }
}
