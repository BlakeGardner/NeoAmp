# Settings file reference

NeoAmp remembers how it was left in two files: `settings.toml`, described key
by key below, and `playlist.m3u8`, the playlist. This page says what each
setting controls, what values it takes and what happens to a value NeoAmp
cannot use. The code behind it is [src/settings.rs](../src/settings.rs) and
[src/m3u.rs](../src/m3u.rs).

## Where the files are

| Platform | Folder |
| --- | --- |
| Linux | `~/.config/neoamp/` (or `$XDG_CONFIG_HOME/neoamp/`) |
| Windows | `%APPDATA%\NeoAmp\` |
| macOS | `~/Library/Application Support/NeoAmp/` |

Set `NEOAMP_CONFIG_DIR=<folder>` to keep both files somewhere else, for
example a portable copy on a USB stick or a test run that should not touch
your real settings. NeoAmp creates the folder the first time it saves.

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
more.

## Example

A first start writes this (the window positions depend on the screen):

```toml
# NeoAmp settings, rewritten whenever they change. Edit them while NeoAmp is closed.
# volume: 0 to 1. balance: -1 (left) to 1 (right). eq: decibels, -12 to 12.
# windows: screen pixels; eq and playlist offsets are from the main window's corner.

volume = 0.78
balance = 0.0
shuffle = false
repeat = false
time_remaining = false
vis = "analyzer"
double_size = false

[eq]
on = true
auto = false
preamp = 0.0
bands = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]

[playlist]
current = 0

[windows]
scale = 1.0
main = [64, 64]

[windows.eq]
open = true
offset = [0, 116]

[windows.playlist]
open = true
offset = [0, 232]
size = [275, 116]
```

`zoom` and `skin` are written only once they differ from the defaults.

## Top-level settings

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `volume` | `0.0` to `1.0` | `0.78` | The volume slider. |
| `balance` | `-1.0` (left) to `1.0` (right) | `0.0` | The balance slider. |
| `shuffle` | `true` / `false` | `false` | The shuffle button (**S**). |
| `repeat` | `true` / `false` | `false` | The repeat button (**R**). |
| `time_remaining` | `true` / `false` | `false` | Whether the time display counts down (**Ctrl+T**, or click it). |
| `vis` | `"analyzer"`, `"oscilloscope"`, `"off"` | `"analyzer"` | What the visualizer shows (click it to cycle). |
| `double_size` | `true` / `false` | `false` | Double size (**Ctrl+D**, or the `D` in the clutter bar). |
| `zoom` | `1`, `1.5`, `2`, `2.5`, `3` | none | The zoom picked with **Ctrl+=** / **Ctrl+-** or `--scale`. |
| `skin` | path to a `.wsz` file | none | The classic skin shown. |

- **`volume`** is the slider's position, not a gain. The sound is scaled by its
  square, which follows loudness more closely: `0.5` plays at a quarter of
  full gain, about 12 dB down. `0.0` is silent.
- **`balance`** turns one side down. At `0.5` the left channel plays at half
  gain; at `1.0` only the right one plays. When you drag the slider, it snaps
  to the centre near the middle, but a value written in the file is used
  exactly.
- **`shuffle`** is remembered, but shuffle does not change the playing order
  yet (see the [roadmap](ROADMAP.md), Phase 3).
- **`repeat`** makes the playlist start over after its last entry. Without it,
  playback stops there.
- **`time_remaining`** needs the track's length. Until that is known, the
  display counts up.
- **`double_size`** doubles whatever the zoom is, as in Winamp: 1.5x becomes 3x.
- **`zoom`** is how far the windows are enlarged, before double size and the
  desktop's own scale factor. Without it, NeoAmp picks a zoom from the
  screen's height each time it starts (1080 lines: 1x, 1440: 1.5x, 2160: 2x).
  **Ctrl+0** goes back to that choice and removes the key at the next save.
  Any value other than the five steps is ignored. `--scale` on the command
  line overrides it, and the new zoom is then saved in turn, unless it is the
  one the screen would get anyway.
- **`skin`** is the full path of the skin file; without it NeoAmp uses its
  built-in base skin. Dropping a `.wsz` onto a window or passing one on the
  command line sets it. Delete the line to go back to the built-in skin. If
  the file has been moved or deleted, NeoAmp shows the built-in skin, logs a
  warning and drops the key at the next save. Windows paths are written in
  single quotes, `skin = 'C:\Skins\Classic.wsz'`; inside double quotes every
  backslash must be doubled. A path that is not valid Unicode, which Linux
  allows, cannot be stored, so such a skin is not remembered.

## `[eq]`: the equalizer

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `on` | `true` / `false` | `true` | The equalizer's **ON** button. |
| `auto` | `true` / `false` | `false` | The **AUTO** button. |
| `preamp` | `-12.0` to `12.0` (dB) | `0.0` | The preamp slider. |
| `bands` | ten numbers, `-12.0` to `12.0` (dB) | all `0.0` | The ten band sliders, left to right. |

- **`on = false`** bypasses the whole equalizer, the preamp included, as in
  Winamp. The sliders keep their positions.
- **`auto`** is remembered, but loading a preset for each file is not
  implemented yet, so it has no effect.
- **`preamp`** raises or lowers the overall level, on top of the bands.
  Values are in decibels and are kept within ±12 dB.
- **`bands`** are, in order: 60 Hz, 170 Hz, 310 Hz, 600 Hz, 1 kHz, 3 kHz,
  6 kHz, 12 kHz, 14 kHz and 16 kHz, each in decibels within ±12 dB. `0.0` is
  flat. A list that does not have exactly ten numbers is ignored, leaving all
  ten bands flat.

## `[playlist]`

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `current` | `0` or more | `0` | The playlist entry NeoAmp starts on, counting from 0. |

NeoAmp starts stopped on this entry, scrolled to it in the playlist window.
It counts entries in `playlist.m3u8`, so `current = 2` is the third track.
Past the end of the playlist, the last entry is used. Files given on the
command line replace the playlist and start playing from their first file, so
this key does not apply then.

## `[windows]`: where the windows are

Window positions are kept in **screen pixels** (physical pixels, not the
desktop's scaled ones). The main window has an absolute position. The
equalizer and playlist are kept **relative to the main window**, so windows
docked to it stay docked when it moves between runs.

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `scale` | a number above `0` | `1.0` | Screen pixels per skin pixel when the positions were saved. |
| `main` | `[x, y]` | none | The main window's top-left corner. |

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

### `[windows.eq]` and `[windows.playlist]`

| Key | Values | Default | What it controls |
| --- | --- | --- | --- |
| `open` | `true` / `false` | `true` | Whether the window is shown (**Alt+G** for the equalizer, **Alt+E** for the playlist). |
| `offset` | `[dx, dy]` | none | From the main window's top-left corner to this window's, in screen pixels at `scale`. |
| `size` | `[width, height]` (playlist only) | `[275, 116]` | The playlist's size, in skin pixels. |

- **`offset`** is remembered for a closed window too, so it reopens where it
  was. Without it, the equalizer docks under the main window, and the
  playlist under the equalizer (or under the main window when the equalizer
  is elsewhere or closed). At 1x, docked under the main window is `[0, 116]`
  and under the equalizer `[0, 232]`; at 1.5x these are `[0, 174]` and
  `[0, 348]`. At startup, an offset that would put the window's title bar
  off every screen is ignored.
- **`size`** is in skin pixels, so it does not change with the zoom. The
  playlist grows in steps of 25 by 29 pixels from its smallest size, 275 by
  116, as when resized from its grip. Other values are moved to the nearest
  step, and anything smaller becomes the smallest size. The equalizer has a
  fixed size and ignores this key.

### Platform notes

- **Pure Wayland:** the desktop does not let applications read or set window
  positions, so NeoAmp neither restores nor updates `main` and the offsets
  there. Values saved by an X11 session are kept for the next one.
- **Minimized windows:** while a window is minimized, its position is not
  saved (Windows reports one far off the screen), and the last position from
  before is kept.

## `playlist.m3u8`

The playlist is an extended M3U file, which other players can open too:

```text
#EXTM3U
#EXTINF:213,Nullsoft - Winamp 2.91 Theme
/home/me/Music/theme.mp3
#EXTINF:-1,02 Untitled
/home/me/Music/02 Untitled.flac
```

- Each entry is an optional `#EXTINF:<seconds>,<title>` line followed by the
  file's path. `-1` means the length is not known yet.
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

## What is not saved

- Whether a track was playing. NeoAmp always starts stopped, as Winamp did.
- How far into the track it was.
- Which playlist rows were selected, and how far the playlist was scrolled
  (it scrolls to the current entry).
- The output device. NeoAmp uses the system's default device.

## The command line and the settings

Anything given on the command line wins over the saved settings and is then
remembered like any other change:

- Audio files or folders replace the saved playlist, and the first one plays.
  Without any, the saved playlist comes back.
- A `.wsz` file (or `--skin`) replaces the saved skin.
- `--scale` replaces the saved zoom.
- `--screenshot` neither reads nor writes either file.
