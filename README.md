# NeoAmp

NeoAmp is a music player in Rust with a pixel-exact reproduction of the
classic Winamp 2.x interface (main window, equalizer and playlist editor),
drawn with classic Winamp skins, that plays MP3, WAV, FLAC and Ogg Vorbis
files. The transport buttons, position bar, volume, balance and ten-band
equalizer all act on the audio, and the visualizer shows what is actually
playing.

NeoAmp is an independent project, not affiliated with or endorsed by the
makers of Winamp. Winamp is a trademark of its owner.

![NeoAmp's main window, equalizer and playlist](docs/screenshot@2x.png)

## How it works

Classic Winamp skins (`.wsz` files) are zip archives of fixed-size sprite
sheets (`main.bmp`, `cbuttons.bmp`, `titlebar.bmp`, ...) plus a few text
files. Winamp itself just blitted rectangles out of those sheets, so this
program does the same:

- `src/canvas.rs` is a small RGBA software framebuffer with clipped blits.
- `src/sprites.rs` holds the sprite coordinates and window layout, which are
  identical for every classic skin.
- `src/skin.rs` reads a `.wsz` (zip + BMP/PNG), `viscolor.txt`,
  `pledit.txt` and `region.txt`, and falls back per sheet to the embedded
  base skin for anything a skin leaves out.
- `src/main_window.rs`, `src/eq_window.rs`, `src/playlist_window.rs` draw the
  three windows; `src/ui.rs` holds state, hit testing and input.
- `src/app.rs` runs one borderless `winit` window per NeoAmp window, presents
  each framebuffer through `pixels` (wgpu) and drives the title-bar drags and
  docking. `src/upscale.rs` draws the framebuffer over the whole window (see
  [Scaling](#scaling)), and `src/zoom.rs` picks the zoom.
- `src/menu.rs` draws popup menus and handles the pointer and keyboard in
  them; `src/menus.rs` holds NeoAmp's own menus, and `src/popup.rs` gives each
  open menu its window (see [Menus](#menus)). `src/eqf.rs` reads and writes
  equalizer presets, and `src/library.rs` lists the skins and presets folders.
- `src/file_dialog.rs` shows the platform's file dialogs (see
  [Opening files](#opening-files)).
- `src/playlist.rs` sorts, moves and shuffles playlist entries;
  `src/playlist_file.rs` loads and saves playlist files through `src/m3u.rs`
  and `src/pls.rs` (see [Playlist](#playlist)).
- `src/jump.rs` is the Jump to file dialog and `src/jump_window.rs` its
  window; `src/classic.rs` holds the Windows 2000 colours and bevels it
  shares with the menus.
- `src/audio/` plays the music (see [Audio](#audio)), and `src/vis.rs` turns
  the samples being played into the spectrum analyzer and oscilloscope.
- `src/settings.rs` remembers the settings and the playlist between runs (see
  [Saved settings](#saved-settings)); `src/m3u.rs` reads and writes the
  playlist file.
- `src/lib.rs` exposes all of the above as a library; `src/main.rs` only
  parses the command line. `cargo test` covers layout, hit testing, input,
  playlist editing and playlist files, the skin parsers, the blitter, the
  dialogs and menus, the saved settings and playlist, and the audio pipeline
  (decoding, resampling, the equalizer, and the decoder and mixer threads
  working together).

Each NeoAmp window is its own borderless OS window. Grab a title bar to
drag one away; the main window carries whatever is docked to it, while the
equalizer and playlist leave on their own. Windows snap to each other's
edges and to the screen edges when they come within 10 pixels, like the
original. Resizing the playlist from its grip leaves the other windows where
they are, even ones docked to it, as Webamp does (how Winamp itself behaved
here is undocumented). `src/dock.rs` holds that geometry.

Docking needs to read and set window positions, which Wayland forbids, so
the app prefers the X11 backend (XWayland on a Wayland desktop) whenever
`DISPLAY` is set. Some compositors, COSMIC among them, also ignore moves of
managed X11 windows; the app notices this at startup and recreates its
windows as override-redirect windows, which are always placed exactly where
asked. Such windows have no taskbar entry, stay above other windows (like
Winamp's "always on top") and take keyboard focus when clicked.

- `NEOAMP_BACKEND=x11|wayland` forces a backend. On pure Wayland, title-bar
  drags are handed to the compositor and nothing snaps.
- `NEOAMP_UNMANAGED=1|0` forces or forbids override-redirect windows instead
  of probing.

## Audio

Playback follows Winamp's own input plugin, DSP, output plugin chain, spread
over three threads:

- A decoder thread reads the file with [symphonia](https://crates.io/crates/symphonia)
  (pure Rust: MP3, WAV, FLAC, Ogg Vorbis), converts it to stereo, resamples it
  to the sound card's rate with [rubato](https://crates.io/crates/rubato), and
  keeps about half a second queued in a lock-free ring buffer.
- The sound card's callback, through [cpal](https://crates.io/crates/cpal),
  takes audio from the ring and runs the equalizer, volume and balance. It
  also copies what it plays to the visualizer.
- The UI thread sends commands (open, seek, stop, pause) and reads back the
  position, bitrate and "track finished" events.

The callback never blocks or allocates, so the threads share only atomics and
ring buffers. Each command carries a generation number: before the decoder
queues audio for a seek or a new track, the callback drops whatever is still
queued from the old one and confirms, so stale audio never plays.

All three platforms behave the same:

- symphonia is pure Rust and needs no system libraries.
- cpal uses WASAPI on Windows, CoreAudio on macOS and ALSA on Linux, which
  reaches PipeWire and PulseAudio through their ALSA plugins.
- Resampling in the app means a 44.1 kHz file plays correctly on a 48 kHz
  device even where the system mixer would not convert it.

A background thread reads tags ("Artist - Title") and lengths for the
playlist as files are added.

## Scaling

Classic skins were drawn for one screen pixel per skin pixel on 800x600 and
1024x768 screens, which looks tiny on a modern monitor. The windows are zoomed
in half steps from 1x to 3x:

- **Default:** chosen from the height of the screen (in the desktop's logical
  pixels). 1080 lines stay at 1x, 1440 get 1.5x, 2160 get 2x. On a 27"
  1440p monitor, 1.5x is about the physical size Winamp had on a 15" CRT.
- **Ctrl+=** / **Ctrl+-** step the zoom, **Ctrl+0** goes back to the default,
  and `--scale 1|1.5|2|2.5|3` overrides it at startup. A zoom picked either
  way is remembered. At the default none is, so the next start picks one for
  whatever screen it finds.
- **Double size** (`D` in the clutter bar, Ctrl+D) doubles whatever the zoom
  is, as in Winamp, so 1.5x becomes 3x.
- The desktop's own scale factor applies on top, so a 200% desktop shows a 1x
  zoom at 2x.

Whole-number scales draw every skin pixel as an exact square of screen pixels.
Scales in between use a sharp-bilinear filter: each skin pixel keeps its flat
colour and only its edges blend over one screen pixel. Plain nearest-neighbour
at those scales would make some rows and columns a pixel wider than others,
which garbles the bitmap fonts. Windows are sized to whole screen pixels and the
image is stretched to fill them exactly, so docked windows meet without gaps.

## Menus

Winamp 2.x used the system's own menus, so NeoAmp draws its menus the way
Windows 2000 did, grey and bevelled, whatever skin is loaded:

- **Main menu**: the options button at the left of the main title bar,
  **Alt+F**, or a right-click on any window. It plays a file or a folder,
  shows or hides the equalizer and playlist, and holds the Skins, Options,
  Playback and Visualization submenus, and Exit.
- **Options**: the `O` in the clutter bar (also in the main menu). Time
  elapsed or remaining, always on top, double size, the zoom, repeat and
  shuffle.
- **Visualization**: the `V` in the clutter bar, or a right-click on the
  visualizer. The mode, and Winamp's analyzer and oscilloscope styles and
  falloff speeds.
- **EQ presets**: the equalizer's PRESETS button. Winamp's built-in presets,
  your own, `.eqf` files, auto-load presets and Reset (see
  [Saved settings](#saved-settings)).
- **Skins**: **Alt+S** (also in the main menu). The skins in NeoAmp's skins
  folder, the built-in base skin, and Open Skin... for any other.
- **Sort** and **Remove misc**: the playlist's MISC > SORT LIST and
  REM > MISC REM (see [Playlist](#playlist)).

The Playback submenu also holds Jump to file... (**J**).

The `A` in the clutter bar toggles always on top, and stays down while it is
on, as `D` does for double size. The `I` (file info) does nothing yet.

Menus open where Windows opened them: dropping down from the button that opens
them, or at the pointer for a right-click, turned the other way at the screen's
edge. Submenus open when the pointer rests on their item for a moment, or at
once on a click. The keyboard works as in Windows: the arrow keys move and open
and close submenus, Enter chooses, Escape closes, and a letter picks the item
it is underlined in (underlines appear once a key has been used). A click
anywhere else, or switching to another application, closes the menu. Menus grow
with the zoom, as Windows' grow with the desktop's scale, but not with double
size, which Winamp's never did; their text is drawn at that size, so it stays
sharp.

The text is Tahoma, Windows 2000's menu font, where the system has it (Windows
and macOS do, and so does Wine); elsewhere it is the playlist's font.

The menus list each item's keyboard shortcut the way the platform writes them:
"Ctrl+Shift+1" on Windows and Linux, "⇧⌘1" on macOS. Tahoma has no ⌘, ⌥ or ⇧,
so on macOS those come from Lucida Grande, Apple Symbols or Menlo, whichever is
found first with all of them; with none, they show as boxes.

Each open menu is a small window of its own, since a menu reaches past the
NeoAmp window it opens from. These windows are drawn in software with
[softbuffer](https://crates.io/crates/softbuffer), from the people behind
winit, because they must appear at once: setting up a GPU surface, as the main
windows use, can take a tenth of a second. softbuffer needs no system
libraries to build. Where the platforms differ:

- **X11** (including XWayland): menus are override-redirect windows, as other
  toolkits' menus are, so they land exactly where they are put. COSMIC's
  compositor redraws override-redirect XWayland windows only about once a
  second, so there the highlight may lag behind the pointer; this has not
  been tried on COSMIC yet.
- **Windows and macOS:** menus are borderless windows kept above the others;
  on Windows they stay off the taskbar. Neither has been tried yet.
- **Pure Wayland** lets no window choose its position, so menus open wherever
  the compositor puts them.

## Opening files

Eject (in the main window, or the small one in the playlist's bottom bar),
**L** or Play File... in the main menu opens the platform's own file dialog,
through [rfd](https://crates.io/crates/rfd). Pick one file or several (or a
playlist file): as with Winamp's "Open file(s)", they replace the playlist and
the first one plays. **Shift+L** or Play Folder... picks a folder instead,
whose playable files (sorted by name, subfolders included) replace the
playlist in the same way. The playlist's ADD menu adds files or a folder to
the end of the playlist instead, without interrupting what is playing, and
its LIST OPTS menu loads and saves playlist files (see [Playlist](#playlist)).
Cancelling changes nothing. The windows keep drawing while the dialog is open.
The same dialogs open skins and equalizer presets from the menus, starting in
NeoAmp's skins and presets folders.

The dialog starts on "All supported types" and also offers MP3, WAV, FLAC,
Ogg Vorbis and playlists one at a time. Saving a playlist offers M3U8, M3U and
PLS, and starts on `Playlist.m3u8`. The platforms differ a little:

- **Windows:** the standard Open dialog, with the file types in its drop-down.
- **macOS:** an open panel that slides down from the main window as a sheet.
  macOS panels have no file type drop-down, so it always shows every
  supported type.
- **Linux:** the desktop's own file chooser, reached through the XDG desktop
  portal (GNOME, KDE, COSMIC and most other desktops run one). The portal is
  reached over D-Bus with the system's `libdbus`, which is loaded at runtime,
  so building needs no extra packages. Some choosers match file types
  case-sensitively, so both `*.mp3` and `*.MP3` are listed; a mixed-case name
  such as `song.Mp3` may be hidden there. Without a portal, rfd runs `zenity`
  instead if it is installed. With neither, eject does nothing and rfd logs
  why.

Saving goes by the extension of the name given: `.pls` writes PLS, anything
else extended M3U. A name without an extension gets `.m3u8`, since not every
platform's save dialog adds the chosen type's extension.

## Playlist

The playlist editor works as Winamp 2.x's did.

**Selecting.** A click selects one entry, Shift-click selects everything from
the last entry clicked, and Ctrl-click (Cmd-click on macOS, where Ctrl-click
means right-click) adds or removes one entry. A double-click or Enter plays
an entry. The keyboard moves a cursor through the list: Up, Down, Page Up,
Page Down, Home and End, with Shift to select as it goes. These keys work
while the playlist window has the focus; in the other windows Up and Down
still set the volume.

**Moving.** Drag selected entries to move them together; dragging past the
top or bottom of the list scrolls it. Alt+Up and Alt+Down move the selection
one place. The mouse wheel scrolls the list (over the main window it sets the
volume).

**The bottom buttons.** ADD, REM, SEL, MISC and LIST OPTS each show a menu
drawn from the skin while held; let go over an item to choose it. Each menu's
bottom item sits over its button, so a plain click chooses that one, as in
Winamp: ADD adds files, REM removes the selection, SEL selects everything,
MISC does nothing (it is Winamp's misc options) and LIST OPTS loads a list.

| Menu | Items |
| --- | --- |
| ADD | URL (not yet: see Phase 6 of the roadmap), folder, files |
| REM | misc (opens a menu with Remove all dead files), all, crop, selected |
| SEL | invert, none, all |
| MISC | sort (opens a menu: by title, file name, or path; reverse; randomize), file info and misc options (neither does anything yet) |
| LIST OPTS | new (empties the list), save, load |

**Removing the track that plays.** As in Winamp, it plays on; when it ends,
the entry that followed it plays next (or playback stops if it was the last
and repeat is off).

**Shuffle.** With shuffle on, every entry plays once, in random order, before
any plays again. At the end of that round playback stops unless repeat is on,
in which case a new round starts. Next always moves on; Previous goes back
through the entries in the order they played.

**Playlist files.** NeoAmp reads and writes extended M3U (`.m3u`, `.m3u8`) and
Winamp's PLS (`.pls`), from LIST OPTS, the file dialogs, the command line and
drag and drop. Loading one from LIST OPTS or the eject dialog replaces the
playlist and plays it; dropping one adds its entries. Entries whose length the
file gives keep its title and length; the others have their tags read. Since
playlists often come from Windows:

- Lines may end in `\r\n` or `\n`.
- Text that is not UTF-8 is read as Windows-1252, the codepage Winamp wrote
  on Western systems. On Linux, a non-UTF-8 name is first tried as the file
  system's own bytes, then as Windows-1252.
- Relative paths are taken from the playlist's folder. Off Windows, a
  relative path with backslashes and no forward slashes (`Rock\01.mp3`) has
  them turned into separators. `.` and `..` are resolved by name, as Winamp
  did, without following symbolic links. Absolute Windows paths (`C:\...`)
  only work on Windows.
- `file://` URLs are read. A `file://server/share/...` URL names a network
  share, which only Windows can open; elsewhere it is skipped. Other URLs
  (internet radio) are skipped and logged until streams are supported.

Saved lists are UTF-8 whatever their extension, with `\r\n` line ends for PLS
and `\n` for M3U. Files inside the playlist's folder (or below it) are written
relative to it, so the folder can move as a whole; other files are written
with their full paths.

**Jump to file.** **J** (in any window, or Playback > Jump to file...) opens a
dialog drawn like Windows 2000's, as Winamp's was: type, and the list narrows
to the entries whose title or file name holds every word typed, ignoring case.
Up, Down, Page Up and Page Down move the highlight, Enter, the Jump button or a
double-click plays it, and Escape or Close closes the dialog. Left, Right,
Home and End move the caret in the search box. Clipboard paste is not
supported. The dialog is an ordinary window with the system's own title bar,
so the window manager places and moves it; it is drawn with softbuffer like
the menus, at the same size, in Tahoma where the system has it.

## Saved settings

NeoAmp starts the way it was left: volume, balance, the equalizer, shuffle,
repeat, the time display, the visualizer and its options, which windows are
open and where, the playlist's size, the skin, the zoom, double size and
always on top, and the playlist with its current entry. The playlist comes
back stopped, as in Winamp. NeoAmp saves within a couple of seconds of a change
and again on exit, so a crash loses little.

Two files share one folder, with the skins and presets the menus list:

| Platform | Folder |
| --- | --- |
| Linux | `~/.config/neoamp/` (or `$XDG_CONFIG_HOME/neoamp/`) |
| Windows | `%APPDATA%\NeoAmp\` |
| macOS | `~/Library/Application Support/NeoAmp/` |

- `settings.toml` holds the settings. Edit it while NeoAmp is closed, since a
  running NeoAmp rewrites it (without comments). A missing key keeps its
  default, and an unusable value (a volume out of range, a zoom between the
  steps) is brought into range or falls back on its own. A file that cannot
  be read as settings is renamed `settings.toml.bad`, and NeoAmp starts with
  the defaults.
- `playlist.m3u8` is the playlist as an extended M3U file, which other players
  can open too. Entries whose length it records are not scanned again at
  startup.
- `skins/` (`Skins\` on Windows and macOS) holds the skins the Skins menu
  lists. A skin opened from anywhere else, dropped onto a window or given on
  the command line is copied in, so it stays listed.
- `presets/` (`Presets\`) holds equalizer presets saved from EQ presets >
  Save > Preset..., as Winamp `.eqf` files, and lists them under Load.
- `autoload.eqf` holds the auto-load presets: equalizer settings kept for one
  file name each (EQ presets > Save > Auto-load preset). With AUTO on, a
  file's preset loads as it starts.

[docs/SETTINGS.md](docs/SETTINGS.md) describes every setting: what it
controls, the values it takes and its default.

`NEOAMP_CONFIG_DIR=<folder>` keeps all of it somewhere else. `--screenshot`
neither reads nor writes any of it. Files, a skin or `--scale` given on the command
line take precedence over what was saved and are remembered in turn, so files
replace the saved playlist.

The equalizer and playlist positions are kept relative to the main window, so
docked windows stay docked, and they are converted when the zoom or the
desktop's scale has changed since. Closing either window also remembers where
it was, and it reopens there, as in Winamp. A window whose title bar would land
off every screen (a monitor unplugged since, say) is placed the default way
instead. Each file is written beside the old one under a temporary name and
then renamed over it, so a crash during a save leaves the old file whole.

Where the platforms differ:

- **Pure Wayland:** window positions can be neither read nor set, so they are
  not saved. Whatever an X11 session saved stays for the next one.
- **Minimized windows** have no position worth keeping (Windows reports one far
  off the screen), so the last one from before is kept.
- **File names that are not valid Unicode**, which Linux allows, are written
  to the playlist as raw bytes and come back exactly. A skin at such a path is
  not remembered, because TOML holds only Unicode text. On Windows, the rare
  path that is not valid Unicode (an unpaired surrogate) is written with
  replacement characters, and that entry will not play after a restart.

## Building and running

The stock `base-2.91.wsz` skin must be present at `skins/base-2.91.wsz`; it is
embedded into the binary at build time.

On Linux, cpal needs the ALSA development files (Windows and macOS need
nothing extra):

```bash
sudo apt install libasound2-dev
```

```bash
cargo run --release
```

Play files, whole folders or playlist files (folders add their MP3, WAV, FLAC
and Ogg Vorbis files, sorted by name; `.m3u`, `.m3u8` and `.pls` files add
their entries); they replace the saved playlist and the first one starts
playing. Without any, the playlist from last time comes back. Dropping files,
folders or playlists onto any window adds them to the playlist too, and starts
them if nothing is playing. Eject picks files in a dialog (see
[Opening files](#opening-files)):

```bash
cargo run --release -- ~/Music/some-album
```

Load another classic skin from the Skins menu (**Alt+S**), by passing it on
the command line or by dropping a `.wsz` onto the window. It is copied into
the skins folder and stays until another one replaces it; **<Base Skin>** in
the Skins menu goes back to the built-in skin:

```bash
cargo run --release -- ~/Downloads/Classic.wsz
```

Pick the zoom yourself instead of letting the screen size decide:

```bash
cargo run --release -- --scale 2
```

Render a frame to a PNG (always at 1x) without opening a window. It shows a
made-up playlist, with a synthetic signal in the visualizer:

```bash
cargo run --release -- --screenshot shot.png
```

## Controls

| Action | Mouse | Keyboard |
| --- | --- | --- |
| Previous / Play / Pause / Stop / Next | transport buttons | Z X C V B |
| Open files (they replace the playlist) | either eject button | L |
| Open a folder (it replaces the playlist) | Play Folder... in the main menu | Shift+L |
| Play a playlist entry | double-click it | Enter (playlist) |
| Jump to file (search the playlist) | Playback > Jump to file... | J |
| Select playlist entries | click, Shift-click, Ctrl-click (Cmd on macOS); SEL menu | Up / Down / PgUp / PgDn / Home / End, with Shift to extend; Ctrl+A all, Ctrl+I invert (playlist) |
| Move the selected entries | drag them | Alt+Up / Alt+Down (playlist) |
| Remove entries | REM menu | Delete: selected, Ctrl+Delete: crop, Alt+Delete: dead files (playlist) |
| Sort, reverse, randomize | MISC > SORT LIST | Ctrl+Shift+1 / 2 / 3: by title / file name / path, Ctrl+R, Ctrl+Shift+R (playlist) |
| Add files or a folder to the playlist | ADD menu | |
| New, load and save playlist files | LIST OPTS menu | Ctrl+N / Ctrl+O / Ctrl+S (playlist) |
| Scroll the playlist | the scroll bar, its arrows, or the mouse wheel | |
| Shuffle / Repeat | toggle buttons | S / R |
| Elapsed vs. remaining time | click the time display | Ctrl+T |
| Equalizer / Playlist windows | EQ / PL buttons, window close buttons | Alt+G / Alt+E |
| Main menu | the options button at the left of the title bar, or right-click | Alt+F |
| Options menu | the `O` in the clutter bar | |
| Always on top | the `A` in the clutter bar | Ctrl+A (but in the playlist, select all) |
| Double size (doubles the zoom) | the `D` in the clutter bar | Ctrl+D |
| Zoom in / out / back to default | Options > Zoom | Ctrl+= / Ctrl+- / Ctrl+0 |
| Volume, balance, EQ bands and preamp | drag the sliders (the marquee shows the value); the wheel over the main window sets the volume | Up / Down: volume (outside the playlist) |
| Seek | drag the position bar; it seeks when you let go | Left / Right: 5 seconds |
| Equalizer on / off | the ON button | |
| EQ presets, and auto-load presets with AUTO on | the PRESETS button | |
| Visualizer mode (analyzer, scope, off) | click the visualizer | |
| Visualizer styles and falloff | the `V` in the clutter bar, or right-click the visualizer | |
| Skins | Skins in the main menu | Alt+S |
| Move a window (docked ones follow the main window) | drag its title bar | |
| Resize the playlist | drag the grip in its bottom-right corner | |
| Playlist menus | hold ADD / REM / SEL / MISC / LIST OPTS, let go over an item | |

Keys marked "(playlist)" work while the playlist window has the focus.

On macOS, Cmd takes Ctrl's place in every shortcut above (Cmd+A, Cmd+Shift+1,
...) and Option is Alt; the menus show them as ⌘A, ⇧⌘1, ⌥E. There are two
differences on macOS:

- macOS keeps Cmd+Shift+3 for screenshots, so sorting the playlist by path is
  only in the MISC > SORT LIST menu there.
- Delete is the forward-delete key (fn+Delete on a laptop keyboard). The key
  labelled "delete" on a Mac is Backspace, which removes nothing.

## Scope and limitations

- Classic 2.x skins (`.wsz`) only. Modern `.wal` skins are a different format.
- Windowshade mode, the file info dialog, cursors (`.cur`) and the minibrowser
  window are not implemented. The menus leave out what NeoAmp cannot do yet
  (Play Location, Preferences and the like) rather than grey it.
- Internet radio is not supported yet: ADD > URL does nothing, Winamp's
  Ctrl+L (open a URL) is not implemented, and stream URLs in playlist files
  are skipped. The playlist's FILE INFO and MISC OPTIONS items do nothing,
  and there is no right-click menu for the playlist's entries (a right-click
  there opens the main menu).
- The Jump to file dialog's search box has no clipboard (paste) or text
  selection, and the dialog keeps its highlight by position, not by entry, if
  the playlist changes while it is open.
- Tracks play back to back with a short gap, not gaplessly.
- The output device is opened once, at startup, and never reopened. On Linux
  that device is PipeWire's or PulseAudio's, which move the stream to another
  output when one disappears. On Windows and macOS, losing the device (say,
  unplugging headphones) silences playback until the app restarts.
- The playlist uses a system TrueType font (Arial, Liberation Sans or
  DejaVu Sans, whichever exists) at the size Winamp used.
