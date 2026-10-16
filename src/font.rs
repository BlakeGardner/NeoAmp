//! Text rendering: the skin's 5x6 bitmap font (text.bmp) and TrueType fonts for the playlist
//! and the menus. The TrueType fonts are built into NeoAmp, which reads none from the system,
//! so the text looks the same everywhere: the font a skin's `pledit.txt` names gets the
//! built-in one made to stand in for it.

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

// The fonts built into NeoAmp. NeoAmp reads no fonts from the system: fontdue draws these the
// same way everywhere, so the text looks the same on every system. `fonts/README.md` says where
// each comes from and under which licence.

/// Liberation Sans, made to Arial's measurements: the playlist's font for Arial (nearly every
/// skin names it) and for any family not built in, and the menus' and dialogs' font.
const SANS: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
/// Liberation Mono, made to Courier New's measurements.
const MONO: &[u8] = include_bytes!("../fonts/LiberationMono-Regular.ttf");
/// Liberation Serif, made to Times New Roman's measurements.
const SERIF: &[u8] = include_bytes!("../fonts/LiberationSerif-Regular.ttf");
/// Wine's Tahoma, its free stand-in for Microsoft's.
const TAHOMA: &[u8] = include_bytes!("../fonts/tahoma.ttf");
/// DejaVu Sans, which has the key symbols macOS writes shortcuts with (⌘ ⌥ ⇧ ← → ⌦), which
/// the menus' font lacks.
const DEJAVU_SANS: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");

/// The font families skins name, matched ignoring case as Windows matched them, and the
/// built-in font each gets: one made to the same measurements where there is one, else the
/// nearest in look and width.
const BUILT_IN: &[(&str, &[u8])] = &[
    ("Arial", SANS),
    ("Helvetica", SANS),
    ("Liberation Sans", SANS),
    ("Courier New", MONO),
    ("Courier", MONO),
    ("Andale Mono", MONO),
    // Within a pixel in 50 characters of Lucida Console's width.
    ("Lucida Console", MONO),
    ("Lucida Sans Typewriter", MONO),
    // Old X11 skins (from XMMS) name X's own fonts.
    ("Fixed", MONO),
    ("Liberation Mono", MONO),
    ("Times New Roman", SERIF),
    ("Times", SERIF),
    ("Liberation Serif", SERIF),
    ("Tahoma", TAHOMA),
    // Windows' bitmap font, which Tahoma was drawn after.
    ("MS Sans Serif", TAHOMA),
    ("Microsoft Sans Serif", TAHOMA),
    // About 3% narrower than Verdana, and like it in look.
    ("Verdana", DEJAVU_SANS),
    ("Lucida Sans", DEJAVU_SANS),
    ("Lucida Sans Unicode", DEJAVU_SANS),
    ("DejaVu Sans", DEJAVU_SANS),
];

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

    /// The playlist's default font: the built-in Liberation Sans, made to Arial's measurements.
    #[must_use]
    pub fn load_default(px: f32) -> TtfFont {
        Self::built_in(SANS, px, Self::PLAYLIST_SPACING)
    }

    /// The playlist font a skin names (`Font=` in its `pledit.txt`): the built-in font for that
    /// family (`BUILT_IN`), else the default (see [`TtfFont::load_default`]).
    #[must_use]
    pub fn load_playlist(family: &str, px: f32) -> TtfFont {
        let family = family.trim();
        let bytes = BUILT_IN
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(family))
            .map(|&(_, bytes)| bytes);
        if bytes.is_none() && !family.is_empty() {
            log::info!("the skin's font {family} is not built in; the playlist uses the default");
        }
        Self::built_in(bytes.unwrap_or(SANS), px, Self::PLAYLIST_SPACING)
    }

    /// The menus' and dialogs' font: the built-in Liberation Sans, without letter spacing, with
    /// a fallback for the symbols this platform's shortcuts are written with.
    #[must_use]
    pub fn load_menu(px: f32) -> TtfFont {
        Self::built_in(SANS, px, 0.0).with_symbols(Style::CURRENT.symbols())
    }

    /// One of the fonts built into NeoAmp, with its sizes worked out for `px`.
    fn built_in(bytes: &[u8], px: f32, letter_spacing: f32) -> TtfFont {
        Self::sized(Self::parse(bytes, px), px, letter_spacing)
    }

    /// This font, drawing those of `symbols` it lacks with the built-in DejaVu Sans, which has
    /// all the symbols shortcuts are written with.
    #[must_use]
    pub fn with_symbols(self, symbols: &[char]) -> TtfFont {
        if symbols.iter().all(|&ch| self.font.has_glyph(ch)) {
            return self;
        }
        let fallback = Some(Self::parse(DEJAVU_SANS, self.px));
        TtfFont { fallback, ..self }
    }

    /// A built-in font file, made ready for `px`.
    fn parse(bytes: &[u8], px: f32) -> fontdue::Font {
        let settings = fontdue::FontSettings {
            scale: px,
            ..Default::default()
        };
        fontdue::Font::from_bytes(bytes, settings)
            .expect("the built-in fonts are TrueType files that fontdue reads")
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

    /// The face's full name, as its file gives it ("Liberation Sans").
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.font.name()
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
    fn the_built_in_symbol_font_draws_the_mac_key_symbols_the_menu_font_lacks() {
        let plain = || TtfFont::built_in(SANS, 12.0, 0.0);
        let font = plain().with_symbols(Style::Mac.symbols());
        assert!(font.fallback.is_some(), "the menu font lacks them");
        for &ch in Style::Mac.symbols() {
            assert!(font.face(ch).has_glyph(ch), "{ch}");
        }
        assert!(
            std::ptr::eq(font.face('A'), &font.font),
            "letters stay in the menu font"
        );
        let resized = font.resized(18.0);
        assert!(resized.fallback.is_some());
        let font = plain().with_symbols(Style::Windows.symbols());
        assert!(font.fallback.is_none(), "no symbols, no fallback");
        // The menus get the fallback where shortcuts are written with symbols (macOS).
        let menu = TtfFont::load_menu(12.0);
        assert_eq!(
            menu.fallback.is_some(),
            !Style::CURRENT.symbols().is_empty()
        );
    }

    #[test]
    fn every_built_in_font_can_be_drawn() {
        for &(family, bytes) in BUILT_IN {
            let font = TtfFont::built_in(bytes, TtfFont::DEFAULT_PX, 0.0);
            assert!(font.name().is_some(), "{family}");
            assert!(font.line_height() > 0, "{family}");
        }
    }

    #[test]
    fn the_skins_font_gets_its_built_in_stand_in_whatever_its_case() {
        let px = TtfFont::DEFAULT_PX;
        for (family, face) in [
            ("Arial", "Liberation Sans"),
            (" arial ", "Liberation Sans"),
            ("COURIER NEW", "Liberation Mono"),
            ("Times New Roman", "Liberation Serif"),
            ("liberation serif", "Liberation Serif"),
            ("tahoma", "Tahoma"),
            ("DejaVu Sans", "DejaVu Sans"),
        ] {
            let font = TtfFont::load_playlist(family, px);
            assert_eq!(font.name(), Some(face), "{family:?}");
            assert_eq!(font.letter_spacing, TtfFont::PLAYLIST_SPACING);
        }
        let menu = TtfFont::load_menu(12.0);
        assert_eq!(menu.name(), Some("Liberation Sans"));
        assert_eq!(menu.letter_spacing, 0.0);
    }

    #[test]
    fn a_font_that_is_not_built_in_gets_the_default() {
        let px = TtfFont::DEFAULT_PX;
        let default = TtfFont::load_default(px);
        assert_eq!(default.name(), Some("Liberation Sans"));
        for family in ["", "No Such Font 12345", "Broadway"] {
            let font = TtfFont::load_playlist(family, px);
            assert_eq!(font.name(), default.name(), "{family:?}");
        }
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
