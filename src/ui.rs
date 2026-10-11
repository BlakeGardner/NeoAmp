//! UI state, window layout, hit testing and input handling.

use std::time::Instant;

use crate::canvas::{Image, Rect};
use crate::font::TtfFont;
use crate::skin::{Skin, region_mask};
use crate::sprites::{eq as e, main as m, pl as p};
use crate::vis::Vis;
use crate::{eq_window, main_window, playlist_window};

pub const MARQUEE_STEP_MS: u128 = 220;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    DragWindow,
    Close,
    Minimize,
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

#[derive(Clone, Debug)]
pub struct Track {
    pub title: String,
    pub seconds: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub main: Rect,
    pub eq: Option<Rect>,
    pub pl: Option<Rect>,
    pub size: (i32, i32),
}

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
    pub active: bool,
    pub cursor: (i32, i32),
    pub marquee_override: Option<String>,
    pub start: Instant,
    last_click: Option<(Control, Instant)>,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    pub fn new() -> Ui {
        let tracks = vec![
            Track { title: "DJ Mike Llama - Llama Whippin' Intro".into(), seconds: 5 },
            Track { title: "Nullsoft - Winamp 2.91 Theme".into(), seconds: 213 },
            Track { title: "Justin F. - Rolling Llama (Radio Edit)".into(), seconds: 254 },
            Track { title: "The Shoutcasters - Buffering...".into(), seconds: 187 },
            Track { title: "MilkDrop Crew - Plasma Visions".into(), seconds: 312 },
            Track { title: "Tom P. - Llama Country".into(), seconds: 198 },
            Track { title: "Unknown Artist - Track 07".into(), seconds: 241 },
        ];
        let mut selected = vec![false; tracks.len()];
        selected[0] = true;
        let now = Instant::now();
        Ui {
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
        }
    }

    // ----- layout -------------------------------------------------------------------------

    pub fn layout(&self) -> Layout {
        let main = Rect::new(0, 0, m::W, m::H);
        let mut y = m::H;
        let eq = if self.eq_open {
            let r = Rect::new(0, y, e::W, e::H);
            y += e::H;
            Some(r)
        } else {
            None
        };
        let pl = if self.pl_open {
            let r = Rect::new(0, y, self.pl_size.0, self.pl_size.1);
            y += self.pl_size.1;
            Some(r)
        } else {
            None
        };
        let w = if self.pl_open { m::W.max(self.pl_size.0) } else { m::W };
        Layout { main, eq, pl, size: (w, y) }
    }

    pub fn scale(&self) -> u32 {
        if self.double_size { 2 } else { 1 }
    }

    // ----- playback model -----------------------------------------------------------------

    pub fn track(&self) -> &Track {
        &self.tracks[self.current]
    }

    pub fn track_len(&self) -> f32 {
        self.track().seconds as f32
    }

    pub fn elapsed(&self, now: Instant) -> f32 {
        let mut t = self.elapsed_base;
        if let Some(r) = self.resumed_at {
            t += (now - r).as_secs_f32();
        }
        t.clamp(0.0, self.track_len())
    }

    fn set_elapsed(&mut self, secs: f32, now: Instant) {
        self.elapsed_base = secs.clamp(0.0, self.track_len());
        if self.state == PlayState::Playing {
            self.resumed_at = Some(now);
        }
    }

    pub fn position(&self, now: Instant) -> f32 {
        let len = self.track_len();
        if len <= 0.0 { 0.0 } else { self.elapsed(now) / len }
    }

    /// Minutes, seconds and sign for the LED display.
    pub fn display_time(&self, now: Instant) -> (u32, u32, bool) {
        let e = self.elapsed(now);
        let secs = if self.remaining_mode { (self.track_len() - e).ceil() } else { e.floor() };
        let secs = secs.max(0.0) as u32;
        (secs / 60, secs % 60, self.remaining_mode)
    }

    pub fn play(&mut self, now: Instant) {
        match self.state {
            PlayState::Paused => self.resumed_at = Some(now),
            _ => {
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
            return;
        }
        self.current = idx % self.tracks.len();
        self.elapsed_base = 0.0;
        if self.state == PlayState::Playing {
            self.resumed_at = Some(now);
        }
        self.ensure_visible(self.current);
    }

    pub fn next(&mut self, now: Instant) {
        self.jump(self.current + 1, now);
    }

    pub fn prev(&mut self, now: Instant) {
        self.jump((self.current + self.tracks.len() - 1) % self.tracks.len(), now);
    }

    fn update(&mut self, now: Instant) {
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

    pub fn marquee_text(&self, now: Instant) -> String {
        if let Some(s) = &self.marquee_override {
            return s.clone();
        }
        let t = self.track();
        let base = format!("{}. {} ({}:{:02})", self.current + 1, t.title, t.seconds / 60, t.seconds % 60);
        if base.chars().count() <= m::MARQUEE_CHARS {
            return base;
        }
        let full: Vec<char> = format!("{base}  ***  ").chars().collect();
        let step = ((now - self.start).as_millis() / MARQUEE_STEP_MS) as usize % full.len();
        full.iter().cycle().skip(step).take(m::MARQUEE_CHARS).collect()
    }

    pub fn time_str(secs: u32) -> String {
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    // ----- drawing ------------------------------------------------------------------------

    pub fn draw(&mut self, c: &mut Image, skin: &Skin, font: Option<&TtfFont>, now: Instant) {
        self.update(now);
        let lay = self.layout();
        c.clear([0, 0, 0, 0]);
        main_window::draw(self, c, skin, lay.main, now);
        if let Some(r) = lay.eq {
            eq_window::draw(self, c, skin, r, now);
        }
        if let Some(r) = lay.pl {
            playlist_window::draw(self, c, skin, font, r, now);
        }
        if let Some(polys) = &skin.regions.normal {
            apply_mask(c, lay.main, &region_mask(polys, lay.main.w, lay.main.h));
        }
        if let (Some(polys), Some(r)) = (&skin.regions.equalizer, lay.eq) {
            apply_mask(c, r, &region_mask(polys, r.w, r.h));
        }
    }

    // ----- hit testing --------------------------------------------------------------------

    pub fn hit(&self, x: i32, y: i32) -> Option<Control> {
        let lay = self.layout();
        if lay.main.contains(x, y) {
            return self.hit_main(x - lay.main.x, y - lay.main.y);
        }
        if let Some(r) = lay.eq
            && r.contains(x, y)
        {
            return self.hit_eq(x - r.x, y - r.y);
        }
        if let Some(r) = lay.pl
            && r.contains(x, y)
        {
            return self.hit_pl(x - r.x, y - r.y, r.w, r.h);
        }
        None
    }

    fn hit_main(&self, x: i32, y: i32) -> Option<Control> {
        use Control::*;
        let table: &[(Rect, Control)] = &[
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
        table.iter().find(|(r, _)| r.contains(x, y)).map(|(_, c)| *c)
    }

    fn hit_eq(&self, x: i32, y: i32) -> Option<Control> {
        use Control::*;
        for i in 0..10 {
            if e::band(i as i32).contains(x, y) {
                return Some(EqBand(i));
            }
        }
        let table: &[(Rect, Control)] = &[
            (e::CLOSE, EqClose),
            (e::SHADE, EqShade),
            (e::TITLE_RECT, EqTitle),
            (e::ON, EqOn),
            (e::AUTO, EqAuto),
            (e::PRESETS, EqPresets),
            (e::PREAMP, EqPreamp),
        ];
        table.iter().find(|(r, _)| r.contains(x, y)).map(|(_, c)| *c)
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
        let table: &[(Rect, Control)] = &[
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
            let idx = self.pl_scroll + ((y - list.y) / p::TRACK_H) as usize;
            if idx < self.tracks.len() {
                return Some(PlTrack(idx));
            }
        }
        None
    }

    /// The black list background between the frame pieces.
    pub fn pl_content_rect(&self, w: i32, h: i32) -> Rect {
        Rect::new(p::LEFT_W, p::TOP_H, w - p::LEFT_W - p::RIGHT_W, h - p::TOP_H - p::BOTTOM_H)
    }

    /// The area actually holding track rows (inset from the list background).
    pub fn pl_list_rect(&self, w: i32, h: i32) -> Rect {
        let c = self.pl_content_rect(w, h);
        Rect::new(c.x, c.y + p::LIST_PAD, c.w, c.h - 2 * p::LIST_PAD)
    }

    pub fn pl_visible_rows(&self) -> usize {
        (self.pl_list_rect(self.pl_size.0, self.pl_size.1).h / p::TRACK_H).max(0) as usize
    }

    fn max_scroll(&self) -> usize {
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

    fn local(&self, x: i32, y: i32, ctl: Control) -> (i32, i32) {
        let lay = self.layout();
        let origin = match ctl {
            Control::EqPreamp | Control::EqBand(_) => lay.eq.map(|r| (r.x, r.y)).unwrap_or((0, 0)),
            Control::PlResize | Control::PlScroll => lay.pl.map(|r| (r.x, r.y)).unwrap_or((0, 0)),
            _ => (lay.main.x, lay.main.y),
        };
        (x - origin.0, y - origin.1)
    }

    fn drag_slider(&mut self, ctl: Control, x: i32, y: i32, now: Instant) {
        let (lx, ly) = self.local(x, y, ctl);
        match ctl {
            Control::Volume => {
                let span = (m::VOLUME.w - m::SLIDER_THUMB_W) as f32;
                self.volume = ((lx - m::VOLUME.x) as f32 - m::SLIDER_THUMB_W as f32 / 2.0) / span;
                self.volume = self.volume.clamp(0.0, 1.0);
                self.marquee_override = Some(format!("Volume: {}%", (self.volume * 100.0).round() as u32));
            }
            Control::Balance => {
                let span = (m::BALANCE.w - m::SLIDER_THUMB_W) as f32;
                let v = ((lx - m::BALANCE.x) as f32 - m::SLIDER_THUMB_W as f32 / 2.0) / span;
                let mut b = (v * 2.0 - 1.0).clamp(-1.0, 1.0);
                if b.abs() < 0.08 {
                    b = 0.0;
                }
                self.balance = b;
                let pct = (b.abs() * 100.0).round() as u32;
                self.marquee_override = Some(if pct == 0 {
                    "Balance: Center".to_string()
                } else if b < 0.0 {
                    format!("Balance: {pct}% Left")
                } else {
                    format!("Balance: {pct}% Right")
                });
            }
            Control::PosBar => {
                let span = (m::POSBAR.w - m::POSBAR_THUMB_W) as f32;
                let pos = (((lx - m::POSBAR.x) as f32 - m::POSBAR_THUMB_W as f32 / 2.0) / span).clamp(0.0, 1.0);
                let secs = pos * self.track_len();
                self.set_elapsed(secs, now);
                self.marquee_override = Some(format!(
                    "Seek to: {}/{} ({}%)",
                    Self::time_str(secs as u32),
                    Self::time_str(self.track().seconds),
                    (pos * 100.0).round() as u32
                ));
            }
            Control::EqPreamp | Control::EqBand(_) => {
                let rect = match ctl {
                    Control::EqBand(i) => e::band(i as i32),
                    _ => e::PREAMP,
                };
                let span = e::SLIDER_TRAVEL as f32;
                let v = 1.0 - (((ly - rect.y) as f32 - e::SLIDER_THUMB_H as f32 / 2.0) / span);
                let v = v.clamp(0.0, 1.0);
                let db = (v - 0.5) * 24.0;
                match ctl {
                    Control::EqBand(i) => {
                        self.bands[i] = v;
                        self.marquee_override = Some(format!("EQ: {} HZ: {:+.1} DB", e::BAND_LABELS[i], db));
                    }
                    _ => {
                        self.preamp = v;
                        self.marquee_override = Some(format!("EQ: PREAMP: {:+.1} DB", db));
                    }
                }
            }
            _ => {}
        }
    }

    fn drag_scroll(&mut self, x: i32, y: i32) {
        let (_, ly) = self.local(x, y, Control::PlScroll);
        let content = self.pl_content_rect(self.pl_size.0, self.pl_size.1);
        let span = (content.h - p::SCROLL_THUMB_H).max(1) as f32;
        let f = (((ly - content.y) as f32 - p::SCROLL_THUMB_H as f32 / 2.0) / span).clamp(0.0, 1.0);
        self.pl_scroll = (f * self.max_scroll() as f32).round() as usize;
    }

    fn drag_resize(&mut self, x: i32, y: i32) -> bool {
        let dx = x - self.drag_start.0;
        let dy = y - self.drag_start.1;
        let w = self.drag_size.0 + dx;
        let h = self.drag_size.1 + dy;
        let snap = |v: i32, min: i32, step: i32| min + (((v - min) as f32 / step as f32).round().max(0.0) as i32) * step;
        let new = (snap(w, p::MIN_W, p::STEP_W), snap(h, p::MIN_H, p::STEP_H));
        if new != self.pl_size {
            self.pl_size = new;
            self.pl_scroll = self.pl_scroll.min(self.max_scroll());
            true
        } else {
            false
        }
    }

    // ----- input --------------------------------------------------------------------------

    pub fn mouse_down(&mut self, x: i32, y: i32, now: Instant) -> Vec<Action> {
        use Control::*;
        self.cursor = (x, y);
        let mut actions = Vec::new();
        let Some(ctl) = self.hit(x, y) else { return actions };
        match ctl {
            MainTitle | EqTitle | PlTitle => {
                actions.push(Action::DragWindow);
            }
            Volume | Balance | PosBar | EqPreamp | EqBand(_) => {
                self.pressed = Some(ctl);
                self.drag_slider(ctl, x, y, now);
            }
            PlScroll => {
                self.pressed = Some(ctl);
                self.drag_scroll(x, y);
            }
            PlResize => {
                self.pressed = Some(ctl);
                self.drag_start = (x, y);
                self.drag_size = self.pl_size;
            }
            PlTrack(i) => {
                let double = matches!(self.last_click, Some((PlTrack(j), t)) if j == i && (now - t).as_millis() < 400);
                for s in self.selected.iter_mut() {
                    *s = false;
                }
                self.selected[i] = true;
                if double {
                    self.jump(i, now);
                    if self.state != PlayState::Playing {
                        self.play(now);
                    }
                }
            }
            _ => {
                self.pressed = Some(ctl);
            }
        }
        self.last_click = Some((ctl, now));
        actions
    }

    #[allow(clippy::collapsible_match)]
    pub fn mouse_move(&mut self, x: i32, y: i32, now: Instant) -> Vec<Action> {
        use Control::*;
        self.cursor = (x, y);
        let mut actions = Vec::new();
        match self.pressed {
            Some(ctl @ (Volume | Balance | PosBar | EqPreamp | EqBand(_))) => self.drag_slider(ctl, x, y, now),
            Some(PlScroll) => self.drag_scroll(x, y),
            Some(PlResize) => {
                if self.drag_resize(x, y) {
                    actions.push(Action::Relayout);
                }
            }
            _ => {}
        }
        actions
    }

    pub fn mouse_up(&mut self, x: i32, y: i32, now: Instant) -> Vec<Action> {
        use Control::*;
        self.cursor = (x, y);
        let mut actions = Vec::new();
        let Some(ctl) = self.pressed.take() else { return actions };
        self.marquee_override = None;
        let still_over = self.hit(x, y) == Some(ctl);
        match ctl {
            Volume | Balance | PosBar | EqPreamp | EqBand(_) | PlScroll | PlResize => {}
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
            _ => {}
        }
        actions
    }

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
            KeyCmd::SeekBack => {
                let t = self.elapsed(now) - 5.0;
                self.set_elapsed(t, now);
            }
            KeyCmd::SeekForward => {
                let t = self.elapsed(now) + 5.0;
                self.set_elapsed(t, now);
            }
        }
        actions
    }
}

fn apply_mask(c: &mut Image, r: Rect, mask: &[bool]) {
    for y in 0..r.h {
        for x in 0..r.w {
            if !mask[(y * r.w + x) as usize] {
                c.put(r.x + x, r.y + y, [0, 0, 0, 0]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_stacks_the_three_windows() {
        let ui = Ui::new();
        let l = ui.layout();
        assert_eq!(l.size, (275, 348));
        assert_eq!(l.eq.unwrap().y, 116);
        assert_eq!(l.pl.unwrap().y, 232);
        let mut ui = ui;
        ui.eq_open = false;
        assert_eq!(ui.layout().pl.unwrap().y, 116);
        assert_eq!(ui.layout().size, (275, 232));
    }

    #[test]
    fn hit_testing_main_window() {
        let ui = Ui::new();
        assert_eq!(ui.hit(40, 95), Some(Control::Play));
        assert_eq!(ui.hit(268, 5), Some(Control::MainClose));
        assert_eq!(ui.hit(100, 5), Some(Control::MainTitle));
        assert_eq!(ui.hit(120, 60), Some(Control::Volume));
        assert_eq!(ui.hit(190, 60), Some(Control::Balance));
        assert_eq!(ui.hit(100, 75), Some(Control::PosBar));
        assert_eq!(ui.hit(50, 50), Some(Control::Vis));
        assert_eq!(ui.hit(12, 49), Some(Control::ClutterD));
        assert_eq!(ui.hit(150, 110), None);
    }

    #[test]
    fn hit_testing_eq_and_playlist() {
        let ui = Ui::new();
        assert_eq!(ui.hit(80, 116 + 50), Some(Control::EqBand(0)));
        assert_eq!(ui.hit(245, 116 + 50), Some(Control::EqBand(9)));
        assert_eq!(ui.hit(25, 116 + 50), Some(Control::EqPreamp));
        assert_eq!(ui.hit(20, 116 + 20), Some(Control::EqOn));
        assert_eq!(ui.hit(100, 232 + 30), Some(Control::PlTrack(0)));
        assert_eq!(ui.hit(100, 232 + 43), Some(Control::PlTrack(1)));
        assert_eq!(ui.hit(275 - 10, 232 + 5), Some(Control::PlClose));
        assert_eq!(ui.hit(275 - 20, 232 + 5), Some(Control::PlShade));
        assert_eq!(ui.hit(275 - 12, 232 + 40), Some(Control::PlScroll));
        assert_eq!(ui.hit(20, 232 + 116 - 25), Some(Control::PlAdd));
        assert_eq!(ui.hit(275 - 10, 232 + 116 - 10), Some(Control::PlResize));
    }

    #[test]
    fn volume_drag_updates_value_and_marquee() {
        let mut ui = Ui::new();
        let now = Instant::now();
        ui.mouse_down(107 + 7, 60, now);
        assert!(ui.volume < 0.01);
        assert_eq!(ui.marquee_override.as_deref(), Some("Volume: 0%"));
        ui.mouse_move(107 + 68 - 7, 60, now);
        assert!(ui.volume > 0.99);
        ui.mouse_up(107 + 68 - 7, 60, now);
        assert!(ui.pressed.is_none());
        assert!(ui.marquee_override.is_none());
    }

    #[test]
    fn transport_buttons_change_state() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let (px, py) = (85 + 5, 88 + 5); // stop
        ui.mouse_down(px, py, now);
        assert_eq!(ui.pressed, Some(Control::Stop));
        ui.mouse_up(px, py, now);
        assert_eq!(ui.state, PlayState::Stopped);
        ui.key(KeyCmd::Play, now);
        assert_eq!(ui.state, PlayState::Playing);
        ui.key(KeyCmd::Pause, now);
        assert_eq!(ui.state, PlayState::Paused);
        ui.key(KeyCmd::Next, now);
        assert_eq!(ui.current, 1);
    }

    #[test]
    fn playlist_resize_snaps_to_grid() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let grip = (275 - 8, 232 + 116 - 8);
        ui.mouse_down(grip.0, grip.1, now);
        assert_eq!(ui.pressed, Some(Control::PlResize));
        let actions = ui.mouse_move(grip.0 + 30, grip.1 + 40, now);
        assert_eq!(actions, vec![Action::Relayout]);
        assert_eq!(ui.pl_size, (300, 145));
        assert_eq!(ui.layout().size, (300, 232 + 145));
    }

    #[test]
    fn marquee_scrolls_long_titles_by_character() {
        let mut ui = Ui::new();
        ui.start = Instant::now();
        let t0 = ui.marquee_text(ui.start);
        let t1 = ui.marquee_text(ui.start + std::time::Duration::from_millis(MARQUEE_STEP_MS as u64));
        assert_eq!(t0.chars().count(), m::MARQUEE_CHARS);
        assert_eq!(&t0[1..], &t1[..t1.len() - 1]);
    }
}
