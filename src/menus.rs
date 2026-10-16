//! NeoAmp's menus, after Winamp 2.x's: what each holds, built afresh from the current state
//! whenever one opens (so check marks are always right), and the commands their items run.
//! Items for features NeoAmp does not have yet are left out rather than greyed.

use std::path::Path;

use crate::eqf;
use crate::library::Entry;
use crate::menu::{Item, escape};
use crate::playlist::SortKey;
use crate::shortcut::{Key, Shortcut};
use crate::ui::{InfoOf, KeyCmd, ListCmd, Ui};
use crate::vis::{AnalyzerStyle, Bands, Falloff, ScopeStyle, VisMode};
use crate::zoom::Zoom;

/// Which menu to show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKind {
    /// The title bar's options button, Alt+F, and a right-click anywhere but the visualizer.
    Main,
    /// The clutter bar's O.
    Options,
    /// The clutter bar's V, and a right-click on the visualizer.
    Vis,
    /// The equalizer's PRESETS button.
    EqPresets,
    /// Alt+S.
    Skins,
    /// The playlist's MISC > SORT LIST.
    PlaylistSort,
    /// The playlist's REM > MISC REM.
    PlaylistRemove,
}

/// What a menu item does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Command {
    /// What a keyboard shortcut does.
    Key(KeyCmd),
    /// Count the time down (`true`) or up.
    TimeRemaining(bool),
    Zoom(Zoom),
    VisMode(VisMode),
    AnalyzerStyle(AnalyzerStyle),
    Bands(Bands),
    TogglePeaks,
    Falloff(Falloff),
    PeakFalloff(Falloff),
    ScopeStyle(ScopeStyle),
    /// Load built-in EQ preset `n` (see [`eqf::built_in`]).
    Preset(usize),
    /// Every equalizer band and the preamp back to 0 dB.
    ResetEq,
    Exit,
    /// A command that needs the files and dialogs of the window layer.
    App(AppCommand),
}

/// Menu commands the window layer carries out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppCommand {
    BaseSkin,
    /// Skin `n` of [`Library::skins`].
    Skin(usize),
    /// Pick a skin file with the platform's dialog.
    OpenSkin,
    /// Preset file `n` of [`Library::presets`].
    UserPreset(usize),
    /// Pick a preset file with the platform's dialog.
    PresetFromFile,
    /// Save the equalizer as a preset file, named in the platform's dialog.
    SavePreset,
    /// Keep the equalizer as the current file's auto-load preset.
    SaveAutoPreset,
    /// Forget the current file's auto-load preset.
    DeleteAutoPreset,
    /// Pick files (or playlists) to add to the playlist.
    AddFiles,
    /// Pick a folder whose files join the playlist.
    AddFolder,
    /// Ask for a URL to play in place of the playlist.
    PlayUrl,
    /// Ask for a URL to add to the playlist.
    AddUrl,
    /// Pick a playlist file to replace the playlist.
    LoadList,
    /// Save the playlist to a file, named in the platform's dialog.
    SaveList,
    /// Show the Jump to file dialog.
    JumpToFile,
    /// Show the File info dialog.
    FileInfo(InfoOf),
}

/// What the menus list from disk, as it was when the menu opened.
#[derive(Clone, Copy, Debug, Default)]
pub struct Library<'a> {
    pub skins: &'a [Entry],
    /// The skin shown; `None` is the base skin.
    pub skin: Option<&'a Path>,
    pub presets: &'a [Entry],
    /// The current file has an auto-load preset.
    pub has_auto_preset: bool,
}

/// The items of menu `kind`.
#[must_use]
pub fn build(kind: MenuKind, ui: &Ui, lib: &Library<'_>) -> Vec<Item<Command>> {
    match kind {
        MenuKind::Main => main_menu(ui, lib),
        MenuKind::Options => options(ui),
        MenuKind::Vis => visualization(ui),
        MenuKind::EqPresets => presets(ui, lib),
        MenuKind::Skins => skins(lib),
        MenuKind::PlaylistSort => playlist_sort(),
        MenuKind::PlaylistRemove => playlist_remove(),
    }
}

fn list(label: &'static str, cmd: ListCmd, shortcut: Shortcut) -> Item<Command> {
    key(label, KeyCmd::List(cmd), shortcut)
}

fn playlist_sort() -> Vec<Item<Command>> {
    vec![
        list(
            "Sort list by &title",
            ListCmd::Sort(SortKey::Title),
            Shortcut::char('1').ctrl().shift(),
        ),
        list(
            "Sort list by &filename",
            ListCmd::Sort(SortKey::FileName),
            Shortcut::char('2').ctrl().shift(),
        ),
        list(
            "Sort list by &path and filename",
            ListCmd::Sort(SortKey::Path),
            Shortcut::char('3').ctrl().shift(),
        ),
        Item::separator(),
        list(
            "&Reverse list",
            ListCmd::Reverse,
            Shortcut::char('R').ctrl(),
        ),
        list(
            "R&andomize list",
            ListCmd::Randomize,
            Shortcut::char('R').ctrl().shift(),
        ),
    ]
}

fn playlist_remove() -> Vec<Item<Command>> {
    vec![list(
        "Remove all &dead files",
        ListCmd::RemoveDead,
        Shortcut::new(Key::Delete).alt(),
    )]
}

fn key(label: &'static str, key: KeyCmd, shortcut: Shortcut) -> Item<Command> {
    Item::new(label, Command::Key(key)).shortcut(shortcut.to_string())
}

fn main_menu(ui: &Ui, lib: &Library<'_>) -> Vec<Item<Command>> {
    vec![
        key("&Play File...", KeyCmd::OpenFiles, Shortcut::char('L')),
        key(
            "Play &Folder...",
            KeyCmd::OpenFolder,
            Shortcut::char('L').shift(),
        ),
        key("Play &URL...", KeyCmd::OpenUrl, Shortcut::char('L').ctrl()),
        Item::separator(),
        key(
            "Playlist &Editor",
            KeyCmd::TogglePl,
            Shortcut::char('E').alt(),
        )
        .checked(ui.pl_open),
        key("E&qualizer", KeyCmd::ToggleEq, Shortcut::char('G').alt()).checked(ui.eq_open),
        Item::separator(),
        Item::submenu("&Skins", skins(lib)),
        Item::submenu("&Options", options(ui)),
        Item::submenu("Play&back", playback()),
        Item::submenu("&Visualization", visualization(ui)),
        Item::separator(),
        Item::new("E&xit", Command::Exit),
    ]
}

fn options(ui: &Ui) -> Vec<Item<Command>> {
    let toggles = format!("({} toggles)", Shortcut::char('T').ctrl());
    vec![
        Item::new("Time &elapsed", Command::TimeRemaining(false))
            .shortcut(toggles.clone())
            .chosen(!ui.remaining_mode),
        Item::new("Time re&maining", Command::TimeRemaining(true))
            .shortcut(toggles)
            .chosen(ui.remaining_mode),
        Item::separator(),
        key(
            "&Always On Top",
            KeyCmd::ToggleAlwaysOnTop,
            Shortcut::char('A').ctrl(),
        )
        .checked(ui.always_on_top),
        key(
            "&Double Size",
            KeyCmd::ToggleDoubleSize,
            Shortcut::char('D').ctrl(),
        )
        .checked(ui.double_size),
        Item::submenu("&Zoom", zoom(ui)),
        Item::separator(),
        key("&Repeat", KeyCmd::ToggleRepeat, Shortcut::char('R')).checked(ui.repeat),
        key("S&huffle", KeyCmd::ToggleShuffle, Shortcut::char('S')).checked(ui.shuffle),
    ]
}

fn zoom(ui: &Ui) -> Vec<Item<Command>> {
    let steps = std::iter::successors(Some(Zoom::MIN), |z| (*z != Zoom::MAX).then(|| z.larger()));
    let mut items: Vec<Item<Command>> = steps
        .map(|z| Item::new(z.to_string(), Command::Zoom(z)).chosen(ui.zoom == z))
        .collect();
    items.extend([
        Item::separator(),
        key("Zoom &In", KeyCmd::ZoomIn, Shortcut::char('=').ctrl()),
        key("Zoom &Out", KeyCmd::ZoomOut, Shortcut::char('-').ctrl()),
        Item::new(
            format!("&Default ({})", ui.default_zoom),
            Command::Key(KeyCmd::ZoomReset),
        )
        .shortcut(Shortcut::char('0').ctrl().to_string()),
    ]);
    items
}

fn playback() -> Vec<Item<Command>> {
    vec![
        key("Pre&vious", KeyCmd::Prev, Shortcut::char('Z')),
        key("&Play", KeyCmd::Play, Shortcut::char('X')),
        key("P&ause", KeyCmd::Pause, Shortcut::char('C')),
        key("&Stop", KeyCmd::Stop, Shortcut::char('V')),
        key(
            "Stop w/ fade&out",
            KeyCmd::FadeOut,
            Shortcut::char('V').shift(),
        ),
        key("&Next", KeyCmd::Next, Shortcut::char('B')),
        Item::separator(),
        key(
            "&Back 5 seconds",
            KeyCmd::SeekBack,
            Shortcut::new(Key::Left),
        ),
        key(
            "&Fwd 5 seconds",
            KeyCmd::SeekForward,
            Shortcut::new(Key::Right),
        ),
        Item::separator(),
        key("&Jump to file...", KeyCmd::JumpToFile, Shortcut::char('J')),
    ]
}

fn visualization(ui: &Ui) -> Vec<Item<Command>> {
    let a = ui.vis.analyzer;
    let mode = |label, m| Item::new(label, Command::VisMode(m)).chosen(ui.vis.mode == m);
    let style = |label, s| Item::new(label, Command::AnalyzerStyle(s)).chosen(a.style == s);
    let bands = |label, b| Item::new(label, Command::Bands(b)).chosen(a.bands == b);
    let scope = |label, s| Item::new(label, Command::ScopeStyle(s)).chosen(ui.vis.scope_style == s);
    vec![
        mode("&Analyzer", VisMode::Analyzer),
        mode("&Oscilloscope", VisMode::Oscilloscope),
        mode("O&ff", VisMode::Off),
        Item::separator(),
        Item::submenu(
            "Analyzer o&ptions",
            vec![
                style("&Normal style", AnalyzerStyle::Normal),
                style("&Fire style", AnalyzerStyle::Fire),
                style("&Line style", AnalyzerStyle::Line),
                Item::separator(),
                Item::new("&Peaks", Command::TogglePeaks).checked(a.peaks),
                Item::separator(),
                bands("&Thin bands", Bands::Thin),
                bands("T&hick bands", Bands::Thick),
            ],
        ),
        Item::submenu(
            "Oscilloscope op&tions",
            vec![
                scope("&Dot scope", ScopeStyle::Dots),
                scope("&Line scope", ScopeStyle::Lines),
                scope("&Solid scope", ScopeStyle::Solid),
            ],
        ),
        Item::separator(),
        Item::submenu("Analyzer fa&lloff", falloff(a.falloff, Command::Falloff)),
        Item::submenu(
            "P&eaks falloff",
            falloff(a.peak_falloff, Command::PeakFalloff),
        ),
    ]
}

fn falloff(current: Falloff, command: fn(Falloff) -> Command) -> Vec<Item<Command>> {
    Falloff::ALL
        .into_iter()
        .map(|f| {
            let label = match f {
                Falloff::Slowest => "Slo&west",
                Falloff::Slow => "&Slow",
                Falloff::Moderate => "&Moderate",
                Falloff::Fast => "&Fast",
                Falloff::Fastest => "Fas&test",
            };
            Item::new(label, command(f)).chosen(f == current)
        })
        .collect()
}

fn presets(ui: &Ui, lib: &Library<'_>) -> Vec<Item<Command>> {
    let mut load: Vec<Item<Command>> = eqf::built_in_names()
        .enumerate()
        .map(|(i, name)| Item::new(escape(name), Command::Preset(i)))
        .collect();
    if !lib.presets.is_empty() {
        load.push(Item::separator());
        load.extend(
            lib.presets
                .iter()
                .enumerate()
                .map(|(i, e)| Item::new(escape(&e.name), Command::App(AppCommand::UserPreset(i)))),
        );
    }
    load.extend([
        Item::separator(),
        Item::new(
            "From .&eqf file...",
            Command::App(AppCommand::PresetFromFile),
        ),
    ]);
    vec![
        Item::submenu("&Load", load),
        Item::submenu(
            "&Save",
            vec![
                Item::new("&Preset...", Command::App(AppCommand::SavePreset)),
                Item::new(
                    "&Auto-load preset",
                    Command::App(AppCommand::SaveAutoPreset),
                )
                .enabled(ui.track().is_some()),
            ],
        ),
        Item::submenu(
            "&Delete",
            vec![
                Item::new(
                    "&Auto-load preset",
                    Command::App(AppCommand::DeleteAutoPreset),
                )
                .enabled(lib.has_auto_preset),
            ],
        ),
        Item::separator(),
        Item::new("&Reset", Command::ResetEq),
    ]
}

fn skins(lib: &Library<'_>) -> Vec<Item<Command>> {
    let mut items = vec![
        Item::new("&Open Skin...", Command::App(AppCommand::OpenSkin)),
        Item::separator(),
        Item::new("<&Base Skin>", Command::App(AppCommand::BaseSkin)).chosen(lib.skin.is_none()),
    ];
    items.extend(lib.skins.iter().enumerate().map(|(i, e)| {
        Item::new(escape(&e.name), Command::App(AppCommand::Skin(i)))
            .chosen(lib.skin == Some(e.path.as_path()))
    }));
    items
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::menu::{Kind, Mark};
    use std::path::PathBuf;

    fn find<'a>(items: &'a [Item<Command>], label: &str) -> &'a Item<Command> {
        items.iter().find(|it| it.label() == label).unwrap()
    }

    fn submenu<'a>(items: &'a [Item<Command>], label: &str) -> &'a [Item<Command>] {
        match find(items, label).kind() {
            Kind::Submenu(sub) => sub,
            other => panic!("{label} is {other:?}"),
        }
    }

    #[test]
    fn the_main_menu_shows_which_windows_are_open() {
        let mut ui = Ui::new();
        ui.eq_open = false;
        let items = build(MenuKind::Main, &ui, &Library::default());
        assert_eq!(find(&items, "Playlist &Editor").mark(), Mark::Check);
        assert_eq!(find(&items, "E&qualizer").mark(), Mark::None);
        assert_eq!(find(&items, "E&xit").kind(), &Kind::Command(Command::Exit));
        let options = submenu(&items, "&Options");
        assert_eq!(find(options, "Time &elapsed").mark(), Mark::Bullet);
        let zoom = submenu(options, "&Zoom");
        assert_eq!(find(zoom, "1x").mark(), Mark::Bullet);
        assert_eq!(
            zoom.iter().filter(|it| it.mark() == Mark::Bullet).count(),
            1
        );
        assert_eq!(
            find(zoom, "3x").kind(),
            &Kind::Command(Command::Zoom(Zoom::MAX))
        );
    }

    #[test]
    fn the_visualization_menu_marks_the_current_options() {
        let mut ui = Ui::new();
        ui.vis.mode = VisMode::Oscilloscope;
        ui.vis.analyzer.peaks = false;
        ui.vis.analyzer.peak_falloff = Falloff::Fastest;
        let items = build(MenuKind::Vis, &ui, &Library::default());
        assert_eq!(find(&items, "&Oscilloscope").mark(), Mark::Bullet);
        assert_eq!(find(&items, "&Analyzer").mark(), Mark::None);
        let options = submenu(&items, "Analyzer o&ptions");
        assert_eq!(find(options, "&Peaks").mark(), Mark::None);
        assert_eq!(find(options, "T&hick bands").mark(), Mark::Bullet);
        let peaks = submenu(&items, "P&eaks falloff");
        assert_eq!(find(peaks, "Fas&test").mark(), Mark::Bullet);
        assert_eq!(
            find(peaks, "&Slow").kind(),
            &Kind::Command(Command::PeakFalloff(Falloff::Slow))
        );
    }

    #[test]
    fn presets_list_built_ins_then_the_users_and_need_a_track_for_auto_load() {
        let ui = Ui::new();
        let mine = [Entry {
            name: "Mine & Yours".into(),
            path: PathBuf::from("Mine & Yours.eqf"),
        }];
        let lib = Library {
            presets: &mine,
            ..Library::default()
        };
        let items = build(MenuKind::EqPresets, &ui, &lib);
        let load = submenu(&items, "&Load");
        assert_eq!(load[0].label(), "Classical");
        assert_eq!(
            find(load, "Full Bass && Treble").kind(),
            &Kind::Command(Command::Preset(4))
        );
        assert_eq!(
            find(load, "Mine && Yours").kind(),
            &Kind::Command(Command::App(AppCommand::UserPreset(0)))
        );
        let save = submenu(&items, "&Save");
        assert!(!find(save, "&Auto-load preset").is_enabled(), "no track");
        let delete = submenu(&items, "&Delete");
        assert!(!find(delete, "&Auto-load preset").is_enabled());
    }

    #[test]
    fn the_playlist_menus_sort_and_remove() {
        let ui = Ui::new();
        let lib = Library::default();
        let sort = build(MenuKind::PlaylistSort, &ui, &lib);
        assert_eq!(
            find(&sort, "Sort list by &filename").kind(),
            &Kind::Command(Command::Key(KeyCmd::List(ListCmd::Sort(SortKey::FileName))))
        );
        assert_eq!(
            find(&sort, "R&andomize list").kind(),
            &Kind::Command(Command::Key(KeyCmd::List(ListCmd::Randomize)))
        );
        let remove = build(MenuKind::PlaylistRemove, &ui, &lib);
        assert_eq!(
            remove[0].kind(),
            &Kind::Command(Command::Key(KeyCmd::List(ListCmd::RemoveDead)))
        );
        let main = build(MenuKind::Main, &ui, &lib);
        let playback = submenu(&main, "Play&back");
        assert_eq!(
            find(playback, "&Jump to file...").kind(),
            &Kind::Command(Command::Key(KeyCmd::JumpToFile))
        );
    }

    #[test]
    fn the_skins_menu_marks_the_skin_shown() {
        let skins = [
            Entry {
                name: "Classic".into(),
                path: PathBuf::from("/skins/Classic.wsz"),
            },
            Entry {
                name: "Other".into(),
                path: PathBuf::from("/skins/Other.wsz"),
            },
        ];
        let lib = Library {
            skins: &skins,
            skin: Some(Path::new("/skins/Other.wsz")),
            ..Library::default()
        };
        let items = build(MenuKind::Skins, &Ui::new(), &lib);
        assert_eq!(find(&items, "<&Base Skin>").mark(), Mark::None);
        assert_eq!(find(&items, "Classic").mark(), Mark::None);
        assert_eq!(find(&items, "Other").mark(), Mark::Bullet);
        let base = build(MenuKind::Skins, &Ui::new(), &Library::default());
        assert_eq!(find(&base, "<&Base Skin>").mark(), Mark::Bullet);
    }
}
