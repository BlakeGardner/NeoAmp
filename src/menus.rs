//! NeoAmp's menus, after Winamp 2.x's: what each holds, built afresh from the current state
//! whenever one opens (so check marks are always right), and the commands their items run.
//! Items for features NeoAmp does not have yet are left out rather than greyed.

use crate::audio::OutputDevice;
use crate::eqf;
use crate::library::Entry;
use crate::menu::{Item, escape};
use crate::milkdrop::built_in::Category;
use crate::milkdrop::visualizer::Change;
use crate::playlist::SortKey;
use crate::settings::SkinChoice;
use crate::shortcut::{Key, Shortcut};
use crate::ui::{InfoOf, KeyCmd, ListCmd, MilkdropCmd, Ui};
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
    /// A right-click in MilkDrop's window.
    Milkdrop,
    /// MilkDrop's L.
    MilkdropPresets,
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
    /// Show NeoAmp's own skin.
    NeoAmpSkin,
    /// Show the Winamp base skin.
    WinampSkin,
    /// Skin `n` of [`Library::skins`].
    Skin(usize),
    /// Pick a skin file with the platform's dialog.
    OpenSkin,
    /// Browse the Winamp Skin Museum.
    SkinMuseum,
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
    /// Play through the system's default output device.
    DefaultOutput,
    /// Play through device `n` of [`Library::devices`].
    Output(usize),
    /// The chosen output device, which is not connected: it stays chosen.
    KeepOutput,
}

/// What the menus list from disk, as it was when the menu opened.
#[derive(Clone, Copy, Debug)]
pub struct Library<'a> {
    pub skins: &'a [Entry],
    /// The skin shown.
    pub skin: &'a SkinChoice,
    pub presets: &'a [Entry],
    /// The current file has an auto-load preset.
    pub has_auto_preset: bool,
    /// The output devices that can be chosen by name; only listed for the menus that show
    /// them.
    pub devices: &'a [OutputDevice],
}

impl Default for Library<'_> {
    fn default() -> Self {
        Library {
            skins: &[],
            skin: &SkinChoice::NeoAmp,
            presets: &[],
            has_auto_preset: false,
            devices: &[],
        }
    }
}

/// The items of menu `kind`.
#[must_use]
pub fn build(kind: MenuKind, ui: &Ui, lib: &Library<'_>) -> Vec<Item<Command>> {
    match kind {
        MenuKind::Main => main_menu(ui, lib),
        MenuKind::Options => options(ui, lib),
        MenuKind::Vis => visualization(ui),
        MenuKind::EqPresets => presets(ui, lib),
        MenuKind::Skins => skins(lib),
        MenuKind::PlaylistSort => playlist_sort(),
        MenuKind::PlaylistRemove => playlist_remove(),
        MenuKind::Milkdrop => milkdrop(ui),
        MenuKind::MilkdropPresets => milkdrop_presets(ui),
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
        key(
            "&MilkDrop",
            KeyCmd::ToggleMilkdrop,
            Shortcut::char('K').ctrl().shift(),
        )
        .checked(ui.md_open),
        Item::separator(),
        Item::submenu("&Skins", skins(lib)),
        Item::submenu("&Options", options(ui, lib)),
        Item::submenu("Play&back", playback()),
        Item::submenu("&Visualization", visualization(ui)),
        Item::separator(),
        Item::new("E&xit", Command::Exit),
    ]
}

/// Whether menu `kind` lists the output devices, which takes a moment to ask the system.
#[must_use]
pub fn lists_devices(kind: MenuKind) -> bool {
    matches!(kind, MenuKind::Main | MenuKind::Options)
}

fn options(ui: &Ui, lib: &Library<'_>) -> Vec<Item<Command>> {
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
        Item::separator(),
        Item::submenu("&Output Device", output_devices(ui, lib)),
    ]
}

/// The system's default, then every device by name. A chosen device that is not connected
/// is listed too, so the menu shows that NeoAmp goes back to it when it is.
fn output_devices(ui: &Ui, lib: &Library<'_>) -> Vec<Item<Command>> {
    let chosen = ui.output.as_ref();
    let mut items = vec![
        Item::new("&System Default", Command::App(AppCommand::DefaultOutput))
            .chosen(chosen.is_none()),
        Item::separator(),
    ];
    items.extend(lib.devices.iter().enumerate().map(|(i, d)| {
        Item::new(escape(&d.name), Command::App(AppCommand::Output(i)))
            .chosen(chosen.is_some_and(|c| c.id == d.id))
    }));
    if let Some(c) = chosen.filter(|c| lib.devices.iter().all(|d| d.id != c.id)) {
        let name = if c.name.is_empty() { &c.id } else { &c.name };
        items.push(
            Item::new(
                format!("{} (not connected)", escape(name)),
                Command::App(AppCommand::KeepOutput),
            )
            .chosen(true)
            .enabled(false),
        );
    }
    if lib.devices.is_empty() && chosen.is_none() {
        items.pop();
    }
    items
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

fn md(label: &'static str, cmd: MilkdropCmd, shortcut: Shortcut) -> Item<Command> {
    key(label, KeyCmd::Milkdrop(cmd), shortcut)
}

/// The key that locks the preset: Scroll Lock, or on macOS F14, which sits where Scroll Lock
/// does on Apple's full-size keyboards.
#[must_use]
pub fn lock_key() -> Shortcut {
    if cfg!(target_os = "macos") {
        Shortcut::new(Key::F(14))
    } else {
        Shortcut::new(Key::ScrollLock)
    }
}

fn milkdrop(ui: &Ui) -> Vec<Item<Command>> {
    let v = &ui.milkdrop;
    let change = |c| MilkdropCmd::Change(c);
    vec![
        md(
            "&Next preset",
            change(Change::Next),
            Shortcut::new(Key::Space),
        ),
        md(
            "&Previous preset",
            change(Change::Previous),
            Shortcut::new(Key::Backspace),
        ),
        md("&Hard cut", change(Change::HardCut), Shortcut::char('H')),
        Item::submenu("Pre&sets", milkdrop_presets(ui)),
        Item::separator(),
        md(
            "&Random order",
            MilkdropCmd::ToggleRandom,
            Shortcut::char('R'),
        )
        .checked(v.random()),
        md("&Lock preset", MilkdropCmd::ToggleLock, lock_key()).checked(v.locked()),
        Item::separator(),
        md(
            "&Fullscreen",
            MilkdropCmd::ToggleFullscreen,
            Shortcut::new(Key::Enter).alt(),
        )
        .checked(ui.md_fullscreen),
        key(
            "&Close",
            KeyCmd::ToggleMilkdrop,
            Shortcut::char('K').ctrl().shift(),
        ),
    ]
}

/// More of the user's presets than this are grouped by their first letter.
const UNGROUPED_PRESETS: usize = 40;

/// The presets to choose from: the built-in ones in a submenu for each of their categories,
/// then the user's in one of their own, grouped by first letter when there are many. The one
/// showing is marked, and so is the submenu it is in.
fn milkdrop_presets(ui: &Ui) -> Vec<Item<Command>> {
    let presets = ui.milkdrop.presets();
    let current = ui.milkdrop.current();
    let item = |i: usize, name: &str| {
        Item::new(
            escape(name),
            Command::Key(KeyCmd::Milkdrop(MilkdropCmd::Change(Change::Select(i)))),
        )
        .chosen(current == Some(i))
    };
    let showing_category = current.and_then(|i| presets.category(i));
    let mut items: Vec<Item<Command>> = Category::ALL
        .into_iter()
        .filter_map(|c| {
            let group: Vec<Item<Command>> = presets
                .names()
                .enumerate()
                .filter(|&(i, _)| presets.category(i) == Some(c))
                .map(|(i, name)| item(i, name))
                .collect();
            (!group.is_empty()).then(|| {
                Item::submenu(format!("&{}", c.name()), group).chosen(showing_category == Some(c))
            })
        })
        .collect();
    let built_in = presets.built_in_count();
    let users: Vec<(usize, &str)> = presets.names().enumerate().skip(built_in).collect();
    if users.is_empty() {
        return items;
    }
    if !items.is_empty() {
        items.push(Item::separator());
    }
    let showing_users = current.is_some_and(|i| i >= built_in);
    items.push(Item::submenu("&Your presets", users_presets(&users, item)).chosen(showing_users));
    items
}

/// The user's presets, given with their places in the list: as they are, or grouped by
/// first letter when there are more than [`UNGROUPED_PRESETS`].
fn users_presets(
    users: &[(usize, &str)],
    item: impl Fn(usize, &str) -> Item<Command>,
) -> Vec<Item<Command>> {
    if users.len() <= UNGROUPED_PRESETS {
        return users.iter().map(|&(i, name)| item(i, name)).collect();
    }
    let letter = |name: &str| {
        name.chars()
            .next()
            .filter(char::is_ascii_alphabetic)
            .map_or('#', |c| c.to_ascii_uppercase())
    };
    // Sorted by name, so each letter's presets are together.
    let mut items = Vec::new();
    let mut rest = users;
    while let Some(&(_, first)) = rest.first() {
        let l = letter(first);
        let n = rest
            .iter()
            .take_while(|(_, name)| letter(name) == l)
            .count();
        let (group, after) = rest.split_at(n.max(1));
        items.push(Item::submenu(
            format!("&{l}"),
            group.iter().map(|&(i, name)| item(i, name)).collect(),
        ));
        rest = after;
    }
    items
}

fn skins(lib: &Library<'_>) -> Vec<Item<Command>> {
    let mut items = vec![
        Item::new("&Open Skin...", Command::App(AppCommand::OpenSkin)),
        Item::new("Skin &Museum...", Command::App(AppCommand::SkinMuseum)),
        Item::separator(),
        Item::new("<&NeoAmp Skin>", Command::App(AppCommand::NeoAmpSkin))
            .chosen(lib.skin.is_neoamp()),
        Item::new("<&Winamp Base Skin>", Command::App(AppCommand::WinampSkin))
            .chosen(*lib.skin == SkinChoice::Winamp),
    ];
    items.extend(lib.skins.iter().enumerate().map(|(i, e)| {
        Item::new(escape(&e.name), Command::App(AppCommand::Skin(i)))
            .chosen(matches!(lib.skin, SkinChoice::File(path) if *path == e.path))
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
    fn the_output_menu_lists_the_devices_and_a_chosen_one_gone() {
        let devices = [
            OutputDevice {
                id: "pulseaudio:usb".into(),
                name: "USB Audio & More".into(),
            },
            OutputDevice {
                id: "pulseaudio:hdmi".into(),
                name: "HDMI".into(),
            },
        ];
        let lib = Library {
            devices: &devices,
            ..Library::default()
        };
        let mut ui = Ui::new();
        let items = build(MenuKind::Options, &ui, &lib);
        let output = submenu(&items, "&Output Device");
        assert_eq!(find(output, "&System Default").mark(), Mark::Bullet);
        assert_eq!(
            find(output, "USB Audio && More").kind(),
            &Kind::Command(Command::App(AppCommand::Output(0)))
        );
        ui.output = Some(crate::ui::OutputChoice {
            id: "pulseaudio:hdmi".into(),
            name: "HDMI".into(),
        });
        let items = build(MenuKind::Main, &ui, &lib);
        let output = submenu(submenu(&items, "&Options"), "&Output Device");
        assert_eq!(find(output, "&System Default").mark(), Mark::None);
        assert_eq!(find(output, "HDMI").mark(), Mark::Bullet);
        assert_eq!(output.len(), 4);
        ui.output = Some(crate::ui::OutputChoice {
            id: "pulseaudio:bluez".into(),
            name: "Earbuds".into(),
        });
        let items = build(MenuKind::Options, &ui, &lib);
        let gone = find(submenu(&items, "&Output Device"), "Earbuds (not connected)");
        assert_eq!(gone.mark(), Mark::Bullet);
        assert!(!gone.is_enabled());
        let items = build(MenuKind::Options, &Ui::new(), &Library::default());
        assert_eq!(submenu(&items, "&Output Device").len(), 1);
        assert!(lists_devices(MenuKind::Main) && !lists_devices(MenuKind::Skins));
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
        let other = SkinChoice::File(PathBuf::from("/skins/Other.wsz"));
        let lib = Library {
            skins: &skins,
            skin: &other,
            ..Library::default()
        };
        let items = build(MenuKind::Skins, &Ui::new(), &lib);
        assert_eq!(
            find(&items, "Skin &Museum...").kind(),
            &Kind::Command(Command::App(AppCommand::SkinMuseum))
        );
        assert_eq!(find(&items, "<&NeoAmp Skin>").mark(), Mark::None);
        assert_eq!(find(&items, "<&Winamp Base Skin>").mark(), Mark::None);
        assert_eq!(find(&items, "Classic").mark(), Mark::None);
        assert_eq!(find(&items, "Other").mark(), Mark::Bullet);
        let neoamp = build(MenuKind::Skins, &Ui::new(), &Library::default());
        assert_eq!(find(&neoamp, "<&NeoAmp Skin>").mark(), Mark::Bullet);
        assert_eq!(find(&neoamp, "<&Winamp Base Skin>").mark(), Mark::None);
        let lib = Library {
            skin: &SkinChoice::Winamp,
            ..lib
        };
        let winamp = build(MenuKind::Skins, &Ui::new(), &lib);
        assert_eq!(find(&winamp, "<&NeoAmp Skin>").mark(), Mark::None);
        assert_eq!(find(&winamp, "<&Winamp Base Skin>").mark(), Mark::Bullet);
        assert_eq!(find(&winamp, "Other").mark(), Mark::None);
    }

    #[test]
    fn milkdrops_menu_marks_its_options_and_the_preset_showing() {
        let mut ui = Ui::new();
        let main = build(MenuKind::Main, &ui, &Library::default());
        assert_eq!(find(&main, "&MilkDrop").mark(), Mark::None);
        ui.md_open = true;
        ui.tick(std::time::Instant::now());
        let main = build(MenuKind::Main, &ui, &Library::default());
        assert_eq!(find(&main, "&MilkDrop").mark(), Mark::Check);
        let items = build(MenuKind::Milkdrop, &ui, &Library::default());
        assert_eq!(find(&items, "&Random order").mark(), Mark::Check);
        assert_eq!(find(&items, "&Lock preset").mark(), Mark::None);
        assert_eq!(find(&items, "&Fullscreen").mark(), Mark::None);
        assert_eq!(
            find(&items, "&Hard cut").kind(),
            &Kind::Command(Command::Key(KeyCmd::Milkdrop(MilkdropCmd::Change(
                Change::HardCut
            ))))
        );
        // The presets come in a submenu for each category; the one showing is marked, and so
        // is its category's submenu, alone.
        let presets = submenu(&items, "Pre&sets");
        let labels: Vec<&str> = presets.iter().map(Item::label).collect();
        let categories: Vec<String> = Category::ALL
            .iter()
            .map(|c| format!("&{}", c.name()))
            .collect();
        assert_eq!(
            labels, categories,
            "no presets of the user's, so no submenu for them"
        );
        let shown = ui.milkdrop.current().unwrap();
        let category = ui.milkdrop.presets().category(shown).unwrap();
        let label = format!("&{}", category.name());
        assert_eq!(find(presets, &label).mark(), Mark::Bullet);
        let bullets =
            |items: &[Item<Command>]| items.iter().filter(|it| it.mark() == Mark::Bullet).count();
        assert_eq!(bullets(presets), 1);
        let group = submenu(presets, &label);
        let name = escape(ui.milkdrop.presets().name(shown).unwrap());
        assert_eq!(find(group, &name).mark(), Mark::Bullet);
        assert_eq!(bullets(group), 1);
        let listed: usize = Category::ALL
            .iter()
            .map(|c| submenu(presets, &format!("&{}", c.name())).len())
            .sum();
        assert_eq!(listed, crate::milkdrop::built_in::PRESETS.len());
    }

    #[test]
    fn many_presets_of_the_users_are_grouped_by_letter() {
        let mut ui = Ui::new();
        let dir = std::env::temp_dir().join(format!("neoamp-menu-milk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..UNGROUPED_PRESETS {
            for first in ["a", "B", "9"] {
                std::fs::write(dir.join(format!("{first}{i:02}.milk")), "").unwrap();
            }
        }
        let presets = crate::milkdrop::presets::Presets::with_folder(
            crate::milkdrop::built_in::PRESETS,
            &dir,
        );
        std::fs::remove_dir_all(&dir).unwrap();
        ui.milkdrop.set_presets(presets);
        let items = build(MenuKind::MilkdropPresets, &ui, &Library::default());
        let built_in = crate::milkdrop::built_in::PRESETS.len();
        let n = Category::ALL.len();
        assert_eq!(items[n].kind(), &Kind::Separator);
        assert_eq!(items.len(), n + 2);
        let users = submenu(&items, "&Your presets");
        let groups: Vec<&str> = users.iter().map(Item::label).collect();
        assert_eq!(groups, ["&#", "&A", "&B"]);
        assert_eq!(submenu(users, "&B").len(), UNGROUPED_PRESETS);
        assert_eq!(
            find(submenu(users, "&A"), "a00").kind(),
            &Kind::Command(Command::Key(KeyCmd::Milkdrop(MilkdropCmd::Change(
                Change::Select(built_in + UNGROUPED_PRESETS)
            ))))
        );
    }
}
