//! UI state, hit testing and input handling, expressed per NeoAmp window so each window can
//! live in its own OS window.

use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::audio::{EQ_RANGE_DB, EqSettings, Event, Location, Station, StreamStatus, TrackInfo};
use crate::canvas::{Canvas, Rect};
use crate::cursor::SkinCursor;
use crate::eqf::{self, Preset};
use crate::font::TtfFont;
use crate::info::StreamDetails;
use crate::menus::{AppCommand, Command, MenuKind};
use crate::playlist::{self, Rng, Shuffle, SortKey};
use crate::skin::{Polygons, Skin, cover_row};
use crate::sprites::{
    eq as e, eq_shade as es, main as m, main_shade as ms, pl as p, pl_shade as ps,
};
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
/// The app's name, which ends the main window's title.
pub const APP_NAME: &str = "NeoAmp";
/// The volume of a first start, before any settings are saved.
pub const DEFAULT_VOLUME: f32 = 0.78;
/// How far Up, Down and a notch of the mouse wheel move the volume.
const VOLUME_STEP: f32 = 0.02;
/// How long Shift+V takes to fade the music out before it stops.
pub const FADE_OUT: Duration = Duration::from_secs(3);

/// An output device chosen by name, which plays even when other devices become the system's
/// default.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutputChoice {
    /// What the system calls it internally (see [`crate::audio::OutputDevice::id`]).
    pub id: String,
    /// Its name, for the menu.
    pub name: String,
}

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
    /// The volume slider of the equalizer's windowshade bar.
    EqVolume,
    /// The balance slider of the equalizer's windowshade bar.
    EqBalance,
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
    /// The volume, from the equalizer's windowshade bar.
    EqVolume,
    /// The balance, from the equalizer's windowshade bar.
    EqBalance,
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
            Control::EqVolume => Some(Slider::EqVolume),
            Control::EqBalance => Some(Slider::EqBalance),
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
            EqTitle | EqClose | EqShade | EqOn | EqAuto | EqPresets | EqPreamp | EqBand(_)
            | EqVolume | EqBalance => WindowKind::Eq,
            PlTitle | PlClose | PlShade | PlAdd | PlRem | PlSel | PlMisc | PlListOpts
            | PlResize | PlScroll | PlScrollUp | PlScrollDown | PlTrack(_) | PlMiniPrev
            | PlMiniPlay | PlMiniPause | PlMiniStop | PlMiniNext | PlMiniEject => {
                WindowKind::Playlist
            }
        }
    }
}

/// Modifier keys held during a click.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub shift: bool,
    /// Ctrl, or Cmd on macOS, where Ctrl-click means right-click.
    pub ctrl: bool,
}

/// The menus that rise from the buttons at the bottom of the playlist while one is held,
/// drawn from the skin. Letting go over an item chooses it. Each menu's bottom item lies over
/// its button, so a plain click chooses that one (ADD FILE, REM SEL, ...), as in Winamp.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlMenu {
    Add,
    Remove,
    Select,
    Misc,
    List,
}

/// An item of a playlist menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlItem {
    AddUrl,
    AddDir,
    AddFile,
    /// Opens a menu of Winamp's other ways to remove entries.
    RemoveMisc,
    RemoveAll,
    Crop,
    RemoveSelected,
    InvertSelection,
    SelectNone,
    SelectAll,
    /// Opens the sort menu.
    SortList,
    FileInfo,
    MiscOptions,
    NewList,
    SaveList,
    LoadList,
}

impl PlItem {
    /// The item's row in its menu's column of `pledit.bmp`. The rows are in the order the
    /// menus show them, except REM's: REM MISC is drawn last in the bitmap but shows first.
    #[must_use]
    pub fn sprite_row(self) -> i32 {
        use PlItem::*;
        match self {
            AddUrl | RemoveAll | InvertSelection | SortList | NewList => 0,
            AddDir | Crop | SelectNone | FileInfo | SaveList => 1,
            AddFile | RemoveSelected | SelectAll | MiscOptions | LoadList => 2,
            RemoveMisc => 3,
        }
    }
}

impl PlMenu {
    /// The menu a button opens.
    #[must_use]
    pub fn of(button: Control) -> Option<PlMenu> {
        match button {
            Control::PlAdd => Some(PlMenu::Add),
            Control::PlRem => Some(PlMenu::Remove),
            Control::PlSel => Some(PlMenu::Select),
            Control::PlMisc => Some(PlMenu::Misc),
            Control::PlListOpts => Some(PlMenu::List),
            _ => None,
        }
    }

    /// The items, top to bottom, as the skin draws them.
    #[must_use]
    pub fn items(self) -> &'static [PlItem] {
        use PlItem::*;
        match self {
            PlMenu::Add => &[AddUrl, AddDir, AddFile],
            PlMenu::Remove => &[RemoveMisc, RemoveAll, Crop, RemoveSelected],
            PlMenu::Select => &[InvertSelection, SelectNone, SelectAll],
            PlMenu::Misc => &[SortList, FileInfo, MiscOptions],
            PlMenu::List => &[NewList, SaveList, LoadList],
        }
    }

    /// The columns of `pledit.bmp` holding the items and the bar beside them.
    #[must_use]
    pub fn sprite_columns(self) -> (i32, i32) {
        match self {
            PlMenu::Add => (p::ADD_MENU_X, p::ADD_BAR_X),
            PlMenu::Remove => (p::REM_MENU_X, p::REM_BAR_X),
            PlMenu::Select => (p::SEL_MENU_X, p::SEL_BAR_X),
            PlMenu::Misc => (p::MISC_MENU_X, p::MISC_BAR_X),
            PlMenu::List => (p::LIST_MENU_X, p::LIST_BAR_X),
        }
    }

    /// Where the menu is in a playlist window `w` x `h`: the button it rises from, and the
    /// column of items, whose bottom one covers the button but for the bar at its left.
    #[must_use]
    pub fn rects(self, w: i32, h: i32) -> (Rect, Rect) {
        let by = h - p::BOTTOM_H;
        let button = match self {
            PlMenu::Add => p::ADD.offset(0, by),
            PlMenu::Remove => p::REM.offset(0, by),
            PlMenu::Select => p::SEL.offset(0, by),
            PlMenu::Misc => p::MISC.offset(0, by),
            PlMenu::List => p::LIST_OPTS.offset(w - 150, by),
        };
        let n = i32::try_from(self.items().len()).unwrap_or(0);
        let items_h = n * p::MENU_ITEM_H;
        let items = Rect::new(
            button.x + 3,
            button.bottom() - items_h,
            p::MENU_ITEM_W,
            items_h,
        );
        (button, items)
    }

    /// The item at (`x`, `y`) in a playlist window `w` x `h`.
    #[must_use]
    pub fn item_at(self, w: i32, h: i32, x: i32, y: i32) -> Option<PlItem> {
        let (_, items) = self.rects(w, h);
        if !items.contains(x, y) {
            return None;
        }
        let i = usize::try_from((y - items.y) / p::MENU_ITEM_H).ok()?;
        self.items().get(i).copied()
    }
}

/// Side effects the window layer must perform after input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// The user grabbed this window's title bar: start moving it.
    DragWindow(WindowKind),
    /// The user grabbed the playlist's resize grip: resize it as the pointer moves, through
    /// [`Ui::resize_playlist`].
    ResizePlaylist,
    Close,
    Minimize,
    /// Window visibility, size or scale changed: re-sync the OS windows.
    Relayout,
    /// This window went into or out of windowshade mode: resize it, keeping the windows docked
    /// below it docked.
    Reshade(WindowKind),
    /// Eject: ask for files to play. They replace the playlist (see [`Ui::clear_playlist`]).
    OpenFiles,
    /// Ask for a folder whose files replace the playlist.
    OpenFolder,
    /// `always_on_top` changed: restack the windows.
    AlwaysOnTop,
    /// Show a menu.
    OpenMenu(MenuRequest),
    /// A menu command that needs files or dialogs.
    App(AppCommand),
    /// Tell the audio engine.
    Audio(Transport),
    /// Playback went on into playlist entry `n` by itself, straight after the track before:
    /// what opening it would have done besides playing it (its auto-load preset).
    Advanced(usize),
}

/// Which menu to open, and where.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MenuRequest {
    pub kind: MenuKind,
    /// The window it opens from.
    pub window: WindowKind,
    /// Drop down from this control (in the window's skin pixels) instead of opening at the
    /// pointer.
    pub below: Option<Rect>,
    /// Opened from the keyboard: the first item starts highlighted, and mnemonics show.
    pub keyboard: bool,
}

/// Which file the File info dialog shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InfoOf {
    /// The current track: the clutter bar's I, and Alt+3 outside the playlist.
    Current,
    /// The playlist's first selected entry, else the one at its keyboard cursor: MISC > FILE
    /// INFO, and Alt+3 in the playlist.
    Selection,
}

/// What the system's media controls ask for: the keyboard's media keys, and the buttons of
/// its "now playing" display.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MediaRequest {
    /// Play, or resume. Unlike Winamp's Play button, it does not start a playing track over.
    Play,
    /// Pause; nothing unless playing.
    Pause,
    /// Pause while playing, else play.
    PlayPause,
    Stop,
    Next,
    Previous,
    /// Move this many seconds on, or back when negative.
    SeekBy(f32),
    /// Jump to this many seconds into the track.
    SeekTo(f32),
    /// The volume slider's position, `0..=1`.
    Volume(f32),
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
    /// Back to the default zoom for the screen's height.
    ZoomReset,
    ToggleEq,
    TogglePl,
    /// Windowshade mode on or off for this window.
    ToggleShade(WindowKind),
    Prev,
    Play,
    Pause,
    Stop,
    /// Fade out, then stop.
    FadeOut,
    Next,
    /// Same as the eject button.
    OpenFiles,
    /// Play a folder.
    OpenFolder,
    /// Ask for a URL to play.
    OpenUrl,
    ToggleAlwaysOnTop,
    /// Open the main menu.
    MainMenu,
    /// Open the skins menu.
    SkinsMenu,
    ToggleShuffle,
    ToggleRepeat,
    ToggleTimeMode,
    VolumeUp,
    VolumeDown,
    SeekBack,
    SeekForward,
    /// Open the Jump to file dialog.
    JumpToFile,
    /// Show the File info dialog.
    FileInfo(InfoOf),
    /// Replace the playlist with a playlist file's entries.
    LoadList,
    /// Save the playlist to a file.
    SaveList,
    /// Edit the playlist.
    List(ListCmd),
}

/// Playlist editing, from the playlist's menus and keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListCmd {
    SelectAll,
    SelectNone,
    InvertSelection,
    RemoveSelected,
    /// Remove every entry that is not selected.
    Crop,
    /// Remove every entry.
    Clear,
    /// Remove the entries whose files are gone.
    RemoveDead,
    Sort(SortKey),
    Reverse,
    Randomize,
    /// Move the selected entries one place up.
    MoveUp,
    /// Move the selected entries one place down.
    MoveDown,
    /// Move the keyboard cursor, selecting the entry it lands on, or with `extend` everything
    /// from where the selection started.
    Cursor {
        to: CursorMove,
        extend: bool,
    },
    /// Play the entry under the keyboard cursor.
    PlayCursor,
}

/// Where the playlist's keyboard cursor goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorMove {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// One playlist entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Track {
    /// "Artist - Title" from the tags, or the file name until they have been read. For a
    /// stream, the station's name, or its URL until the station has named itself.
    pub title: String,
    /// Length in whole seconds; `None` until known, and for streams.
    pub seconds: Option<u32>,
    pub location: Location,
}

impl Track {
    /// An entry for `path` whose tags have not been read: titled by its file name.
    #[must_use]
    pub fn from_path(path: PathBuf) -> Track {
        Track {
            title: path.file_stem().map_or_else(
                || path.display().to_string(),
                |s| s.to_string_lossy().into_owned(),
            ),
            seconds: None,
            location: Location::File(path),
        }
    }

    /// An entry for a stream, titled by its URL until the station names itself.
    #[must_use]
    pub fn from_url(url: String) -> Track {
        Track {
            title: url.clone(),
            seconds: None,
            location: Location::Url(url),
        }
    }

    /// An entry for a file or a stream (see [`Track::from_path`] and [`Track::from_url`]).
    #[must_use]
    pub fn new(location: Location) -> Track {
        match location {
            Location::File(path) => Track::from_path(path),
            Location::Url(url) => Track::from_url(url),
        }
    }

    /// A stream still titled by its URL, which its station's name may replace.
    #[must_use]
    pub fn is_untitled_stream(&self) -> bool {
        self.location.url() == Some(self.title.as_str())
    }
}

/// Where each window sits when the three are composed into one canvas (screenshots).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub main: Rect,
    pub eq: Option<Rect>,
    pub pl: Option<Rect>,
    pub size: (i32, i32),
}

/// A fade-out before stopping (Shift+V).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fade {
    Off,
    /// Asked for: it starts at the next frame (see [`Ui::advance_fade`]).
    Requested,
    /// Fading since then.
    Since(Instant),
}

/// Selected playlist entries being dragged to a new place.
#[derive(Clone, Copy, Debug)]
struct RowDrag {
    /// The row under the pointer when the entries last moved. It may lie beyond either end of
    /// the list.
    row: isize,
    /// Pressing on an entry that was already selected keeps the selection, so that it can be
    /// dragged as a whole; unless it is, letting go selects only this entry.
    select_on_release: Option<usize>,
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
    /// A fade-out under way. Playing, pausing or stopping otherwise cancels it.
    fade: Fade,
    /// What the stream playing has told about itself; `None` unless a stream plays.
    stream: Option<StreamDetails>,
    /// How the stream playing is getting on; `Playing` when no stream plays.
    stream_status: StreamStatus,
    pub volume: f32,
    pub balance: f32,
    pub shuffle: bool,
    pub repeat: bool,
    pub eq_open: bool,
    pub pl_open: bool,
    /// Windowshade mode: the window is collapsed to a bar 14 pixels high.
    pub main_shade: bool,
    pub eq_shade: bool,
    pub pl_shade: bool,
    pub eq_on: bool,
    pub eq_auto: bool,
    pub preamp: f32,
    pub bands: [f32; 10],
    pub remaining_mode: bool,
    pub pressed: Option<Control>,
    pub pl_size: (i32, i32),
    pub tracks: Vec<Track>,
    /// The current entry. While the playing track's entry has been removed (see `orphan`), the
    /// entry that followed it, which may be one past the end.
    pub current: usize,
    pub selected: Vec<bool>,
    pub pl_scroll: usize,
    /// The playlist's keyboard cursor: where Up and Down move from and Enter plays.
    pl_cursor: usize,
    /// Where Shift-selection extends from.
    pl_anchor: usize,
    row_drag: Option<RowDrag>,
    /// The track that was playing when its entry was removed from the playlist. It plays on,
    /// as in Winamp, and the entry that followed it plays next.
    orphan: Option<Track>,
    shuffler: Shuffle,
    /// Counts changes to which entries the playlist holds and their order, so that views of it
    /// know when to look again.
    pl_version: u64,
    /// Modifier keys, as the window layer last reported them. Clicks in the playlist use them.
    pub mods: Mods,
    pub kbps: u32,
    pub khz: u32,
    pub stereo: bool,
    pub vis: Vis,
    pub double_size: bool,
    /// The windows stay above other applications' windows.
    pub always_on_top: bool,
    /// The output device chosen in the Options menu; `None` is the system's default.
    pub output: Option<OutputChoice>,
    /// How far the windows are enlarged; double size doubles it again (see [`Ui::scale`]).
    pub zoom: Zoom,
    /// The zoom the screen's height calls for, restored by [`KeyCmd::ZoomReset`]. While
    /// `zoom` equals it, the settings save no zoom and the next start picks one afresh.
    pub default_zoom: Zoom,
    /// Any of the app's windows has focus: title bars draw in their active state.
    pub active: bool,
    /// Last pointer position inside the playlist window (menu highlight).
    pub cursor: (i32, i32),
    /// Temporary marquee text shown while a slider is dragged.
    pub marquee_override: Option<String>,
    pub start: Instant,
    last_click: Option<(Control, Instant)>,
    /// "N. Title" for the current track, rebuilt only when the track changes.
    title_text: String,
    /// The current track's marquee text: `title_text` and the length.
    title_chars: Vec<char>,
    title_scrolls: bool,
    /// The marquee scrolls from here: it starts over from its first character when its text
    /// changes, as Winamp's did for a new song.
    title_from: Instant,
    /// The marquee's text changed since the last [`Ui::tick`].
    title_moved: bool,
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
pub(crate) fn slider_db(v: f32) -> f32 {
    (v - 0.5) * 2.0 * EQ_RANGE_DB
}

/// Where an equalizer slider sits for `db`: the inverse of [`slider_db`], kept on the slider.
pub(crate) fn db_slider(db: f32) -> f32 {
    (db / (2.0 * EQ_RANGE_DB) + 0.5).clamp(0.0, 1.0)
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
            fade: Fade::Off,
            stream: None,
            stream_status: StreamStatus::Playing,
            volume: DEFAULT_VOLUME,
            balance: 0.0,
            shuffle: false,
            repeat: false,
            eq_open: true,
            pl_open: true,
            main_shade: false,
            eq_shade: false,
            pl_shade: false,
            eq_on: true,
            eq_auto: false,
            preamp: 0.5,
            bands: [0.5; 10],
            remaining_mode: false,
            pressed: None,
            pl_size: (p::MIN_W, p::MIN_H),
            tracks: Vec::new(),
            current: 0,
            selected: Vec::new(),
            pl_scroll: 0,
            pl_cursor: 0,
            pl_anchor: 0,
            row_drag: None,
            orphan: None,
            shuffler: Shuffle::new(Rng::new()),
            pl_version: 0,
            mods: Mods::default(),
            kbps: 0,
            khz: 0,
            stereo: false,
            vis: Vis::new(),
            double_size: false,
            always_on_top: false,
            output: None,
            zoom: Zoom::MIN,
            default_zoom: Zoom::MIN,
            active: true,
            cursor: (-1, -1),
            marquee_override: None,
            start: now,
            last_click: None,
            title_text: String::new(),
            title_chars: Vec::new(),
            title_scrolls: false,
            title_from: now,
            title_moved: false,
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
                location: Location::File(PathBuf::from(format!("demo/{:02}.mp3", i + 1))),
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

    /// Size of a window's canvas in skin pixels. A window in windowshade mode keeps its width.
    #[must_use]
    pub fn window_size(&self, kind: WindowKind) -> (i32, i32) {
        self.window_size_shaded(kind, self.is_shaded(kind))
    }

    /// The size window `kind` has in windowshade mode, or out of it.
    #[must_use]
    pub fn window_size_shaded(&self, kind: WindowKind, shaded: bool) -> (i32, i32) {
        match (kind, shaded) {
            (WindowKind::Main, false) => (m::W, m::H),
            (WindowKind::Main, true) => (m::W, ms::H),
            (WindowKind::Eq, false) => (e::W, e::H),
            (WindowKind::Eq, true) => (e::W, es::H),
            (WindowKind::Playlist, false) => self.pl_size,
            (WindowKind::Playlist, true) => (self.pl_size.0, ps::H),
        }
    }

    /// Whether the window is in windowshade mode, collapsed to a bar.
    #[must_use]
    pub fn is_shaded(&self, kind: WindowKind) -> bool {
        match kind {
            WindowKind::Main => self.main_shade,
            WindowKind::Eq => self.eq_shade,
            WindowKind::Playlist => self.pl_shade,
        }
    }

    /// Collapse a window to its windowshade bar, or open it out again.
    fn toggle_shade(&mut self, kind: WindowKind, actions: &mut Vec<Action>) {
        let shade = match kind {
            WindowKind::Main => &mut self.main_shade,
            WindowKind::Eq => &mut self.eq_shade,
            WindowKind::Playlist => &mut self.pl_shade,
        };
        *shade = !*shade;
        actions.push(Action::Reshade(kind));
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
        let mut y = 0;
        let mut place = |kind| {
            let (w, h) = self.window_size(kind);
            let r = Rect::new(0, y, w, h);
            y += h;
            r
        };
        let main = place(WindowKind::Main);
        let eq = self.eq_open.then(|| place(WindowKind::Eq));
        let pl = self.pl_open.then(|| place(WindowKind::Playlist));
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

    /// The current track, if the playlist has any: the one playing, even if its entry has been
    /// removed from the playlist since.
    #[must_use]
    pub fn track(&self) -> Option<&Track> {
        self.orphan
            .as_ref()
            .or_else(|| self.tracks.get(self.current))
    }

    /// The entry the playlist shows as current. `None` while the track playing is no longer in
    /// the playlist, or the playlist is empty.
    #[must_use]
    pub fn current_row(&self) -> Option<usize> {
        (self.orphan.is_none() && self.current < self.tracks.len()).then_some(self.current)
    }

    /// The track the File info dialog shows: the current one, or the playlist's first selected
    /// entry, else the one at its keyboard cursor. `None` with an empty playlist.
    #[must_use]
    pub fn info_track(&self, of: InfoOf) -> Option<&Track> {
        match of {
            InfoOf::Current => self.track(),
            InfoOf::Selection => {
                let first = self.selected.iter().position(|s| *s);
                let i = first.unwrap_or(self.pl_cursor);
                self.tracks.get(i).or_else(|| self.track())
            }
        }
    }

    /// Whether the current track is an internet stream.
    #[must_use]
    pub fn is_stream(&self) -> bool {
        self.track().is_some_and(|t| t.location.is_url())
    }

    /// Whether the "working" light is lit: a stream is connecting or buffering.
    #[must_use]
    pub fn is_working(&self) -> bool {
        self.state == PlayState::Playing && self.stream_status != StreamStatus::Playing
    }

    /// Whether the position bar shows its thumb: not while stopped, nor for a stream.
    #[must_use]
    pub fn shows_position(&self) -> bool {
        self.state != PlayState::Stopped && !self.is_stream()
    }

    /// The song the stream playing names, if it names one.
    #[must_use]
    pub fn stream_song(&self) -> Option<&str> {
        self.stream.as_ref()?.song.as_deref()
    }

    /// What the File info dialog shows about `location`, if it is a stream: what it has told
    /// while playing, if it is the stream playing, else nothing yet.
    #[must_use]
    pub fn stream_details(&self, location: &Location) -> Option<StreamDetails> {
        location.url()?;
        let playing = self.state != PlayState::Stopped
            && self.track().is_some_and(|t| &t.location == location);
        Some(match &self.stream {
            Some(details) if playing => details.clone(),
            _ => StreamDetails::default(),
        })
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
        self.begin();
        actions.push(Action::Audio(Transport::Open(self.current)));
    }

    /// What the current track starting changes in the interface.
    fn begin(&mut self) {
        self.state = PlayState::Playing;
        self.elapsed = 0.0;
        self.seek_to = None;
        self.fade = Fade::Off;
        self.shuffler.mark(self.current);
        let stream = self.is_stream();
        self.stream = stream.then(StreamDetails::default);
        self.stream_status = if stream {
            StreamStatus::Connecting
        } else {
            StreamStatus::Playing
        };
        self.refresh_title();
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
        self.fade = Fade::Off;
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
        self.fade = Fade::Off;
        self.vis.reset();
        let was_stream = self.stream.take().is_some();
        self.stream_status = StreamStatus::Playing;
        if self.orphan.take().is_some() {
            // Back to an entry of the playlist: the one that followed, or the last.
            self.current = self.current.min(self.tracks.len().saturating_sub(1));
            self.refresh_title();
        } else if was_stream {
            // The song it named, and its status, leave the title.
            self.refresh_title();
        }
    }

    /// Fade the music out, then stop (Shift+V, or Shift-click on Stop). Paused, it stops at
    /// once; asked again while fading, nothing changes.
    pub fn fade_out(&mut self, actions: &mut Vec<Action>) {
        match self.state {
            PlayState::Playing if self.fade == Fade::Off => self.fade = Fade::Requested,
            PlayState::Playing | PlayState::Stopped => {}
            PlayState::Paused => self.stop(actions),
        }
    }

    /// Whether a fade-out is under way.
    #[must_use]
    pub fn is_fading(&self) -> bool {
        self.fade != Fade::Off
    }

    /// Move a fade-out on to `now`, once a frame: it starts at the first frame after it was
    /// asked for, and stops playback once it has run for [`FADE_OUT`].
    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn advance_fade(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        match self.fade {
            Fade::Requested => self.fade = Fade::Since(now),
            Fade::Since(t) if now.saturating_duration_since(t) >= FADE_OUT => {
                self.stop(&mut actions);
            }
            Fade::Off | Fade::Since(_) => {}
        }
        actions
    }

    /// The volume to play at, at `now`: the slider's, lowered as a fade-out goes on, the way
    /// sliding the volume down evenly would. The slider itself stays where it is. Stopped, it
    /// is nothing: the device may play a last block before a stop reaches it, which must not
    /// come back at full volume after a fade.
    #[must_use]
    pub fn output_volume(&self, now: Instant) -> f32 {
        match self.fade {
            _ if self.state == PlayState::Stopped => 0.0,
            Fade::Since(t) => {
                let secs = now.saturating_duration_since(t).as_secs_f32();
                self.volume * (1.0 - secs / FADE_OUT.as_secs_f32()).clamp(0.0, 1.0)
            }
            Fade::Off | Fade::Requested => self.volume,
        }
    }

    /// Make playlist entry `idx` current without starting it, scrolling the playlist to it.
    pub fn select(&mut self, idx: usize) {
        self.orphan = None;
        self.stream = None;
        self.stream_status = StreamStatus::Playing;
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

    /// Select, scroll to and play entry `idx`, as a double-click or Enter does.
    pub fn play_entry(&mut self, idx: usize, actions: &mut Vec<Action>) {
        if idx >= self.tracks.len() {
            return;
        }
        self.select_only(idx);
        self.pl_cursor = idx;
        self.pl_anchor = idx;
        self.play_track(idx, actions);
    }

    /// The next entry, or with shuffle on, one not played yet. Past the end of the playlist
    /// this wraps around, even without repeat, as Winamp's Next button did.
    pub fn next(&mut self, actions: &mut Vec<Action>) {
        if let Some(i) = self.following(true) {
            self.jump(i, actions);
        }
    }

    /// The previous entry, or with shuffle on, the one that played before.
    pub fn prev(&mut self, actions: &mut Vec<Action>) {
        let len = self.tracks.len();
        if len == 0 {
            return;
        }
        if self.shuffle
            && let Some(i) = self.shuffler.back().filter(|&i| i < len)
        {
            self.jump(i, actions);
            return;
        }
        // With the playing entry removed, `current` is the entry after it.
        self.jump((self.current.min(len) + len - 1) % len, actions);
    }

    /// The file the end of the current track moves on to, for the engine to open early and
    /// play straight after it, without a gap. `None` when playback will stop there, while
    /// stopped or fading out, and when this track or the next is an internet stream.
    pub fn upcoming(&mut self) -> Option<&Path> {
        if self.state == PlayState::Stopped || self.is_fading() || self.is_stream() {
            return None;
        }
        let i = self.peek_following(self.repeat)?;
        self.tracks.get(i)?.location.path()
    }

    /// The engine went on, without a gap, into `path`, which [`Ui::upcoming`] named: make its
    /// entry current, as the end of a track does. A pause or a fade-out begun just before
    /// carries on: the engine had queued the next track already.
    fn advanced(&mut self, path: &Path, actions: &mut Vec<Action>) {
        let (state, fade) = (self.state, self.fade);
        let is_path = |t: &Track| t.location.path() == Some(path);
        let expected = self
            .peek_following(self.repeat)
            .filter(|&i| self.tracks.get(i).is_some_and(is_path));
        // The playlist may have changed in the moment since the file was opened.
        let row = expected.or_else(|| self.tracks.iter().position(is_path));
        let _ = self.following(self.repeat);
        match row {
            Some(row) => {
                self.select(row);
                actions.push(Action::Advanced(row));
            }
            None => {
                // Its entry was removed just as it was opened: it plays on without one, like
                // a playing track whose entry is removed, and the entry after the last one
                // plays next.
                let next = if self.orphan.is_some() {
                    self.current
                } else {
                    self.current + 1
                };
                self.current = next.min(self.tracks.len());
                self.orphan = Some(Track::from_path(path.to_path_buf()));
            }
        }
        self.begin();
        if state == PlayState::Paused {
            self.state = state;
        }
        self.fade = fade;
    }

    /// After a track: the next one, or stop at the end of the playlist (or of a shuffled
    /// round) unless repeating.
    fn advance(&mut self, actions: &mut Vec<Action>) {
        match self.following(self.repeat) {
            Some(i) => self.jump(i, actions),
            None => self.stop(actions),
        }
    }

    /// The entry after the current one: the following entry, or with shuffle on, one that has
    /// not played in this round. At the end it starts over if `wrap` is set.
    fn following(&mut self, wrap: bool) -> Option<usize> {
        self.next_entry(wrap, true)
    }

    /// What [`Ui::following`] will answer, without moving shuffle on.
    fn peek_following(&mut self, wrap: bool) -> Option<usize> {
        self.next_entry(wrap, false)
    }

    fn next_entry(&mut self, wrap: bool, take: bool) -> Option<usize> {
        let len = self.tracks.len();
        if len == 0 {
            return None;
        }
        if self.shuffle {
            let current = self.current_row();
            return if take {
                self.shuffler.next(len, current, wrap)
            } else {
                self.shuffler.peek(len, current, wrap)
            };
        }
        // While the playing track's entry is removed, the entry after it is `current`.
        let next = if self.orphan.is_some() {
            self.current
        } else {
            self.current + 1
        };
        if next < len {
            Some(next)
        } else {
            wrap.then_some(0)
        }
    }

    /// Turn shuffle on or off. Turning it on starts a new round with the current entry.
    fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.shuffler.reset();
            if let Some(c) = self.current_row() {
                self.shuffler.mark(c);
            }
        }
    }

    fn change_volume(&mut self, steps: i32) {
        // A handful of steps at a time, so the cast is exact.
        self.volume = (self.volume + VOLUME_STEP * steps as f32).clamp(0.0, 1.0);
    }

    /// Jump to `secs` into the current track, within its length when that is known. Streams
    /// cannot seek.
    fn seek(&mut self, secs: f32, actions: &mut Vec<Action>) {
        if self.state == PlayState::Stopped || self.is_stream() {
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
        self.state != PlayState::Stopped && self.track_len() > 0.0 && !self.is_stream()
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
                if let Some(stream) = self.stream.as_mut() {
                    stream.info = Some(info.clone());
                }
                // A stream's tags, if it has any, would name whatever played as it connected;
                // its entry keeps the station's name.
                if let Some(path) = self
                    .track()
                    .and_then(|t| t.location.path())
                    .map(Path::to_path_buf)
                {
                    self.set_track_info(self.current, &path, &info);
                }
            }
            // A track that ends while fading out stops there, rather than starting the next.
            Event::Finished if self.is_fading() => self.stop(actions),
            Event::Finished => self.advance(actions),
            Event::Failed(why) => {
                if let Some(t) = self.track() {
                    log::warn!("cannot play {}: {why}", t.location);
                }
                self.failures += 1;
                if self.failures >= self.tracks.len() {
                    self.stop(actions);
                } else {
                    self.advance(actions);
                }
            }
            Event::Station(station) => self.station(*station),
            Event::StreamTitle(title) => {
                if let Some(stream) = self.stream.as_mut() {
                    stream.song = (!title.is_empty()).then_some(title);
                    self.refresh_title();
                }
            }
            Event::Status(status) => {
                if self.stream.is_some() && status != self.stream_status {
                    self.stream_status = status;
                    self.refresh_title();
                }
            }
            Event::Advanced(path) => self.advanced(&path, actions),
        }
    }

    /// The stream playing has said what station it is. An entry still titled by its URL takes
    /// the station's name; one titled already, by a playlist file or by hand, keeps its title.
    fn station(&mut self, station: Station) {
        if self.stream.is_none() {
            return;
        }
        if let (Some(name), Some(row)) = (&station.name, self.current_row())
            && let Some(track) = self.tracks.get_mut(row).filter(|t| t.is_untitled_stream())
        {
            track.title.clone_from(name);
        }
        if let Some(stream) = self.stream.as_mut() {
            stream.station = Some(station);
        }
        self.refresh_title();
    }

    /// Append files to the playlist, titled by file name until their tags are read. Returns
    /// the indices of the new entries.
    pub fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> Range<usize> {
        self.add_tracks(paths.into_iter().map(Track::from_path))
    }

    /// Append entries whose titles and lengths may already be known, such as those of a saved
    /// playlist. Returns the indices of the new entries.
    pub fn add_tracks(&mut self, tracks: impl IntoIterator<Item = Track>) -> Range<usize> {
        let start = self.tracks.len();
        self.tracks.extend(tracks);
        self.selected.resize(self.tracks.len(), false);
        if start == 0 && !self.tracks.is_empty() {
            self.refresh_title();
        }
        self.pl_version += 1;
        start..self.tracks.len()
    }

    /// Empty the playlist, stopping whatever is playing. Winamp's "Open file(s)" (eject) did
    /// this before adding the files chosen.
    pub fn clear_playlist(&mut self, actions: &mut Vec<Action>) {
        self.stop(actions);
        self.tracks.clear();
        self.selected.clear();
        self.current = 0;
        self.pl_scroll = 0;
        self.pl_cursor = 0;
        self.pl_anchor = 0;
        self.row_drag = None;
        self.failures = 0;
        self.shuffler.reset();
        self.pl_version += 1;
        self.refresh_title();
    }

    /// A number that changes whenever entries are added, removed or moved: while it stays the
    /// same, an index still names the same entry.
    #[must_use]
    pub fn playlist_version(&self) -> u64 {
        self.pl_version
    }

    /// Rearrange the playlist: entry `i` becomes the one that was at `order[i]`, which must
    /// hold every index once. The selection, the current entry, the keyboard cursor and
    /// shuffle's memory go with their entries.
    fn reorder(&mut self, order: &[usize]) {
        let len = self.tracks.len();
        if order.len() != len {
            debug_assert!(false, "BUG: an order of {} for {len} entries", order.len());
            return;
        }
        let new_index = playlist::new_indices(order);
        let mut moved: Vec<(usize, Track)> = new_index
            .iter()
            .copied()
            .zip(std::mem::take(&mut self.tracks))
            .collect();
        moved.sort_unstable_by_key(|(at, _)| *at);
        self.tracks = moved.into_iter().map(|(_, t)| t).collect();
        let selected: Vec<bool> = order
            .iter()
            .map(|&old| self.selected.get(old).copied().unwrap_or(false))
            .collect();
        self.selected = selected;
        // An index past the end (the entry after a removed one that was last) stays there.
        let to = |i: usize| new_index.get(i).copied().unwrap_or(i);
        self.current = to(self.current);
        self.pl_cursor = to(self.pl_cursor);
        self.pl_anchor = to(self.pl_anchor);
        self.shuffler.reorder(&new_index);
        self.pl_version += 1;
        // The marquee numbers the current entry.
        self.refresh_title();
    }

    /// Take the entries marked in `remove` out of the playlist. The track playing plays on
    /// even if its entry goes, and then the entry that followed it plays next.
    fn remove_rows(&mut self, remove: &[bool]) {
        let len = self.tracks.len();
        if remove.len() != len || !remove.contains(&true) {
            return;
        }
        // Where each kept entry goes.
        let mut kept = 0;
        let kept_at: Vec<Option<usize>> = remove
            .iter()
            .map(|&gone| {
                if gone {
                    None
                } else {
                    kept += 1;
                    Some(kept - 1)
                }
            })
            .collect();
        // Where an index lands: on its own entry, or the next one kept after it.
        let after = |i: usize| {
            kept_at
                .get(i..)
                .into_iter()
                .flatten()
                .flatten()
                .next()
                .copied()
                .unwrap_or(kept)
        };
        if self.orphan.is_none()
            && self.state != PlayState::Stopped
            && remove.get(self.current) == Some(&true)
        {
            self.orphan = self.tracks.get(self.current).cloned();
        }
        self.current = after(self.current);
        let last = kept.saturating_sub(1);
        if self.orphan.is_none() {
            self.current = self.current.min(last);
        }
        self.pl_cursor = after(self.pl_cursor).min(last);
        self.pl_anchor = after(self.pl_anchor).min(last);
        self.shuffler.retain(&kept_at);
        let mut gone = remove.iter();
        self.tracks
            .retain(|_| !gone.next().copied().unwrap_or(false));
        let mut gone = remove.iter();
        self.selected
            .retain(|_| !gone.next().copied().unwrap_or(false));
        self.row_drag = None;
        self.pl_scroll = self.pl_scroll.min(self.max_scroll());
        self.pl_version += 1;
        self.refresh_title();
    }

    /// Remove the entries whose file `exists` says is gone (Winamp's "Remove all dead files").
    /// Streams stay: whether one is there can only be found out by playing it.
    pub fn remove_missing(&mut self, exists: impl Fn(&Path) -> bool) {
        let remove: Vec<bool> = self
            .tracks
            .iter()
            .map(|t| t.location.path().is_some_and(|p| !exists(p)))
            .collect();
        self.remove_rows(&remove);
    }

    /// Move the selected entries `delta` places down (up if negative), as far as they can go.
    /// Returns how far they moved.
    fn move_selection(&mut self, delta: isize) -> isize {
        match playlist::move_order(&self.selected, delta) {
            Some((order, moved)) => {
                self.reorder(&order);
                moved
            }
            None => 0,
        }
    }

    fn select_only(&mut self, idx: usize) {
        self.selected.fill(false);
        if let Some(s) = self.selected.get_mut(idx) {
            *s = true;
        }
    }

    /// Select entries `a` to `b`, in either order, besides those already selected.
    fn select_range(&mut self, a: usize, b: usize) {
        let (from, to) = (a.min(b), a.max(b));
        for s in self.selected.iter_mut().take(to + 1).skip(from) {
            *s = true;
        }
    }

    fn move_cursor(&mut self, to: CursorMove, extend: bool) {
        let Some(last) = self.tracks.len().checked_sub(1) else {
            return;
        };
        let page = self.pl_visible_rows().max(1);
        let from = self.pl_cursor.min(last);
        let at = match to {
            CursorMove::Up => from.saturating_sub(1),
            CursorMove::Down => (from + 1).min(last),
            CursorMove::PageUp => from.saturating_sub(page),
            CursorMove::PageDown => (from + page).min(last),
            CursorMove::Home => 0,
            CursorMove::End => last,
        };
        self.pl_cursor = at;
        if extend {
            self.selected.fill(false);
            self.select_range(self.pl_anchor.min(last), at);
        } else {
            self.pl_anchor = at;
            self.select_only(at);
        }
        self.ensure_visible(at);
    }

    /// Carry out a playlist editing command.
    fn list_command(&mut self, cmd: ListCmd, actions: &mut Vec<Action>) {
        let len = self.tracks.len();
        match cmd {
            ListCmd::SelectAll => self.selected.fill(true),
            ListCmd::SelectNone => self.selected.fill(false),
            ListCmd::InvertSelection => self.selected.iter_mut().for_each(|s| *s = !*s),
            ListCmd::RemoveSelected => {
                let remove = self.selected.clone();
                self.remove_rows(&remove);
            }
            ListCmd::Crop => {
                // With nothing selected there is nothing to keep, and that is more likely a
                // slip than a wish to empty the playlist.
                if self.selected.contains(&true) {
                    let remove: Vec<bool> = self.selected.iter().map(|s| !s).collect();
                    self.remove_rows(&remove);
                }
            }
            ListCmd::Clear => self.remove_rows(&vec![true; len]),
            ListCmd::RemoveDead => self.remove_missing(Path::exists),
            ListCmd::Sort(key) => {
                let order = playlist::sort_order(&self.tracks, key);
                self.reorder(&order);
            }
            ListCmd::Reverse => self.reorder(&playlist::reverse_order(len)),
            ListCmd::Randomize => {
                let order = playlist::random_order(len, self.shuffler.rng());
                self.reorder(&order);
            }
            ListCmd::MoveUp | ListCmd::MoveDown => {
                let delta = if cmd == ListCmd::MoveUp { -1 } else { 1 };
                if self.move_selection(delta) != 0 {
                    self.ensure_visible(self.pl_cursor);
                }
            }
            ListCmd::Cursor { to, extend } => self.move_cursor(to, extend),
            ListCmd::PlayCursor => self.play_entry(self.pl_cursor, actions),
        }
    }

    /// Fill in a playlist entry's title and length once its file has been read. The entry is
    /// `idx` if that still plays `path`; if entries have moved since, it is found by its file.
    pub fn set_track_info(&mut self, idx: usize, path: &Path, info: &TrackInfo) {
        let plays = |t: &Track| t.location.path() == Some(path);
        let idx = if self.tracks.get(idx).is_some_and(plays) {
            idx
        } else {
            match self.tracks.iter().position(plays) {
                Some(i) => i,
                None => return,
            }
        };
        let Some(track) = self.tracks.get_mut(idx) else {
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

    /// Set the equalizer's sliders to `preset`. Whether it is on stays as it was.
    pub fn apply_preset(&mut self, preset: &Preset) {
        self.preamp = db_slider(preset.preamp);
        self.bands = preset.bands.map(db_slider);
    }

    /// The equalizer's sliders as a preset called `name`.
    #[must_use]
    pub fn eq_preset(&self, name: &str) -> Preset {
        Preset {
            name: name.to_owned(),
            preamp: slider_db(self.preamp),
            bands: self.bands.map(slider_db),
        }
    }

    /// Advance the visualiser. Call once per frame before drawing.
    pub fn tick(&mut self, now: Instant) {
        self.vis.update(now, self.state == PlayState::Playing);
        if std::mem::take(&mut self.title_moved) {
            self.title_from = now;
        }
    }

    // ----- text ---------------------------------------------------------------------------

    /// Rebuild the cached titles: `"N. Title"`, and the marquee's `"N. Title (m:ss)"`, padded
    /// when it scrolls. A stream's title is the song it names, else the entry's (the station),
    /// and while it connects or buffers the marquee says so first: `"[Buffering: 45%] N. ..."`.
    fn refresh_title(&mut self) {
        if self.orphan.is_some() {
            // The track playing has left the playlist; its title stays as it was.
            return;
        }
        self.title_text.clear();
        self.title_scrolls = false;
        // Without an orphan, the current track is the current entry.
        let Some(t) = self.tracks.get(self.current) else {
            self.title_moved |= !self.title_chars.is_empty();
            self.title_chars.clear();
            return;
        };
        let name = self
            .stream
            .as_ref()
            .and_then(|s| s.song.as_deref())
            .unwrap_or(&t.title);
        let mut s = String::with_capacity(64);
        match self.stream_status {
            StreamStatus::Playing => {}
            StreamStatus::Connecting => s.push_str("[Connecting] "),
            StreamStatus::Buffering(percent) => {
                push_fmt(&mut s, format_args!("[Buffering: {percent}%] "));
            }
            StreamStatus::Reconnecting => s.push_str("[Reconnecting] "),
        }
        let status = s.len();
        push_fmt(&mut s, format_args!("{}. {name}", self.current + 1));
        self.title_text.clear();
        self.title_text.push_str(&s[status..]);
        if let Some(secs) = t.seconds {
            s.push_str(" (");
            write_time(&mut s, secs);
            s.push(')');
        }
        self.title_scrolls = s.chars().count() > m::MARQUEE_CHARS;
        if self.title_scrolls {
            s.push_str(MARQUEE_SEPARATOR);
        }
        if !self.title_chars.iter().copied().eq(s.chars()) {
            self.title_chars.clear();
            self.title_chars.extend(s.chars());
            self.title_moved = true;
        }
    }

    /// The current track's number in the playlist and its title ("3. Artist - Title"), as the
    /// playlist's windowshade bar shows them. Empty with an empty playlist.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title_text
    }

    /// Write the main window's title into `buf`, as Winamp's read in the taskbar: the current
    /// entry's number and title, then the app's name, and "\[Paused\]" or "\[Stopped\]" unless it
    /// plays ("3. Artist - Title - NeoAmp \[Paused\]"). With an empty playlist, just the name.
    pub fn write_window_title(&self, buf: &mut String) {
        if self.title_text.is_empty() {
            buf.push_str(APP_NAME);
            return;
        }
        push_fmt(buf, format_args!("{} - {APP_NAME}", self.title_text));
        match self.state {
            PlayState::Playing => {}
            PlayState::Paused => buf.push_str(" [Paused]"),
            PlayState::Stopped => buf.push_str(" [Stopped]"),
        }
    }

    /// The characters the marquee shows for the current track at `now`, without allocating.
    pub fn title_scroll_chars(&self, now: Instant) -> impl Iterator<Item = char> + '_ {
        let len = self.title_chars.len().max(1);
        let step = if self.title_scrolls {
            // Reduce modulo the length first so the narrowing cast cannot truncate.
            let steps =
                now.saturating_duration_since(self.title_from).as_millis() / MARQUEE_STEP_MS;
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
        c: &mut Canvas,
        skin: &Skin,
        font: Option<&TtfFont>,
        now: Instant,
    ) {
        let (w, h) = self.window_size(kind);
        c.clear([0, 0, 0, 0]);
        self.draw_at(kind, c, skin, font, Rect::new(0, 0, w, h), now);
    }

    /// Advance one frame and draw every open window stacked into a single canvas.
    pub fn draw_stacked(
        &mut self,
        c: &mut Canvas,
        skin: &Skin,
        font: Option<&TtfFont>,
        now: Instant,
    ) {
        self.tick(now);
        let lay = self.layout();
        c.clear([0, 0, 0, 0]);
        self.draw_at(WindowKind::Main, c, skin, font, lay.main, now);
        if let Some(r) = lay.eq {
            self.draw_at(WindowKind::Eq, c, skin, font, r, now);
        }
        if let Some(r) = lay.pl {
            self.draw_at(WindowKind::Playlist, c, skin, font, r, now);
        }
    }

    /// Draw window `kind`, or its windowshade bar, into `r`, cut to the skin's shape for it.
    fn draw_at(
        &self,
        kind: WindowKind,
        c: &mut Canvas,
        skin: &Skin,
        font: Option<&TtfFont>,
        r: Rect,
        now: Instant,
    ) {
        let shape = match (kind, self.is_shaded(kind)) {
            (WindowKind::Main, false) => {
                main_window::draw(self, c, skin, r, now);
                skin.regions.normal.as_ref()
            }
            (WindowKind::Main, true) => {
                main_window::draw_shade(self, c, skin, r, now);
                skin.regions.windowshade.as_ref()
            }
            (WindowKind::Eq, false) => {
                eq_window::draw(self, c, skin, r, now);
                skin.regions.equalizer.as_ref()
            }
            (WindowKind::Eq, true) => {
                eq_window::draw_shade(self, c, skin, r);
                skin.regions.equalizer_ws.as_ref()
            }
            // region.txt has no section for the playlist.
            (WindowKind::Playlist, false) => {
                playlist_window::draw(self, c, skin, font, r, now);
                None
            }
            (WindowKind::Playlist, true) => {
                playlist_window::draw_shade(self, c, skin, r);
                None
            }
        };
        if let Some(polys) = shape {
            cut_to_shape(c, r, polys);
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
        match (kind, self.is_shaded(kind)) {
            (WindowKind::Main, false) => Self::hit_main(x, y),
            (WindowKind::Main, true) => Some(Self::hit_main_shade(x, y)),
            (WindowKind::Eq, false) => Self::hit_eq(x, y),
            (WindowKind::Eq, true) => Some(Self::hit_eq_shade(x, y)),
            (WindowKind::Playlist, false) => self.hit_pl(x, y, w, h),
            (WindowKind::Playlist, true) => Some(Self::hit_pl_shade(x, y, w)),
        }
    }

    /// The skin's pointer for window-local point (`x`, `y`) of `kind`, as Winamp chose them:
    /// one per button of the title bars, the sliders and the playlist's grip and scroll bar,
    /// and one for the rest of each window. In the windowshade bars the buttons keep theirs,
    /// except the main window's options button, which has its own (Webamp and Winamp's
    /// Skinner's Atlas are the sources).
    #[must_use]
    pub fn cursor_at(&self, kind: WindowKind, x: i32, y: i32) -> SkinCursor {
        use Control::*;
        let hit = self.hit(kind, x, y);
        match (kind, self.is_shaded(kind)) {
            (WindowKind::Main, false) => match hit {
                Some(MainClose) => SkinCursor::Close,
                Some(MainMinimize) => SkinCursor::Min,
                Some(MainShade) => SkinCursor::WinBut,
                Some(MainOptions) => SkinCursor::MainMenu,
                Some(MainTitle) => SkinCursor::TitleBar,
                Some(PosBar) => SkinCursor::PosBar,
                Some(Volume | Balance) => SkinCursor::VolBal,
                _ if m::MARQUEE_TARGET.contains(x, y) => SkinCursor::SongName,
                _ => SkinCursor::Normal,
            },
            (WindowKind::Main, true) => match hit {
                Some(MainClose) => SkinCursor::Close,
                Some(MainMinimize) => SkinCursor::Min,
                Some(MainShade) => SkinCursor::WinBut,
                Some(MainOptions) => SkinCursor::MMenu,
                Some(PosBar) => SkinCursor::WsPosBar,
                _ => SkinCursor::WsNormal,
            },
            (WindowKind::Eq, false) => match hit {
                Some(EqClose) => SkinCursor::EqClose,
                Some(EqTitle | EqShade) => SkinCursor::EqTitle,
                Some(EqBand(_) | EqPreamp) => SkinCursor::EqSlid,
                _ => SkinCursor::EqNormal,
            },
            (WindowKind::Eq, true) => match hit {
                Some(EqClose) => SkinCursor::EqClose,
                _ => SkinCursor::EqTitle,
            },
            (WindowKind::Playlist, false) => match hit {
                Some(PlClose) => SkinCursor::PClose,
                Some(PlShade) => SkinCursor::PWinBut,
                Some(PlTitle) => SkinCursor::PTBar,
                Some(PlResize) => SkinCursor::PSize,
                Some(PlScroll) => SkinCursor::PVScroll,
                _ => SkinCursor::PNormal,
            },
            (WindowKind::Playlist, true) => match hit {
                Some(PlClose) => SkinCursor::PClose,
                Some(PlShade) => SkinCursor::PWinBut,
                Some(PlResize) => SkinCursor::PWsSize,
                _ => SkinCursor::PWsNorm,
            },
        }
    }

    /// The main window's windowshade bar. Whatever is not a control there drags the window,
    /// as with the other two bars.
    fn hit_main_shade(x: i32, y: i32) -> Control {
        use Control::*;
        const TABLE: &[(Rect, Control)] = &[
            (m::OPTIONS, MainOptions),
            (m::MINIMIZE, MainMinimize),
            (m::SHADE, MainShade),
            (m::CLOSE, MainClose),
            (ms::VIS, Control::Vis),
            (ms::TIME, TimeDisplay),
            (ms::PREV, Prev),
            (ms::PLAY, Play),
            (ms::PAUSE, Pause),
            (ms::STOP, Stop),
            (ms::NEXT, Next),
            (ms::EJECT, Eject),
            (ms::POSBAR, PosBar),
        ];
        let hit = TABLE.iter().find(|(r, _)| r.contains(x, y));
        hit.map_or(MainTitle, |(_, c)| *c)
    }

    fn hit_eq_shade(x: i32, y: i32) -> Control {
        use Control::*;
        const TABLE: &[(Rect, Control)] = &[
            (e::CLOSE, EqClose),
            (e::SHADE, EqShade),
            (es::VOLUME, EqVolume),
            (es::BALANCE, EqBalance),
        ];
        let hit = TABLE.iter().find(|(r, _)| r.contains(x, y));
        hit.map_or(EqTitle, |(_, c)| *c)
    }

    /// The playlist's windowshade bar, `w` wide: its grip only changes the width.
    fn hit_pl_shade(x: i32, y: i32, w: i32) -> Control {
        use Control::*;
        let button = |from_right: i32| Rect::new(w - from_right, p::TITLE_BTN_Y, 9, 9);
        let table = [
            (button(p::CLOSE_FROM_RIGHT), PlClose),
            (button(p::SHADE_FROM_RIGHT), PlShade),
            (button(ps::RESIZE_FROM_RIGHT), PlResize),
        ];
        let hit = table.iter().find(|(r, _)| r.contains(x, y));
        hit.map_or(PlTitle, |(_, c)| *c)
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

    /// The playlist row at window-local height `y`, counting from the first entry; beyond the
    /// list's ends it carries on past them.
    fn pl_row_at(&self, y: i32) -> isize {
        let list = self.pl_list_rect(self.pl_size.0, self.pl_size.1);
        let row = (y - list.y).div_euclid(p::TRACK_H);
        let scroll = isize::try_from(self.pl_scroll).unwrap_or(isize::MAX);
        scroll.saturating_add(isize::try_from(row).unwrap_or(0))
    }

    /// The playlist menu showing: the one whose button is held.
    #[must_use]
    pub fn pl_menu(&self) -> Option<PlMenu> {
        self.pressed.and_then(PlMenu::of)
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
            Slider::Volume | Slider::EqVolume => {
                self.volume = if slider == Slider::Volume {
                    Self::h_fraction(m::VOLUME, m::SLIDER_THUMB_W, lx)
                } else {
                    Self::h_fraction(es::VOLUME, es::THUMB_W, lx)
                };
                push_fmt(
                    &mut text,
                    format_args!("Volume: {}%", (self.volume * 100.0).round()),
                );
            }
            Slider::Balance | Slider::EqBalance => {
                let v = if slider == Slider::Balance {
                    Self::h_fraction(m::BALANCE, m::SLIDER_THUMB_W, lx)
                } else {
                    Self::h_fraction(es::BALANCE, es::THUMB_W, lx)
                };
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
                let pos = if self.main_shade {
                    Self::h_fraction(ms::POSBAR, ms::POSBAR_THUMB_W, lx)
                } else {
                    Self::h_fraction(m::POSBAR, m::POSBAR_THUMB_W, lx)
                };
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

    /// Resize the playlist to the size on Winamp's 25x29 grid nearest to `size` (skin pixels),
    /// never below the minimum. The window layer works `size` out from the grip's travel, since
    /// the pointer leaves the window as it grows. True if the size changed.
    pub fn resize_playlist(&mut self, size: (i32, i32)) -> bool {
        // Rounds to the nearest step (the steps are odd, so there are no ties).
        let snap = |v: i32, min: i32, step: i32| {
            let steps = v.saturating_sub(min).max(0).saturating_add(step / 2) / step;
            min.saturating_add(steps.saturating_mul(step))
        };
        let new = (
            snap(size.0, p::MIN_W, p::STEP_W),
            snap(size.1, p::MIN_H, p::STEP_H),
        );
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
        let double = matches!(self.last_click,
            Some((last, t)) if last == ctl && now.saturating_duration_since(t).as_millis() < DOUBLE_CLICK_MS);
        if let Some(slider) = ctl.slider() {
            if slider == Slider::PosBar && !self.can_seek() {
                return actions;
            }
            self.pressed = Some(ctl);
            self.drag_slider(slider, x, y);
        } else {
            match ctl {
                // A double-click on a title bar collapses the window to its windowshade bar, or
                // opens it out again. A third click starts afresh.
                MainTitle | EqTitle | PlTitle if double => {
                    self.toggle_shade(kind, &mut actions);
                    self.last_click = None;
                    return actions;
                }
                MainTitle | EqTitle | PlTitle => actions.push(Action::DragWindow(kind)),
                PlScroll => {
                    self.pressed = Some(ctl);
                    self.drag_scroll(y);
                }
                PlResize => actions.push(Action::ResizePlaylist),
                PlTrack(i) => self.click_row(i, y, double, &mut actions),
                // Every other control is a button, the playlist's menu buttons included: show
                // it pressed (or its menu) until release.
                _ => self.pressed = Some(ctl),
            }
        }
        self.last_click = Some((ctl, now));
        actions
    }

    /// A click on playlist entry `i`, at window height `y`. Shift selects from the last entry
    /// clicked, Ctrl adds or removes one entry, and a plain click selects one and may start
    /// dragging the selection. A double-click plays the entry.
    fn click_row(&mut self, i: usize, y: i32, double: bool, actions: &mut Vec<Action>) {
        let Mods { shift, ctrl } = self.mods;
        if double {
            self.play_entry(i, actions);
        } else if shift {
            if !ctrl {
                self.selected.fill(false);
            }
            self.select_range(self.pl_anchor, i);
            self.pl_cursor = i;
        } else if ctrl {
            if let Some(s) = self.selected.get_mut(i) {
                *s = !*s;
            }
            self.pl_cursor = i;
            self.pl_anchor = i;
        } else {
            let was_selected = self.selected.get(i).copied().unwrap_or(false);
            if !was_selected {
                self.select_only(i);
            }
            self.pl_cursor = i;
            self.pl_anchor = i;
            self.pressed = Some(Control::PlTrack(i));
            self.row_drag = Some(RowDrag {
                row: self.pl_row_at(y),
                select_on_release: was_selected.then_some(i),
            });
        }
    }

    /// Move the dragged entries to follow the pointer at window height `y`, scrolling when it
    /// goes past the top or bottom of the list.
    fn drag_rows(&mut self, y: i32) {
        let Some(drag) = self.row_drag else {
            return;
        };
        let row = self.pl_row_at(y);
        let moved = self.move_selection(row.saturating_sub(drag.row));
        if moved == 0 {
            return;
        }
        self.row_drag = Some(RowDrag {
            row: drag.row + moved,
            select_on_release: None,
        });
        let last = self.tracks.len().saturating_sub(1);
        self.ensure_visible(usize::try_from(row).map_or(0, |r| r.min(last)));
    }

    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn mouse_move(&mut self, kind: WindowKind, x: i32, y: i32) -> Vec<Action> {
        self.note_cursor(kind, x, y);
        // A drag only follows pointer motion reported by the window it started in.
        let Some(ctl) = self.pressed.filter(|c| c.window() == kind) else {
            return Vec::new();
        };
        if let Some(slider) = ctl.slider() {
            self.drag_slider(slider, x, y);
        } else if ctl == Control::PlScroll {
            self.drag_scroll(y);
        } else if let Control::PlTrack(_) = ctl {
            self.drag_rows(y);
        }
        Vec::new()
    }

    /// The mouse wheel turned `steps` notches over window `kind`, positive away from the user:
    /// the playlist scrolls a row per step (up for positive), and over the main window the
    /// volume moves a step, as Up and Down do. The playlist's windowshade bar has no list to
    /// scroll.
    pub fn wheel(&mut self, kind: WindowKind, steps: i32) {
        match kind {
            WindowKind::Playlist if self.pl_shade => {}
            WindowKind::Playlist => {
                let rows = usize::try_from(steps.unsigned_abs()).unwrap_or(usize::MAX);
                self.pl_scroll = if steps > 0 {
                    self.pl_scroll.saturating_sub(rows)
                } else {
                    self.pl_scroll.saturating_add(rows).min(self.max_scroll())
                };
            }
            WindowKind::Main => self.change_volume(steps),
            WindowKind::Eq => {}
        }
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
        if let Some(menu) = PlMenu::of(ctl) {
            // Letting go over an item chooses it; anywhere else, nothing.
            let (w, h) = self.pl_size;
            if kind == WindowKind::Playlist
                && let Some(item) = menu.item_at(w, h, x, y)
            {
                self.pl_item(item, &mut actions);
            }
            return actions;
        }
        let still_over = ctl.window() == kind && self.hit(kind, x, y) == Some(ctl);
        match ctl {
            // Letting go of the position bar seeks to where the thumb was left.
            PosBar => {
                if let Some(secs) = self.seek_to.take() {
                    self.seek(secs, &mut actions);
                }
            }
            // A click on an entry that was already selected, which did not become a drag.
            PlTrack(_) => {
                if let Some(i) = self.row_drag.take().and_then(|d| d.select_on_release) {
                    self.select_only(i);
                }
            }
            // Other drags finish silently.
            Volume | Balance | EqPreamp | EqBand(_) | EqVolume | EqBalance | PlScroll
            | PlResize => {}
            // Releasing outside the pressed button cancels it, as in Win32.
            _ if !still_over => {}
            MainClose => actions.push(Action::Close),
            MainMinimize => actions.push(Action::Minimize),
            MainShade => self.toggle_shade(WindowKind::Main, &mut actions),
            EqShade => self.toggle_shade(WindowKind::Eq, &mut actions),
            PlShade => self.toggle_shade(WindowKind::Playlist, &mut actions),
            ClutterD => {
                self.double_size = !self.double_size;
                actions.push(Action::Relayout);
            }
            PlScrollUp => self.pl_scroll = self.pl_scroll.saturating_sub(1),
            PlScrollDown => self.pl_scroll = (self.pl_scroll + 1).min(self.max_scroll()),
            Prev | PlMiniPrev => self.prev(&mut actions),
            Play | PlMiniPlay => self.play(&mut actions),
            Pause | PlMiniPause => self.pause(&mut actions),
            // Shift-click fades out first, as Shift+V does.
            Stop | PlMiniStop if self.mods.shift => self.fade_out(&mut actions),
            Stop | PlMiniStop => self.stop(&mut actions),
            Next | PlMiniNext => self.next(&mut actions),
            Eject | PlMiniEject => actions.push(Action::OpenFiles),
            Shuffle => self.toggle_shuffle(),
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
            MainOptions => actions.push(Self::menu(MenuKind::Main, kind, Some(m::OPTIONS))),
            ClutterO => actions.push(Self::menu(MenuKind::Options, kind, None)),
            ClutterV => actions.push(Self::menu(MenuKind::Vis, kind, None)),
            EqPresets => actions.push(Self::menu(MenuKind::EqPresets, kind, Some(e::PRESETS))),
            ClutterA => self.toggle_always_on_top(&mut actions),
            ClutterI => actions.push(Action::App(AppCommand::FileInfo(InfoOf::Current))),
            // Title bars only drag; the playlist's menu buttons were handled above.
            MainTitle | EqTitle | PlTitle | PlAdd | PlRem | PlSel | PlMisc | PlListOpts => {}
        }
        actions
    }

    /// Carry out an item of a playlist menu.
    fn pl_item(&mut self, item: PlItem, actions: &mut Vec<Action>) {
        use PlItem::*;
        let popup = |kind| Self::menu(kind, WindowKind::Playlist, None);
        match item {
            AddUrl => actions.push(Action::App(AppCommand::AddUrl)),
            AddDir => actions.push(Action::App(AppCommand::AddFolder)),
            AddFile => actions.push(Action::App(AppCommand::AddFiles)),
            RemoveMisc => actions.push(popup(MenuKind::PlaylistRemove)),
            RemoveAll | NewList => self.list_command(ListCmd::Clear, actions),
            Crop => self.list_command(ListCmd::Crop, actions),
            RemoveSelected => self.list_command(ListCmd::RemoveSelected, actions),
            InvertSelection => self.list_command(ListCmd::InvertSelection, actions),
            SelectNone => self.list_command(ListCmd::SelectNone, actions),
            SelectAll => self.list_command(ListCmd::SelectAll, actions),
            SortList => actions.push(popup(MenuKind::PlaylistSort)),
            SaveList => actions.push(Action::App(AppCommand::SaveList)),
            LoadList => actions.push(Action::App(AppCommand::LoadList)),
            FileInfo => actions.push(Action::App(AppCommand::FileInfo(InfoOf::Selection))),
            // Winamp's miscellaneous options are not implemented.
            MiscOptions => {}
        }
    }

    fn menu(kind: MenuKind, window: WindowKind, below: Option<Rect>) -> Action {
        Action::OpenMenu(MenuRequest {
            kind,
            window,
            below,
            keyboard: false,
        })
    }

    fn toggle_always_on_top(&mut self, actions: &mut Vec<Action>) {
        self.always_on_top = !self.always_on_top;
        actions.push(Action::AlwaysOnTop);
    }

    /// The right button was let go at (`x`, `y`) in window `kind`: the visualizer's menu over
    /// the visualizer, the main menu anywhere else.
    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn context_menu(&mut self, kind: WindowKind, x: i32, y: i32) -> Vec<Action> {
        if self.pressed.is_some() {
            // In the middle of a left-button drag.
            return Vec::new();
        }
        let menu = if kind == WindowKind::Main && self.hit(kind, x, y) == Some(Control::Vis) {
            MenuKind::Vis
        } else {
            MenuKind::Main
        };
        vec![Self::menu(menu, kind, None)]
    }

    /// Carry out a menu command.
    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn command(&mut self, cmd: Command) -> Vec<Action> {
        let mut actions = Vec::new();
        match cmd {
            Command::Key(k) => return self.key(k),
            Command::TimeRemaining(on) => self.remaining_mode = on,
            Command::Zoom(zoom) => self.set_zoom(zoom, &mut actions),
            Command::VisMode(mode) => self.vis.mode = mode,
            Command::AnalyzerStyle(style) => self.vis.analyzer.style = style,
            Command::Bands(bands) => self.vis.analyzer.bands = bands,
            Command::TogglePeaks => self.vis.analyzer.peaks = !self.vis.analyzer.peaks,
            Command::Falloff(f) => self.vis.analyzer.falloff = f,
            Command::PeakFalloff(f) => self.vis.analyzer.peak_falloff = f,
            Command::ScopeStyle(style) => self.vis.scope_style = style,
            Command::Preset(i) => {
                if let Some(preset) = eqf::built_in(i) {
                    self.apply_preset(&preset);
                }
            }
            Command::ResetEq => self.apply_preset(&Preset::flat("")),
            Command::Exit => actions.push(Action::Close),
            Command::App(c) => actions.push(Action::App(c)),
        }
        actions
    }

    /// Carry out a request from the system's media controls.
    #[must_use = "the returned actions must be performed by the window layer"]
    pub fn media(&mut self, request: MediaRequest) -> Vec<Action> {
        let mut actions = Vec::new();
        let playing = self.state == PlayState::Playing;
        match request {
            MediaRequest::Play | MediaRequest::PlayPause if !playing => self.play(&mut actions),
            MediaRequest::Pause | MediaRequest::PlayPause if playing => self.pause(&mut actions),
            MediaRequest::Play | MediaRequest::Pause | MediaRequest::PlayPause => {}
            MediaRequest::Stop => self.stop(&mut actions),
            MediaRequest::Next => self.next(&mut actions),
            MediaRequest::Previous => self.prev(&mut actions),
            MediaRequest::SeekBy(secs) => self.seek(self.elapsed + secs, &mut actions),
            MediaRequest::SeekTo(secs) => self.seek(secs, &mut actions),
            MediaRequest::Volume(v) if v.is_finite() => self.volume = v.clamp(0.0, 1.0),
            MediaRequest::Volume(_) => {}
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
            KeyCmd::ToggleShade(kind) => self.toggle_shade(kind, &mut actions),
            KeyCmd::Prev => self.prev(&mut actions),
            KeyCmd::Play => self.play(&mut actions),
            KeyCmd::Pause => self.pause(&mut actions),
            KeyCmd::Stop => self.stop(&mut actions),
            KeyCmd::FadeOut => self.fade_out(&mut actions),
            KeyCmd::Next => self.next(&mut actions),
            KeyCmd::OpenFiles => actions.push(Action::OpenFiles),
            KeyCmd::OpenFolder => actions.push(Action::OpenFolder),
            KeyCmd::OpenUrl => actions.push(Action::App(AppCommand::PlayUrl)),
            KeyCmd::ToggleAlwaysOnTop => self.toggle_always_on_top(&mut actions),
            KeyCmd::MainMenu | KeyCmd::SkinsMenu => {
                let kind = if cmd == KeyCmd::MainMenu {
                    MenuKind::Main
                } else {
                    MenuKind::Skins
                };
                // Where the options button opens it.
                actions.push(Action::OpenMenu(MenuRequest {
                    kind,
                    window: WindowKind::Main,
                    below: Some(m::OPTIONS),
                    keyboard: true,
                }));
            }
            KeyCmd::ToggleShuffle => self.toggle_shuffle(),
            KeyCmd::ToggleRepeat => self.repeat = !self.repeat,
            KeyCmd::ToggleTimeMode => self.remaining_mode = !self.remaining_mode,
            KeyCmd::VolumeUp => self.change_volume(1),
            KeyCmd::VolumeDown => self.change_volume(-1),
            KeyCmd::SeekBack => self.seek(self.elapsed - 5.0, &mut actions),
            KeyCmd::SeekForward => self.seek(self.elapsed + 5.0, &mut actions),
            KeyCmd::JumpToFile => actions.push(Action::App(AppCommand::JumpToFile)),
            KeyCmd::FileInfo(of) => actions.push(Action::App(AppCommand::FileInfo(of))),
            KeyCmd::LoadList => actions.push(Action::App(AppCommand::LoadList)),
            KeyCmd::SaveList => actions.push(Action::App(AppCommand::SaveList)),
            KeyCmd::List(cmd) => self.list_command(cmd, &mut actions),
        }
        actions
    }
}

/// Make every pixel of `r` outside `mask` fully transparent.
/// Clear the pixels of window rectangle `r` outside the shape `polys` gives it, at the canvas's
/// density, so a denser canvas has a smoother outline.
fn cut_to_shape(c: &mut Canvas, r: Rect, polys: &Polygons) {
    let k = c.density().get();
    let area = r.scaled(k);
    let w = usize::try_from(area.w).unwrap_or(0);
    if w == 0 {
        return;
    }
    let scale = k as f32;
    let mut row = vec![false; w];
    let mut xs = Vec::new();
    let image = c.image_mut();
    for y in 0..area.h {
        row.fill(false);
        cover_row(polys, (y as f32 + 0.5) / scale, scale, &mut row, &mut xs);
        for (x, covered) in row.iter().enumerate() {
            if !covered {
                image.put(area.x + x as i32, area.y + y, [0, 0, 0, 0]);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::canvas::Density;
    use crate::vis::{Bands, VisMode};
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
    fn shift_v_fades_out_then_stops() {
        let mut ui = Ui::demo();
        ui.volume = 0.8;
        let t0 = Instant::now();
        assert_eq!(ui.key(KeyCmd::FadeOut), vec![], "nothing stops yet");
        assert!(ui.is_fading());
        assert_eq!(ui.output_volume(t0), 0.8, "it starts at the next frame");
        assert_eq!(ui.advance_fade(t0), vec![]);
        let half = t0 + FADE_OUT / 2;
        assert!((ui.output_volume(half) - 0.4).abs() < 1e-6);
        assert_eq!(ui.volume, 0.8, "the slider stays put");
        assert_eq!(ui.advance_fade(half), vec![]);
        assert_eq!(ui.state, PlayState::Playing);
        assert_eq!(ui.advance_fade(t0 + FADE_OUT), vec![audio(Transport::Stop)]);
        assert_eq!(ui.state, PlayState::Stopped);
        assert!(!ui.is_fading());
        assert_eq!(ui.output_volume(t0 + FADE_OUT), 0.0, "silent while stopped");
        assert_eq!(ui.key(KeyCmd::FadeOut), vec![], "stopped: nothing to fade");
        assert!(!ui.is_fading());
        let _ = ui.key(KeyCmd::Play);
        assert_eq!(ui.output_volume(t0), 0.8, "full volume for what plays next");
    }

    #[test]
    fn a_fade_out_gives_way_to_other_transport_and_to_the_end_of_the_track() {
        let mut ui = Ui::demo();
        let now = Instant::now();
        let _ = ui.key(KeyCmd::FadeOut);
        let _ = ui.advance_fade(now);
        assert_eq!(ui.key(KeyCmd::Next), vec![audio(Transport::Open(1))]);
        assert!(!ui.is_fading(), "a new track plays at full volume");
        let _ = ui.key(KeyCmd::FadeOut);
        let _ = ui.key(KeyCmd::Pause);
        assert!(!ui.is_fading(), "pausing cancels it");
        // Paused, it just stops.
        assert_eq!(ui.key(KeyCmd::FadeOut), vec![audio(Transport::Stop)]);
        let _ = ui.key(KeyCmd::Play);
        let _ = ui.key(KeyCmd::FadeOut);
        let mut actions = Vec::new();
        ui.audio_event(Event::Finished, &mut actions);
        assert_eq!(actions, vec![audio(Transport::Stop)], "not the next track");
        // Shift-click on Stop fades out too.
        let _ = ui.key(KeyCmd::Play);
        ui.mods.shift = true;
        assert_eq!(click(&mut ui, Main, m::STOP), vec![]);
        assert!(ui.is_fading());
        ui.mods.shift = false;
        assert_eq!(click(&mut ui, Main, m::STOP), vec![audio(Transport::Stop)]);
    }

    #[test]
    fn file_info_shows_the_current_track_or_the_selected_entry() {
        let mut ui = Ui::demo();
        let _ = ui.key(KeyCmd::Next);
        let info = |of| Action::App(AppCommand::FileInfo(of));
        assert_eq!(
            click(&mut ui, Main, m::CLUTTER_I),
            vec![info(InfoOf::Current)]
        );
        assert_eq!(
            ui.key(KeyCmd::FileInfo(InfoOf::Selection)),
            vec![info(InfoOf::Selection)]
        );
        let path = |t: Option<&Track>| t.map(|t| t.location.clone());
        assert_eq!(path(ui.info_track(InfoOf::Current)), path(ui.tracks.get(1)));
        // The demo selects the first entry.
        assert_eq!(
            path(ui.info_track(InfoOf::Selection)),
            path(ui.tracks.first())
        );
        ui.selected.fill(false);
        ui.selected[4] = true;
        ui.selected[5] = true;
        assert_eq!(
            path(ui.info_track(InfoOf::Selection)),
            path(ui.tracks.get(4))
        );
        assert!(Ui::new().info_track(InfoOf::Selection).is_none());
    }

    #[test]
    fn both_eject_buttons_and_l_ask_for_files() {
        let mut ui = Ui::demo();
        let now = Instant::now();
        let (ex, ey) = (m::EJECT.x + 5, m::EJECT.y + 5);
        let _ = ui.mouse_down(Main, ex, ey, now);
        assert_eq!(ui.pressed, Some(Control::Eject));
        assert_eq!(ui.mouse_up(Main, ex, ey), vec![Action::OpenFiles]);
        // Letting go somewhere else cancels, as with every button.
        let _ = ui.mouse_down(Main, ex, ey, now);
        assert!(ui.mouse_up(Main, ex, ey + 40).is_empty());
        let mini = p::MINI_EJECT.offset(p::MIN_W - 150, p::MIN_H - p::BOTTOM_H);
        let _ = ui.mouse_down(Playlist, mini.x + 2, mini.y + 2, now);
        assert_eq!(ui.pressed, Some(Control::PlMiniEject));
        assert_eq!(
            ui.mouse_up(Playlist, mini.x + 2, mini.y + 2),
            vec![Action::OpenFiles]
        );
        assert_eq!(ui.key(KeyCmd::OpenFiles), vec![Action::OpenFiles]);
        assert_eq!(
            ui.state,
            PlayState::Playing,
            "nothing changes until files are chosen"
        );
    }

    #[test]
    fn clearing_the_playlist_stops_and_starts_over() {
        let mut ui = Ui::demo();
        let now = Instant::now();
        ui.play_track(3, &mut Vec::new());
        let mut actions = Vec::new();
        ui.clear_playlist(&mut actions);
        assert_eq!(actions, vec![audio(Transport::Stop)]);
        assert!(ui.tracks.is_empty() && ui.selected.is_empty());
        assert_eq!(
            (ui.current, ui.pl_scroll, ui.state),
            (0, 0, PlayState::Stopped)
        );
        assert_eq!(ui.marquee_text(now), "");
        assert_eq!(ui.add_files([PathBuf::from("new.flac")]), 0..1);
        assert_eq!(ui.marquee_text(now), "1. new");
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
    fn playlist_grip_hands_the_resize_to_the_window_layer() {
        let mut ui = Ui::new();
        let grip = (275 - 8, 116 - 8);
        let actions = ui.mouse_down(Playlist, grip.0, grip.1, Instant::now());
        assert_eq!(actions, vec![Action::ResizePlaylist]);
        assert_eq!(ui.pressed, None);
        assert_eq!(ui.mouse_move(Playlist, grip.0 + 30, grip.1 + 40), vec![]);
        assert_eq!(ui.pl_size, (p::MIN_W, p::MIN_H));
    }

    #[test]
    fn playlist_resize_snaps_to_grid() {
        let mut ui = Ui::new();
        // Half a step (12 of 25, 14 of 29) is not enough; one pixel more is.
        assert!(!ui.resize_playlist((275 + 12, 116 + 14)));
        assert!(ui.resize_playlist((275 + 13, 116 + 15)));
        assert_eq!(ui.pl_size, (300, 145));
        assert_eq!(ui.window_size(Playlist), (300, 145));
        assert_eq!(ui.layout().size, (300, 232 + 145));
        assert!(!ui.resize_playlist((300 + 10, 145 - 10)), "same grid cell");
        assert!(ui.resize_playlist((-500, i32::MIN)));
        assert_eq!(ui.pl_size, (p::MIN_W, p::MIN_H), "never below the minimum");
    }

    #[test]
    fn a_taller_playlist_scrolls_back_into_range() {
        let mut ui = Ui::demo();
        // 7 tracks, 4 rows visible at the minimum size, 8 two steps taller.
        ui.pl_scroll = ui.max_scroll();
        assert_eq!(ui.pl_scroll, 3);
        assert!(ui.resize_playlist((275, 116 + 2 * 29)));
        assert_eq!(ui.pl_visible_rows(), 8);
        assert_eq!(ui.pl_scroll, 0);
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
    fn the_marquee_starts_over_when_its_text_changes() {
        let mut ui = Ui::demo();
        let later = ui.start + Duration::from_secs(10);
        assert!(!ui.marquee_text(later).starts_with("1. "), "scrolled along");
        let _ = ui.key(KeyCmd::Next);
        ui.tick(later);
        assert!(ui.marquee_text(later).starts_with("2. Nullsoft"));
        // Text that stays the same scrolls on from there.
        let after = later + Duration::from_secs(1);
        ui.tick(after);
        assert!(!ui.marquee_text(after).starts_with("2. "));
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
            let mut c = Canvas::new(w, h, Density::ONE);
            ui.draw_window(kind, &mut c, &skin, None, now);
            singles.push(c);
        }
        let lay = ui.layout();
        let mut stacked = Canvas::new(lay.size.0, lay.size.1, Density::ONE);
        ui.draw_stacked(&mut stacked, &skin, None, now);
        let vis = m::VIS; // time-seeded, so excluded from the comparison
        for (i, origin) in [lay.main, lay.eq.unwrap(), lay.pl.unwrap()]
            .into_iter()
            .enumerate()
        {
            let single = &singles[i];
            let (w, h) = single.size();
            for y in 0..h {
                for x in 0..w {
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

    fn click(ui: &mut Ui, kind: WindowKind, r: Rect) -> Vec<Action> {
        let (x, y) = (r.x + 2, r.y + 2);
        let _ = ui.mouse_down(kind, x, y, Instant::now());
        ui.mouse_up(kind, x, y)
    }

    fn menu(kind: MenuKind, window: WindowKind, below: Option<Rect>) -> Vec<Action> {
        vec![Action::OpenMenu(MenuRequest {
            kind,
            window,
            below,
            keyboard: false,
        })]
    }

    #[test]
    fn option_buttons_open_their_menus() {
        let mut ui = Ui::demo();
        assert_eq!(
            click(&mut ui, Main, m::OPTIONS),
            menu(MenuKind::Main, Main, Some(m::OPTIONS))
        );
        assert_eq!(
            click(&mut ui, Main, m::CLUTTER_O),
            menu(MenuKind::Options, Main, None)
        );
        assert_eq!(
            click(&mut ui, Main, m::CLUTTER_V),
            menu(MenuKind::Vis, Main, None)
        );
        assert_eq!(
            click(&mut ui, Eq, e::PRESETS),
            menu(MenuKind::EqPresets, Eq, Some(e::PRESETS))
        );
        assert_eq!(
            click(&mut ui, Main, m::CLUTTER_A),
            vec![Action::AlwaysOnTop]
        );
        assert!(ui.always_on_top);
        assert_eq!(ui.key(KeyCmd::ToggleAlwaysOnTop), vec![Action::AlwaysOnTop]);
        assert!(!ui.always_on_top);
        let keyboard = ui.key(KeyCmd::SkinsMenu);
        assert!(matches!(
            keyboard[..],
            [Action::OpenMenu(MenuRequest {
                kind: MenuKind::Skins,
                keyboard: true,
                ..
            })]
        ));
    }

    #[test]
    fn right_clicks_open_the_visualizer_menu_or_the_main_one() {
        let mut ui = Ui::demo();
        let vis = (m::VIS.x + 5, m::VIS.y + 5);
        assert_eq!(
            ui.context_menu(Main, vis.0, vis.1),
            menu(MenuKind::Vis, Main, None)
        );
        assert_eq!(
            ui.context_menu(Main, 100, 5),
            menu(MenuKind::Main, Main, None)
        );
        assert_eq!(
            ui.context_menu(Playlist, 100, 30),
            menu(MenuKind::Main, Playlist, None)
        );
        // Not in the middle of a drag.
        let _ = ui.mouse_down(Main, 120, 60, Instant::now());
        assert!(ui.context_menu(Main, 100, 5).is_empty());
    }

    #[test]
    fn menu_commands_change_the_interface() {
        let mut ui = Ui::demo();
        assert!(ui.command(Command::VisMode(VisMode::Off)).is_empty());
        assert_eq!(ui.vis.mode, VisMode::Off);
        let _ = ui.command(Command::Bands(Bands::Thin));
        let _ = ui.command(Command::TogglePeaks);
        assert_eq!(ui.vis.analyzer.bands, Bands::Thin);
        assert!(!ui.vis.analyzer.peaks);
        let _ = ui.command(Command::TimeRemaining(true));
        assert!(ui.remaining_mode);
        // Rock: bass up, low mids down.
        let _ = ui.command(Command::Preset(12));
        let db = ui.eq_settings().bands_db;
        assert!(db[0] > 4.0 && db[3] < -4.0, "{db:?}");
        assert_eq!(ui.eq_preset("x").bands, db);
        let _ = ui.command(Command::ResetEq);
        assert_eq!(ui.eq_settings().bands_db, [0.0; 10]);
        assert_eq!(ui.command(Command::Zoom(Zoom::MAX)), vec![Action::Relayout]);
        assert_eq!(ui.command(Command::Key(KeyCmd::ToggleShuffle)), vec![]);
        assert!(ui.shuffle);
        assert_eq!(ui.command(Command::Exit), vec![Action::Close]);
        assert_eq!(
            ui.command(Command::App(AppCommand::OpenSkin)),
            vec![Action::App(AppCommand::OpenSkin)]
        );
    }

    #[test]
    fn always_on_top_and_double_size_hold_their_clutter_buttons_down() {
        let skin = crate::skin::Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut ui = Ui::new();
        let now = Instant::now();
        let draw = |ui: &Ui| {
            let mut c = Canvas::new(m::W, m::H, Density::ONE);
            ui.draw_window(Main, &mut c, &skin, None, now);
            c
        };
        let up = draw(&ui);
        ui.always_on_top = true;
        ui.double_size = true;
        let down = draw(&ui);
        let differs = |r: Rect| {
            (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| up.get(x, y) != down.get(x, y)))
        };
        assert!(differs(m::CLUTTER_A) && differs(m::CLUTTER_D));
        assert!(!differs(m::CLUTTER_O));
    }

    #[test]
    fn windowshade_collapses_each_window_to_a_bar_keeping_its_width() {
        let mut ui = Ui::new();
        assert!(ui.resize_playlist((300, 145)));
        for kind in WindowKind::ALL {
            assert_eq!(
                ui.key(KeyCmd::ToggleShade(kind)),
                vec![Action::Reshade(kind)]
            );
            assert!(ui.is_shaded(kind));
        }
        assert_eq!(ui.window_size(Main), (275, 14));
        assert_eq!(ui.window_size(Eq), (275, 14));
        assert_eq!(ui.window_size(Playlist), (300, 14));
        assert_eq!(ui.pl_size, (300, 145), "the height it opens out to is kept");
        let l = ui.layout();
        assert_eq!(
            (l.eq.unwrap().y, l.pl.unwrap().y, l.size),
            (14, 28, (300, 42))
        );
        let _ = ui.key(KeyCmd::ToggleShade(Playlist));
        assert_eq!(ui.window_size(Playlist), (300, 145));
        assert_eq!(ui.window_size_shaded(Main, false), (275, 116));
    }

    #[test]
    fn shade_buttons_and_title_double_clicks_toggle_windowshade() {
        let mut ui = Ui::new();
        let now = Instant::now();
        let later = |ms| now + Duration::from_millis(ms);
        assert_eq!(click(&mut ui, Main, m::SHADE), vec![Action::Reshade(Main)]);
        assert!(ui.main_shade);
        // The bar's shade button opens the window out again.
        assert_eq!(click(&mut ui, Main, m::SHADE), vec![Action::Reshade(Main)]);
        assert!(!ui.main_shade);
        assert_eq!(click(&mut ui, Eq, e::SHADE), vec![Action::Reshade(Eq)]);
        assert_eq!(click(&mut ui, Eq, e::SHADE), vec![Action::Reshade(Eq)]);
        let pl_shade = Rect::new(275 - p::SHADE_FROM_RIGHT, p::TITLE_BTN_Y, 9, 9);
        assert_eq!(
            click(&mut ui, Playlist, pl_shade),
            vec![Action::Reshade(Playlist)]
        );
        assert!(ui.pl_shade);
        assert_eq!(
            click(&mut ui, Playlist, pl_shade),
            vec![Action::Reshade(Playlist)]
        );

        // A single click on a title bar drags; a second soon after toggles, a third drags.
        assert_eq!(
            ui.mouse_down(Main, 100, 5, now),
            vec![Action::DragWindow(Main)]
        );
        let _ = ui.mouse_up(Main, 100, 5);
        assert_eq!(
            ui.mouse_down(Main, 100, 5, later(200)),
            vec![Action::Reshade(Main)]
        );
        assert!(ui.main_shade);
        assert!(ui.mouse_up(Main, 100, 5).is_empty());
        // Anywhere on a bar that is not a control counts as its title bar.
        assert_eq!(
            ui.mouse_down(Main, 30, 7, later(300)),
            vec![Action::DragWindow(Main)]
        );
        let _ = ui.mouse_up(Main, 30, 7);
        assert_eq!(
            ui.mouse_down(Main, 30, 7, later(350)),
            vec![Action::Reshade(Main)]
        );
        assert!(!ui.main_shade);
        // Too slow.
        let _ = ui.mouse_down(Eq, 100, 5, later(1000));
        let _ = ui.mouse_up(Eq, 100, 5);
        let actions = ui.mouse_down(Eq, 100, 5, later(1000 + DOUBLE_CLICK_MS as u64));
        assert_eq!(actions, vec![Action::DragWindow(Eq)]);
    }

    #[test]
    fn hit_testing_the_windowshade_bars() {
        let mut ui = Ui::demo();
        ui.main_shade = true;
        ui.eq_shade = true;
        ui.pl_shade = true;
        let at = |ui: &Ui, kind, r: Rect| ui.hit(kind, r.x + 1, r.y + 1);
        assert_eq!(at(&ui, Main, ms::PLAY), Some(Control::Play));
        assert_eq!(at(&ui, Main, ms::EJECT), Some(Control::Eject));
        assert_eq!(at(&ui, Main, ms::POSBAR), Some(Control::PosBar));
        assert_eq!(at(&ui, Main, ms::TIME), Some(Control::TimeDisplay));
        assert_eq!(at(&ui, Main, ms::VIS), Some(Control::Vis));
        assert_eq!(at(&ui, Main, m::OPTIONS), Some(Control::MainOptions));
        assert_eq!(at(&ui, Main, m::SHADE), Some(Control::MainShade));
        assert_eq!(ui.hit(Main, 40, 7), Some(Control::MainTitle));
        assert_eq!(ui.hit(Main, 40, 50), None, "below the bar");
        assert_eq!(at(&ui, Eq, es::VOLUME), Some(Control::EqVolume));
        assert_eq!(at(&ui, Eq, es::BALANCE), Some(Control::EqBalance));
        assert_eq!(at(&ui, Eq, e::CLOSE), Some(Control::EqClose));
        assert_eq!(ui.hit(Eq, 30, 7), Some(Control::EqTitle));
        assert_eq!(ui.hit(Playlist, 275 - 25, 5), Some(Control::PlResize));
        assert_eq!(ui.hit(Playlist, 275 - 17, 5), Some(Control::PlShade));
        assert_eq!(ui.hit(Playlist, 275 - 7, 5), Some(Control::PlClose));
        assert_eq!(ui.hit(Playlist, 100, 5), Some(Control::PlTitle));
        // The right-click over the small visualizer opens its menu.
        assert_eq!(
            ui.context_menu(Main, ms::VIS.x + 1, ms::VIS.y + 1),
            menu(MenuKind::Vis, Main, None)
        );
    }

    #[test]
    fn the_windowshade_bars_play_seek_and_set_the_volume() {
        let mut ui = Ui::demo(); // the first track is 5 seconds long
        ui.main_shade = true;
        ui.eq_shade = true;
        assert_eq!(click(&mut ui, Main, ms::STOP), vec![audio(Transport::Stop)]);
        assert_eq!(
            click(&mut ui, Main, ms::PLAY),
            vec![audio(Transport::Open(0))]
        );
        let _ = click(&mut ui, Main, ms::TIME);
        assert!(ui.remaining_mode);
        // The position bar seeks when let go.
        let bar = ms::POSBAR;
        let _ = ui.mouse_down(Main, bar.x + 1, bar.y + 2, Instant::now());
        let actions = ui.mouse_up(Main, bar.right(), bar.y + 2);
        assert_eq!(actions, vec![audio(Transport::Seek(0.0))]);
        let _ = ui.mouse_down(Main, bar.x + 1, bar.y + 2, Instant::now());
        let _ = ui.mouse_move(Main, bar.right(), bar.y + 2);
        assert_eq!(
            ui.mouse_up(Main, bar.right(), bar.y + 2),
            vec![audio(Transport::Seek(5.0))]
        );
        // The equalizer's bar sets the volume and balance, following the pointer in its window.
        let vol = es::VOLUME;
        let _ = ui.mouse_down(Eq, vol.x, vol.y + 2, Instant::now());
        assert!(ui.volume < 0.01);
        let _ = ui.mouse_move(Eq, vol.right(), vol.y + 2);
        assert!(ui.volume > 0.99);
        assert_eq!(ui.marquee_override.as_deref(), Some("Volume: 100%"));
        assert!(ui.mouse_up(Eq, vol.right(), vol.y + 2).is_empty());
        let bal = es::BALANCE;
        let _ = ui.mouse_down(Eq, bal.x, bal.y + 2, Instant::now());
        assert_eq!(ui.balance, -1.0);
        let _ = ui.mouse_move(Eq, bal.x + bal.w / 2, bal.y + 2);
        assert_eq!(ui.balance, 0.0, "snaps to the centre");
        let _ = ui.mouse_up(Eq, bal.x + bal.w / 2, bal.y + 2);
        // The shaded playlist has no list for the wheel to scroll.
        ui.pl_shade = true;
        ui.pl_scroll = 2;
        ui.wheel(Playlist, -1);
        assert_eq!(ui.pl_scroll, 2);
    }

    #[test]
    fn windowshade_bars_draw_their_own_pieces_and_shapes() {
        let mut skin = crate::skin::Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut ui = Ui::demo();
        ui.main_shade = true;
        ui.eq_shade = true;
        ui.pl_shade = true;
        let now = Instant::now();
        let draw = |ui: &Ui, skin: &Skin, kind| {
            let (w, h) = ui.window_size(kind);
            let mut c = Canvas::new(w, h, Density::ONE);
            ui.draw_window(kind, &mut c, skin, None, now);
            c
        };
        for kind in WindowKind::ALL {
            let c = draw(&ui, &skin, kind);
            assert_eq!(c.size().1, 14);
            assert!(
                c.image().data.chunks_exact(4).all(|px| px[3] == 255),
                "{kind:?} opaque"
            );
        }
        // The equalizer's bar, against the inactive one from eq_ex.bmp.
        ui.active = false;
        let inactive = draw(&ui, &skin, Eq);
        let ex = skin.sheet(crate::sprites::Sheet::EqEx);
        assert_eq!(inactive.get(30, 7), ex.get(30, 15 + 7));
        ui.active = true;
        // [WindowShade] and [EqualizerWS] cut the bars to shape; [Normal] does not apply.
        let regions = crate::skin::parse_region(
            "[Normal]\nNumPoints=4\nPointList=0,0,9,0,9,9,0,9\n\
             [WindowShade]\nNumPoints=4\nPointList=0,0,275,0,275,13,0,13\n\
             [EqualizerWS]\nNumPoints=4\nPointList=1,0,275,0,275,14,1,14\n",
        );
        skin.regions = regions;
        let main = draw(&ui, &skin, Main);
        assert_eq!(main.get(100, 13)[3], 0, "the last row is cut away");
        assert_eq!(main.get(100, 12)[3], 255, "the rest of the bar is not");
        let eq = draw(&ui, &skin, Eq);
        assert_eq!((eq.get(0, 5)[3], eq.get(1, 5)[3]), (0, 255));
    }

    #[test]
    fn per_window_drawing_matches_the_stacked_composition_in_windowshade() {
        let skin = crate::skin::Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut ui = Ui::demo();
        ui.main_shade = true;
        ui.pl_shade = true;
        let now = Instant::now();
        ui.tick(now);
        let lay = ui.layout();
        let mut stacked = Canvas::new(lay.size.0, lay.size.1, Density::ONE);
        ui.draw_stacked(&mut stacked, &skin, None, now);
        for (kind, origin) in [
            (Main, lay.main),
            (Eq, lay.eq.unwrap()),
            (Playlist, lay.pl.unwrap()),
        ] {
            let (w, h) = ui.window_size(kind);
            let mut c = Canvas::new(w, h, Density::ONE);
            ui.draw_window(kind, &mut c, &skin, None, now);
            for y in 0..h {
                for x in 0..w {
                    if kind == Main && ms::VIS.contains(x, y) {
                        continue; // time-seeded
                    }
                    assert_eq!(
                        c.get(x, y),
                        stacked.get(origin.x + x, origin.y + y),
                        "{kind:?} pixel ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn dense_windows_draw_the_skin_bitmaps_exactly_enlarged() {
        let skin = crate::skin::Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut ui = Ui::demo();
        let now = Instant::now();
        ui.tick(now);
        for shaded in [false, true] {
            (ui.main_shade, ui.eq_shade, ui.pl_shade) = (shaded, shaded, shaded);
            for kind in WindowKind::ALL {
                let (w, h) = ui.window_size(kind);
                let mut one = Canvas::new(w, h, Density::ONE);
                let mut three = Canvas::new(w, h, Density::new(3).unwrap());
                // Without the TrueType font every piece comes from the skin's bitmaps.
                ui.draw_window(kind, &mut one, &skin, None, now);
                ui.draw_window(kind, &mut three, &skin, None, now);
                let dense = three.image();
                for y in 0..dense.h {
                    for x in 0..dense.w {
                        assert_eq!(
                            dense.get(x, y),
                            one.get(x / 3, y / 3),
                            "{kind:?} (shaded: {shaded}) image pixel ({x},{y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_dense_canvas_cuts_slanted_shapes_finer_than_a_skin_pixel() {
        let mut skin = crate::skin::Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        skin.regions =
            crate::skin::parse_region("[Normal]\nNumPoints=3\nPointList=0,0,275,0,0,116\n");
        let ui = Ui::new();
        let now = Instant::now();
        let mut two = Canvas::new(m::W, m::H, Density::new(2).unwrap());
        ui.draw_window(Main, &mut two, &skin, None, now);
        let img = two.image();
        let alpha = |x: i32, y: i32| img.get(x, y)[3];
        assert_eq!((alpha(0, 0), alpha(549, 231)), (255, 0));
        // Along the slanted edge some skin pixels are cut part way through.
        let split = (0..m::H).any(|sy| {
            (0..m::W).any(|sx| {
                let block = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .map(|(dx, dy)| alpha(sx * 2 + dx, sy * 2 + dy));
                block.contains(&0) && block.contains(&255)
            })
        });
        assert!(split, "the outline has steps half a skin pixel apart");
    }

    #[test]
    fn the_playlist_bar_names_the_current_entry() {
        let mut ui = Ui::demo();
        assert_eq!(ui.title(), "1. DJ Mike Llama - Llama Whippin' Intro");
        ui.jump(2, &mut Vec::new());
        assert_eq!(ui.title(), "3. Justin F. - Rolling Llama (Radio Edit)");
        // The entry playing is removed: it plays on under its old name.
        ui.selected = vec![false, false, true, false, false, false, false];
        let _ = ui.key(KeyCmd::List(ListCmd::RemoveSelected));
        assert_eq!(ui.title(), "3. Justin F. - Rolling Llama (Radio Edit)");
        ui.clear_playlist(&mut Vec::new());
        assert_eq!(ui.title(), "");
    }

    #[test]
    fn the_window_title_names_the_track_and_whether_it_plays() {
        let title = |ui: &Ui| {
            let mut s = String::new();
            ui.write_window_title(&mut s);
            s
        };
        let mut ui = Ui::demo();
        assert_eq!(
            title(&ui),
            "1. DJ Mike Llama - Llama Whippin' Intro - NeoAmp"
        );
        let _ = ui.key(KeyCmd::Pause);
        assert_eq!(
            title(&ui),
            "1. DJ Mike Llama - Llama Whippin' Intro - NeoAmp [Paused]"
        );
        let _ = ui.key(KeyCmd::Next);
        let _ = ui.key(KeyCmd::Stop);
        assert_eq!(
            title(&ui),
            "2. Nullsoft - Winamp 2.91 Theme - NeoAmp [Stopped]"
        );
        assert_eq!(title(&Ui::new()), "NeoAmp");
    }

    #[test]
    fn each_part_of_the_windows_has_its_pointer() {
        let mut ui = Ui::demo();
        let at = |ui: &Ui, kind, r: Rect| ui.cursor_at(kind, r.x + 1, r.y + 1);
        assert_eq!(at(&ui, Main, m::CLOSE), SkinCursor::Close);
        assert_eq!(at(&ui, Main, m::OPTIONS), SkinCursor::MainMenu);
        assert_eq!(ui.cursor_at(Main, 100, 5), SkinCursor::TitleBar);
        assert_eq!(at(&ui, Main, m::BALANCE), SkinCursor::VolBal);
        assert_eq!(at(&ui, Main, m::POSBAR), SkinCursor::PosBar);
        assert_eq!(at(&ui, Main, m::MARQUEE), SkinCursor::SongName);
        assert_eq!(at(&ui, Main, m::PLAY), SkinCursor::Normal);
        assert_eq!(at(&ui, Eq, e::band(3)), SkinCursor::EqSlid);
        assert_eq!(at(&ui, Eq, e::SHADE), SkinCursor::EqTitle);
        assert_eq!(at(&ui, Eq, e::ON), SkinCursor::EqNormal);
        let (w, h) = ui.pl_size;
        assert_eq!(ui.cursor_at(Playlist, w - 3, h - 3), SkinCursor::PSize);
        assert_eq!(ui.cursor_at(Playlist, 50, 5), SkinCursor::PTBar);
        assert_eq!(ui.cursor_at(Playlist, w - 10, 40), SkinCursor::PVScroll);
        assert_eq!(ui.cursor_at(Playlist, 50, 40), SkinCursor::PNormal);
        ui.main_shade = true;
        ui.eq_shade = true;
        ui.pl_shade = true;
        assert_eq!(at(&ui, Main, m::OPTIONS), SkinCursor::MMenu);
        assert_eq!(at(&ui, Main, m::CLOSE), SkinCursor::Close);
        assert_eq!(at(&ui, Main, ms::POSBAR), SkinCursor::WsPosBar);
        assert_eq!(at(&ui, Main, ms::PLAY), SkinCursor::WsNormal);
        assert_eq!(at(&ui, Eq, es::VOLUME), SkinCursor::EqTitle);
        assert_eq!(at(&ui, Eq, e::CLOSE), SkinCursor::EqClose);
        assert_eq!(
            ui.cursor_at(Playlist, w - ps::RESIZE_FROM_RIGHT + 2, 5),
            SkinCursor::PWsSize
        );
        assert_eq!(ui.cursor_at(Playlist, 50, 5), SkinCursor::PWsNorm);
    }

    #[test]
    fn time_formatting() {
        assert_eq!(Ui::time_str(5), "0:05");
        assert_eq!(Ui::time_str(3661), "61:01");
    }

    /// Window height of the middle of playlist row `row` (with the list scrolled to the top).
    fn row_y(row: i32) -> i32 {
        p::TOP_H + p::LIST_PAD + row * p::TRACK_H + p::TRACK_H / 2
    }

    fn click_row(ui: &mut Ui, row: i32, mods: Mods) {
        ui.mods = mods;
        let _ = ui.mouse_down(Playlist, 100, row_y(row), Instant::now());
        let _ = ui.mouse_up(Playlist, 100, row_y(row));
        // Far apart, so that two clicks never make a double-click.
        ui.last_click = None;
    }

    fn selected_rows(ui: &Ui) -> Vec<usize> {
        (0..ui.tracks.len()).filter(|&i| ui.selected[i]).collect()
    }

    fn titles(ui: &Ui) -> Vec<&str> {
        ui.tracks.iter().map(|t| &t.title[..4]).collect()
    }

    const SHIFT: Mods = Mods {
        shift: true,
        ctrl: false,
    };
    const CTRL: Mods = Mods {
        shift: false,
        ctrl: true,
    };

    #[test]
    fn shift_and_ctrl_clicks_select_several_entries() {
        let mut ui = Ui::demo();
        click_row(&mut ui, 1, Mods::default());
        assert_eq!(selected_rows(&ui), [1]);
        click_row(&mut ui, 3, SHIFT);
        assert_eq!(selected_rows(&ui), [1, 2, 3]);
        click_row(&mut ui, 0, SHIFT);
        assert_eq!(selected_rows(&ui), [0, 1], "a range from the same anchor");
        click_row(&mut ui, 3, CTRL);
        assert_eq!(selected_rows(&ui), [0, 1, 3]);
        click_row(&mut ui, 1, CTRL);
        assert_eq!(selected_rows(&ui), [0, 3]);
        click_row(&mut ui, 2, Mods::default());
        assert_eq!(selected_rows(&ui), [2]);
    }

    #[test]
    fn dragging_moves_the_selected_entries_together() {
        let mut ui = Ui::demo(); // the first entry is selected and current
        click_row(&mut ui, 2, CTRL);
        assert_eq!(selected_rows(&ui), [0, 2]);
        ui.mods = Mods::default();
        let now = Instant::now();
        let _ = ui.mouse_down(Playlist, 100, row_y(0), now);
        let _ = ui.mouse_move(Playlist, 100, row_y(1));
        assert_eq!(&titles(&ui)[..4], ["Null", "DJ M", "The ", "Just"]);
        let _ = ui.mouse_move(Playlist, 100, row_y(2));
        assert_eq!(&titles(&ui)[..5], ["Null", "The ", "DJ M", "Milk", "Just"]);
        let _ = ui.mouse_up(Playlist, 100, row_y(2));
        assert_eq!(selected_rows(&ui), [2, 4], "the selection moved with them");
        assert_eq!(ui.current_row(), Some(2), "so did the current entry");
        assert!(ui.marquee_text(ui.start).starts_with("3. DJ Mike"));
    }

    #[test]
    fn a_click_on_a_selected_entry_keeps_the_selection_until_let_go() {
        let mut ui = Ui::demo();
        click_row(&mut ui, 2, SHIFT);
        assert_eq!(selected_rows(&ui), [0, 1, 2]);
        ui.mods = Mods::default();
        let _ = ui.mouse_down(Playlist, 100, row_y(1), Instant::now());
        assert_eq!(
            selected_rows(&ui),
            [0, 1, 2],
            "it may be about to be dragged"
        );
        let _ = ui.mouse_up(Playlist, 100, row_y(1));
        assert_eq!(selected_rows(&ui), [1]);
    }

    #[test]
    fn the_playlist_menus_act_when_let_go_over_an_item() {
        let mut ui = Ui::demo();
        let (w, h) = ui.pl_size;
        let (button, items) = PlMenu::Select.rects(w, h);
        let now = Instant::now();
        let _ = ui.mouse_down(Playlist, button.x + 5, button.y + 5, now);
        assert_eq!(ui.pl_menu(), Some(PlMenu::Select));
        // SELECT ALL is the bottom item, over the button: a plain click chooses it.
        let _ = ui.mouse_up(Playlist, button.x + 5, button.y + 5);
        assert_eq!(selected_rows(&ui), (0..7).collect::<Vec<_>>());
        assert_eq!(ui.pl_menu(), None);
        // INVERT is the top item.
        let _ = ui.mouse_down(Playlist, button.x + 5, button.y + 5, now);
        let _ = ui.mouse_move(Playlist, items.x + 5, items.y + 5);
        let _ = ui.mouse_up(Playlist, items.x + 5, items.y + 5);
        assert!(selected_rows(&ui).is_empty());
        // Letting go anywhere else does nothing.
        let _ = ui.mouse_down(Playlist, button.x + 5, button.y + 5, now);
        assert!(ui.mouse_up(Playlist, 200, 30).is_empty());
        assert_eq!(ui.pl_menu(), None);

        let (add, _) = PlMenu::Add.rects(w, h);
        let _ = ui.mouse_down(Playlist, add.x + 5, add.y + 5, now);
        assert_eq!(
            ui.mouse_up(Playlist, add.x + 5, add.y + 5),
            vec![Action::App(AppCommand::AddFiles)]
        );
        let (list, list_items) = PlMenu::List.rects(w, h);
        let _ = ui.mouse_down(Playlist, list.x + 5, list.y + 5, now);
        let middle = list_items.y + p::MENU_ITEM_H + 5;
        assert_eq!(
            ui.mouse_up(Playlist, list_items.x + 5, middle),
            vec![Action::App(AppCommand::SaveList)]
        );
        let (misc, misc_items) = PlMenu::Misc.rects(w, h);
        let _ = ui.mouse_down(Playlist, misc.x + 5, misc.y + 5, now);
        assert_eq!(
            ui.mouse_up(Playlist, misc_items.x + 5, misc_items.y + 5),
            menu(MenuKind::PlaylistSort, Playlist, None)
        );
    }

    #[test]
    fn removing_entries_keeps_the_current_one_and_the_selection_in_step() {
        let mut ui = Ui::demo();
        let _ = ui.key(KeyCmd::Stop);
        ui.select(4);
        ui.selected = vec![false, true, false, true, false, false, false];
        let _ = ui.key(KeyCmd::List(ListCmd::RemoveSelected));
        assert_eq!(ui.tracks.len(), 5);
        assert_eq!(ui.current, 2, "entry 4 is now entry 2");
        assert!(selected_rows(&ui).is_empty());
        assert!(ui.marquee_text(ui.start).starts_with("3. MilkDrop Crew"));
        // Crop keeps only the selection, but never empties the list by accident.
        let _ = ui.key(KeyCmd::List(ListCmd::Crop));
        assert_eq!(ui.tracks.len(), 5);
        ui.selected[0] = true;
        ui.selected[1] = true;
        let _ = ui.key(KeyCmd::List(ListCmd::Crop));
        assert_eq!(ui.tracks.len(), 2);
        assert_eq!(ui.current, 1, "the current entry went: the last one left");
        let _ = ui.key(KeyCmd::List(ListCmd::Clear));
        assert!(ui.tracks.is_empty() && ui.current_row().is_none());
        assert_eq!(ui.marquee_text(Instant::now()), "");
    }

    #[test]
    fn a_playing_track_plays_on_when_its_entry_is_removed() {
        let mut ui = Ui::demo();
        let mut actions = Vec::new();
        ui.play_track(2, &mut actions);
        let title = ui.marquee_text(ui.start);
        ui.selected = vec![false, false, true, false, false, false, false];
        assert!(ui.key(KeyCmd::List(ListCmd::RemoveSelected)).is_empty());
        assert_eq!(ui.state, PlayState::Playing);
        assert_eq!(
            ui.track().unwrap().title,
            "Justin F. - Rolling Llama (Radio Edit)"
        );
        assert_eq!(ui.current_row(), None);
        assert_eq!(ui.marquee_text(ui.start), title, "the marquee is unchanged");
        // When it ends, the entry that followed it plays.
        actions.clear();
        ui.audio_event(Event::Finished, &mut actions);
        assert_eq!(actions, vec![audio(Transport::Open(2))]);
        assert_eq!(ui.tracks[2].title, "The Shoutcasters - Buffering...");
        assert_eq!(ui.current_row(), Some(2));
    }

    #[test]
    fn removing_the_last_playing_entry_stops_after_it_unless_repeating() {
        let mut ui = Ui::demo();
        let mut actions = Vec::new();
        ui.play_track(6, &mut actions);
        let _ = ui.key(KeyCmd::List(ListCmd::Cursor {
            to: CursorMove::End,
            extend: false,
        }));
        let _ = ui.key(KeyCmd::List(ListCmd::RemoveSelected));
        assert_eq!((ui.tracks.len(), ui.current), (6, 6), "one past the end");
        actions.clear();
        ui.audio_event(Event::Finished, &mut actions);
        assert_eq!(actions, vec![audio(Transport::Stop)]);
        assert_eq!(ui.current_row(), Some(5));
        // Previous from a removed entry goes to the one before it.
        ui.play_track(5, &mut actions);
        ui.selected[5] = true;
        let _ = ui.key(KeyCmd::List(ListCmd::RemoveSelected));
        actions.clear();
        ui.prev(&mut actions);
        assert_eq!(actions, vec![audio(Transport::Open(4))]);
    }

    #[test]
    fn sorting_reversing_and_randomizing_follow_the_current_entry() {
        let mut ui = Ui::demo();
        let _ = ui.key(KeyCmd::List(ListCmd::Sort(SortKey::Title)));
        assert_eq!(
            titles(&ui),
            ["DJ M", "Just", "Milk", "Null", "The ", "Tom ", "Unkn"]
        );
        let _ = ui.key(KeyCmd::List(ListCmd::Reverse));
        assert_eq!(ui.tracks[6].title, "DJ Mike Llama - Llama Whippin' Intro");
        assert_eq!(ui.current_row(), Some(6));
        assert_eq!(selected_rows(&ui), [6]);
        let before = ui.playlist_version();
        let _ = ui.key(KeyCmd::List(ListCmd::Randomize));
        assert_ne!(ui.playlist_version(), before);
        let current = ui.current_row().unwrap();
        assert_eq!(
            ui.tracks[current].title,
            "DJ Mike Llama - Llama Whippin' Intro"
        );
        let mut sorted: Vec<&str> = titles(&ui);
        sorted.sort_unstable();
        assert_eq!(sorted.len(), 7);
    }

    #[test]
    fn the_keyboard_cursor_moves_selects_and_plays() {
        let mut ui = Ui::demo();
        let cursor = |to, extend| KeyCmd::List(ListCmd::Cursor { to, extend });
        let _ = ui.key(cursor(CursorMove::Down, false));
        let _ = ui.key(cursor(CursorMove::Down, false));
        assert_eq!(selected_rows(&ui), [2]);
        let _ = ui.key(cursor(CursorMove::Down, true));
        let _ = ui.key(cursor(CursorMove::Down, true));
        assert_eq!(selected_rows(&ui), [2, 3, 4]);
        assert_eq!(ui.pl_scroll, 1, "scrolled to keep the cursor in sight");
        let _ = ui.key(cursor(CursorMove::Home, true));
        assert_eq!(selected_rows(&ui), [0, 1, 2]);
        let _ = ui.key(cursor(CursorMove::PageDown, false));
        assert_eq!(selected_rows(&ui), [4], "four rows show");
        let _ = ui.key(cursor(CursorMove::End, false));
        assert_eq!(
            ui.key(KeyCmd::List(ListCmd::PlayCursor)),
            vec![audio(Transport::Open(6))]
        );
        let _ = ui.key(KeyCmd::List(ListCmd::MoveUp));
        assert_eq!(
            ui.current_row(),
            Some(5),
            "Alt+Up moved the playing entry up"
        );
        let _ = ui.key(KeyCmd::List(ListCmd::SelectAll));
        assert_eq!(selected_rows(&ui).len(), 7);
        let _ = ui.key(KeyCmd::List(ListCmd::InvertSelection));
        assert!(selected_rows(&ui).is_empty());
    }

    #[test]
    fn shuffle_plays_every_entry_before_repeating_and_previous_goes_back() {
        let mut ui = Ui::demo();
        ui.shuffler = Shuffle::new(Rng::with_seed(11));
        let _ = ui.key(KeyCmd::ToggleShuffle);
        let mut played = vec![ui.current];
        let mut actions = Vec::new();
        for _ in 1..7 {
            ui.audio_event(Event::Finished, &mut actions);
            played.push(ui.current);
        }
        assert_ne!(played, (0..7).collect::<Vec<_>>(), "not in order");
        let mut sorted = played.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..7).collect::<Vec<_>>(), "each entry once");
        actions.clear();
        ui.audio_event(Event::Finished, &mut actions);
        assert_eq!(actions, vec![audio(Transport::Stop)], "the round is over");
        let _ = ui.key(KeyCmd::Play);
        ui.prev(&mut actions);
        assert_eq!(ui.current, played[5]);
        // Next always moves on, starting a new round.
        actions.clear();
        ui.next(&mut actions);
        assert_eq!(actions.len(), 1);
    }

    fn demo_path(n: usize) -> PathBuf {
        PathBuf::from(format!("demo/{n:02}.mp3"))
    }

    #[test]
    fn the_upcoming_file_is_what_the_end_of_the_track_moves_on_to() {
        let mut ui = Ui::demo();
        assert_eq!(ui.upcoming(), Some(demo_path(2).as_path()));
        ui.current = 6;
        assert_eq!(ui.upcoming(), None, "the end of the playlist stops");
        ui.repeat = true;
        assert_eq!(ui.upcoming(), Some(demo_path(1).as_path()));
        ui.state = PlayState::Stopped;
        assert_eq!(ui.upcoming(), None);
        ui.state = PlayState::Paused;
        assert!(ui.upcoming().is_some(), "paused, it is still coming");
        let _ = ui.key(KeyCmd::FadeOut);
        assert_eq!(ui.upcoming(), None, "a fade-out stops at the end");
        // Streams are never preloaded, nor is a file after one.
        let mut ui = Ui::demo();
        ui.tracks[1].location = Location::Url("http://radio.example/".into());
        assert_eq!(ui.upcoming(), None);
        ui.current = 1;
        assert_eq!(ui.upcoming(), None);
    }

    #[test]
    fn a_gapless_advance_makes_the_entry_current_without_opening_it() {
        let mut ui = Ui::demo();
        ui.eq_auto = true;
        ui.elapsed = 4.0;
        let mut actions = Vec::new();
        ui.audio_event(Event::Advanced(demo_path(2)), &mut actions);
        assert_eq!(actions, vec![Action::Advanced(1)]);
        assert_eq!(
            (ui.current, ui.state, ui.elapsed),
            (1, PlayState::Playing, 0.0)
        );
        // An entry moved meanwhile is found where it went; one removed plays on without one.
        ui.tracks.swap(4, 5);
        actions.clear();
        ui.audio_event(Event::Advanced(demo_path(5)), &mut actions);
        assert_eq!(actions, vec![Action::Advanced(5)]);
        actions.clear();
        ui.audio_event(Event::Advanced(PathBuf::from("gone.mp3")), &mut actions);
        assert!(actions.is_empty());
        assert_eq!(ui.track().map(|t| t.title.as_str()), Some("gone"));
        assert_eq!(ui.current_row(), None);
        ui.audio_event(Event::Finished, &mut actions);
        assert_eq!(
            actions,
            vec![audio(Transport::Open(6))],
            "the entry after the last"
        );
    }

    #[test]
    fn a_gapless_advance_keeps_a_pause_or_a_fade_out() {
        let mut ui = Ui::demo();
        let mut actions = Vec::new();
        ui.pause(&mut actions);
        ui.audio_event(Event::Advanced(demo_path(2)), &mut actions);
        assert_eq!((ui.current, ui.state), (1, PlayState::Paused));
        let mut ui = Ui::demo();
        let _ = ui.key(KeyCmd::FadeOut);
        let start = Instant::now();
        let _ = ui.advance_fade(start);
        ui.audio_event(Event::Advanced(demo_path(2)), &mut actions);
        assert!(ui.is_fading());
        assert_eq!(
            ui.advance_fade(start + FADE_OUT),
            vec![audio(Transport::Stop)]
        );
    }

    #[test]
    fn shuffle_preloads_the_entry_it_then_plays() {
        let mut ui = Ui::demo();
        ui.shuffler = Shuffle::new(Rng::with_seed(4));
        let _ = ui.key(KeyCmd::ToggleShuffle);
        let mut played = vec![ui.current];
        let mut actions = Vec::new();
        for _ in 1..7 {
            let next = ui.upcoming().map(Path::to_path_buf).unwrap();
            assert_eq!(
                ui.upcoming(),
                Some(next.as_path()),
                "the same pick every frame"
            );
            ui.audio_event(Event::Advanced(next.clone()), &mut actions);
            assert_eq!(ui.track().map(|t| &t.location), Some(&Location::File(next)));
            played.push(ui.current);
        }
        played.sort_unstable();
        assert_eq!(played, (0..7).collect::<Vec<_>>(), "each entry once");
        assert_eq!(ui.upcoming(), None, "the round is over");
    }

    #[test]
    fn media_keys_play_and_pause_without_starting_over() {
        let mut ui = Ui::demo();
        ui.select(1);
        ui.elapsed = 30.0;
        assert!(ui.media(MediaRequest::Play).is_empty(), "already playing");
        assert_eq!(
            ui.media(MediaRequest::PlayPause),
            vec![audio(Transport::Pause)]
        );
        assert_eq!(ui.media(MediaRequest::Pause), Vec::<Action>::new());
        assert_eq!(ui.media(MediaRequest::Play), vec![audio(Transport::Resume)]);
        let seek = |secs| vec![audio(Transport::Seek(secs))];
        assert_eq!(ui.media(MediaRequest::SeekBy(-10.0)), seek(20.0));
        assert_eq!(ui.media(MediaRequest::SeekTo(4.5)), seek(4.5));
        assert_eq!(
            ui.media(MediaRequest::Next),
            vec![audio(Transport::Open(2))]
        );
        assert_eq!(
            ui.media(MediaRequest::Previous),
            vec![audio(Transport::Open(1))]
        );
        assert_eq!(ui.media(MediaRequest::Stop), vec![audio(Transport::Stop)]);
        assert_eq!(
            ui.media(MediaRequest::PlayPause),
            vec![audio(Transport::Open(1))]
        );
        let _ = ui.media(MediaRequest::Volume(1.5));
        assert_eq!(ui.volume, 1.0);
        let _ = ui.media(MediaRequest::Volume(f32::NAN));
        assert_eq!(ui.volume, 1.0);
    }

    #[test]
    fn the_wheel_scrolls_the_playlist_and_sets_the_volume() {
        let mut ui = Ui::demo();
        ui.wheel(Playlist, -2);
        assert_eq!(ui.pl_scroll, 2);
        ui.wheel(Playlist, -9);
        assert_eq!(ui.pl_scroll, ui.max_scroll());
        ui.wheel(Playlist, 10);
        assert_eq!(ui.pl_scroll, 0);
        ui.volume = 0.5;
        ui.wheel(Main, 2);
        assert!((ui.volume - 0.54).abs() < 1e-6);
        ui.wheel(Main, -100);
        assert_eq!(ui.volume, 0.0);
        ui.wheel(Eq, 3);
        assert_eq!(ui.volume, 0.0);
    }

    #[test]
    fn tags_read_late_find_their_entry_after_it_moved() {
        let mut ui = Ui::new();
        let _ = ui.add_files(["/m/a.mp3", "/m/b.mp3"].map(PathBuf::from));
        let _ = ui.key(KeyCmd::List(ListCmd::Reverse));
        let info = TrackInfo {
            title: Some("Band - A".into()),
            ..TrackInfo::default()
        };
        ui.set_track_info(0, Path::new("/m/a.mp3"), &info);
        assert_eq!(ui.tracks[1].title, "Band - A");
        assert_eq!(ui.tracks[0].title, "b");
    }

    #[test]
    fn dead_files_are_removed() {
        let mut ui = Ui::demo();
        ui.remove_missing(|p| p.ends_with("03.mp3"));
        assert_eq!(ui.tracks.len(), 1);
        assert_eq!(ui.tracks[0].title, "Justin F. - Rolling Llama (Radio Edit)");
        // Whether a stream is there can only be found out by playing it.
        let _ = ui.add_tracks([Track::from_url("http://radio.example/".into())]);
        ui.remove_missing(|_| false);
        assert_eq!(ui.tracks.len(), 1);
        assert!(ui.tracks[0].location.is_url());
    }

    fn radio() -> Ui {
        let mut ui = Ui::new();
        let _ = ui.add_tracks([
            Track::from_url("http://radio.example/live".into()),
            Track {
                title: "My Station".into(),
                seconds: None,
                location: Location::Url("https://other.example/".into()),
            },
        ]);
        ui
    }

    fn window_title(ui: &Ui) -> String {
        let mut s = String::new();
        ui.write_window_title(&mut s);
        s
    }

    #[test]
    fn a_stream_shows_its_progress_its_station_and_its_songs() {
        let mut ui = radio();
        let now = ui.start;
        let mut actions = Vec::new();
        ui.play_track(0, &mut actions);
        assert_eq!(actions, [audio(Transport::Open(0))]);
        assert!(ui.is_working() && !ui.shows_position());
        assert!(
            ui.marquee_text(now)
                .starts_with("[Connecting] 1. http://radio")
        );
        let event = |ui: &mut Ui, e| {
            let mut actions = Vec::new();
            ui.audio_event(e, &mut actions);
            assert!(actions.is_empty(), "{actions:?}");
        };
        event(&mut ui, Event::Status(StreamStatus::Buffering(40)));
        assert!(ui.marquee_text(now).starts_with("[Buffering: 40%] 1. "));
        assert_eq!(ui.title(), "1. http://radio.example/live");
        let info = TrackInfo {
            sample_rate: 44_100,
            channels: 2,
            kbps: Some(128),
            ..TrackInfo::default()
        };
        event(&mut ui, Event::Started(info.clone()));
        assert_eq!((ui.khz, ui.kbps, ui.stereo), (44, 128, true));
        assert_eq!(ui.tracks[0].seconds, None);
        let station = Station {
            name: Some("Radio Llama".into()),
            ..Station::default()
        };
        event(&mut ui, Event::Station(Box::new(station.clone())));
        event(&mut ui, Event::Status(StreamStatus::Playing));
        assert!(!ui.is_working());
        assert_eq!(
            ui.tracks[0].title, "Radio Llama",
            "the entry takes its name"
        );
        assert_eq!(ui.marquee_text(now), "1. Radio Llama");
        event(&mut ui, Event::StreamTitle("Band - Song".into()));
        assert_eq!(ui.stream_song(), Some("Band - Song"));
        assert_eq!(ui.marquee_text(now), "1. Band - Song");
        assert_eq!(window_title(&ui), "1. Band - Song - NeoAmp");
        assert_eq!(
            ui.tracks[0].title, "Radio Llama",
            "the entry keeps the station"
        );
        let details = ui.stream_details(&ui.tracks[0].location.clone()).unwrap();
        assert_eq!(details.station, Some(station));
        assert_eq!(details.song.as_deref(), Some("Band - Song"));
        assert_eq!(details.info, Some(info));
        assert_eq!(
            ui.stream_details(&ui.tracks[1].location.clone()),
            Some(StreamDetails::default()),
            "a stream not playing has told nothing"
        );
        assert_eq!(ui.stream_details(&Location::File("/a.mp3".into())), None);
        event(&mut ui, Event::StreamTitle(String::new()));
        assert_eq!(ui.title(), "1. Radio Llama");
        // Streams cannot seek.
        assert!(ui.key(KeyCmd::SeekForward).is_empty());
        assert!(ui.key(KeyCmd::SeekBack).is_empty());
        let _ = ui.key(KeyCmd::Stop);
        assert_eq!(ui.marquee_text(now), "1. Radio Llama");
        assert!(ui.stream_song().is_none());
    }

    #[test]
    fn a_stream_titled_by_its_playlist_keeps_its_title() {
        let mut ui = radio();
        let mut actions = Vec::new();
        ui.play_track(1, &mut actions);
        let station = Station {
            name: Some("Server's own name".into()),
            ..Station::default()
        };
        ui.audio_event(Event::Station(Box::new(station)), &mut actions);
        assert_eq!(ui.tracks[1].title, "My Station");
        // A station's news about a stream no longer playing changes nothing.
        let _ = ui.key(KeyCmd::Stop);
        ui.audio_event(Event::StreamTitle("Late".into()), &mut actions);
        ui.audio_event(Event::Status(StreamStatus::Reconnecting), &mut actions);
        assert_eq!(ui.marquee_text(ui.start), "2. My Station");
    }

    #[test]
    fn a_stream_that_cannot_be_kept_going_gives_way_to_the_next_entry() {
        let mut ui = radio();
        let mut actions = Vec::new();
        ui.play_track(0, &mut actions);
        actions.clear();
        ui.audio_event(Event::Failed("gave up".into()), &mut actions);
        assert_eq!(actions, [audio(Transport::Open(1))]);
        assert!(
            ui.marquee_text(ui.start)
                .starts_with("[Connecting] 2. My Station")
        );
    }
}
