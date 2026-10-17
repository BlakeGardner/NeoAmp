<p align="center">
  <img src="resources/icons/neoamp.svg" alt="NeoAmp logo" width="160">
</p>

<h1 align="center">NeoAmp</h1>

<p align="center">
  <strong>A complete recreation of Winamp, with some added modern conveniences.</strong>
</p>

<p align="center">
  Runs on <strong>Windows</strong>, <strong>Linux</strong> and <strong>macOS</strong>.
</p>

<p align="center">
  <a href="https://github.com/BlakeGardner/NeoAmp/releases">Download</a> ·
  <a href="docs/GUIDE.md">User guide</a> ·
  <a href="docs/SHORTCUTS.md">Shortcuts</a> ·
  <a href="docs/ROADMAP.md">Roadmap</a>
</p>

![NeoAmp's main window, equalizer and playlist](docs/screenshot@2x.png)

NeoAmp is Winamp 2.x the way you remember it: the main window, the ten-band
equalizer and the playlist editor, drawn pixel for pixel from classic `.wsz`
skins, docked together and snapping to each other's edges. The buttons,
sliders, menus and shortcuts work the way Winamp's did.

Underneath, it's a new native player written in Rust, and it knows a few
things Winamp never had to: media keys, high-resolution screens, and
Bluetooth headphones that come and go.

## The goal

A modern and complete recreation of Winamp, with some added modern
conveniences, like media key controls.

- **Complete.** Every window, button, menu and keyboard shortcut of the
  classic Winamp player, acting as it did. Where Winamp's behaviour was never
  written down, [Webamp](https://webamp.org/)'s careful reconstruction is the
  reference.
- **Modern.** A native app for today's desktops and screens that fits into
  the system around it: its media keys, its "now playing" display, its sound
  devices.

## Features

### Winamp, as you remember it

- **Any classic skin.** Drop a `.wsz` onto the window to wear it, with the
  skin's own colours, playlist font, window shapes and mouse pointers.
  Thousands are in the [Winamp Skin Museum](https://skins.webamp.org/).
- **Docking windows.** Drag the equalizer and playlist away, snap them back,
  and roll any window up into a 14-pixel windowshade bar.
- **The real equalizer.** Ten bands and a preamp, Winamp's presets and your
  own, `.eqf` files, and presets that load per file.
- **The visualizer.** Spectrum analyzer and oscilloscope, with Winamp's
  styles and falloff, showing what's actually playing.
- **The playlist editor.** Sort, shuffle, crop, remove dead files, drag to
  reorder, M3U and PLS playlists, and **J** to jump to any track by typing.
- **Winamp's keys.** Z X C V B for the transport, and the rest of the
  shortcuts your fingers still remember.

### Plays what you have

- MP3, AAC and Apple Lossless (M4A), FLAC, Ogg Vorbis, WAV and AIFF.
- SHOUTcast and Icecast internet radio, with each station's song titles
  scrolling by.
- Gapless playback, so a live album or a DJ mix plays without breaks
  between tracks.

### Modern conveniences

- **Media keys and "now playing".** Play, pause and skip from the keyboard's
  media keys whatever has the focus, and see the track in the system's media
  controls: MPRIS on Linux, the media overlay on Windows, Control Center on
  macOS.
- **Sharp on any screen.** The windows zoom from 1x to 3x to suit your
  monitor, and the pixels stay crisp.
- **Follows your headphones.** Pick any output device. When it goes away,
  playback moves to the default and comes back when it returns, without
  losing its place.
- **Remembers everything.** Volume, equalizer, skin, window positions and the
  playlist are saved as you go.
- **One NeoAmp at a time.** Opening another file hands it to the NeoAmp
  already running instead of starting a second one.
- **Looks the same everywhere.** Fonts are built in, so a skin looks
  identical on Linux, Windows and macOS.

## Download

Get NeoAmp from the
[releases page](https://github.com/BlakeGardner/NeoAmp/releases):

| System | Download |
| --- | --- |
| Windows 10 and 11 | an installer, or a zip to run without installing |
| macOS 11 or newer | a disk image, for Apple silicon or Intel |
| Linux (x86-64) | a Flatpak bundle, or the program in a `.tar.gz` |

The Windows and macOS builds aren't signed yet, so the system asks before
running them the first time. The [user guide](docs/GUIDE.md#installing)
shows how to get past that on each.

## Getting started

Start NeoAmp and drop music onto it: files, folders or playlists. Or:

- **L** opens files, **Shift+L** a folder, and **Ctrl+L** an internet radio
  station's address.
- **Z X C V B** are previous, play, pause, stop and next.
- **J** finds a track in the playlist as you type.
- **Alt+S** opens the Skins menu, or drop a `.wsz` skin onto any window.

On macOS, use ⌘ for Ctrl and ⌥ for Alt. [SHORTCUTS.md](docs/SHORTCUTS.md)
lists every key, and the [user guide](docs/GUIDE.md) covers everything
NeoAmp does.

## Building from source

With Rust 1.88 or newer, from the top of this repository:

```bash
cargo run --release
```

On Linux, install ALSA's development files first (`libasound2-dev` on
Debian and Ubuntu); Windows and macOS need nothing extra.
[DEVELOPMENT.md](docs/DEVELOPMENT.md) covers the rest: how NeoAmp is put
together, the checks to run before sending a change, and how the release
packages are built.

## Where it stands

The main window, equalizer and playlist editor already do nearly everything
Winamp 2.x's did. The [roadmap](docs/ROADMAP.md) tracks what's still missing,
and the user guide lists the
[current limitations](docs/GUIDE.md#limitations).

## Documentation

- [User guide](docs/GUIDE.md): installing, everything NeoAmp does, and how
  it differs between Linux, Windows and macOS.
- [Keyboard shortcuts](docs/SHORTCUTS.md): every key, with its macOS spelling.
- [Settings](docs/SETTINGS.md): what NeoAmp saves, and where.
- [Development](docs/DEVELOPMENT.md): building, how it works, and packaging.
- [Roadmap](docs/ROADMAP.md): what's done and what's next.

---

NeoAmp is an independent project, not affiliated with or endorsed by the
makers of Winamp. Winamp is a trademark of its owner.
