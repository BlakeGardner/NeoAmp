//! UI state, hit testing and input handling, expressed per Winamp window so each window can
//! live in its own OS window.

use std::fmt::Write as _;
use std::time::Instant;

use crate::canvas::{Image, Rect};
use crate::font::TtfFont;
use crate::skin::{Mask, Skin};
use crate::sprites::{eq as e, main as m, pl as p};
use crate::vis::Vis;
use crate::{eq_window, main_window, playlist_window};

/// The marquee advances one character this often, like Winamp 2.x.
pub const MARQUEE_STEP_MS: u128 = 220;
/// Inserted between repeats of a scrolling title.
const MARQUEE_SEPARATOR: &str = "  ***  ";
/// Two clicks on the same control closer than this count as a double-click.
const DOUBLE_CLICK_MS: u128 = 400;
/// Time display blink period while paused (on for half, off for half).
const BLINK_MS: u128 = 1000;

/// The three classic windows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowKind {
    Main,
    Eq,
    Playlist,
}

impl WindowKind {
    pub const ALL: [WindowKind; 3] = [WindowKind::Main, WindowKind::Eq, WindowKind::Playlist];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

/// Every clickable thing in the three windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    MainTitle,
    MainOptions,
    MainMinimize,
    MainShade,
    MainClose,
    ClutterO,
    ClutterA,
    ClutterI,
    ClutterD,
    ClutterV,
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
    Shuffle,
    Repeat,
    EqToggle,
    PlToggle,
    Volume,
    Balance,
    PosBar,
    TimeDisplay,
    Vis,
    EqTitle,
    EqClose,
    EqShade,
    EqOn,
    EqAuto,
    EqPresets,
    EqPreamp,
    EqBand(usize),
    PlTitle,
    PlClose,
    PlShade,
    PlAdd,
    PlRem,
    PlSel,
    PlMisc,
    PlListOpts,
    PlResize,
    PlScroll,
    PlScrollUp,
    PlScrollDown,
    PlTrack(usize),
    PlMiniPrev,
    PlMiniPlay,
    PlMiniPause,
    PlMiniStop,
    PlMiniNext,
    PlMiniEject,
}

/// The draggable value controls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slider {
    Volume,
    Balance,
    PosBar,
    EqPreamp,
    EqBand(usize),
}

impl Control {
    /// The slider behind this control, if it is one.
    #[must_use]
    pub fn slider(self) -> Option<Slider> {
        match self {
            Control::Volume => Some(Slider::Volume),
            Control::Balance => Some(Slider::Balance),
            Control::PosBar => Some(Slider::PosBar),
            Control::EqPreamp => Some(Slider::EqPreamp),
            Control::EqBand(i) => Some(Slider::EqBand(i)),
            // Everything else is a button, a title bar or a list row.
            _ => None,
        }
    }

    /// The window this control lives in.
    #[must_use]
    pub fn window(self) -> WindowKind {
        use Control::*;
        match self {
            MainTitle
            | MainOptions
            | MainMinimize
            | MainShade
            | MainClose
            | ClutterO
            | ClutterA
            | ClutterI
            | ClutterD
            | ClutterV
            | Prev
            | Play
            | Pause
            | Stop
            | Next
            | Eject
            | Shuffle
            | Repeat
            | EqToggle
            | PlToggle
            | Volume
            | Balance
            | PosBar
            | TimeDisplay
            | Control::Vis => WindowKind::Main,
            EqTitle | EqClose | EqShade | EqOn | EqAuto | EqPresets | EqPreamp | EqBand(_) => {
                WindowKind::Eq
            }
            PlTitle | PlClose | PlShade | PlAdd | PlRem | PlSel | PlMisc | PlListOpts
            | PlResize | PlScroll | PlScrollUp | PlScrollDown | PlTrack(_) | PlMiniPrev
            | PlMiniPlay | PlMiniPause | PlMiniStop | PlMiniNext | PlMiniEject => {
                WindowKind::Playlist
            }
        }
    }
}

/// Side effects the window layer must perform after input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// The user grabbed this window's title bar: start moving it.
    DragWindow(WindowKind),
    Close,
    Minimize,
    /// Window visibility, size or scale changed: re-sync the OS windows.
    Relayout,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyCmd {
    ToggleDoubleSize,
    ToggleEq,
    TogglePl,
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    ToggleShuffle,
    ToggleRepeat,
    ToggleTimeMode,
    VolumeUp,
    VolumeDown,
    SeekBack,
    SeekForward,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Track {
    pub title: String,
    pub seconds: u32,
}

/// Where each window sits when the three are composed into one canvas (screenshots).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub main: Rect,
    pub eq: Option<Rect>,
    pub pl: Option<Rect>,
    pub size: (i32, i32),
}

#[derive(Debug)]
pub struct Ui {
    pub state: PlayState,
    elapsed_base: f32,
    resumed_at: Option<Instant>,
    pub volume: f32,
    pub balance: f32,
    pub shuffle: bool,
    pub repeat: bool,
    pub eq_open: bool,
    pub pl_open: bool,
    pub eq_on: bool,
    pub eq_auto: bool,
    pub preamp: f32,
    pub bands: [f32; 10],
    pub remaining_mode: bool,
    pub pressed: Option<Control>,
    drag_start: (i32, i32),
    drag_size: (i32, i32),
    pub pl_size: (i32, i32),
    pub tracks: Vec<Track>,
    pub current: usize,
    pub selected: Vec<bool>,
    pub pl_scroll: usize,
    pub kbps: u32,
    pub khz: u32,
    pub stereo: bool,
    pub vis: Vis,
    pub double_size: bool,
    /// Any of the app's windows has focus: title bars draw in their active state.
    pub active: bool,
    /// Last pointer position inside the playlist window (menu highlight).
    pub cursor: (i32, i32),
    /// Temporary marquee text shown while a slider is dragged.
    pub marquee_override: Option<String>,
    pub start: Instant,
    last_click: Option<(Control, Instant)>,
    /// The current track's marquee text, rebuilt only when the track changes.
    title_chars: Vec<char>,
    title_scrolls: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

/// Append formatted text to `buf`. Writing to a `String` cannot fail.
pub fn push_fmt(buf: &mut String, args: std::fmt::Arguments<'_>) {
    buf.write_fmt(args)
        .expect("BUG: formatting into a String never fails");
}

/// Append `m:ss` to `buf`.
pub fn write_time(buf: &mut String, secs: u32) {
    push_fmt(buf, format_args!("{}:{:02}", secs / 60, secs % 60));
}

impl Ui {
    #[must_use]
    pub fn new() -> Ui {
        let tracks = vec![
            Track {
                title: "DJ Mike Llama - Llama Whippin' Intro".into(),
                seconds: 5,
            },
            Track {
                title: "Nullsoft - Winamp 2.91 Theme".into(),
                seconds: 213,
            },
            Track {
                title: "Justin F. - Rolling Llama (Radio Edit)".into(),
                seconds: 254,
            },
            Track {
                title: "The Shoutcasters - Buffering...".into(),
                seconds: 187,
            },
            Track {
                title: "MilkDrop Crew - Plasma Visions".into(),
                seconds: 312,
            },
            Track {
                title: "Tom P. - Llama Country".into(),
                seconds: 198,
            },
            Track {
                title: "Unknown Artist - Track 07".into(),
                seconds: 241,
            },
        ];
        let mut selected = vec![false; tracks.len()];
        if let Some(first) = selected.first_mut() {
            *first = true;
        }
        let now = Instant::now();
        let mut ui = Ui {
            state: PlayState::Playing,
            elapsed_base: 0.0,
            resumed_at: Some(now),
            volume: 0.78,
            balance: 0.0,
            shuffle: false,
            repeat: false,
            eq_open: true,
            pl_open: true,
            eq_on: true,
            eq_auto: false,
            preamp: 0.5,
            bands: [0.5; 10],
            remaining_mode: false,
            pressed: None,
            drag_start: (0, 0),
            drag_size: (0, 0),
            pl_size: (p::MIN_W, p::MIN_H),
            tracks,
            current: 0,
            selected,
            pl_scroll: 0,
            kbps: 56,
            khz: 22,
            stereo: true,
            vis: Vis::new(),
            double_size: false,
            active: true,
            cursor: (-1, -1),
            marquee_override: None,
            start: now,
            last_click: None,
            title_chars: Vec::new(),
            title_scrolls: false,
        };
        ui.refresh_title();
        ui
    }

    // ----- windows ------------------------------------------------------------------------

    /// Size of a window's canvas in skin pixels.
    #[must_use]
    pub fn window_size(&self, kind: WindowKind) -> (i32, i32) {
        match kind {
            WindowKind::Main => (m::W, m::H),
            WindowKind::Eq => (e::W, e::H),
            WindowKind::Playlist => self.pl_size,
        }
    }

    #[must_use]
    pub fn is_open(&self, kind: WindowKind) -> bool {
        match kind {
            WindowKind::Main => true,
            WindowKind::Eq => self.eq_open,
            WindowKind::Playlist => self.pl_open,
        }
    }

    /// Open or close the equalizer or playlist (the main window is always open).
    pub fn set_open(&mut self, kind: WindowKind, open: bool) {
        match kind {
            WindowKind::Main => {}
            WindowKind::Eq => self.eq_open = open,
            WindowKind::Playlist => self.pl_open = open,
        }
    }

    /// The classic stacked arrangement, used when composing all windows into one canvas.
    #[must_use]
    pub fn layout(&self) -> Layout {
        let main = Rect::new(0, 0, m::W, m::H);
        let mut y = m::H;
        let eq = self.eq_open.then(|| {
            let r = Rect::new(0, y, e::W, e::H);
            y += e::H;
            r
        });
        let pl = self.pl_open.then(|| {
            let r = Rect::new(0, y, self.pl_size.0, self.pl_size.1);
            y += self.pl_size.1;
            r
        });
        let w = if self.pl_open {
            m::W.max(self.pl_size.0)
        } else {
            m::W
        };
        Layout {
            main,
            eq,
            pl,
            size: (w, y),
        }
    }

    /// Window scale factor: 1, or 2 in double-size mode.
    #[must_use]
    pub fn scale(&self) -> u8 {
        if self.double_size { 2 } else { 1 }
    }

    // ----- playback model -----------------------------------------------------------------

    /// The current track. The playlist is never empty, but fall back gracefully if it were.
    #[must_use]
    pub fn track(&self) -> Option<&Track> {
        self.tracks.get(self.current)
    }

    #[must_use]
    pub fn track_len(&self) -> f32 {
        self.track().map_or(0.0, |t| t.seconds as f32)
    }

    #[must_use]
    pub fn elapsed(&self, now: Instant) -> f32 {
        let mut t = self.elapsed_base;
        if let Some(r) = self.resumed_at {
            t += now.saturating_duration_since(r).as_secs_f32();
        }
        t.clamp(0.0, self.track_len())
    }

    fn set_elapsed(&mut self, secs: f32, now: Instant) {
        self.elapsed_base = secs.clamp(0.0, self.track_len());
        if self.state == PlayState::Playing {
            self.resumed_at = Some(now);
        }
    }

    /// Playback position in `0..=1`.
    #[must_use]
    pub fn position(&self, now: Instant) -> f32 {
        let len = self.track_len();
        if len <= 0.0 {
            0.0
        } else {
            self.elapsed(now) / len
        }
    }

    /// Minutes, seconds and whether a minus sign is shown (remaining-time mode).
    #[must_use]
    pub fn display_time(&self, now: Instant) -> (u32, u32, bool) {
        let e = self.elapsed(now);
        let secs = if self.remaining_mode {
            (self.track_len() - e).ceil()
        } else {
            e.floor()
        };
        // Non-negative and far below u32::MAX, so the cast cannot truncate.
        let secs = secs.max(0.0) as u32;
        (secs / 60, secs % 60, self.remaining_mode)
    }

    /// True during the "off" half of the paused blink.
    #[must_use]
    pub fn blink_off(&self, now: Instant) -> bool {
        self.state == PlayState::Paused
            && (now.saturating_duration_since(self.start).as_millis() % BLINK_MS) >= BLINK_MS / 2
    }

    pub fn play(&mut self, now: Instant) {
        match self.state {
            PlayState::Paused => self.resumed_at = Some(now),
            PlayState::Playing | PlayState::Stopped => {
                self.elapsed_base = 0.0;
                self.resumed_at = Some(now);
            }
        }
        self.state = PlayState::Playing;
    }

    pub fn pause(&mut self, now: Instant) {
        match self.state {
            PlayState::Playing => {
                self.elapsed_base = self.elapsed(now);
                self.resumed_at = None;
                self.state = PlayState::Paused;
            }
            PlayState::Paused => {
                self.resumed_at = Some(now);
                self.state = PlayState::Playing;
            }
            PlayState::Stopped => {}
        }
    }

    pub fn stop(&mut self) {
        self.state = PlayState::Stopped;
        self.elapsed_base = 0.0;
        self.resumed_at = None;
        self.vis.reset();
    }

    pub fn jump(&mut self, idx: usize, now: Instant) {
        if self.tracks.is_empty() {
            self.current = 0;
            self.refresh_title();
            return;
        }
        self.current = idx % self.tracks.len();
        self.elapsed_base = 0.0;
        if self.state == PlayState::Playing {
            self.resumed_at = Some(now);
        }
        self.ensure_visible(self.current);
        self.refresh_title();
    }

    pub fn next(&mut self, now: Instant) {
        self.jump(self.current.wrapping_add(1), now);
    }

    pub fn prev(&mut self, now: Instant) {
        let len = self.tracks.len();
        if len == 0 {
            return;
        }
        self.jump((self.current + len - 1) % len, now);
    }

    /// Advance the fake playback and the visualiser. Call once per frame before drawing.
    pub fn tick(&mut self, now: Instant) {
        if self.state == PlayState::Playing && self.elapsed(now) >= self.track_len() {
            if self.repeat || self.current + 1 < self.tracks.len() {
                self.next(now);
            } else {
                self.stop();
            }
        }
        self.vis.update(now, self.state == PlayState::Playing);
    }

    // ----- text ---------------------------------------------------------------------------

    /// Rebuild the cached marquee text (`"N. Title (m:ss)"`, padded when it scrolls).
    fn refresh_title(&mut self) {
        self.title_chars.clear();
        let Some(t) = self.track() else { return };
        let mut s = String::with_capacity(64);
        push_fmt(&mut s, format_args!("{}. {} (", self.current + 1, t.title));
        write_time(&mut s, t.seconds);
        s.push(')');
        self.title_scrolls = s.chars().count() > m::MARQUEE_CHARS;
        if self.title_scrolls {
            s.push_str(MARQUEE_SEPARATOR);
        }
        self.title_chars.extend(s.chars());
    }

    /// The characters the marquee shows for the current track at `now`, without allocating.
    pub fn title_scroll_chars(&self, now: Instant) -> impl Iterator<Item = char> + '_ {
        let len = self.title_chars.len().max(1);
        let step = if self.title_scrolls {
            // Reduce modulo the length first so the narrowing cast cannot truncate.
            let steps = now.saturating_duration_since(self.start).as_millis() / MARQUEE_STEP_MS;
            usize::try_from(steps % len as u128).unwrap_or(0)
        } else {
            0
        };
        let shown = if self.title_scrolls {
            m::MARQUEE_CHARS
        } else {
            self.title_chars.len()
        };
        self.title_chars
            .iter()
            .copied()
            .cycle()
            .skip(step)
            .take(shown)
    }

    /// The marquee text as a `String` (allocates; the renderer uses the iterator form).
    #[must_use]
    pub fn marquee_text(&self, now: Instant) -> String {
        match &self.marquee_override {
            Some(s) => s.clone(),
            None => self.title_scroll_chars(now).collect(),
        }
    }

    #[must_use]
    pub fn time_str(secs: u32) -> String {
        let mut s = String::with_capacity(8);
        write_time(&mut s, secs);
        s
    }

    // ----- drawing ------------------------------------------------------------------------

    /// Draw one window into `c`, which must be sized by [`Ui::window_size`].
    pub fn draw_window(
        &self,
        kind: WindowKind,
        c: &mut Image,
        skin: &Skin,
        font: Option<&TtfFont>,
        now: Instant,
    ) {
        let (w, h) = self.window_size(kind);
        let r = Rect::new(0, 0, w, h);
        c.clear([0, 0, 0, 0]);
        match kind {
            WindowKind::Main => {
                main_window::draw(self, c, skin, r, now);
                if let Some(mask) = &skin.main_mask {
                    apply_mask(c, r, mask);
                }
            }
            WindowKind::Eq => {
                eq_window::draw(self, c, skin, r, now);
                if let Some(mask) = &skin.eq_mask {
                    apply_mask(c, r, mask);
                }
            }
            WindowKind::Playlist => playlist_window::draw(self, c, skin, font, r, now),
        }
    }

    /// Advance one frame and draw every open window stacked into a single canvas.
    pub fn draw_stacked(
        &mut self,
        c: &mut Image,
        skin: &Skin,
        font: Option<&TtfFont>,
        now: Instant,
    ) {
        self.tick(now);
        let lay = self.layout();
        c.clear([0, 0, 0, 0]);
        main_window::draw(self, c, skin, lay.main, now);
        if let Some(mask) = &skin.main_mask {
            apply_mask(c, lay.main, mask);
        }
        if let Some(r) = lay.eq {
            eq_window::draw(self, c, skin, r, now);
            if let Some(mask) = &skin.eq_mask {
                apply_mask(c, r, mask);
            }
        }
        if let Some(r) = lay.pl {
            playlist_window::draw(self, c, skin, font, r, now);
        }
    }

    // ----- hit testing --------------------------------------------------------------------

    /// The control under window-local point (`x`, `y`) of `kind`.
    #[must_use]
    pub fn hit(&self, kind: WindowKind, x: i32, y: i32) -> Option<Control> {
        let (w, h) = self.window_size(kind);
        if !Rect::new(0, 0, w, h).contains(x, y) {
            return None;
        }
        match kind {
            WindowKind::Main => Self::hit_main(x, y),
            WindowKind::Eq => Self::hit_eq(x, y),
            WindowKind::Playlist => self.hit_pl(x, y, w, h),
        }
    }

    fn hit_main(x: i32, y: i32) -> Option<Control> {
        use Control::*;
        const TABLE: &[(Rect, Control)] = &[
            (m::OPTIONS, MainOptions),
            (m::MINIMIZE, MainMinimize),
            (m::SHADE, MainShade),
            (m::CLOSE, MainClose),
            (m::TITLE_RECT, MainTitle),
            (m::CLUTTER_O, ClutterO),
            (m::CLUTTER_A, ClutterA),
            (m::CLUTTER_I, ClutterI),
            (m::CLUTTER_D, ClutterD),
            (m::CLUTTER_V, ClutterV),
            (m::PREV, Prev),
            (m::PLAY, Play),
            (m::PAUSE, Pause),
            (m::STOP, Stop),
            (m::NEXT, Next),
            (m::EJECT, Eject),
            (m::SHUFFLE, Shuffle),
            (m::REPEAT, Repeat),
            (m::EQ_BTN, EqToggle),
            (m::PL_BTN, PlToggle),
            (m::VOLUME, Volume),
            (m::BALANCE, Balance),
            (m::POSBAR, PosBar),
            (m::TIME, TimeDisplay),
            (m::VIS, Control::Vis),
        ];
        TABLE
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, c)| *c)
    }

    fn hit_eq(x: i32, y: i32) -> Option<Control> {
        use Control::*;
        if let Some(i) = (0..10).find(|&i| e::band(i).contains(x, y)) {
            return usize::try_from(i).ok().map(EqBand);
        }
        const TABLE: &[(Rect, Control)] = &[
            (e::CLOSE, EqClose),
            (e::SHADE, EqShade),
            (e::TITLE_RECT, EqTitle),
            (e::ON, EqOn),
            (e::AUTO, EqAuto),
            (e::PRESETS, EqPresets),
            (e::PREAMP, EqPreamp),
        ];
        TABLE
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, c)| *c)
    }

    fn hit_pl(&self, x: i32, y: i32, w: i32, h: i32) -> Option<Control> {
        use Control::*;
        if Rect::new(w - p::CLOSE_FROM_RIGHT, p::TITLE_BTN_Y, 9, 9).contains(x, y) {
            return Some(PlClose);
        }
        if Rect::new(w - p::SHADE_FROM_RIGHT, p::TITLE_BTN_Y, 9, 9).contains(x, y) {
            return Some(PlShade);
        }
        if y < p::TOP_H {
            return Some(PlTitle);
        }
        let bl = (0, h - p::BOTTOM_H);
        let br = (w - 150, h - p::BOTTOM_H);
        let table = [
            (p::ADD.offset(bl.0, bl.1), PlAdd),
            (p::REM.offset(bl.0, bl.1), PlRem),
            (p::SEL.offset(bl.0, bl.1), PlSel),
            (p::MISC.offset(bl.0, bl.1), PlMisc),
            (p::LIST_OPTS.offset(br.0, br.1), PlListOpts),
            (p::RESIZE.offset(br.0, br.1), PlResize),
            (p::SCROLL_UP.offset(br.0, br.1), PlScrollUp),
            (p::SCROLL_DOWN.offset(br.0, br.1), PlScrollDown),
            (p::MINI_PREV.offset(br.0, br.1), PlMiniPrev),
            (p::MINI_PLAY.offset(br.0, br.1), PlMiniPlay),
            (p::MINI_PAUSE.offset(br.0, br.1), PlMiniPause),
            (p::MINI_STOP.offset(br.0, br.1), PlMiniStop),
            (p::MINI_NEXT.offset(br.0, br.1), PlMiniNext),
            (p::MINI_EJECT.offset(br.0, br.1), PlMiniEject),
        ];
        if let Some((_, c)) = table.iter().find(|(r, _)| r.contains(x, y)) {
            return Some(*c);
        }
        let content = self.pl_content_rect(w, h);
        if Rect::new(w - p::SCROLL_FROM_RIGHT, p::TOP_H, 8, content.h).contains(x, y) {
            return Some(PlScroll);
        }
        let list = self.pl_list_rect(w, h);
        if list.contains(x, y) {
            let row = usize::try_from((y - list.y) / p::TRACK_H).unwrap_or(0);
            let idx = self.pl_scroll + row;
            if idx < self.tracks.len() {
                return Some(PlTrack(idx));
            }
        }
        None
    }

    /// The black list background between the frame pieces.
    #[must_use]
    pub fn pl_content_rect(&self, w: i32, h: i32) -> Rect {
        Rect::new(
            p::LEFT_W,
            p::TOP_H,
            w - p::LEFT_W - p::RIGHT_W,
            h - p::TOP_H - p::BOTTOM_H,
        )
    }

    /// The area actually holding track rows (inset from the list background).
    #[must_use]
    pub fn pl_list_rect(&self, w: i32, h: i32) -> Rect {
        let c = self.pl_content_rect(w, h);
        Rect::new(c.x, c.y + p::LIST_PAD, c.w, c.h - 2 * p::LIST_PAD)
    }

    #[must_use]
    pub fn pl_visible_rows(&self) -> usize {
        usize::try_from(self.pl_list_rect(self.pl_size.0, self.pl_size.1).h / p::TRACK_H)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn max_scroll(&self) -> usize {
        self.tracks.len().saturating_sub(self.pl_visible_rows())
    }

    fn ensure_visible(&mut self, idx: usize) {
        let rows = self.pl_visible_rows().max(1);
        if idx < self.pl_scroll {
            self.pl_scroll = idx;
        } else if idx >= self.pl_scroll + rows {
            self.pl_scroll = idx + 1 - rows;
        }
        self.pl_scroll = self.pl_scroll.min(self.max_scroll());
    }

    // ----- slider helpers -----------------------------------------------------------------

    /// Fraction `0..=1` of a horizontal slider for a pointer at `lx`, centring the thumb on it.
    fn h_fraction(rect: Rect, thumb_w: i32, lx: i32) -> f32 {
        let span = (rect.w - thumb_w) as f32;
        (((lx - rect.x) as f32 - thumb_w as f32 / 2.0) / span).clamp(0.0, 1.0)
    }

    /// Update a slider from a window-local pointer position and show its value in the marquee.
    fn drag_slider(&mut self, slider: Slider, lx: i32, ly: i32, now: Instant) {
        let mut text = self.marquee_override.take().unwrap_or_default();
        text.clear();
        match slider {
            Slider::Volume => {
                self.volume = Self::h_fraction(m::VOLUME, m::SLIDER_THUMB_W, lx);
                push_fmt(
                    &mut text,
                    format_args!("Volume: {}%", (self.volume * 100.0).round()),
                );
            }
            Slider::Balance => {
                let v = Self::h_fraction(m::BALANCE, m::SLIDER_THUMB_W, lx);
                let mut b = (v * 2.0 - 1.0).clamp(-1.0, 1.0);
                if b.abs() < 0.08 {
                    b = 0.0;
                }
                self.balance = b;
                let pct = (b.abs() * 100.0).round();
                if pct == 0.0 {
                    text.push_str("Balance: Center");
                } else {
                    let side = if b < 0.0 { "Left" } else { "Right" };
                    push_fmt(&mut text, format_args!("Balance: {pct}% {side}"));
                }
            }
            Slider::PosBar => {
                let pos = Self::h_fraction(m::POSBAR, m::POSBAR_THUMB_W, lx);
                let secs = pos * self.track_len();
                self.set_elapsed(secs, now);
                text.push_str("Seek to: ");
                write_time(&mut text, secs as u32);
                text.push('/');
                write_time(&mut text, self.track().map_or(0, |t| t.seconds));
                push_fmt(&mut text, format_args!(" ({}%)", (pos * 100.0).round()));
            }
            Slider::EqPreamp | Slider::EqBand(_) => {
                let rect = match slider {
                    Slider::EqBand(i) => e::band(i32::try_from(i).unwrap_or(0)),
                    _ => e::PREAMP,
                };
                let span = e::SLIDER_TRAVEL as f32;
                let v = (1.0 - (((ly - rect.y) as f32 - e::SLIDER_THUMB_H as f32 / 2.0) / span))
                    .clamp(0.0, 1.0);
                let db = (v - 0.5) * 24.0;
                match slider {
                    Slider::EqBand(i) if i < self.bands.len() => {
                        self.bands[i] = v;
                        push_fmt(
                            &mut text,
                            format_args!("EQ: {} HZ: {db:+.1} DB", e::BAND_LABELS[i]),
                        );
                    }
                    _ => {
                        self.preamp = v;
                        push_fmt(&mut text, format_args!("EQ: PREAMP: {db:+.1} DB"));
                    }
                }
            }
        }
        self.marquee_override = Some(text);
    }

    fn drag_scroll(&mut self, ly: i32) {
        let content = self.pl_content_rect(self.pl_size.0, self.pl_size.1);
        let span = (content.h - p::SCROLL_THUMB_H).max(1) as f32;
        let f = (((ly - content.y) as f32 - p::SCROLL_THUMB_H as f32 / 2.0) / span).clamp(0.0, 1.0);
        self.pl_scroll = (f * self.max_scroll() as f32).round() as usize;
    }

    /// Resize the playlist by the drag delta, snapped to Winamp's 25x29 grid. True if it changed.
    fn drag_resize(&mut self, x: i32, y: i32) -> bool {
        let snap = |v: i32, min: i32, step: i32| {
            let steps = ((v - min) as f32 / step as f32).round().max(0.0) as i32;
            min + steps * step
        };
        let w = self.drag_size.0 + (x - self.drag_start.0);
        let h = self.drag_size.1 + (y - self.drag_start.1);
        let new = (snap(w, p::MIN_W, p::STEP_W), snap(h, p::MIN_H, p::STEP_H));
        if new == self.pl_size {
            return false;
        }
        self.pl_size = new;
        self.pl_scroll = self.pl_scroll.min(self.max_scroll());
        true
    }

    // ----- input (all coordinates are local to the window of `kind`) ----------------------

    fn note_cursor(&mut self, kind: WindowKind, x: i32, y: i32) {
        if kind == WindowKind::Playlist {
            self.cursor = (x, y);
        }
    }

    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn mouse_down(&mut self, kind: WindowKind, x: i32, y: i32, now: Instant) -> Vec<Action> {
        use Control::*;
        self.note_cursor(kind, x, y);
        let mut actions = Vec::new();
        let Some(ctl) = self.hit(kind, x, y) else {
            return actions;
        };
        if let Some(slider) = ctl.slider() {
            self.pressed = Some(ctl);
            self.drag_slider(slider, x, y, now);
        } else {
            match ctl {
                MainTitle | EqTitle | PlTitle => actions.push(Action::DragWindow(kind)),
                PlScroll => {
                    self.pressed = Some(ctl);
                    self.drag_scroll(y);
                }
                PlResize => {
                    self.pressed = Some(ctl);
                    self.drag_start = (x, y);
                    self.drag_size = self.pl_size;
                }
                PlTrack(i) => {
                    let double = matches!(self.last_click,
                        Some((PlTrack(j), t)) if j == i && now.saturating_duration_since(t).as_millis() < DOUBLE_CLICK_MS);
                    self.selected.iter_mut().for_each(|s| *s = false);
                    if let Some(s) = self.selected.get_mut(i) {
                        *s = true;
                    }
                    if double {
                        self.jump(i, now);
                        if self.state != PlayState::Playing {
                            self.play(now);
                        }
                    }
                }
                // Every other control is a button: show it pressed until release.
                _ => self.pressed = Some(ctl),
            }
        }
        self.last_click = Some((ctl, now));
        actions
    }

    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn mouse_move(&mut self, kind: WindowKind, x: i32, y: i32, now: Instant) -> Vec<Action> {
        self.note_cursor(kind, x, y);
        let mut actions = Vec::new();
        // A drag only follows pointer motion reported by the window it started in.
        let Some(ctl) = self.pressed.filter(|c| c.window() == kind) else {
            return actions;
        };
        if let Some(slider) = ctl.slider() {
            self.drag_slider(slider, x, y, now);
        } else if ctl == Control::PlScroll {
            self.drag_scroll(y);
        } else if ctl == Control::PlResize && self.drag_resize(x, y) {
            actions.push(Action::Relayout);
        }
        actions
    }

    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn mouse_up(&mut self, kind: WindowKind, x: i32, y: i32, now: Instant) -> Vec<Action> {
        use Control::*;
        self.note_cursor(kind, x, y);
        let mut actions = Vec::new();
        let Some(ctl) = self.pressed.take() else {
            return actions;
        };
        self.marquee_override = None;
        let still_over = ctl.window() == kind && self.hit(kind, x, y) == Some(ctl);
        match ctl {
            // Drags finish silently.
            Volume | Balance | PosBar | EqPreamp | EqBand(_) | PlScroll | PlResize => {}
            // Releasing outside the pressed button cancels it, as in Win32.
            _ if !still_over => {}
            MainClose => actions.push(Action::Close),
            MainMinimize => actions.push(Action::Minimize),
            ClutterD => {
                self.double_size = !self.double_size;
                actions.push(Action::Relayout);
            }
            PlScrollUp => self.pl_scroll = self.pl_scroll.saturating_sub(1),
            PlScrollDown => self.pl_scroll = (self.pl_scroll + 1).min(self.max_scroll()),
            Prev | PlMiniPrev => self.prev(now),
            Play | PlMiniPlay => self.play(now),
            Pause | PlMiniPause => self.pause(now),
            Stop | PlMiniStop => self.stop(),
            Next | PlMiniNext => self.next(now),
            Shuffle => self.shuffle = !self.shuffle,
            Repeat => self.repeat = !self.repeat,
            EqToggle => {
                self.eq_open = !self.eq_open;
                actions.push(Action::Relayout);
            }
            PlToggle => {
                self.pl_open = !self.pl_open;
                actions.push(Action::Relayout);
            }
            TimeDisplay => self.remaining_mode = !self.remaining_mode,
            Control::Vis => self.vis.next_mode(),
            EqClose => {
                self.eq_open = false;
                actions.push(Action::Relayout);
            }
            PlClose => {
                self.pl_open = false;
                actions.push(Action::Relayout);
            }
            EqOn => self.eq_on = !self.eq_on,
            EqAuto => self.eq_auto = !self.eq_auto,
            // Press-only visuals: no behaviour in an interface-only build.
            MainTitle | EqTitle | PlTitle | MainOptions | MainShade | Eject | ClutterO
            | ClutterA | ClutterI | ClutterV | EqShade | EqPresets | PlShade | PlAdd | PlRem
            | PlSel | PlMisc | PlListOpts | PlMiniEject | PlTrack(_) => {}
        }
        actions
    }

    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn key(&mut self, cmd: KeyCmd, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        match cmd {
            KeyCmd::ToggleDoubleSize => {
                self.double_size = !self.double_size;
                actions.push(Action::Relayout);
            }
            KeyCmd::ToggleEq => {
                self.eq_open = !self.eq_open;
                actions.push(Action::Relayout);
            }
            KeyCmd::TogglePl => {
                self.pl_open = !self.pl_open;
                actions.push(Action::Relayout);
            }
            KeyCmd::Prev => self.prev(now),
            KeyCmd::Play => self.play(now),
            KeyCmd::Pause => self.pause(now),
            KeyCmd::Stop => self.stop(),
            KeyCmd::Next => self.next(now),
            KeyCmd::ToggleShuffle => self.shuffle = !self.shuffle,
            KeyCmd::ToggleRepeat => self.repeat = !self.repeat,
            KeyCmd::ToggleTimeMode => self.remaining_mode = !self.remaining_mode,
            KeyCmd::VolumeUp => self.volume = (self.volume + 0.02).min(1.0),
            KeyCmd::VolumeDown => self.volume = (self.volume - 0.02).max(0.0),
            KeyCmd::SeekBack => self.set_elapsed(self.elapsed(now) - 5.0, now),
            KeyCmd::SeekForward => self.set_elapsed(self.elapsed(now) + 5.0, now),
        }
        actions
    }
}

/// Make every pixel of `r` outside `mask` fully transparent.
fn apply_mask(c: &mut Image, r: Rect, mask: &Mask) {
    let w = usize::try_from(r.w).unwrap_or(0);
    if w == 0 {
        return;
    }
    for (i, visible) in mask.iter().enumerate() {
        if !visible {
            let (x, y) = (i % w, i / w);
            c.put(r.x + x as i32, r.y + y as i32, [0, 0, 0, 0]);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use WindowKind::{Eq, Main, Playlist};
    use std::time::Duration;

    #[test]
    fn stacked_layout_places_the_three_windows() {
        let mut ui = Ui::new();
        let l = ui.layout();
        assert_eq!(l.size, (275, 348));
        assert_eq!(l.eq.unwrap().y, 116);
        assert_eq!(l.pl.unwrap().y, 232);
        ui.eq_open = false;
        assert_eq!(ui.layout().pl.unwrap().y, 116);
        assert_eq!(ui.layout().size, (275, 232));
        assert_eq!(ui.window_size(Playlist), (275, 116));
        assert!(!ui.is_open(Eq) && ui.is_open(Main));
    }

    #[test]
    fn hit_testing_main_window() {
        let ui = Ui::new();
        assert_eq!(ui.hit(Main, 40, 95), Some(Control::Play));
        assert_eq!(ui.hit(Main, 268, 5), Some(Control::MainClose));
        assert_eq!(ui.hit(Main, 100, 5), Some(Control::MainTitle));
        assert_eq!(ui.hit(Main, 120, 60), Some(Control::Volume));
        assert_eq!(ui.hit(Main, 190, 60), Some(Control::Balance));
        assert_eq!(ui.hit(Main, 100, 75), Some(Control::PosBar));
        assert_eq!(ui.hit(Main, 50, 50), Some(Control::Vis));
        assert_eq!(ui.hit(Main, 12, 49), Some(Control::ClutterD));
        assert_eq!(ui.hit(Main, 150, 110), None);
        assert_eq!(ui.hit(Main, -1, 5), None);
        assert_eq!(ui.hit(Main, 40, 200), None);
    }

    #[test]
    fn hit_testing_eq_and_playlist() {
        let ui = Ui::new();
        assert_eq!(ui.hit(Eq, 80, 50), Some(Control::EqBand(0)));
        assert_eq!(ui.hit(Eq, 245, 50), Some(Control::EqBand(9)));
        assert_eq!(ui.hit(Eq, 25, 50), Some(Control::EqPreamp));
        assert_eq!(ui.hit(Eq, 20, 20), Some(Control::EqOn));
        assert_eq!(ui.hit(Eq, 100, 5), Some(Control::EqTitle));
        assert_eq!(ui.hit(Playlist, 100, 30), Some(Control::PlTrack(0)));
        assert_eq!(ui.hit(Playlist, 100, 43), Some(Control::PlTrack(1)));
        assert_eq!(ui.hit(Playlist, 275 - 10, 5), Some(Control::PlClose));
        assert_eq!(ui.hit(Playlist, 275 - 20, 5), Some(Control::PlShade));
        assert_eq!(ui.hit(Playlist, 275 - 12, 40), Some(Control::PlScroll));
        assert_eq!(ui.hit(Playlist, 20, 116 - 25), Some(Control::PlAdd));
        assert_eq!(
            ui.hit(Playlist, 275 - 10, 116 - 10),
            Some(Control::PlResize)
        );
    }

    #[test]
    fn every_control_reports_its_window() {
        assert_eq!(Control::Play.window(), Main);
        assert_eq!(Control::EqBand(3).window(), Eq);
        assert_eq!(Control::PlTrack(0).window(), Playlist);
    }

    #[test]
    fn title_bars_request_a_window_drag() {
        let mut ui = Ui::new();
        let now = Instant::now();
        assert_eq!(
            ui.mouse_down(Main, 100, 5, now),
            vec![Action::DragWindow(Main)]
        );
        assert_eq!(ui.mouse_down(Eq, 100, 5, now), vec![Action::DragWindow(Eq)]);
        assert_eq!(
            ui.mouse_down(Playlist, 100, 5, now),
            vec![Action::DragWindow(Playlist)]
        );
        assert!(ui.pressed.is_none());
    }

    #[test]
    fn volume_drag_updates_value_and_marquee() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let _ = ui.mouse_down(Main, 107 + 7, 60, now);
        assert!(ui.volume < 0.01);
        assert_eq!(ui.marquee_override.as_deref(), Some("Volume: 0%"));
        let _ = ui.mouse_move(Main, 107 + 68 - 7, 60, now);
        assert!(ui.volume > 0.99);
        // Motion reported by another window does not steer the drag.
        let _ = ui.mouse_move(Eq, 107, 60, now);
        assert!(ui.volume > 0.99);
        let _ = ui.mouse_up(Main, 107 + 68 - 7, 60, now);
        assert!(ui.pressed.is_none());
        assert!(ui.marquee_override.is_none());
    }

    #[test]
    fn transport_buttons_change_state() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let (px, py) = (85 + 5, 88 + 5); // stop
        let _ = ui.mouse_down(Main, px, py, now);
        assert_eq!(ui.pressed, Some(Control::Stop));
        let _ = ui.mouse_up(Main, px, py, now);
        assert_eq!(ui.state, PlayState::Stopped);
        let _ = ui.key(KeyCmd::Play, now);
        assert_eq!(ui.state, PlayState::Playing);
        let _ = ui.key(KeyCmd::Pause, now);
        assert_eq!(ui.state, PlayState::Paused);
        let _ = ui.key(KeyCmd::Next, now);
        assert_eq!(ui.current, 1);
    }

    #[test]
    fn releasing_outside_the_pressed_button_cancels_it() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let _ = ui.mouse_down(Main, 85 + 5, 88 + 5, now); // stop
        let actions = ui.mouse_up(Main, 5, 5, now);
        assert!(actions.is_empty());
        assert_eq!(ui.state, PlayState::Playing);
        let _ = ui.mouse_down(Main, 85 + 5, 88 + 5, now);
        let actions = ui.mouse_up(Eq, 85 + 5, 88 + 5, now);
        assert!(actions.is_empty());
        assert_eq!(ui.state, PlayState::Playing);
    }

    #[test]
    fn closing_and_toggling_windows_relayouts() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let _ = ui.mouse_down(Eq, 268, 5, now);
        assert_eq!(ui.mouse_up(Eq, 268, 5, now), vec![Action::Relayout]);
        assert!(!ui.eq_open);
        assert_eq!(ui.key(KeyCmd::ToggleEq, now), vec![Action::Relayout]);
        assert!(ui.eq_open);
        ui.set_open(Playlist, false);
        assert!(!ui.is_open(Playlist));
    }

    #[test]
    fn playlist_resize_snaps_to_grid() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let grip = (275 - 8, 116 - 8);
        let _ = ui.mouse_down(Playlist, grip.0, grip.1, now);
        assert_eq!(ui.pressed, Some(Control::PlResize));
        let actions = ui.mouse_move(Playlist, grip.0 + 30, grip.1 + 40, now);
        assert_eq!(actions, vec![Action::Relayout]);
        assert_eq!(ui.pl_size, (300, 145));
        assert_eq!(ui.window_size(Playlist), (300, 145));
        assert_eq!(ui.layout().size, (300, 232 + 145));
    }

    #[test]
    fn marquee_scrolls_long_titles_by_character() {
        let mut ui = Ui::new();
        ui.start = Instant::now();
        let t0 = ui.marquee_text(ui.start);
        let t1 = ui.marquee_text(ui.start + Duration::from_millis(MARQUEE_STEP_MS as u64));
        assert_eq!(t0.chars().count(), m::MARQUEE_CHARS);
        assert_eq!(&t0[1..], &t1[..t1.len() - 1]);
        assert!(t0.starts_with("1. DJ Mike Llama"));
    }

    #[test]
    fn short_titles_do_not_scroll_and_track_changes_refresh_the_title() {
        let mut ui = Ui::new();
        let now = Instant::now();
        ui.jump(5, now); // "6. Tom P. - Llama Country (3:18)" is 32 chars: scrolls
        assert!(ui.title_scrolls);
        ui.tracks[5].title = "Short".into();
        ui.jump(5, now);
        assert!(!ui.title_scrolls);
        assert_eq!(
            ui.marquee_text(now + Duration::from_secs(5)),
            "6. Short (3:18)"
        );
    }

    #[test]
    fn empty_playlist_never_panics() {
        let mut ui = Ui::new();
        let now = Instant::now();
        ui.tracks.clear();
        ui.selected.clear();
        ui.prev(now);
        ui.next(now);
        let _ = ui.key(KeyCmd::SeekForward, now);
        assert_eq!(ui.track_len(), 0.0);
        assert_eq!(ui.display_time(now), (0, 0, false));
        assert_eq!(ui.marquee_text(now), "");
    }

    #[test]
    fn per_window_drawing_matches_the_stacked_composition() {
        let skin = crate::skin::Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut ui = Ui::new();
        let now = Instant::now();
        ui.tick(now);
        let mut singles = Vec::new();
        for kind in WindowKind::ALL {
            let (w, h) = ui.window_size(kind);
            let mut c = Image::new(w, h);
            ui.draw_window(kind, &mut c, &skin, None, now);
            singles.push(c);
        }
        let lay = ui.layout();
        let mut stacked = Image::new(lay.size.0, lay.size.1);
        ui.draw_stacked(&mut stacked, &skin, None, now);
        let vis = m::VIS; // time-seeded, so excluded from the comparison
        for (i, origin) in [lay.main, lay.eq.unwrap(), lay.pl.unwrap()]
            .into_iter()
            .enumerate()
        {
            let single = &singles[i];
            for y in 0..single.h {
                for x in 0..single.w {
                    if i == 0 && vis.contains(x, y) {
                        continue;
                    }
                    assert_eq!(
                        single.get(x, y),
                        stacked.get(origin.x + x, origin.y + y),
                        "window {i} pixel ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn time_formatting() {
        assert_eq!(Ui::time_str(5), "0:05");
        assert_eq!(Ui::time_str(3661), "61:01");
    }
}
