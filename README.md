<p align="center">
  <img src="resources/icons/neoamp.svg" alt="NeoAmp logo" width="160">
</p>

<h1 align="center">NeoAmp</h1>

<p align="center">
  <strong>Classic Winamp, rebuilt for modern Windows, Mac and Linux.</strong>
</p>

<p align="center">
  Every button, menu and shortcut you remember, plus what Winamp never had:
  media keys, sharp high-res screens, and a built-in browser to try on and
  install over 100,000 classic skins.
</p>

<p align="center">
  <a href="https://github.com/BlakeGardner/NeoAmp/releases/latest"><strong>Download NeoAmp</strong></a>
  ·
  <a href="docs/GUIDE.md">User guide</a>
</p>

<p align="center">
  <img src="docs/screenshot@2x.png" alt="NeoAmp's main window, equalizer and playlist">
</p>

NeoAmp is Winamp 2.x the way you remember it: the main window, the ten-band
equalizer and the playlist editor, drawn pixel for pixel from classic `.wsz`
skins, docked together and snapping to each other's edges. The buttons,
sliders, menus and shortcuts work the way Winamp's did, and where Winamp's
behaviour was never written down, [Webamp](https://webamp.org/)'s careful
reconstruction is the reference.

Underneath, it's a new native player written in Rust, made to fit the
desktop around it: its media keys, its "now playing" display, its sound
devices.

## Features

### Winamp, as you remember it

- **Any classic skin.** Drop a `.wsz` onto the window to wear it, with the
  skin's own colours, playlist font, window shapes and mouse pointers. Or
  open Skins > Skin Museum... to search the
  [Winamp Skin Museum](https://skins.webamp.org/)'s thousands and try them
  on before you keep one.
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
  monitor, the playlist text stays crisp, and skins can bring SVG artwork that
  stays sharp at any size, as NeoAmp's own skin does.
- **Follows your headphones.** Pick any output device. When it goes away,
  playback moves to the default and comes back when it returns, without
  losing its place.
- **Opens from your file manager.** Double-click music, playlists and skins,
  or choose NeoAmp under Open With, and they play in the NeoAmp that's
  already open. Explorer gets Winamp's Play and Enqueue in NeoAmp, the Mac
  its Dock icon.
- **Remembers everything.** Volume, equalizer, skin, window positions and the
  playlist are saved as you go.
- **Looks the same everywhere.** Fonts are built in, so a skin looks
  identical on Linux, Windows and macOS.

## Download

Get NeoAmp from the
[latest release](https://github.com/BlakeGardner/NeoAmp/releases/latest):

| System | Download |
| --- | --- |
| Windows 10 and 11 | an installer, or a zip to run without installing |
| macOS 11 or newer (Apple silicon) | a disk image |
| macOS 10.12.2 or newer (Intel) | a disk image |
| Linux (x86-64) | a Flatpak bundle, or the program in a `.tar.gz` |

The Windows build isn't signed yet, so Windows asks before running it the
first time. The [user guide](docs/GUIDE.md#windows) shows how to get past
that.

## Getting started

Start NeoAmp and drop music onto it: files, folders or playlists. Or:

- **L** opens files, **Shift+L** a folder, and **Ctrl+L** an internet radio
  station's address.
- **Z X C V B** are previous, play, pause, stop and next.
- **J** finds a track in the playlist as you type.
- **Alt+S** opens the Skins menu, or drop a `.wsz` skin onto any window.
- **Ctrl+Shift+K** opens MilkDrop, the visualizer Winamp came with.

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
