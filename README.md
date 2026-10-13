# Winamp 2.x classic interface in Rust

A pixel-exact reproduction of the classic Winamp 2.x interface (main window,
equalizer and playlist editor), drawn with real Winamp skins, that plays
MP3, WAV, FLAC and Ogg Vorbis files. The transport buttons, position bar,
volume, balance and ten-band equalizer all act on the audio, and the
visualizer shows what is actually playing.

![Winamp 2.x classic interface](docs/screenshot@2x.png)

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
- `src/app.rs` runs one borderless `winit` window per Winamp window, presents
  each framebuffer through `pixels` (wgpu) and drives the title-bar drags and
  docking. `src/upscale.rs` draws the framebuffer over the whole window (see
  [Scaling](#scaling)), and `src/zoom.rs` picks the zoom.
- `src/audio/` plays the music (see [Audio](#audio)), and `src/vis.rs` turns
  the samples being played into the spectrum analyzer and oscilloscope.
- `src/lib.rs` exposes all of the above as a library; `src/main.rs` only
  parses the command line. `cargo test` covers layout, hit testing, input,
  the skin parsers, the blitter and the audio pipeline (decoding, resampling,
  the equalizer, and the decoder and mixer threads working together).

Each Winamp window is its own borderless OS window. Grab a title bar to
drag one away; the main window carries whatever is docked to it, while the
equalizer and playlist leave on their own. Windows snap to each other's
edges and to the screen edges when they come within 10 pixels, like the
original. `src/dock.rs` holds that geometry.

Docking needs to read and set window positions, which Wayland forbids, so
the app prefers the X11 backend (XWayland on a Wayland desktop) whenever
`DISPLAY` is set. Some compositors, COSMIC among them, also ignore moves of
managed X11 windows; the app notices this at startup and recreates its
windows as override-redirect windows, which are always placed exactly where
asked. Such windows have no taskbar entry, stay above other windows (like
Winamp's "always on top") and take keyboard focus when clicked.

- `WINAMP_BACKEND=x11|wayland` forces a backend. On pure Wayland, title-bar
  drags are handed to the compositor and nothing snaps.
- `WINAMP_UNMANAGED=1|0` forces or forbids override-redirect windows instead
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
  and `--scale 1|1.5|2|2.5|3` overrides it at startup.
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

Play files or whole folders (folders add their MP3, WAV, FLAC and Ogg Vorbis
files, sorted by name); the first one starts playing. Dropping files or
folders onto any window adds them to the playlist too, and starts them if
nothing is playing:

```bash
cargo run --release -- ~/Music/some-album
```

Load another classic skin by passing it on the command line or by dropping a
`.wsz` onto the window:

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
| Play a playlist entry | double-click it | |
| Shuffle / Repeat | toggle buttons | S / R |
| Elapsed vs. remaining time | click the time display | Ctrl+T |
| Equalizer / Playlist windows | EQ / PL buttons, window close buttons | Alt+G / Alt+E |
| Double size (doubles the zoom) | the `D` in the clutter bar | Ctrl+D |
| Zoom in / out / back to default | | Ctrl+= / Ctrl+- / Ctrl+0 |
| Volume, balance, EQ bands and preamp | drag the sliders (the marquee shows the value) | Up / Down: volume |
| Seek | drag the position bar; it seeks when you let go | Left / Right: 5 seconds |
| Equalizer on / off | the ON button | |
| Visualizer mode (analyzer, scope, off) | click the visualizer | |
| Move a window (docked ones follow the main window) | drag its title bar | |
| Resize the playlist | drag the grip in its bottom-right corner | |
| Playlist menus | hold ADD / REM / SEL / MISC / LIST OPTS | |

## Scope and limitations

- Classic 2.x skins (`.wsz`) only. Modern `.wal` skins are a different format.
- Windowshade mode, the options menus, cursors (`.cur`) and the minibrowser
  window are not implemented.
- Files come from the command line or drag and drop. Eject opens no file
  dialog, the playlist cannot be edited, and shuffle does not change the
  order yet. EQ presets and AUTO are not implemented.
- Tracks play back to back with a short gap, not gaplessly.
- The output device is opened once, at startup, and never reopened. On Linux
  that device is PipeWire's or PulseAudio's, which move the stream to another
  output when one disappears. On Windows and macOS, losing the device (say,
  unplugging headphones) silences playback until the app restarts.
- The playlist uses a system TrueType font (Arial, Liberation Sans or
  DejaVu Sans, whichever exists) at the size Winamp used.
