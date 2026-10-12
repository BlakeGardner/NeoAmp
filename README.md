# Winamp 2.x classic interface in Rust

A pixel-exact reproduction of the classic Winamp 2.x interface (main window,
equalizer and playlist editor), drawn with real Winamp skins. It is an
interface only: nothing plays audio, but every button, slider and toggle
reacts the way the original did.

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
- `src/lib.rs` exposes all of the above as a library; `src/main.rs` only
  parses the command line. `cargo test` covers layout, hit testing, input,
  the skin parsers and the blitter.

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

```bash
cargo run --release
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

Render a frame to a PNG (always at 1x) without opening a window:

```bash
cargo run --release -- --screenshot shot.png
```

## Controls

| Action | Mouse | Keyboard |
| --- | --- | --- |
| Previous / Play / Pause / Stop / Next | transport buttons | Z X C V B |
| Shuffle / Repeat | toggle buttons | S / R |
| Elapsed vs. remaining time | click the time display | Ctrl+T |
| Equalizer / Playlist windows | EQ / PL buttons, window close buttons | Alt+G / Alt+E |
| Double size (doubles the zoom) | the `D` in the clutter bar | Ctrl+D |
| Zoom in / out / back to default | | Ctrl+= / Ctrl+- / Ctrl+0 |
| Volume, balance, seek, EQ bands | drag the sliders (the marquee shows the value) | arrows: volume and seek |
| Visualizer mode (analyzer, scope, off) | click the visualizer | |
| Move a window (docked ones follow the main window) | drag its title bar | |
| Resize the playlist | drag the grip in its bottom-right corner | |
| Playlist menus | hold ADD / REM / SEL / MISC / LIST OPTS | |

## Scope and limitations

- Classic 2.x skins (`.wsz`) only. Modern `.wal` skins are a different format.
- Windowshade mode, the options menus, cursors (`.cur`) and the minibrowser
  window are not implemented.
- The playlist uses a system TrueType font (Arial, Liberation Sans or
  DejaVu Sans, whichever exists) at the size Winamp used.
