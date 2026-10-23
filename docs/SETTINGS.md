# Settings file reference

NeoAmp remembers how it was left in two files: `settings.toml`, described key
by key below, and `playlist.m3u8`, the playlist. This page says what each
setting controls, what values it takes and what happens to a value NeoAmp
cannot use. The code behind it is [src/settings.rs](../src/settings.rs) and
[src/m3u.rs](../src/m3u.rs). The same folder also holds your skins,
equalizer presets and MilkDrop presets; see [Skins, presets and auto-load
presets](#skins-presets-and-auto-load-presets). The [environment
variables](#environment-variables) NeoAmp reads are listed at the end.

## Where the files are

| Platform | Folder |
| --- | --- |
| Linux | `~/.config/neoamp/` (or `$XDG_CONFIG_HOME/neoamp/`) |
| Linux, the Flatpak | `~/.var/app/io.github.BlakeGardner.NeoAmp/config/neoamp/` |
| Windows | `%APPDATA%\NeoAmp\` |
| macOS | `~/Library/Application Support/NeoAmp/` |

Set `NEOAMP_CONFIG_DIR=<folder>` to keep both files somewhere else, for
example a portable copy on a USB stick or a test run that should not touch
your real settings. NeoAmp creates the folder the first time it saves. The
Skin Museum's downloads, which otherwise go in the platform's cache folder,
then go in `cache/museum` inside it too (see [The Skin Museum's
downloads](#the-skin-museums-downloads)). The
Flatpak can write only below its own folder, so there the folder must be
inside `~/.var/app/io.github.BlakeGardner.NeoAmp/`, given with
`flatpak run --env=NEOAMP_CONFIG_DIR=<folder> io.github.BlakeGardner.NeoAmp`.

Uninstalling NeoAmp leaves the folder, so a later install starts the way the
last one was left: Windows' uninstaller removes only the program, as does
dragging the macOS app to the Bin, and `flatpak uninstall` keeps it unless
given `--delete-data`. Delete the folder to remove your settings, skins and
presets too, and the Skin Museum's cache folder for its downloads.

## Editing the file

NeoAmp writes `settings.toml` within a couple of seconds of any change and
again when it exits, so **edit it only while NeoAmp is closed**. Changes made
while it runs are overwritten.

The file is [TOML](https://toml.io). NeoAmp is forgiving about what it reads:

- **Missing keys** keep their defaults. An empty file, or none at all, gives
  the defaults listed below.
- **Unknown keys** are ignored, and they disappear at the next save.
- **Comments** are allowed but are not kept: NeoAmp rewrites the whole file,
  with a short comment of its own at the top.
- **Out-of-range numbers** are brought into range (`volume = 5.0` plays at
  full volume). `nan` and `inf` count as missing.
- **A value of the right type that still makes no sense** (a zoom between the
  steps, a visualizer mode NeoAmp does not know, a band list of the wrong
  length) falls back to its default on its own, with a warning in the log.
- **A value of the wrong type** (`shuffle = "yes"`, `volume = "loud"`) or
  broken TOML makes the whole file unreadable. NeoAmp renames it
  `settings.toml.bad`, so your edit is not lost, and starts with the defaults.
  The log names the line at fault.

Whole numbers work wherever a decimal is expected (`volume = 1` is the same as
`volume = 1.0`). Booleans are `true` or `false`, in lowercase.

Run NeoAmp from a terminal to see the warnings; `RUST_LOG=neoamp=debug` shows
more (see [Environment variables](#environment-variables)).

## Example

A first start writes this (the window positions depend on the screen):

```toml
# NeoAmp settings, rewritten whenever they change. Edit them while NeoAmp is closed.
# volume: 0 to 1. balance: -1 (left) to 1 (right). eq: decibels, -12 to 12.
# windows: screen pixels; the other windows' offsets are from the main window's corner.

volume = 0.78
balance = 0.0
shuffle = false
repeat = false
time_remaining = false
vis = "analyzer"
double_size = false
always_on_top = false

[analyzer]
style = "normal"
bands = "thick"
peaks = true
falloff = "moderate"
peak_falloff = "slow"

[oscilloscope]
style = "lines"

[eq]
on = true
auto = false
preamp = 0.0
bands = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]

[playlist]
current = 0

[milkdrop]
random = true
seconds = 16.0

[windows]
scale = 1.0
main = [64, 64]
main_shade = false

[windows.eq]
open = true
shade = false
offset = [0, 116]

[windows.playlist]
open = true
shade = false
offset = [0, 232]
size = [275, 116]

[windows.milkdrop]
open = false
shade = false
size = [275, 232]
```

`zoom`, `skin`, MilkDrop's `preset` and the `[output]` table are written
only once they differ from the defaults, and MilkDrop's `offset` once its
window has been open. Choosing the Winamp base skin, for one, adds this line
under `always_on_top`:

```toml
skin = "winamp"
```

## Top-level settings

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `volume` | `0.0` to `1.0` | `0.78` | The volume slider. |
| `balance` | `-1.0` (left) to `1.0` (right) | `0.0` | The balance slider. |
| `shuffle` | `true` / `false` | `false` | The shuffle button (**S**). |
| `repeat` | `true` / `false` | `false` | The repeat button (**R**). |
| `time_remaining` | `true` / `false` | `false` | Whether the time display counts down (**Ctrl+T**, or click it). |
| `vis` | `"analyzer"`, `"oscilloscope"`, `"off"` | `"analyzer"` | What the visualizer shows (click it to cycle, or pick it from its menu). |
| `double_size` | `true` / `false` | `false` | Double size (**Ctrl+D**, or the `D` in the clutter bar). |
| `always_on_top` | `true` / `false` | `false` | Always on top (**Ctrl+A**, or the `A` in the clutter bar). |
| `zoom` | `1`, `1.5`, `2`, `2.5`, `3` | none | The zoom picked with **Ctrl+=** / **Ctrl+-**, Options > Zoom or `--scale`. |
| `skin` | `"winamp"`, or the path of a `.wsz` file | none (NeoAmp's own skin) | The skin shown. |

- **`volume`** is the slider's position, not a gain. The sound is scaled by its
  square, which follows loudness more closely: `0.5` plays at a quarter of
  full gain, about 12 dB down. `0.0` is silent.
- **`balance`** turns one side down. At `0.5` the left channel plays at half
  gain; at `1.0` only the right one plays. When you drag the slider, it snaps
  to the centre near the middle, but a value written in the file is used
  exactly.
- **`shuffle`** plays every entry once, in a random order, before any plays
  again. The order itself is not remembered: a new round starts each run.
- **`repeat`** makes the playlist start over after its last entry. Without it,
  playback stops there.
- **`time_remaining`** needs the track's length. Until that is known, the
  display counts up.
- **`double_size`** doubles whatever the zoom is, as in Winamp: 1.5x becomes 3x.
- **`always_on_top`** keeps NeoAmp's windows above other applications'. It has
  no effect on pure Wayland, which does not let applications ask for that, or
  where NeoAmp uses override-redirect windows (see [Windows, docking and
  zoom](GUIDE.md#windows-docking-and-zoom) in the user guide), which stay on
  top anyway.
- **`zoom`** is how far the windows are enlarged, before double size and the
  desktop's own scale factor. Without it, NeoAmp picks a zoom from the
  screen's height each time it starts (1080 lines: 1x, 1440: 1.5x, 2160: 2x).
  **Ctrl+0** goes back to that choice and removes the key at the next save.
  Any value other than the five steps is ignored. `--scale` on the command
  line overrides it, and the new zoom is then saved in turn, unless it is the
  one the screen would get anyway.
- **`skin`** names one of the two skins built into NeoAmp, or a skin file.
  Without it NeoAmp shows its own skin, drawn in SVG; `"neoamp"` means the
  same, but NeoAmp never writes it. `"winamp"` is Winamp 2.91's base skin,
  chosen with **<Winamp Base Skin>** in the Skins menu. Anything else is the
  full path of a `.wsz` file. The Skins menu sets it, and so does dropping a
  `.wsz` onto a window or passing one on the command line. A skin from
  outside the skins folder is copied into it first, and `skin` names the
  copy. Choose **<NeoAmp Skin>** in the Skins menu, or delete the line, to go
  back to NeoAmp's own skin. If the file has been moved or deleted, NeoAmp
  shows its own skin, logs a warning and drops the key at the next save.
  Windows paths are written in single quotes,
  `skin = 'C:\Skins\Classic.wsz'`; inside double quotes every backslash must
  be doubled. A path that is not valid Unicode, which Linux allows, cannot be
  stored, so such a skin is not remembered, and the next start shows NeoAmp's
  own skin. Versions of NeoAmp before the NeoAmp skin wrote no `skin` key for
  the Winamp base skin, so they start with NeoAmp's own skin after an
  update.

## `[eq]`: the equalizer

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `on` | `true` / `false` | `true` | The equalizer's **ON** button. |
| `auto` | `true` / `false` | `false` | The **AUTO** button. |
| `preamp` | `-12.0` to `12.0` (dB) | `0.0` | The preamp slider. |
| `bands` | ten numbers, `-12.0` to `12.0` (dB) | all `0.0` | The ten band sliders, left to right. |

- **`on = false`** bypasses the whole equalizer, the preamp included, as in
  Winamp. The sliders keep their positions.
- **`auto`** loads each file's auto-load preset as the file starts, when it
  has one (see [auto-load presets](#skins-presets-and-auto-load-presets)).
  Files without one leave the equalizer as it is.
- **`preamp`** raises or lowers the overall level, on top of the bands.
  Values are in decibels and are kept within ±12 dB.
- **`bands`** are, in order: 60 Hz, 170 Hz, 310 Hz, 600 Hz, 1 kHz, 3 kHz,
  6 kHz, 12 kHz, 14 kHz and 16 kHz, each in decibels within ±12 dB. `0.0` is
  flat. A list that does not have exactly ten numbers is ignored, leaving all
  ten bands flat.

## `[analyzer]` and `[oscilloscope]`: the visualizer

These are the choices in the visualizer's menu (the `V` in the clutter bar, or
right-click the visualizer). Each value that NeoAmp does not know falls back
to its own default.

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `analyzer.style` | `"normal"`, `"fire"`, `"line"` | `"normal"` | How the bars are coloured. |
| `analyzer.bands` | `"thick"`, `"thin"` | `"thick"` | 19 bars three pixels wide, or 75 one pixel wide. |
| `analyzer.peaks` | `true` / `false` | `true` | The dots that mark recent peaks. |
| `analyzer.falloff` | `"slowest"`, `"slow"`, `"moderate"`, `"fast"`, `"fastest"` | `"moderate"` | How fast the bars fall. |
| `analyzer.peak_falloff` | the same five | `"slow"` | How fast the peaks fall. |
| `oscilloscope.style` | `"dots"`, `"lines"`, `"solid"` | `"lines"` | How the waveform is drawn. |

- **`style`**: the colours come from the skin's `viscolor.txt`. `"normal"`
  gives each row its own colour, so only tall bars reach the top colours.
  `"fire"` starts the colours at the top of every bar. `"line"` paints each bar
  in the one colour of the row its top reaches.
- **`falloff`**: `"moderate"` drops a bar 30 pixels a second; the others are
  in Winamp's proportions, from a quarter of that to over twice it.
- **`peak_falloff`**: a peak hangs for a moment and then falls ever faster;
  faster settings speed it up sooner.
- **`oscilloscope.style`**: `"dots"` draws one dot per column, `"lines"` joins
  them up and `"solid"` fills each column from the centre line.

## `[playlist]`

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `current` | `0` or more | `0` | The playlist entry NeoAmp starts on, counting from 0. |

NeoAmp starts stopped on this entry, scrolled to it in the playlist window.
It counts entries in `playlist.m3u8`, so `current = 2` is the third track.
Past the end of the playlist, the last entry is used. Files given on the
command line replace the playlist and start playing from their first file, so
this key does not apply then (it does with `--enqueue`, which adds them to the
end).

## `[milkdrop]`: MilkDrop's presets

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `preset` | a preset's name | none | The preset MilkDrop shows, which it starts with next time. |
| `random` | `true` / `false` | `true` | Whether presets come in random order or in the order they are listed (**R** in MilkDrop's window). |
| `seconds` | `1.0` to `3600.0` | `16.0` | How long each preset runs before the next blends in. |

- **`preset`** is a built-in preset's title, or one of your files' names
  without `.milk` (see [Skins, presets and auto-load
  presets](#skins-presets-and-auto-load-presets)). Without it, or when no
  preset has that name (a file deleted since, say), MilkDrop starts with any
  preset, and the one it shows is saved instead. A value that is not text is
  ignored, with a warning in the log.
- **`seconds`**: each preset also runs a random extra of up to 62.5% more,
  then blends into the next over 2.7 seconds. The default gives MilkDrop's own
  16 seconds plus up to 10. Values outside the range are brought into it;
  `nan` and `inf` keep the default. Only the settings file changes it.

## `[windows]`: where the windows are

Window positions are kept in **screen pixels** (physical pixels, not the
desktop's scaled ones). The main window has an absolute position. The
equalizer, playlist and MilkDrop are kept **relative to the main window**, so
windows docked to it stay docked when it moves between runs.

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `scale` | a number above `0` | `1.0` | Screen pixels per skin pixel when the positions were saved. |
| `main` | `[x, y]` | none | The main window's top-left corner. |
| `main_shade` | `true` / `false` | `false` | Whether the main window is in windowshade mode, collapsed to a bar (its shade button, a double-click on its title bar, or **Ctrl+W**). |

- **`scale`** is the zoom times double size times the desktop's scale factor,
  as it was when NeoAmp saved. It lets NeoAmp carry the offsets below over to
  another scale: offsets saved at 3x open at half the distance at 1.5x. NeoAmp
  sets it whenever it saves positions. If you edit positions by hand, leave
  it alone; if it is zero, negative or not a number, all saved positions are
  ignored.
- **`main`** is measured from the top-left corner of the desktop. On a setup
  with several monitors it can be negative, for a monitor left of or above
  the primary one. Without it, the main window opens 64 pixels in from the
  primary monitor's top-left corner. NeoAmp also ignores it when the middle
  of the main window's title bar would land off every connected screen (a
  monitor unplugged since, say), so the window can always be grabbed.

### `[windows.eq]`, `[windows.playlist]` and `[windows.milkdrop]`

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `open` | `true` / `false` | `true` (MilkDrop: `false`) | Whether the window is shown (**Alt+G** for the equalizer, **Alt+E** for the playlist, **Ctrl+Shift+K** for MilkDrop). |
| `shade` | `true` / `false` | `false` | Whether the window is in windowshade mode, collapsed to a bar (its shade button, a double-click on its title bar, or for the playlist **Ctrl+W**). MilkDrop's window has no such mode, and ignores it. |
| `offset` | `[dx, dy]` | none | From the main window's top-left corner to this window's, in screen pixels at `scale`. |
| `size` | `[width, height]` (playlist and MilkDrop only) | `[275, 116]` (MilkDrop: `[275, 232]`) | The window's size, in skin pixels. |

- **`offset`** is remembered for a closed window too, so it reopens where it
  was. Without it, the equalizer docks under the main window, the playlist
  under the equalizer (or under the main window when the equalizer is
  elsewhere or closed), and MilkDrop to the right of the main window. At 1x,
  docked under the main window is `[0, 116]`, under the equalizer `[0, 232]`
  and to the right of the main window `[275, 0]`; at 1.5x these are
  `[0, 174]`, `[0, 348]` and `[413, 0]`. A window in windowshade mode is 14
  skin pixels high, so under a shaded main window at 1x is `[0, 14]`. MilkDrop
  closed while it fills the screen keeps the offset its window had. At
  startup, an offset that would put the window's title bar off every screen
  is ignored.
- **`shade`**: a window collapsed to its bar keeps its width. The windows
  docked below it follow when it is shaded or opened out (see
  [Windowshade](GUIDE.md#windowshade) in the user guide), so the offsets saved are those of the windows as they are.
- **`size`** is in skin pixels, so it does not change with the zoom. The
  playlist and MilkDrop grow in steps of 25 by 29 pixels from their smallest
  size, 275 by 116, as when resized from the grip. Other values are moved to
  the nearest step, and anything smaller becomes the smallest size. In
  windowshade mode `size` keeps the height the playlist opens out to; the
  bar's own grip changes only the width. The equalizer has a fixed size and
  ignores this key. MilkDrop filling the screen keeps the size of its window.

### Platform notes

- **Pure Wayland:** the desktop does not let applications read or set window
  positions, so NeoAmp neither restores nor updates `main` and the offsets
  there. Values saved by an X11 session are kept for the next one.
- **Minimized windows:** while a window is minimized, its position is not
  saved (Windows reports one far off the screen), and the last position from
  before is kept.

## `[output]`: the output device

Written once a device is chosen in Options > Output Device. Without it,
NeoAmp plays through the system's default device and follows the system when
the default changes.

```toml
[output]
device = "pulseaudio:bluez_output.E4_58_BC_32_2C_CE.1"
name = "Bose QC Ultra Earbuds"
```

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `device` | the system's identifier for a device | none | The device NeoAmp plays through. |
| `name` | text | `""` | The device's name, which the menu shows while it is not connected. |

- **`device`** is `host:device`, as the system names the device internally:
  `pulseaudio:alsa_output.…` or `pulseaudio:bluez_output.…` for the sound
  server's outputs on Linux (`alsa:plughw:CARD=…,DEV=…` where no sound server
  runs), `wasapi:{0.0.0.00000000}.{…}` on Windows, and `coreaudio:` and the
  device's UID on macOS. Pick devices from the menu rather than typing these.
- **A device that is not connected**, when NeoAmp starts or later (headphones
  switched off, a USB DAC unplugged), stays chosen: NeoAmp plays through the
  system's default meanwhile, looks for it every two seconds, and goes back
  to it when it is there again. The menu lists it as "*name* (not
  connected)". An identifier that names no device at all is treated the same
  way.
- **To go back to the default**, choose Options > Output Device > System
  Default, or delete the table.
- **Unusable values:** a `device` that is missing or blank means the system's
  default; an `[output]` that is not a table (`output = 3`) is ignored, with a
  warning in the log.

## `playlist.m3u8`

The playlist is an extended M3U file, which other players can open too:

```text
#EXTM3U
#EXTINF:213,Nullsoft - Winamp 2.91 Theme
/home/me/Music/theme.mp3
#EXTINF:-1,02 Untitled
/home/me/Music/02 Untitled.flac
#EXTINF:-1,Radio Llama
https://radio.example/llama.pls
```

- Each entry is an optional `#EXTINF:<seconds>,<title>` line followed by the
  file's path, or for an internet radio stream its `http://` or `https://`
  URL. `-1` means the length is not known yet; a stream's is always `-1`.
- NeoAmp shows the title and length from the file straight away. At startup
  it reads the tags again only for entries whose length is unknown, so a tag
  edited since shows up when that track next plays.
- Paths are absolute when NeoAmp writes them. In a hand-edited file, relative
  paths are taken from the folder holding `playlist.m3u8`.
- Lines starting with `#` other than `#EXTINF` are skipped, and so are blank
  lines. Windows (`\r\n`) and Unix (`\n`) line endings both work, and so does
  a UTF-8 byte order mark.
- On Linux and macOS, paths are written as raw bytes, so a file name that is
  not valid UTF-8 comes back exactly. On Windows, the rare path that is not
  valid Unicode is written with replacement characters and will not play
  after a restart.
- An entry whose path contains a line break cannot be written in M3U and is
  left out, with a warning.
- Tracks that no longer exist stay in the playlist and are skipped when they
  fail to play.
- **Streams** are written as their URLs, exactly as they were given. Their
  title is the station's name: the one a playlist file gave it, or else the
  name the station's server sent the first time it played (until then, the
  URL itself). The song a station is playing is never saved. Streams are not
  read at startup; only playing one tells what it is. URLs of other kinds
  (`rtsp://`, `mms://`, ...) in a hand-edited file are skipped, with a
  message in the log.

## Skins, presets and auto-load presets

Beside the two files, the folder holds:

| Name | What it is |
| --- | --- |
| `skins/` (`Skins\` on Windows and macOS) | The skins the Skins menu lists: every `.wsz` file in it. |
| `presets/` (`Presets\` on Windows and macOS) | The equalizer presets listed under EQ presets > Load: every `.eqf` file in it. |
| `autoload.eqf` | Auto-load presets, one per file name. |
| `milkdrop/` (`MilkDrop\` on Windows and macOS) | Your MilkDrop presets, every `.milk` file in it and in the folders inside it, and the pictures they use. |

- **Skins** opened through the menu, dropped onto a window or given on the
  command line are copied into the skins folder, so the menu lists them from
  then on. A different skin with the same file name is kept, and the new one is
  numbered: `Name (2).wsz`. To remove a skin from the menu, delete its file.
- **Presets** are Winamp `.eqf` files, which Winamp and Webamp can load too.
  EQ presets > Save > Preset... opens the platform's Save dialog in this
  folder; the file name becomes the preset's name. Load lists Winamp's
  built-in presets first, then these. Load > From .eqf file... opens any
  `.eqf` file, and uses the first preset in it.
- **`autoload.eqf`** is also in the `.eqf` format, like Winamp's `winamp.q2`:
  each preset is named after the file it belongs to, such as
  `01 Intro.mp3`, without the folder. EQ presets > Save > Auto-load preset
  keeps the current settings for the playing file, and Delete > Auto-load
  preset forgets them. With **AUTO** on, a file's preset loads as the file
  starts. Names match regardless of case. A file NeoAmp cannot read as
  presets is renamed `autoload.eqf.bad`, and NeoAmp starts without any.

- **MilkDrop's folder** is made when MilkDrop's window first opens, and read
  each time it opens: its `.milk` files, eight folders deep, are listed after
  the built-in presets, by file name without `.milk`. A file over 1 MB is
  not read. The pictures presets name (`sampler_worms` is `worms.jpg`, say)
  are found there too, by name regardless of case, as `.jpg`, `.jpeg`,
  `.png`, `.bmp` or `.tga` files up to 4096 pixels a side; where two have the
  same name, the first in alphabetical order of path is used.

Preset names are written in UTF-8. Winamp wrote them in the Windows code page,
so names with accents from Winamp's files show correctly in NeoAmp, but
NeoAmp's show garbled in Winamp. Plain ASCII names are the same in both.

## The Skin Museum's downloads

The Skin Museum dialog (see [Skin Museum](GUIDE.md#skin-museum) in the user
guide) keeps the skins it fetches, named after each one's MD5
(`<md5>.wsz`), so that they come at once the next time. They are not
settings: the folder holds at most 100 MB, the files used longest ago go
first, and deleting it is always safe.

| Platform | Folder |
| --- | --- |
| Linux | `~/.cache/neoamp/museum/` (or `$XDG_CACHE_HOME/neoamp/museum/`) |
| Linux, the Flatpak | `~/.var/app/io.github.BlakeGardner.NeoAmp/cache/neoamp/museum/` |
| Windows | `%LOCALAPPDATA%\NeoAmp\cache\museum\` |
| macOS | `~/Library/Caches/NeoAmp/museum/` |

With `NEOAMP_CONFIG_DIR` set, it is `cache/museum` inside that folder. A skin
installed from the dialog is copied out of it into the skins folder, so it
stays when the cache is cleared.

## What is not saved

- Whether a track was playing. NeoAmp always starts stopped, as Winamp did.
- How far into the track it was.
- What a stream was playing, and what its station said about itself (genre,
  web site, bitrate), apart from the station's name, which becomes its
  entry's title. The Open URL dialog keeps no list of recent addresses.
- Which playlist rows were selected, and how far the playlist was scrolled
  (it scrolls to the current entry).
- Whether MilkDrop filled the screen or had its preset locked, and the
  presets it showed before the current one (what Backspace goes back
  through).
- The Skin Museum's list and searches, which are kept only while NeoAmp runs,
  and a skin being tried on that was not installed: `skin` keeps naming the
  one chosen before, which comes back when the dialog closes and at the next
  start.

## The command line and the settings

Anything given on the command line wins over the saved settings and is then
remembered like any other change:

- Audio files, folders, playlist files or stream URLs replace the saved
  playlist, and the first one plays. Without any, the saved playlist comes
  back.
- With `--enqueue`, they join the end of the saved playlist instead, and
  nothing starts playing.
- A `.wsz` file (or `--skin`) replaces the saved skin, and is copied into the
  skins folder. `--skin` takes only a file; the two built-in skins are chosen
  from the Skins menu.
- `--scale` replaces the saved zoom.
- `--screenshot` neither reads nor writes any of these files.

While NeoAmp runs, starting it again with the same config folder hands the
files and skin to the running copy, which takes them in the same way (files
replace its playlist and play, or join it with `--enqueue`) and saves them as
usual; the new start then ends. `--scale` is not handed over. A config folder
of its own (`NEOAMP_CONFIG_DIR`) makes a NeoAmp of its own, which runs beside
the others.

## Environment variables

NeoAmp reads these when it starts. They are not saved, so set them each time,
in a terminal, a launcher or a script. In the Flatpak, give them with
`flatpak run --env=NAME=value io.github.BlakeGardner.NeoAmp`.

| Variable | Values | What it controls |
| --- | --- | --- |
| `NEOAMP_CONFIG_DIR` | a folder | Where the settings, playlist, skins, presets and downloads are kept, instead of the platform's folders. |
| `NEOAMP_BACKEND` | `x11`, `wayland` | Which window system NeoAmp uses on Linux. |
| `NEOAMP_UNMANAGED` | `1`, `true`, `yes` / `0`, `false`, `no` | Whether NeoAmp uses override-redirect windows on X11, instead of finding out at startup. |
| `RUST_LOG` | a log filter | Which messages the log shows. Default `info,sctk_adwaita=error`. |
| `RUST_LOG_STYLE` | `auto`, `always`, `never` | Whether the log is coloured. Default `auto`. |

- **`NEOAMP_CONFIG_DIR`** is described under [Where the files
  are](#where-the-files-are). An empty value counts as unset. Give a full
  path: a relative one is taken from the folder NeoAmp starts in, which a
  launcher may choose differently. Each config folder makes a NeoAmp of its
  own, so starting NeoAmp with another folder runs a second copy beside the
  first rather than handing it the files.
- **`NEOAMP_BACKEND`** matters only on Linux and the BSDs. Docking needs to
  place windows, which Wayland forbids, so without it NeoAmp uses X11
  (XWayland on a Wayland desktop) whenever `DISPLAY` is set. `x11` asks for
  X11 even when `DISPLAY` is not set; if no X server answers, NeoAmp logs a
  warning and uses Wayland, where windows do not dock. `wayland` drops the
  preference for X11, so NeoAmp uses Wayland whenever `WAYLAND_DISPLAY` is
  set. Case does not matter, and any other value counts as unset. The
  Flatpak has no access to Wayland, so `wayland` changes nothing there. See
  [Windows, docking and zoom](GUIDE.md#windows-docking-and-zoom) in the user
  guide for what each one can do.
- **`NEOAMP_UNMANAGED`**: some X11 window managers, COSMIC among them,
  ignore requests to move windows. NeoAmp checks for this at startup and, if
  so, recreates its windows as override-redirect windows, which the window
  manager leaves where NeoAmp puts them. Such windows have no taskbar entry
  and stay above other windows. `1` uses them from the start and `0` never
  does; either way the check is skipped. Only X11 has such windows. The
  values are lowercase, and any other value counts as unset.
- **`RUST_LOG`** takes [env_logger's
  filters](https://docs.rs/env_logger/0.11/env_logger/#enabling-logging): a
  level (`error`, `warn`, `info`, `debug` or `trace`) for every message, or
  `target=level` pairs separated by commas. NeoAmp's own targets are
  `neoamp` and its source files below it, so `neoamp=debug` shows NeoAmp's
  messages down to debug level and no other library's, and
  `info,neoamp::http=debug` keeps the usual messages and adds internet
  radio's debug ones. Setting it replaces the default entirely. The log goes
  to standard error: the terminal on Linux, and on macOS when the program
  inside the app is started from Terminal (see [macOS](GUIDE.md#macos)).
  Windows opens no console for NeoAmp, so redirect it there:
  `neoamp.exe 2> neoamp.log` (see [Windows](GUIDE.md#windows)).
- **`RUST_LOG_STYLE`**: `auto` colours the log only when it goes to a
  terminal.

### The platform's variables

The platform's own variables change some of the same things:

| Variable | Platform | What it controls |
| --- | --- | --- |
| `HOME` | Linux, macOS | The home folder that the config and cache folders are in. |
| `XDG_CONFIG_HOME` | Linux | The config folder: `$XDG_CONFIG_HOME/neoamp/` instead of `~/.config/neoamp/`. |
| `XDG_CACHE_HOME` | Linux | The Skin Museum's downloads: `$XDG_CACHE_HOME/neoamp/museum/` instead of `~/.cache/neoamp/museum/`. |
| `XDG_RUNTIME_DIR` | Linux | Where the running NeoAmp listens for a second start's files (without it, in the config folder). |
| `TMPDIR` | macOS | The same, on macOS. The system sets it to a folder of the user's own. |
| `DISPLAY` | Linux | Whether X11 is there, which NeoAmp prefers (see `NEOAMP_BACKEND` above). |
| `WAYLAND_DISPLAY` | Linux | Whether Wayland is there, for when NeoAmp does not use X11. |
| `WINIT_X11_SCALE_FACTOR` | Linux, on X11 | The desktop's scale factor, which multiplies the zoom: a number above `0`, or `randr` to work it out from the monitor's size. Any other value crashes NeoAmp. |

- The `XDG_` folders are used only when given as full paths; relative ones
  are ignored. The Flatpak sets `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` to
  folders inside `~/.var/app/io.github.BlakeGardner.NeoAmp/`, which is why
  its files are there.
- On Windows the folders come from the system's known folders, not from
  `%APPDATA%` and `%LOCALAPPDATA%`, so changing those variables does not
  move them. Use `NEOAMP_CONFIG_DIR` instead.
- NeoAmp reads no proxy settings (`HTTP_PROXY`, `HTTPS_PROXY`): internet
  radio and the Skin Museum connect to their servers directly.
- The sound server, D-Bus and window system libraries read variables of their
  own, which this page does not list.
