//! The dialog that tells of a new release (see [`crate::update`]), drawn like the Windows
//! 2000 dialogs Winamp 2.x showed. Download opens the release's page in the browser, Skip
//! This Version tells of that release no more, and Later closes it until the next day's check.
//!
//! This is the dialog's state, layout, input and drawing; `update_window` gives it an OS
//! window. Geometry is in physical pixels, as in [`crate::jump`].

use crate::canvas::{Image, Rect};
use crate::classic::{self, FACE, TEXT, line, px};
use crate::font::TtfFont;
use crate::menu::Measure;
use crate::update::Version;

/// Sizes at 96 DPI, which `px` scales.
mod metric {
    /// The narrowest the dialog gets, between its margins.
    pub const MIN_W: i32 = 300;
    pub const MARGIN: i32 = 8;
    pub const GAP: i32 = 6;
    /// A line of text.
    pub const LINE_H: i32 = 14;
    pub const BUTTON_W: i32 = 75;
    pub const BUTTON_H: i32 = 23;
    /// Room either side of a label too long for a button of the usual width.
    pub const BUTTON_PAD: i32 = 8;
}

/// The window's title.
pub const TITLE: &str = "NeoAmp Update";
const DOWNLOAD: &str = "Download";
const SKIP: &str = "Skip This Version";
const LATER: &str = "Later";
const QUESTION: &str = "Download it from its release page?";
const HINT: &str = "Options > Check for Updates at Startup turns these checks off.";

/// What a key does in the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Enter,
    Escape,
}

/// What the window layer must do after an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    None,
    Redraw,
    /// Open the release's page and close the dialog.
    Download,
    /// Tell of this release no more, and close the dialog.
    Skip,
    /// Close the dialog.
    Close,
}

/// A button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Download,
    Skip,
    Later,
}

/// Where everything is, in the dialog's pixels.
#[derive(Clone, Copy, Debug)]
struct Layout {
    size: (i32, i32),
    /// One-pixel lines at this scale.
    t: i32,
    /// Where each line of text starts.
    lines: [(i32, i32); 3],
    download: Rect,
    skip: Rect,
    later: Rect,
}

impl Layout {
    /// The layout at `scale` for `text`, measured with `m`, the menu font at that scale. The
    /// dialog is as wide as its longest line or its buttons need.
    fn new(scale: f64, text: &[String; 3], m: &dyn Measure) -> Layout {
        let p = |v| px(v, scale);
        let margin = p(metric::MARGIN);
        let gap = p(metric::GAP);
        let line_h = p(metric::LINE_H);
        // The hint is set apart from the news.
        let lines = [
            (margin, margin),
            (margin, margin + line_h),
            (margin, margin + 2 * line_h + gap),
        ];
        let button_w =
            |label: &str| p(metric::BUTTON_W).max(m.text_width(label) + 2 * p(metric::BUTTON_PAD));
        let widths = [button_w(DOWNLOAD), button_w(SKIP), button_w(LATER)];
        let buttons = widths.iter().sum::<i32>() + 2 * gap;
        let widest = text.iter().map(|l| m.text_width(l)).max().unwrap_or(0);
        let w = p(metric::MIN_W).max(widest).max(buttons) + 2 * margin;
        let y = lines[2].1 + line_h + 2 * gap;
        let h = p(metric::BUTTON_H);
        let later = Rect::new(w - margin - widths[2], y, widths[2], h);
        let skip = Rect::new(later.x - gap - widths[1], y, widths[1], h);
        let download = Rect::new(skip.x - gap - widths[0], y, widths[0], h);
        Layout {
            size: (w, later.bottom() + margin),
            t: line(scale),
            lines,
            download,
            skip,
            later,
        }
    }
}

/// The dialog.
#[derive(Debug)]
pub struct UpdateDialog {
    version: Version,
    text: [String; 3],
    layout: Layout,
    pressed: Option<Part>,
    /// The last pointer position: a held button shows pushed in only while under it.
    pointer: (i32, i32),
}

impl UpdateDialog {
    /// A dialog telling of NeoAmp `version`, at `scale` times Windows' 96 DPI sizes. `m` must
    /// be the menu font at this scale ([`crate::menu::font_px`]).
    #[must_use]
    pub fn new(version: Version, scale: f64, m: &dyn Measure) -> UpdateDialog {
        let text = [
            format!(
                "NeoAmp {version} is available. You have {}.",
                Version::CURRENT
            ),
            QUESTION.to_owned(),
            HINT.to_owned(),
        ];
        let layout = Layout::new(scale, &text, m);
        UpdateDialog {
            version,
            text,
            layout,
            pressed: None,
            pointer: (-1, -1),
        }
    }

    /// The release it tells of.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// The window's size in pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        self.layout.size
    }

    /// Lay the dialog out again at another scale, measuring with `m`, the menu font at it.
    pub fn rescale(&mut self, scale: f64, m: &dyn Measure) {
        self.layout = Layout::new(scale, &self.text, m);
    }

    /// A key: Enter presses Download, the default button, and Escape closes.
    pub fn key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Enter => Outcome::Download,
            Key::Escape => Outcome::Close,
        }
    }

    fn part_at(&self, x: i32, y: i32) -> Option<Part> {
        let l = &self.layout;
        [
            (l.download, Part::Download),
            (l.skip, Part::Skip),
            (l.later, Part::Later),
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
    pub fn pointer_down(&mut self, x: i32, y: i32) -> Outcome {
        self.pointer = (x, y);
        let Some(part) = self.part_at(x, y) else {
            return Outcome::None;
        };
        self.pressed = Some(part);
        Outcome::Redraw
    }

    /// The pointer moved to (`x`, `y`): a held button pops out while the pointer is off it.
    pub fn pointer_moved(&mut self, x: i32, y: i32) -> Outcome {
        let was = self.pressed.is_some_and(|p| self.pushed(p));
        self.pointer = (x, y);
        match self.pressed {
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
        if self.part_at(x, y) != Some(part) {
            return Outcome::Redraw;
        }
        match part {
            Part::Download => Outcome::Download,
            Part::Skip => Outcome::Skip,
            Part::Later => Outcome::Close,
        }
    }

    /// Draw the dialog into `c`, which must be [`UpdateDialog::size`]. `font` must be the menu
    /// font at this scale.
    pub fn draw(&self, c: &mut Image, font: &TtfFont) {
        let l = &self.layout;
        c.clear(FACE);
        let all = c.bounds();
        for (text, &(x, y)) in self.text.iter().zip(&l.lines) {
            font.draw(c, text, x, y, TEXT, all);
        }
        // Download is the default button, which Enter presses.
        for (part, r, label, default) in [
            (Part::Download, l.download, DOWNLOAD, true),
            (Part::Skip, l.skip, SKIP, false),
            (Part::Later, l.later, LATER, false),
        ] {
            classic::push_button(c, font, r, l.t, label, default, self.pushed(part));
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

    fn dialog(scale: f64) -> UpdateDialog {
        UpdateDialog::new(Version::new(0, 3, 0), scale, &Fixed)
    }

    fn click(d: &mut UpdateDialog, r: Rect) -> Outcome {
        let (x, y) = centre(r);
        assert_eq!(d.pointer_down(x, y), Outcome::Redraw);
        d.pointer_up(x, y)
    }

    #[test]
    fn it_tells_of_the_release_and_this_version() {
        let d = dialog(1.0);
        assert_eq!(d.version(), Version::new(0, 3, 0));
        assert_eq!(
            d.text[0],
            format!(
                "NeoAmp 0.3.0 is available. You have {}.",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn the_buttons_download_skip_and_close() {
        let mut d = dialog(1.5);
        let l = d.layout;
        assert_eq!(click(&mut d, l.download), Outcome::Download);
        assert_eq!(click(&mut d, l.skip), Outcome::Skip);
        assert_eq!(click(&mut d, l.later), Outcome::Close);
        let (x, y) = centre(l.skip);
        let _ = d.pointer_down(x, y);
        assert!(d.pushed(Part::Skip));
        assert_eq!(d.pointer_moved(0, 0), Outcome::Redraw, "pops out off it");
        assert!(!d.pushed(Part::Skip));
        assert_eq!(d.pointer_up(0, 0), Outcome::Redraw, "let go elsewhere");
        assert_eq!(d.pointer_down(0, 0), Outcome::None, "not on a button");
        assert_eq!(d.pointer_up(0, 0), Outcome::None);
    }

    #[test]
    fn enter_downloads_and_escape_closes() {
        let mut d = dialog(1.0);
        assert_eq!(d.key(Key::Enter), Outcome::Download);
        assert_eq!(d.key(Key::Escape), Outcome::Close);
    }

    #[test]
    fn the_layout_fits_its_text_and_buttons() {
        for scale in [1.0, 1.5, 3.0] {
            let mut d = dialog(1.0);
            d.rescale(scale, &Fixed);
            let l = d.layout;
            let window = Rect::new(0, 0, l.size.0, l.size.1);
            for r in [l.download, l.skip, l.later] {
                assert_eq!(window.intersect(&r), r, "{r:?} at {scale}");
            }
            assert!(l.download.right() < l.skip.x && l.skip.right() < l.later.x);
            assert!(l.skip.w >= Fixed.text_width(SKIP), "the label fits");
            for (text, &(x, y)) in d.text.iter().zip(&l.lines) {
                assert!(x + Fixed.text_width(text) <= l.size.0, "{text} at {scale}");
                assert!(y < l.download.y);
            }
        }
    }

    #[test]
    fn it_draws_its_text_and_buttons() {
        let font = TtfFont::load_menu(crate::menu::font_px(1.0));
        let d = UpdateDialog::new(Version::new(0, 3, 0), 1.0, &font);
        let (w, h) = d.size();
        let mut c = Image::new(w, h);
        d.draw(&mut c, &font);
        let l = d.layout;
        assert_eq!(
            c.get(l.download.x, l.download.y),
            classic::FRAME,
            "the default button"
        );
        assert_eq!(c.get(l.skip.x, l.skip.y), classic::HILIGHT);
        let (x, y) = l.lines[0];
        let inked = (y..y + font.line_height())
            .any(|y| (x..x + font.width(&d.text[0])).any(|x| c.get(x, y) != FACE));
        assert!(inked, "the first line is drawn");
    }
}
