//! UI state, hit testing and input handling, expressed per Winamp window so each window can
//! live in its own OS window.

use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::audio::{EQ_RANGE_DB, EqSettings, Event, TrackInfo};
use crate::canvas::{Image, Rect};
use crate::font::TtfFont;
use crate::skin::{Mask, Skin};
use crate::sprites::{eq as e, main as m, pl as p};
use crate::vis::Vis;
use crate::zoom::Zoom;
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
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// The user grabbed this window's title bar: start moving it.
    DragWindow(WindowKind),
    Close,
    Minimize,
    /// Window visibility, size or scale changed: re-sync the OS windows.
    Relayout,
    /// Tell the audio engine.
    Audio(Transport),
}

/// Playback commands for the audio engine. The UI state has already changed to match.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transport {
    /// Play playlist entry `n` from the start.
    Open(usize),
    Pause,
    Resume,
    Stop,
    /// Jump to this many seconds into the current track.
    Seek(f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyCmd {
    ToggleDoubleSize,
    /// One half step larger, e.g. 1.5x to 2x.
    ZoomIn,
    /// One half step smaller.
    ZoomOut,
    /// Back to the zoom chosen at startup.
    ZoomReset,
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

/// One playlist entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Track {
    /// "Artist - Title" from the tags, or the file name until they have been read.
    pub title: String,
    /// Length in whole seconds; `None` until known.
    pub seconds: Option<u32>,
    pub path: PathBuf,
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
    /// Seconds into the current track, as reported by the audio engine.
    pub elapsed: f32,
    /// Where a position bar drag will seek to when released.
    seek_to: Option<f32>,
    /// Tracks that failed to open in a row; when every track has, playback stops.
    failures: usize,
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
    /// How far the windows are enlarged; double size doubles it again (see [`Ui::scale`]).
    pub zoom: Zoom,
    /// The zoom chosen at startup, restored by [`KeyCmd::ZoomReset`].
    pub default_zoom: Zoom,
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

/// Decibels for an equalizer slider at `v` (`0..=1`, flat in the middle).
fn slider_db(v: f32) -> f32 {
    (v - 0.5) * 2.0 * EQ_RANGE_DB
}

impl Ui {
    /// An empty playlist, stopped.
    #[must_use]
    pub fn new() -> Ui {
        let now = Instant::now();
        Ui {
            state: PlayState::Stopped,
            elapsed: 0.0,
            seek_to: None,
            failures: 0,
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
            tracks: Vec::new(),
            current: 0,
            selected: Vec::new(),
            pl_scroll: 0,
            kbps: 0,
            khz: 0,
            stereo: false,
            vis: Vis::new(),
            double_size: false,
            zoom: Zoom::MIN,
            default_zoom: Zoom::MIN,
            active: true,
            cursor: (-1, -1),
            marquee_override: None,
            start: now,
            last_click: None,
            title_chars: Vec::new(),
            title_scrolls: false,
        }
    }

    /// A made-up playlist shown as if playing, for screenshots and tests. Its files do not
    /// exist.
    #[must_use]
    pub fn demo() -> Ui {
        const TRACKS: [(&str, u32); 7] = [
            ("DJ Mike Llama - Llama Whippin' Intro", 5),
            ("Nullsoft - Winamp 2.91 Theme", 213),
            ("Justin F. - Rolling Llama (Radio Edit)", 254),
            ("The Shoutcasters - Buffering...", 187),
            ("MilkDrop Crew - Plasma Visions", 312),
            ("Tom P. - Llama Country", 198),
            ("Unknown Artist - Track 07", 241),
        ];
        let mut ui = Ui::new();
        ui.tracks = TRACKS
            .iter()
            .enumerate()
            .map(|(i, (title, seconds))| Track {
                title: (*title).into(),
                seconds: Some(*seconds),
                path: PathBuf::from(format!("demo/{:02}.mp3", i + 1)),
            })
            .collect();
        ui.selected = vec![false; ui.tracks.len()];
        if let Some(first) = ui.selected.first_mut() {
            *first = true;
        }
        ui.state = PlayState::Playing;
        ui.kbps = 56;
        ui.khz = 22;
        ui.stereo = true;
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

    /// Screen pixels per skin pixel before the desktop's own scaling: the zoom, doubled in
    /// double-size mode. Always a multiple of 0.5.
    #[must_use]
    pub fn scale(&self) -> f64 {
        let double = if self.double_size { 2.0 } else { 1.0 };
        self.zoom.factor() * double
    }

    /// Change the zoom, asking for a relayout only if it actually changed.
    fn set_zoom(&mut self, zoom: Zoom, actions: &mut Vec<Action>) {
        if zoom != self.zoom {
            self.zoom = zoom;
            actions.push(Action::Relayout);
        }
    }

    // ----- playback model -----------------------------------------------------------------

    /// The current track, if the playlist has any.
    #[must_use]
    pub fn track(&self) -> Option<&Track> {
        self.tracks.get(self.current)
    }

    /// Length of the current track in seconds; 0 while unknown.
    #[must_use]
    pub fn track_len(&self) -> f32 {
        self.track()
            .and_then(|t| t.seconds)
            .map_or(0.0, |s| s as f32)
    }

    /// Playback position in `0..=1`, following a position bar drag while there is one.
    #[must_use]
    pub fn position(&self) -> f32 {
        let len = self.track_len();
        if len <= 0.0 {
            0.0
        } else {
            (self.seek_to.unwrap_or(self.elapsed) / len).clamp(0.0, 1.0)
        }
    }

    /// Minutes, seconds and whether a minus sign is shown (remaining-time mode, which needs a
    /// known length).
    #[must_use]
    pub fn display_time(&self) -> (u32, u32, bool) {
        let len = self.track_len();
        let remaining = self.remaining_mode && len > 0.0;
        let secs = if remaining {
            (len - self.elapsed).ceil()
        } else {
            self.elapsed.floor()
        };
        // Non-negative and far below u32::MAX, so the cast cannot truncate.
        let secs = secs.max(0.0) as u32;
        (secs / 60, secs % 60, remaining)
    }

    /// True during the "off" half of the paused blink.
    #[must_use]
    pub fn blink_off(&self, now: Instant) -> bool {
        self.state == PlayState::Paused
            && (now.saturating_duration_since(self.start).as_millis() % BLINK_MS) >= BLINK_MS / 2
    }

    /// Play the current track from the top (Play while playing restarts it, as in Winamp).
    fn start(&mut self, actions: &mut Vec<Action>) {
        if self.track().is_none() {
            return;
        }
        self.state = PlayState::Playing;
        self.elapsed = 0.0;
        self.seek_to = None;
        actions.push(Action::Audio(Transport::Open(self.current)));
    }

    pub fn play(&mut self, actions: &mut Vec<Action>) {
        if self.state == PlayState::Paused {
            self.state = PlayState::Playing;
            actions.push(Action::Audio(Transport::Resume));
        } else {
            self.start(actions);
        }
    }

    /// Pause, or resume when already paused.
    pub fn pause(&mut self, actions: &mut Vec<Action>) {
        match self.state {
            PlayState::Playing => {
                self.state = PlayState::Paused;
                actions.push(Action::Audio(Transport::Pause));
            }
            PlayState::Paused => {
                self.state = PlayState::Playing;
                actions.push(Action::Audio(Transport::Resume));
            }
            PlayState::Stopped => {}
        }
    }

    pub fn stop(&mut self, actions: &mut Vec<Action>) {
        if self.state != PlayState::Stopped {
            actions.push(Action::Audio(Transport::Stop));
        }
        self.state = PlayState::Stopped;
        self.elapsed = 0.0;
        self.seek_to = None;
        self.vis.reset();
    }

    /// Make playlist entry `idx` current without starting it.
    fn select(&mut self, idx: usize) {
        if self.tracks.is_empty() {
            self.current = 0;
        } else {
            self.current = idx % self.tracks.len();
            self.ensure_visible(self.current);
        }
        self.elapsed = 0.0;
        self.seek_to = None;
        self.refresh_title();
    }

    /// Move to playlist entry `idx`; it starts playing unless playback is stopped.
    pub fn jump(&mut self, idx: usize, actions: &mut Vec<Action>) {
        self.select(idx);
        if self.state != PlayState::Stopped {
            self.start(actions);
        }
    }

    /// Move to playlist entry `idx` and play it.
    pub fn play_track(&mut self, idx: usize, actions: &mut Vec<Action>) {
        self.select(idx);
        self.start(actions);
    }

    pub fn next(&mut self, actions: &mut Vec<Action>) {
        self.jump(self.current.wrapping_add(1), actions);
    }

    pub fn prev(&mut self, actions: &mut Vec<Action>) {
        let len = self.tracks.len();
        if len == 0 {
            return;
        }
        self.jump((self.current + len - 1) % len, actions);
    }

    /// After a track: the next one, or stop at the end of the playlist unless repeating.
    fn advance(&mut self, actions: &mut Vec<Action>) {
        if self.repeat || self.current + 1 < self.tracks.len() {
            self.next(actions);
        } else {
            self.stop(actions);
        }
    }

    /// Jump to `secs` into the current track, within its length when that is known.
    fn seek(&mut self, secs: f32, actions: &mut Vec<Action>) {
        if self.state == PlayState::Stopped {
            return;
        }
        let len = self.track_len();
        let secs = if len > 0.0 {
            secs.clamp(0.0, len)
        } else {
            secs.max(0.0)
        };
        self.elapsed = secs;
        actions.push(Action::Audio(Transport::Seek(secs)));
    }

    /// The position bar works while a track with a known length is playing or paused.
    fn can_seek(&self) -> bool {
        self.state != PlayState::Stopped && self.track_len() > 0.0
    }

    /// React to the audio engine: show what it opened, and move on when a track ends or fails.
    pub fn audio_event(&mut self, event: Event, actions: &mut Vec<Action>) {
        match event {
            Event::Started(info) => {
                self.failures = 0;
                self.khz = info.sample_rate / 1000;
                self.stereo = info.channels >= 2;
                if let Some(kbps) = info.kbps {
                    self.kbps = kbps;
                }
                if let Some(path) = self.track().map(|t| t.path.clone()) {
                    self.set_track_info(self.current, &path, &info);
                }
            }
            Event::Finished => self.advance(actions),
            Event::Failed(why) => {
                if let Some(t) = self.track() {
                    log::warn!("cannot play {}: {why}", t.path.display());
                }
                self.failures += 1;
                if self.failures >= self.tracks.len() {
                    self.stop(actions);
                } else {
                    self.advance(actions);
                }
            }
        }
    }

    /// Append files to the playlist, titled by file name until their tags are read. Returns
    /// the indices of the new entries.
    pub fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> Range<usize> {
        let start = self.tracks.len();
        self.tracks.extend(paths.into_iter().map(|path| Track {
            title: path.file_stem().map_or_else(
                || path.display().to_string(),
                |s| s.to_string_lossy().into_owned(),
            ),
            seconds: None,
            path,
        }));
        self.selected.resize(self.tracks.len(), false);
        if start == 0 && !self.tracks.is_empty() {
            self.refresh_title();
        }
        start..self.tracks.len()
    }

    /// Fill in a playlist entry's title and length once its file has been read. Ignored if the
    /// entry no longer plays `path`.
    pub fn set_track_info(&mut self, idx: usize, path: &Path, info: &TrackInfo) {
        let Some(track) = self.tracks.get_mut(idx).filter(|t| t.path == path) else {
            return;
        };
        if let Some(title) = &info.title {
            track.title.clone_from(title);
        }
        if let Some(secs) = info.seconds {
            // Rounded, non-negative and far below u32::MAX, so the cast cannot truncate.
            track.seconds = Some(secs.round().max(0.0) as u32);
        }
        if idx == self.current {
            self.refresh_title();
        }
    }

    /// The equalizer window's settings in decibels.
    #[must_use]
    pub fn eq_settings(&self) -> EqSettings {
        EqSettings {
            enabled: self.eq_on,
            preamp_db: slider_db(self.preamp),
            bands_db: self.bands.map(slider_db),
        }
    }

    /// Advance the visualiser. Call once per frame before drawing.
    pub fn tick(&mut self, now: Instant) {
        self.vis.update(now, self.state == PlayState::Playing);
    }

    // ----- text ---------------------------------------------------------------------------

    /// Rebuild the cached marquee text (`"N. Title (m:ss)"`, padded when it scrolls).
    fn refresh_title(&mut self) {
        self.title_chars.clear();
        let Some(t) = self.track() else { return };
        let mut s = String::with_capacity(64);
        push_fmt(&mut s, format_args!("{}. {}", self.current + 1, t.title));
        if let Some(secs) = t.seconds {
            s.push_str(" (");
            write_time(&mut s, secs);
            s.push(')');
        }
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
    fn drag_slider(&mut self, slider: Slider, lx: i32, ly: i32) {
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
                // Winamp seeks when the thumb is let go; until then only the thumb moves.
                let pos = Self::h_fraction(m::POSBAR, m::POSBAR_THUMB_W, lx);
                let len = self.track_len();
                let secs = pos * len;
                self.seek_to = Some(secs);
                text.push_str("Seek to: ");
                // Both lie in 0..=len, far below u32::MAX, so the casts cannot truncate.
                write_time(&mut text, secs as u32);
                text.push('/');
                write_time(&mut text, len as u32);
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
                let db = slider_db(v);
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
            if slider == Slider::PosBar && !self.can_seek() {
                return actions;
            }
            self.pressed = Some(ctl);
            self.drag_slider(slider, x, y);
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
                        self.play_track(i, &mut actions);
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
    pub fn mouse_move(&mut self, kind: WindowKind, x: i32, y: i32) -> Vec<Action> {
        self.note_cursor(kind, x, y);
        let mut actions = Vec::new();
        // A drag only follows pointer motion reported by the window it started in.
        let Some(ctl) = self.pressed.filter(|c| c.window() == kind) else {
            return actions;
        };
        if let Some(slider) = ctl.slider() {
            self.drag_slider(slider, x, y);
        } else if ctl == Control::PlScroll {
            self.drag_scroll(y);
        } else if ctl == Control::PlResize && self.drag_resize(x, y) {
            actions.push(Action::Relayout);
        }
        actions
    }

    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn mouse_up(&mut self, kind: WindowKind, x: i32, y: i32) -> Vec<Action> {
        use Control::*;
        self.note_cursor(kind, x, y);
        let mut actions = Vec::new();
        let Some(ctl) = self.pressed.take() else {
            return actions;
        };
        self.marquee_override = None;
        let still_over = ctl.window() == kind && self.hit(kind, x, y) == Some(ctl);
        match ctl {
            // Letting go of the position bar seeks to where the thumb was left.
            PosBar => {
                if let Some(secs) = self.seek_to.take() {
                    self.seek(secs, &mut actions);
                }
            }
            // Other drags finish silently.
            Volume | Balance | EqPreamp | EqBand(_) | PlScroll | PlResize => {}
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
            Prev | PlMiniPrev => self.prev(&mut actions),
            Play | PlMiniPlay => self.play(&mut actions),
            Pause | PlMiniPause => self.pause(&mut actions),
            Stop | PlMiniStop => self.stop(&mut actions),
            Next | PlMiniNext => self.next(&mut actions),
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
            // Press-only visuals: menus, dialogs and window shading are not implemented.
            MainTitle | EqTitle | PlTitle | MainOptions | MainShade | Eject | ClutterO
            | ClutterA | ClutterI | ClutterV | EqShade | EqPresets | PlShade | PlAdd | PlRem
            | PlSel | PlMisc | PlListOpts | PlMiniEject | PlTrack(_) => {}
        }
        actions
    }

    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn key(&mut self, cmd: KeyCmd) -> Vec<Action> {
        let mut actions = Vec::new();
        match cmd {
            KeyCmd::ToggleDoubleSize => {
                self.double_size = !self.double_size;
                actions.push(Action::Relayout);
            }
            KeyCmd::ZoomIn => self.set_zoom(self.zoom.larger(), &mut actions),
            KeyCmd::ZoomOut => self.set_zoom(self.zoom.smaller(), &mut actions),
            KeyCmd::ZoomReset => self.set_zoom(self.default_zoom, &mut actions),
            KeyCmd::ToggleEq => {
                self.eq_open = !self.eq_open;
                actions.push(Action::Relayout);
            }
            KeyCmd::TogglePl => {
                self.pl_open = !self.pl_open;
                actions.push(Action::Relayout);
            }
            KeyCmd::Prev => self.prev(&mut actions),
            KeyCmd::Play => self.play(&mut actions),
            KeyCmd::Pause => self.pause(&mut actions),
            KeyCmd::Stop => self.stop(&mut actions),
            KeyCmd::Next => self.next(&mut actions),
            KeyCmd::ToggleShuffle => self.shuffle = !self.shuffle,
            KeyCmd::ToggleRepeat => self.repeat = !self.repeat,
            KeyCmd::ToggleTimeMode => self.remaining_mode = !self.remaining_mode,
            KeyCmd::VolumeUp => self.volume = (self.volume + 0.02).min(1.0),
            KeyCmd::VolumeDown => self.volume = (self.volume - 0.02).max(0.0),
            KeyCmd::SeekBack => self.seek(self.elapsed - 5.0, &mut actions),
            KeyCmd::SeekForward => self.seek(self.elapsed + 5.0, &mut actions),
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
        let ui = Ui::demo();
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
        let _ = ui.mouse_move(Main, 107 + 68 - 7, 60);
        assert!(ui.volume > 0.99);
        // Motion reported by another window does not steer the drag.
        let _ = ui.mouse_move(Eq, 107, 60);
        assert!(ui.volume > 0.99);
        let _ = ui.mouse_up(Main, 107 + 68 - 7, 60);
        assert!(ui.pressed.is_none());
        assert!(ui.marquee_override.is_none());
    }

    fn audio(t: Transport) -> Action {
        Action::Audio(t)
    }

    #[test]
    fn transport_buttons_change_state_and_tell_the_engine() {
        let mut ui = Ui::demo();
        let now = Instant::now();
        let (px, py) = (85 + 5, 88 + 5); // stop
        let _ = ui.mouse_down(Main, px, py, now);
        assert_eq!(ui.pressed, Some(Control::Stop));
        assert_eq!(ui.mouse_up(Main, px, py), vec![audio(Transport::Stop)]);
        assert_eq!(ui.state, PlayState::Stopped);
        assert_eq!(ui.key(KeyCmd::Stop), vec![], "already stopped");
        // Stopped: only the selection moves.
        assert_eq!(ui.key(KeyCmd::Next), vec![]);
        assert_eq!(ui.current, 1);
        assert_eq!(ui.key(KeyCmd::Play), vec![audio(Transport::Open(1))]);
        assert_eq!(ui.state, PlayState::Playing);
        assert_eq!(ui.key(KeyCmd::Pause), vec![audio(Transport::Pause)]);
        assert_eq!(ui.state, PlayState::Paused);
        assert_eq!(ui.key(KeyCmd::Play), vec![audio(Transport::Resume)]);
        let _ = ui.key(KeyCmd::Pause);
        assert_eq!(ui.key(KeyCmd::Next), vec![audio(Transport::Open(2))]);
        // Changing track while paused plays the new one.
        assert_eq!(ui.state, PlayState::Playing);
        assert_eq!(ui.key(KeyCmd::Prev), vec![audio(Transport::Open(1))]);
    }

    #[test]
    fn double_clicking_a_playlist_entry_plays_it() {
        let mut ui = Ui::demo();
        let now = Instant::now();
        let _ = ui.key(KeyCmd::Stop);
        assert!(ui.mouse_down(Playlist, 100, 43, now).is_empty());
        let actions = ui.mouse_down(Playlist, 100, 43, now + Duration::from_millis(100));
        assert_eq!(actions, vec![audio(Transport::Open(1))]);
        assert_eq!((ui.current, ui.state), (1, PlayState::Playing));
    }

    #[test]
    fn finished_tracks_advance_and_the_last_one_stops() {
        let mut ui = Ui::demo();
        let mut actions = Vec::new();
        ui.audio_event(Event::Finished, &mut actions);
        assert_eq!(actions, vec![audio(Transport::Open(1))]);
        actions.clear();
        ui.play_track(6, &mut actions);
        actions.clear();
        ui.audio_event(Event::Finished, &mut actions);
        assert_eq!(actions, vec![audio(Transport::Stop)]);
        assert_eq!((ui.current, ui.state), (6, PlayState::Stopped));
        ui.repeat = true;
        ui.play_track(6, &mut actions);
        actions.clear();
        ui.audio_event(Event::Finished, &mut actions);
        // Repeat wraps around.
        assert_eq!(actions, vec![audio(Transport::Open(0))]);
    }

    #[test]
    fn failed_tracks_are_skipped_but_not_forever() {
        let mut ui = Ui::demo();
        ui.repeat = true;
        let mut actions = Vec::new();
        for i in 1..ui.tracks.len() {
            ui.audio_event(Event::Failed("nope".into()), &mut actions);
            assert_eq!(ui.current, i);
            assert_eq!(ui.state, PlayState::Playing);
        }
        actions.clear();
        ui.audio_event(Event::Failed("nope".into()), &mut actions);
        assert_eq!(actions, vec![audio(Transport::Stop)], "every track failed");
        // One success resets the count.
        let _ = ui.key(KeyCmd::Play);
        ui.audio_event(Event::Started(TrackInfo::default()), &mut actions);
        assert_eq!(ui.failures, 0);
    }

    #[test]
    fn the_position_bar_seeks_on_release() {
        let mut ui = Ui::demo(); // the first track is 5 seconds long
        let now = Instant::now();
        let bar = (m::POSBAR.x, m::POSBAR.y + 5);
        assert!(ui.mouse_down(Main, bar.0 + 120, bar.1, now).is_empty());
        let _ = ui.mouse_move(Main, bar.0 + 240, bar.1);
        assert_eq!(ui.elapsed, 0.0, "dragging only moves the thumb");
        assert!(ui.position() > 0.99);
        let actions = ui.mouse_up(Main, bar.0 + 240, bar.1);
        assert_eq!(actions, vec![audio(Transport::Seek(5.0))]);
        assert_eq!(ui.elapsed, 5.0);
        // Nothing to seek while stopped.
        let _ = ui.key(KeyCmd::Stop);
        let _ = ui.mouse_down(Main, bar.0 + 120, bar.1, now);
        assert_eq!(ui.pressed, None);
        assert!(ui.key(KeyCmd::SeekForward).is_empty());
    }

    #[test]
    fn arrow_keys_seek_within_the_track() {
        let mut ui = Ui::demo();
        ui.elapsed = 3.0;
        let seek = |secs| vec![audio(Transport::Seek(secs))];
        assert_eq!(ui.key(KeyCmd::SeekForward), seek(5.0));
        assert_eq!(ui.key(KeyCmd::SeekBack), seek(0.0));
    }

    #[test]
    fn added_files_are_named_until_their_tags_are_read() {
        let mut ui = Ui::new();
        let added = ui.add_files([PathBuf::from("/music/01 Intro.mp3"), "/music/b.wav".into()]);
        assert_eq!(added, 0..2);
        assert_eq!(ui.tracks[0].title, "01 Intro");
        assert_eq!(ui.tracks[0].seconds, None);
        assert_eq!(ui.selected.len(), 2);
        assert_eq!(ui.marquee_text(Instant::now()), "1. 01 Intro");
        let info = TrackInfo {
            title: Some("Band - Intro".into()),
            seconds: Some(61.4),
            ..TrackInfo::default()
        };
        ui.set_track_info(0, Path::new("/music/elsewhere.mp3"), &info);
        // A different file is ignored.
        assert_eq!(ui.tracks[0].title, "01 Intro");
        ui.set_track_info(0, Path::new("/music/01 Intro.mp3"), &info);
        assert_eq!(ui.tracks[0].title, "Band - Intro");
        assert_eq!(ui.tracks[0].seconds, Some(61));
        assert_eq!(ui.marquee_text(Instant::now()), "1. Band - Intro (1:01)");
        assert_eq!(ui.add_files([PathBuf::from("c.ogg")]), 2..3);
    }

    #[test]
    fn started_tracks_fill_in_the_readouts() {
        let mut ui = Ui::new();
        let _ = ui.add_files([PathBuf::from("song.mp3")]);
        let mut actions = Vec::new();
        ui.play_track(0, &mut actions);
        let info = TrackInfo {
            title: None,
            seconds: Some(200.0),
            sample_rate: 44_100,
            channels: 1,
            kbps: Some(128),
        };
        ui.audio_event(Event::Started(info), &mut actions);
        assert_eq!((ui.kbps, ui.khz, ui.stereo), (128, 44, false));
        assert_eq!(ui.tracks[0].seconds, Some(200));
        assert_eq!(ui.tracks[0].title, "song");
    }

    #[test]
    fn remaining_time_needs_a_known_length() {
        let mut ui = Ui::new();
        let _ = ui.add_files([PathBuf::from("song.mp3")]);
        ui.elapsed = 61.5;
        ui.remaining_mode = true;
        assert_eq!(ui.display_time(), (1, 1, false));
        ui.tracks[0].seconds = Some(100);
        assert_eq!(ui.display_time(), (0, 39, true));
    }

    #[test]
    fn eq_sliders_map_to_decibels() {
        let mut ui = Ui::new();
        ui.preamp = 1.0;
        ui.bands[0] = 0.0;
        ui.bands[9] = 0.75;
        let s = ui.eq_settings();
        assert!(s.enabled);
        assert_eq!(s.preamp_db, 12.0);
        assert_eq!(s.bands_db[0], -12.0);
        assert_eq!(s.bands_db[5], 0.0);
        assert_eq!(s.bands_db[9], 6.0);
    }

    #[test]
    fn releasing_outside_the_pressed_button_cancels_it() {
        let mut ui = Ui::demo();
        let now = Instant::now();
        let _ = ui.mouse_down(Main, 85 + 5, 88 + 5, now); // stop
        let actions = ui.mouse_up(Main, 5, 5);
        assert!(actions.is_empty());
        assert_eq!(ui.state, PlayState::Playing);
        let _ = ui.mouse_down(Main, 85 + 5, 88 + 5, now);
        let actions = ui.mouse_up(Eq, 85 + 5, 88 + 5);
        assert!(actions.is_empty());
        assert_eq!(ui.state, PlayState::Playing);
    }

    #[test]
    fn closing_and_toggling_windows_relayouts() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let _ = ui.mouse_down(Eq, 268, 5, now);
        assert_eq!(ui.mouse_up(Eq, 268, 5), vec![Action::Relayout]);
        assert!(!ui.eq_open);
        assert_eq!(ui.key(KeyCmd::ToggleEq), vec![Action::Relayout]);
        assert!(ui.eq_open);
        ui.set_open(Playlist, false);
        assert!(!ui.is_open(Playlist));
    }

    #[test]
    fn zoom_keys_step_and_reset_and_double_size_multiplies() {
        fn assert_scale(ui: &Ui, expected: f64) {
            let got = ui.scale();
            assert!(
                (got - expected).abs() < 1e-9,
                "scale {got}, expected {expected}"
            );
        }
        let mut ui = Ui::new();
        ui.default_zoom = Zoom::try_from(1.5).unwrap();
        assert_scale(&ui, 1.0);
        assert_eq!(ui.key(KeyCmd::ZoomOut), vec![], "already at 1x");
        assert_eq!(ui.key(KeyCmd::ZoomIn), vec![Action::Relayout]);
        assert_scale(&ui, 1.5);
        let _ = ui.key(KeyCmd::ToggleDoubleSize);
        assert_scale(&ui, 3.0);
        let _ = ui.key(KeyCmd::ZoomIn);
        assert_scale(&ui, 4.0);
        assert_eq!(ui.key(KeyCmd::ZoomReset), vec![Action::Relayout]);
        assert_eq!(ui.zoom, ui.default_zoom);
        assert_eq!(ui.key(KeyCmd::ZoomReset), vec![]);
    }

    #[test]
    fn playlist_resize_snaps_to_grid() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let grip = (275 - 8, 116 - 8);
        let _ = ui.mouse_down(Playlist, grip.0, grip.1, now);
        assert_eq!(ui.pressed, Some(Control::PlResize));
        let actions = ui.mouse_move(Playlist, grip.0 + 30, grip.1 + 40);
        assert_eq!(actions, vec![Action::Relayout]);
        assert_eq!(ui.pl_size, (300, 145));
        assert_eq!(ui.window_size(Playlist), (300, 145));
        assert_eq!(ui.layout().size, (300, 232 + 145));
    }

    #[test]
    fn marquee_scrolls_long_titles_by_character() {
        let mut ui = Ui::demo();
        ui.start = Instant::now();
        let t0 = ui.marquee_text(ui.start);
        let t1 = ui.marquee_text(ui.start + Duration::from_millis(MARQUEE_STEP_MS as u64));
        assert_eq!(t0.chars().count(), m::MARQUEE_CHARS);
        assert_eq!(&t0[1..], &t1[..t1.len() - 1]);
        assert!(t0.starts_with("1. DJ Mike Llama"));
    }

    #[test]
    fn short_titles_do_not_scroll_and_track_changes_refresh_the_title() {
        let mut ui = Ui::demo();
        let now = Instant::now();
        let mut actions = Vec::new();
        ui.jump(5, &mut actions); // "6. Tom P. - Llama Country (3:18)" is 32 chars: scrolls
        assert!(ui.title_scrolls);
        ui.tracks[5].title = "Short".into();
        ui.jump(5, &mut actions);
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
        let mut actions = Vec::new();
        ui.prev(&mut actions);
        ui.next(&mut actions);
        ui.play(&mut actions);
        let _ = ui.key(KeyCmd::SeekForward);
        assert!(actions.is_empty());
        assert_eq!(ui.state, PlayState::Stopped);
        assert_eq!(ui.track_len(), 0.0);
        assert_eq!(ui.display_time(), (0, 0, false));
        assert_eq!(ui.marquee_text(now), "");
    }

    #[test]
    fn per_window_drawing_matches_the_stacked_composition() {
        let skin = crate::skin::Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut ui = Ui::demo();
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
