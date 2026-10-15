//! Sprite-sheet coordinates and window layout for classic Winamp 2.x skins.
//!
//! Every classic skin is a fixed set of bitmaps with fixed sprite positions; only the
//! pixels change. These tables are the de-facto skin specification.

use crate::canvas::Rect;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sheet {
    Main,
    TitleBar,
    CButtons,
    ShufRep,
    PosBar,
    Volume,
    Balance,
    MonoSter,
    PlayPaus,
    Numbers,
    NumsEx,
    Text,
    EqMain,
    /// The equalizer's windowshade bar and its pressed title-bar buttons.
    EqEx,
    PlEdit,
}

#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub sheet: Sheet,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Sprite {
    /// The sprite's rectangle inside its sheet.
    #[must_use]
    pub const fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }
}

const fn sp(sheet: Sheet, x: i32, y: i32, w: i32, h: i32) -> Sprite {
    Sprite { sheet, x, y, w, h }
}

/// Bitmap font (text.bmp): 31 columns x 3 rows of 5x6 glyphs.
pub mod font {
    use super::*;
    pub const GLYPH_W: i32 = 5;
    pub const GLYPH_H: i32 = 6;
    pub const fn glyph(col: i32, row: i32) -> Sprite {
        sp(Sheet::Text, col * GLYPH_W, row * GLYPH_H, GLYPH_W, GLYPH_H)
    }
}

/// LED digits (numbers.bmp / nums_ex.bmp): 9x13 each.
pub mod digits {
    use super::*;
    pub const fn digit(d: u8) -> Sprite {
        sp(Sheet::Numbers, d as i32 * 9, 0, 9, 13)
    }
    pub const BLANK: Sprite = sp(Sheet::Numbers, 90, 0, 9, 13);
    pub const fn digit_ex(d: u8) -> Sprite {
        sp(Sheet::NumsEx, d as i32 * 9, 0, 9, 13)
    }
    pub const BLANK_EX: Sprite = sp(Sheet::NumsEx, 90, 0, 9, 13);
    pub const MINUS_EX: Sprite = sp(Sheet::NumsEx, 99, 0, 9, 13);
}

/// Main window (275x116).
pub mod main {
    use super::*;
    pub const W: i32 = 275;
    pub const H: i32 = 116;

    pub const BACKGROUND: Sprite = sp(Sheet::Main, 0, 0, 275, 116);
    pub const TITLEBAR: Sprite = sp(Sheet::TitleBar, 27, 0, 275, 14);
    pub const TITLEBAR_INACTIVE: Sprite = sp(Sheet::TitleBar, 27, 15, 275, 14);
    pub const TITLE_RECT: Rect = Rect::new(0, 0, 275, 14);

    pub const OPTIONS: Rect = Rect::new(6, 3, 9, 9);
    pub const OPTIONS_PRESSED: Sprite = sp(Sheet::TitleBar, 0, 9, 9, 9);
    pub const MINIMIZE: Rect = Rect::new(244, 3, 9, 9);
    pub const MINIMIZE_PRESSED: Sprite = sp(Sheet::TitleBar, 9, 9, 9, 9);
    pub const SHADE: Rect = Rect::new(254, 3, 9, 9);
    pub const SHADE_PRESSED: Sprite = sp(Sheet::TitleBar, 9, 18, 9, 9);
    pub const CLOSE: Rect = Rect::new(264, 3, 9, 9);
    pub const CLOSE_PRESSED: Sprite = sp(Sheet::TitleBar, 18, 9, 9, 9);

    pub const CLUTTER: Rect = Rect::new(10, 22, 8, 43);
    pub const CLUTTER_BG: Sprite = sp(Sheet::TitleBar, 304, 0, 8, 43);
    pub const CLUTTER_O: Rect = Rect::new(10, 25, 8, 8);
    pub const CLUTTER_O_PRESSED: Sprite = sp(Sheet::TitleBar, 304, 47, 8, 8);
    pub const CLUTTER_A: Rect = Rect::new(10, 33, 8, 7);
    pub const CLUTTER_A_PRESSED: Sprite = sp(Sheet::TitleBar, 312, 55, 8, 7);
    pub const CLUTTER_I: Rect = Rect::new(10, 40, 8, 7);
    pub const CLUTTER_I_PRESSED: Sprite = sp(Sheet::TitleBar, 320, 62, 8, 7);
    pub const CLUTTER_D: Rect = Rect::new(10, 47, 8, 8);
    pub const CLUTTER_D_PRESSED: Sprite = sp(Sheet::TitleBar, 328, 69, 8, 8);
    pub const CLUTTER_V: Rect = Rect::new(10, 55, 8, 7);
    pub const CLUTTER_V_PRESSED: Sprite = sp(Sheet::TitleBar, 336, 77, 8, 7);

    pub const WORK_INDICATOR_POS: (i32, i32) = (24, 28);
    pub const NOT_WORKING: Sprite = sp(Sheet::PlayPaus, 36, 0, 3, 9);
    pub const WORKING: Sprite = sp(Sheet::PlayPaus, 39, 0, 3, 9);
    pub const STATUS_POS: (i32, i32) = (26, 28);
    pub const STATUS_PLAY: Sprite = sp(Sheet::PlayPaus, 0, 0, 9, 9);
    pub const STATUS_PAUSE: Sprite = sp(Sheet::PlayPaus, 9, 0, 9, 9);
    pub const STATUS_STOP: Sprite = sp(Sheet::PlayPaus, 18, 0, 9, 9);

    pub const TIME: Rect = Rect::new(39, 26, 59, 13);
    pub const MINUS_POS: (i32, i32) = (36, 26);
    pub const DIGIT_X: [i32; 4] = [48, 60, 78, 90];
    pub const DIGIT_Y: i32 = 26;

    pub const MARQUEE: Rect = Rect::new(111, 27, 154, 6);
    /// Where the song title's pointer (`SONGNAME.CUR`) shows: the text and 3 pixels above and
    /// below it, as in Webamp.
    pub const MARQUEE_TARGET: Rect = Rect::new(111, 24, 154, 12);
    pub const MARQUEE_CHARS: usize = 31;
    pub const KBPS_POS: (i32, i32) = (111, 43);
    pub const KHZ_POS: (i32, i32) = (156, 43);

    pub const MONO: Rect = Rect::new(212, 41, 27, 12);
    pub const STEREO: Rect = Rect::new(239, 41, 29, 12);
    pub const MONO_ON: Sprite = sp(Sheet::MonoSter, 29, 0, 27, 12);
    pub const MONO_OFF: Sprite = sp(Sheet::MonoSter, 29, 12, 27, 12);
    pub const STEREO_ON: Sprite = sp(Sheet::MonoSter, 0, 0, 29, 12);
    pub const STEREO_OFF: Sprite = sp(Sheet::MonoSter, 0, 12, 29, 12);

    pub const VIS: Rect = Rect::new(24, 43, 76, 16);

    pub const VOLUME: Rect = Rect::new(107, 57, 68, 13);
    pub const VOLUME_THUMB: Sprite = sp(Sheet::Volume, 15, 422, 14, 11);
    pub const VOLUME_THUMB_PRESSED: Sprite = sp(Sheet::Volume, 0, 422, 14, 11);
    pub const fn volume_frame(i: i32) -> Sprite {
        sp(Sheet::Volume, 0, i * 15, 68, 13)
    }
    pub const BALANCE: Rect = Rect::new(177, 57, 38, 13);
    pub const BALANCE_THUMB: Sprite = sp(Sheet::Balance, 15, 422, 14, 11);
    pub const BALANCE_THUMB_PRESSED: Sprite = sp(Sheet::Balance, 0, 422, 14, 11);
    pub const fn balance_frame(i: i32) -> Sprite {
        sp(Sheet::Balance, 9, i * 15, 38, 13)
    }
    pub const SLIDER_FRAMES: i32 = 28;
    pub const SLIDER_THUMB_W: i32 = 14;

    pub const POSBAR: Rect = Rect::new(16, 72, 248, 10);
    pub const POSBAR_BG: Sprite = sp(Sheet::PosBar, 0, 0, 248, 10);
    pub const POSBAR_THUMB: Sprite = sp(Sheet::PosBar, 248, 0, 29, 10);
    pub const POSBAR_THUMB_PRESSED: Sprite = sp(Sheet::PosBar, 278, 0, 29, 10);
    pub const POSBAR_THUMB_W: i32 = 29;

    pub const PREV: Rect = Rect::new(16, 88, 23, 18);
    pub const PLAY: Rect = Rect::new(39, 88, 23, 18);
    pub const PAUSE: Rect = Rect::new(62, 88, 23, 18);
    pub const STOP: Rect = Rect::new(85, 88, 23, 18);
    pub const NEXT: Rect = Rect::new(108, 88, 22, 18);
    pub const EJECT: Rect = Rect::new(136, 89, 22, 16);
    pub const fn prev_sprite(pressed: bool) -> Sprite {
        sp(Sheet::CButtons, 0, if pressed { 18 } else { 0 }, 23, 18)
    }
    pub const fn play_sprite(pressed: bool) -> Sprite {
        sp(Sheet::CButtons, 23, if pressed { 18 } else { 0 }, 23, 18)
    }
    pub const fn pause_sprite(pressed: bool) -> Sprite {
        sp(Sheet::CButtons, 46, if pressed { 18 } else { 0 }, 23, 18)
    }
    pub const fn stop_sprite(pressed: bool) -> Sprite {
        sp(Sheet::CButtons, 69, if pressed { 18 } else { 0 }, 23, 18)
    }
    pub const fn next_sprite(pressed: bool) -> Sprite {
        sp(Sheet::CButtons, 92, if pressed { 18 } else { 0 }, 22, 18)
    }
    pub const fn eject_sprite(pressed: bool) -> Sprite {
        sp(Sheet::CButtons, 114, if pressed { 16 } else { 0 }, 22, 16)
    }

    pub const SHUFFLE: Rect = Rect::new(164, 89, 47, 15);
    pub const REPEAT: Rect = Rect::new(210, 89, 28, 15);
    const fn toggle_row(on: bool, pressed: bool) -> i32 {
        match (on, pressed) {
            (false, false) => 0,
            (false, true) => 15,
            (true, false) => 30,
            (true, true) => 45,
        }
    }
    pub const fn shuffle_sprite(on: bool, pressed: bool) -> Sprite {
        sp(Sheet::ShufRep, 28, toggle_row(on, pressed), 47, 15)
    }
    pub const fn repeat_sprite(on: bool, pressed: bool) -> Sprite {
        sp(Sheet::ShufRep, 0, toggle_row(on, pressed), 28, 15)
    }
    pub const EQ_BTN: Rect = Rect::new(219, 58, 23, 12);
    pub const PL_BTN: Rect = Rect::new(242, 58, 23, 12);
    pub const fn eq_btn_sprite(on: bool, pressed: bool) -> Sprite {
        sp(
            Sheet::ShufRep,
            if pressed { 46 } else { 0 },
            if on { 73 } else { 61 },
            23,
            12,
        )
    }
    pub const fn pl_btn_sprite(on: bool, pressed: bool) -> Sprite {
        sp(
            Sheet::ShufRep,
            if pressed { 69 } else { 23 },
            if on { 73 } else { 61 },
            23,
            12,
        )
    }
}

/// Thirds of a windowshade slider's travel; each has its own thumb sprite.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Third {
    Left,
    Middle,
    Right,
}

impl Third {
    /// The third `fraction` (`0..=1`) falls in. The edges at 1/3 and 2/3 go to the outer
    /// thirds, as in Webamp.
    #[must_use]
    pub fn of(fraction: f32) -> Third {
        if fraction <= 1.0 / 3.0 {
            Third::Left
        } else if fraction >= 2.0 / 3.0 {
            Third::Right
        } else {
            Third::Middle
        }
    }

    /// Sprite columns are 3 pixels apart, left to right.
    const fn column(self) -> i32 {
        match self {
            Third::Left => 0,
            Third::Middle => 3,
            Third::Right => 6,
        }
    }
}

/// The main window in windowshade mode: a 275x14 bar from titlebar.bmp. The options,
/// minimize and close buttons are where the full window has them (see [`main`]).
pub mod main_shade {
    use super::*;
    pub const H: i32 = 14;

    pub const BACKGROUND: Sprite = sp(Sheet::TitleBar, 27, 29, 275, 14);
    pub const BACKGROUND_INACTIVE: Sprite = sp(Sheet::TitleBar, 27, 42, 275, 14);
    /// The shade button, which opens the window out again here, pressed.
    pub const UNSHADE_PRESSED: Sprite = sp(Sheet::TitleBar, 9, 27, 9, 9);

    /// The visualizer: the analyzer or oscilloscope at 38x5.
    pub const VIS: Rect = Rect::new(79, 5, 38, 5);
    /// The time, in the text font: a minus sign, two digits for the minutes and two for the
    /// seconds. The colon between them is part of the bar.
    pub const TIME: Rect = Rect::new(127, 4, 30, 6);
    pub const MINUS_X: i32 = 128;
    pub const DIGIT_X: [i32; 4] = [134, 139, 147, 152];
    pub const TEXT_Y: i32 = 4;

    /// The transport buttons are drawn into the bar; they have no pressed sprites.
    pub const PREV: Rect = Rect::new(169, 2, 7, 10);
    pub const PLAY: Rect = Rect::new(176, 2, 10, 10);
    pub const PAUSE: Rect = Rect::new(186, 2, 9, 10);
    pub const STOP: Rect = Rect::new(195, 2, 9, 10);
    pub const NEXT: Rect = Rect::new(204, 2, 10, 10);
    pub const EJECT: Rect = Rect::new(215, 2, 10, 10);

    pub const POSBAR: Rect = Rect::new(226, 4, 17, 7);
    pub const POSBAR_BG: Sprite = sp(Sheet::TitleBar, 0, 36, 17, 7);
    pub const POSBAR_THUMB_W: i32 = 3;
    /// The position bar's thumb, which looks different in each third of the bar.
    pub const fn posbar_thumb(third: Third) -> Sprite {
        sp(Sheet::TitleBar, 17 + third.column(), 36, 3, 7)
    }
}

/// Equalizer window (275x116).
pub mod eq {
    use super::*;
    pub const W: i32 = 275;
    pub const H: i32 = 116;

    pub const BACKGROUND: Sprite = sp(Sheet::EqMain, 0, 0, 275, 116);
    pub const TITLEBAR: Sprite = sp(Sheet::EqMain, 0, 134, 275, 14);
    pub const TITLEBAR_INACTIVE: Sprite = sp(Sheet::EqMain, 0, 149, 275, 14);
    pub const TITLE_RECT: Rect = Rect::new(0, 0, 275, 14);
    pub const CLOSE: Rect = Rect::new(264, 3, 9, 9);
    /// eqmain.bmp has the close button at (0, 116) as the title bar draws it, then pressed.
    pub const CLOSE_PRESSED: Sprite = sp(Sheet::EqMain, 0, 125, 9, 9);
    pub const SHADE: Rect = Rect::new(254, 3, 9, 9);
    /// Only in eq_ex.bmp, so it is drawn only for skins that have their own (as in Webamp):
    /// the base skin's would not match theirs.
    pub const SHADE_PRESSED: Sprite = sp(Sheet::EqEx, 1, 38, 9, 9);

    pub const ON: Rect = Rect::new(14, 18, 26, 12);
    pub const fn on_sprite(on: bool, pressed: bool) -> Sprite {
        let x = match (on, pressed) {
            (false, false) => 10,
            (false, true) => 128,
            (true, false) => 69,
            (true, true) => 187,
        };
        sp(Sheet::EqMain, x, 119, 26, 12)
    }
    pub const AUTO: Rect = Rect::new(40, 18, 32, 12);
    pub const fn auto_sprite(on: bool, pressed: bool) -> Sprite {
        let x = match (on, pressed) {
            (false, false) => 36,
            (false, true) => 154,
            (true, false) => 95,
            (true, true) => 213,
        };
        sp(Sheet::EqMain, x, 119, 32, 12)
    }
    pub const PRESETS: Rect = Rect::new(217, 18, 44, 12);
    pub const PRESETS_UP: Sprite = sp(Sheet::EqMain, 224, 164, 44, 12);
    pub const PRESETS_DOWN: Sprite = sp(Sheet::EqMain, 224, 176, 44, 12);

    pub const GRAPH: Rect = Rect::new(86, 17, 113, 19);
    pub const GRAPH_BG: Sprite = sp(Sheet::EqMain, 0, 294, 113, 19);
    pub const GRAPH_GRADIENT: Sprite = sp(Sheet::EqMain, 115, 294, 1, 19);
    pub const PREAMP_LINE: Sprite = sp(Sheet::EqMain, 0, 314, 113, 1);

    pub const PREAMP: Rect = Rect::new(21, 38, 14, 63);
    pub const fn band(i: i32) -> Rect {
        Rect::new(78 + 18 * i, 38, 14, 63)
    }
    pub const fn slider_frame(i: i32) -> Sprite {
        sp(
            Sheet::EqMain,
            13 + (i % 14) * 15,
            164 + (i / 14) * 65,
            14,
            63,
        )
    }
    pub const SLIDER_FRAMES: i32 = 28;
    pub const SLIDER_THUMB: Sprite = sp(Sheet::EqMain, 0, 164, 11, 11);
    pub const SLIDER_THUMB_PRESSED: Sprite = sp(Sheet::EqMain, 0, 176, 11, 11);
    pub const SLIDER_THUMB_H: i32 = 11;
    /// Thumb travel in pixels (the 63px track minus a 1px bottom margin and the thumb).
    pub const SLIDER_TRAVEL: i32 = 51;
    pub const BAND_LABELS: [&str; 10] = [
        "60", "170", "310", "600", "1K", "3K", "6K", "12K", "14K", "16K",
    ];
}

/// The equalizer in windowshade mode: a 275x14 bar from eq_ex.bmp, with the volume and balance
/// sliders. The shade and close buttons are where the full window has them (see [`eq`]).
pub mod eq_shade {
    use super::*;
    pub const H: i32 = 14;

    pub const BACKGROUND: Sprite = sp(Sheet::EqEx, 0, 0, 275, 14);
    pub const BACKGROUND_INACTIVE: Sprite = sp(Sheet::EqEx, 0, 15, 275, 14);
    pub const UNSHADE_PRESSED: Sprite = sp(Sheet::EqEx, 1, 47, 9, 9);
    pub const CLOSE_PRESSED: Sprite = sp(Sheet::EqEx, 11, 47, 9, 9);

    pub const VOLUME: Rect = Rect::new(61, 4, 97, 6);
    pub const BALANCE: Rect = Rect::new(164, 4, 43, 6);
    pub const THUMB_W: i32 = 3;
    /// The thumbs look different in each third of their slider.
    pub const fn volume_thumb(third: Third) -> Sprite {
        sp(Sheet::EqEx, 1 + third.column(), 30, 3, 7)
    }
    pub const fn balance_thumb(third: Third) -> Sprite {
        sp(Sheet::EqEx, 11 + third.column(), 30, 3, 7)
    }
}

/// Playlist editor (resizable; 275x116 minimum, grows in 25x29 steps).
pub mod pl {
    use super::*;
    pub const MIN_W: i32 = 275;
    pub const MIN_H: i32 = 116;
    pub const STEP_W: i32 = 25;
    pub const STEP_H: i32 = 29;
    pub const TOP_H: i32 = 20;
    pub const BOTTOM_H: i32 = 38;
    pub const LEFT_W: i32 = 12;
    pub const RIGHT_W: i32 = 20;
    pub const TRACK_H: i32 = 13;
    /// The track list is inset this much from the top and bottom of the list area.
    pub const LIST_PAD: i32 = 3;

    const fn row(active: bool) -> i32 {
        if active { 0 } else { 21 }
    }
    pub const fn top_left(active: bool) -> Sprite {
        sp(Sheet::PlEdit, 0, row(active), 25, 20)
    }
    pub const fn title(active: bool) -> Sprite {
        sp(Sheet::PlEdit, 26, row(active), 100, 20)
    }
    pub const fn top_fill(active: bool) -> Sprite {
        sp(Sheet::PlEdit, 127, row(active), 25, 20)
    }
    pub const fn top_right(active: bool) -> Sprite {
        sp(Sheet::PlEdit, 153, row(active), 25, 20)
    }
    pub const LEFT: Sprite = sp(Sheet::PlEdit, 0, 42, 12, 29);
    pub const RIGHT: Sprite = sp(Sheet::PlEdit, 31, 42, 20, 29);
    pub const BOTTOM_LEFT: Sprite = sp(Sheet::PlEdit, 0, 72, 125, 38);
    pub const BOTTOM_RIGHT: Sprite = sp(Sheet::PlEdit, 126, 72, 150, 38);
    pub const BOTTOM_FILL: Sprite = sp(Sheet::PlEdit, 179, 0, 25, 38);

    pub const SCROLL_THUMB: Sprite = sp(Sheet::PlEdit, 52, 53, 8, 18);
    pub const SCROLL_THUMB_PRESSED: Sprite = sp(Sheet::PlEdit, 61, 53, 8, 18);
    /// Thumb x measured from the window's right edge.
    pub const SCROLL_FROM_RIGHT: i32 = 15;
    pub const SCROLL_THUMB_H: i32 = 18;

    pub const CLOSE_PRESSED: Sprite = sp(Sheet::PlEdit, 52, 42, 9, 9);
    pub const SHADE_PRESSED: Sprite = sp(Sheet::PlEdit, 62, 42, 9, 9);
    pub const CLOSE_FROM_RIGHT: i32 = 11;
    pub const SHADE_FROM_RIGHT: i32 = 21;
    pub const TITLE_BTN_Y: i32 = 3;

    /// Bottom-left menu buttons, relative to the bottom-left piece.
    pub const ADD: Rect = Rect::new(14, 8, 22, 18);
    pub const REM: Rect = Rect::new(43, 8, 22, 18);
    pub const SEL: Rect = Rect::new(72, 8, 22, 18);
    pub const MISC: Rect = Rect::new(101, 8, 22, 18);
    /// Relative to the bottom-right piece.
    pub const LIST_OPTS: Rect = Rect::new(106, 8, 22, 18);
    pub const RESIZE: Rect = Rect::new(130, 18, 20, 20);
    pub const SCROLL_UP: Rect = Rect::new(135, 2, 8, 5);
    pub const SCROLL_DOWN: Rect = Rect::new(135, 8, 8, 5);
    pub const MINI_PREV: Rect = Rect::new(3, 22, 10, 10);
    pub const MINI_PLAY: Rect = Rect::new(13, 22, 10, 10);
    pub const MINI_PAUSE: Rect = Rect::new(23, 22, 10, 10);
    pub const MINI_STOP: Rect = Rect::new(33, 22, 10, 10);
    pub const MINI_NEXT: Rect = Rect::new(43, 22, 10, 10);
    pub const MINI_EJECT: Rect = Rect::new(53, 22, 10, 10);
    /// Running-time readout ("0:05/0:05"), relative to the bottom-right piece.
    pub const RUNNING_TIME_POS: (i32, i32) = (7, 10);
    /// Mini time display: minutes end here (exclusive), seconds start here. Colon is baked in.
    pub const MINI_TIME_Y: i32 = 23;
    pub const MINI_TIME_MIN_END: i32 = 83;
    pub const MINI_TIME_SEC_X: i32 = 86;

    /// Pop-up menus (22x18 items, 3px bar on the left).
    pub const MENU_ITEM_W: i32 = 22;
    pub const MENU_ITEM_H: i32 = 18;
    pub const fn menu_item(x: i32, idx: i32, selected: bool) -> Sprite {
        sp(
            Sheet::PlEdit,
            x + if selected { 23 } else { 0 },
            111 + idx * 19,
            22,
            18,
        )
    }
    pub const fn menu_bar(x: i32, items: i32) -> Sprite {
        sp(Sheet::PlEdit, x, 111, 3, items * 18)
    }
    pub const ADD_MENU_X: i32 = 0;
    pub const ADD_BAR_X: i32 = 48;
    pub const REM_MENU_X: i32 = 54;
    pub const REM_BAR_X: i32 = 100;
    pub const SEL_MENU_X: i32 = 104;
    pub const SEL_BAR_X: i32 = 150;
    pub const MISC_MENU_X: i32 = 154;
    pub const MISC_BAR_X: i32 = 200;
    pub const LIST_MENU_X: i32 = 204;
    pub const LIST_BAR_X: i32 = 250;
}

/// The playlist in windowshade mode: a bar 14 high and as wide as the playlist, from
/// pledit.bmp, showing the current track. Its close and shade buttons are where the full
/// window has them (see [`pl`]); only its width can be changed.
pub mod pl_shade {
    use super::*;
    pub const H: i32 = 14;

    /// Tiled across the whole bar, under the two ends.
    pub const FILL: Sprite = sp(Sheet::PlEdit, 72, 57, 25, 14);
    pub const LEFT: Sprite = sp(Sheet::PlEdit, 72, 42, 25, 14);
    pub const RIGHT_W: i32 = 50;
    pub const fn right(active: bool) -> Sprite {
        sp(Sheet::PlEdit, 99, if active { 42 } else { 57 }, 50, 14)
    }
    pub const UNSHADE_PRESSED: Sprite = sp(Sheet::PlEdit, 150, 42, 9, 9);
    /// The width-only resize grip, measured from the right edge.
    pub const RESIZE_FROM_RIGHT: i32 = 29;

    /// "N. Title" in the text font, cut short to fit the bar.
    pub const TITLE_POS: (i32, i32) = (5, 4);
    /// The bar shows `(width - TITLE_SPARE) / 5` characters of the title: 41 at the
    /// narrowest.
    pub const TITLE_SPARE: i32 = 70;
    /// The track's length ends this far from the right edge.
    pub const TIME_FROM_RIGHT: i32 = 30;
}
