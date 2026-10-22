# Developing NeoAmp

How to build NeoAmp from source, how it is put together, and how the release
packages are made. What NeoAmp does, and how it differs between platforms, is
in the [user guide](GUIDE.md).

- [Building and running](#building-and-running)
- [Before sending a change](#before-sending-a-change)
- [How it works](#how-it-works)
- [Release packages](#release-packages)

## Building and running

NeoAmp needs Rust 1.88 or newer. Two skins in the repository are embedded
into the binary at build time: `skins/NeoAmp.wsz`, NeoAmp's own skin and the
one it shows by default (see [The NeoAmp skin](#the-neoamp-skin)), and the
stock Winamp 2.91 skin, `skins/base-2.91.wsz`, which every other skin
borrows what it lacks from.

On Linux, cpal needs the ALSA development files (Windows and macOS need
nothing extra):

```bash
sudo apt install libasound2-dev
```

HTTPS for internet radio comes from rustls, whose cryptography (ring)
compiles a little C and assembly with the C compiler Rust already uses on each
platform; it needs no system library. The media controls, the list of output
devices and handing files to a running NeoAmp are pure Rust on Linux and use
only the system's own libraries on Windows and macOS, so they need nothing
extra either. The file dialogs reach the XDG desktop portal on Linux through
the system's `libdbus`, which is loaded at runtime, so building needs no
package for it. On Windows, `build.rs` builds the icon into `neoamp.exe` with
the Windows SDK's resource compiler, which comes with the Visual Studio Build
Tools that Rust's MSVC toolchain needs anyway.

```bash
cargo run --release
```

On Windows a release build is a windowed program (see
[Windows](GUIDE.md#windows) in the user guide), so it prints nothing to the
terminal; a debug build, `cargo run`, keeps the console and its log.

Everything after `--` goes to NeoAmp, with the options described in
[The command line](GUIDE.md#the-command-line):

```bash
cargo run --release -- ~/Music/some-album
```

Render a frame to a PNG (always at 1x) without opening a window. It shows a
made-up playlist, with a synthetic signal in the visualizer, and neither
reads nor writes the saved settings:

```bash
cargo run --release -- --screenshot shot.png
```

## Before sending a change

Run these from the top of the repository; all three must pass:

```bash
cargo fmt
```

```bash
cargo clippy --all-targets -- -D warnings
```

```bash
cargo test
```

Anything new must build, run and behave the same on Linux, Windows and macOS.
Where one platform cannot do something, write the difference down in the
[user guide](GUIDE.md). A change to a key updates
[SHORTCUTS.md](SHORTCUTS.md), and a change to what is saved updates
[SETTINGS.md](SETTINGS.md).

## How it works

Classic Winamp skins (`.wsz` files) are zip archives of fixed-size sprite
sheets (`main.bmp`, `cbuttons.bmp`, `titlebar.bmp`, ...) plus a few text
files. Winamp itself just blitted rectangles out of those sheets, so this
program does the same:

- `src/canvas.rs` is a small RGBA software framebuffer with clipped blits.
  The windows and the skin sheets are `Canvas`es, which take every position
  and size in skin pixels and draw each skin pixel as a `Density` × `Density`
  block of image pixels. `app.rs` draws each window at its scale (zoom,
  double size and the desktop's scale) rounded up, `Density::covering`: skin
  sheets are enlarged pixel for pixel as they are blitted, the playlist text
  is rasterised at that density, and the shapes from `region.txt` are cut at
  it. `src/upscale.rs` shrinks a canvas finer than the screen by averaging.
  `--screenshot` draws at density 1.
- `src/sprites.rs` holds the sprite coordinates and window layout, which are
  identical for every classic skin.
- `src/skin.rs` reads a `.wsz` (zip + BMP/PNG), `viscolor.txt`,
  `pledit.txt`, `region.txt` and the mouse pointers (`src/cursor.rs`), and
  falls back per file to the embedded Winamp base skin for anything a skin
  leaves out, NeoAmp's own skin included (see [Skin fonts and
  pointers](GUIDE.md#skin-fonts-and-pointers)). A
  sheet can also come as SVG ([SVG_SKINS.md](SVG_SKINS.md)): `src/svg.rs`
  parses it with usvg and rasterises it with resvg at each density a window
  asks for, caching each raster. resvg is built without its default features,
  so it reads no fonts, decodes no embedded bitmaps, and `svg.rs` keeps it
  from loading files an SVG names. Archives are untrusted: `skin.rs` reads at
  most 32 MB of `.wsz`, unpacks only the files a skin is made of, each at most
  8 MB and 32 MB together, refuses archives listing over 4096 files, and
  reads a bitmap's size before decoding it.
- `src/main_window.rs`, `src/eq_window.rs`, `src/playlist_window.rs` draw the
  three windows and their windowshade bars; `src/ui.rs` holds state, hit
  testing and input.
- `src/app.rs` runs one borderless `winit` window per NeoAmp window, presents
  each framebuffer through `pixels` (wgpu) and drives the title-bar drags and
  docking; `src/dock.rs` holds the docking geometry. `src/upscale.rs` draws
  the framebuffer over the whole window (see [Zoom](GUIDE.md#zoom)), and
  `src/zoom.rs` picks the zoom.
- `src/menu.rs` draws popup menus and handles the pointer and keyboard in
  them; `src/menus.rs` holds NeoAmp's own menus, and `src/popup.rs` gives each
  open menu its window (see [Menus](#menus)). `src/eqf.rs` reads and writes
  equalizer presets, and `src/library.rs` lists the skins and presets folders.
- `src/file_dialog.rs` shows the platform's file dialogs through
  [rfd](https://crates.io/crates/rfd) (see
  [Opening files](GUIDE.md#opening-files)).
- `src/playlist.rs` sorts, moves and shuffles playlist entries;
  `src/playlist_file.rs` loads and saves playlist files through `src/m3u.rs`
  and `src/pls.rs` (see [Playlist](GUIDE.md#playlist)).
- `src/jump.rs` is the Jump to file dialog, `src/info.rs` the File info
  dialog and `src/url_dialog.rs` the Open URL dialog; `src/jump_window.rs`,
  `src/info_window.rs` and `src/url_window.rs` give them windows through
  `src/dialog_window.rs`. `src/text_box.rs` is their edit box,
  `src/list_box.rs` their list with its scroll bar, `src/clipboard.rs` reads
  the clipboard for the edit box through
  [arboard](https://crates.io/crates/arboard), and `src/classic.rs` holds
  the Windows 2000 colours, bevels and buttons they share with the menus.
- `src/museum_dialog.rs` is the Skin Museum dialog (see [Skin
  Museum](GUIDE.md#skin-museum)) and `src/museum_window.rs` its window, which
  fetches what the dialog asks for on threads of their own, each sending
  back what it brought over a channel, and hands a skin to try on to
  `app.rs`, which wears it in place of the chosen one until it is installed
  or the dialog closes. `src/museum.rs` talks to the museum's GraphQL API
  (plain `GET` requests through `src/http.rs`, answers read with
  [serde_json](https://crates.io/crates/serde_json)) and keeps the skins
  it fetched in a cache folder of at most 100 MB.
- `src/audio/` plays the music (see [Audio](#audio)), internet radio included
  (`src/audio/stream.rs`, fetched by `src/http.rs`; see
  [Internet radio](#internet-radio)), and `src/vis.rs` turns the samples being
  played into the spectrum analyzer and oscilloscope.
- `src/now_playing.rs` joins the system's media controls through
  [souvlaki](https://crates.io/crates/souvlaki), and `src/instance.rs` hands
  files to a NeoAmp that is already running (see
  [Single instance](#single-instance)).
- `src/font.rs` draws text in the built-in fonts with
  [fontdue](https://crates.io/crates/fontdue), not through the system, so the
  same font gives the same pixels everywhere.
- `src/settings.rs` remembers the settings and the playlist between runs (see
  [Saved settings](GUIDE.md#saved-settings)); `src/m3u.rs` reads and writes
  the playlist file.
- `src/lib.rs` exposes all of the above as a library; `src/main.rs` only
  parses the command line. `cargo test` covers layout, hit testing, input,
  playlist editing and playlist files, the skin parsers, the blitter, the
  dialogs and menus, the saved settings and playlist, the audio pipeline
  (decoding, resampling, the equalizer, and the decoder and mixer threads
  working together), and the HTTP client, internet radio and the Skin
  Museum's requests, against small servers the tests run on the computer
  itself. No test reaches the real museum.

### The NeoAmp skin

`skins/NeoAmp.wsz` is a skin drawn entirely as SVG
([SVG_SKINS.md](SVG_SKINS.md)): the base skin's layout and colours, traced,
with NeoAmp's name and icon in place of Winamp's. It is built into NeoAmp as
the default skin (`NEOAMP_SKIN` in `src/app.rs`), and takes its mouse
pointers from the Winamp base skin, as any skin without them does; the
Skins menu's **<Winamp Base Skin>** shows the base skin itself.
`skins/neo/make.py` draws
it. It needs only Python 3 and its standard library (`py` instead of
`python3` on Windows), and nothing else in the build uses it:

```bash
python3 skins/neo/make.py
```

It writes an SVG for every sheet but the optional `nums_ex` (which the base
skin lacks too), `viscolor.txt` and `pledit.txt` into `skins/neo/`, and packs
them into `skins/NeoAmp.wsz`. Change the script rather
than the SVGs, and commit the script, the SVGs and the `.wsz` together; then
rebuild, since the `.wsz` is embedded. The skin has no bitmaps, so Winamp and
Webamp, which ignore SVG sheets, show their own base skin instead.

### Audio

Playback follows Winamp's own input plugin, DSP, output plugin chain, spread
over three threads:

- A decoder thread reads the file with [symphonia](https://crates.io/crates/symphonia)
  (pure Rust: MP3, MP1 and MP2, AAC, AAC and Apple Lossless in MP4, WAV,
  AIFF, FLAC, Ogg Vorbis), converts it to stereo, resamples it
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

**Gapless playback.** While a file plays, the interface tells the decoder
which file comes next. Once the current file is decoded to its end, the
decoder opens the next one and queues its audio straight after, with a mark
that tells the callback where it begins; as its first sample plays, the
interface hears of it and moves on.

**Output devices.** On Linux the device list comes from cpal's PulseAudio
host: a pure-Rust client of the PulseAudio protocol, which PipeWire speaks
too, so building needs nothing more. Without a sound server it falls back to
ALSA's hardware devices.

A background thread reads tags and lengths for the playlist as files are
added.

### Menus

Each open menu is a small window of its own, since a menu reaches past the
NeoAmp window it opens from. These windows, like the dialogs', are drawn in
software with [softbuffer](https://crates.io/crates/softbuffer), from the
people behind winit, because they must appear at once: setting up a GPU
surface, as the main windows use, can take a tenth of a second. softbuffer
needs no system libraries to build.

### Internet radio

`src/http.rs` is a small HTTP/1.1 client written for this, because
SHOUTcast 1.x servers answer `ICY 200 OK` instead of `HTTP/1.0 200 OK`,
which general-purpose clients reject. HTTPS uses
[rustls](https://crates.io/crates/rustls) with Mozilla's root certificates
built in ([webpki-roots](https://crates.io/crates/webpki-roots)), so every
platform checks certificates the same way. `src/audio/stream.rs` connects on
a thread of its own, follows playlists, takes the song titles out of the
audio and passes the audio to the decoder thread through a buffer, which
symphonia reads like a file that cannot seek.

### Media controls

souvlaki uses MPRIS on Linux, talking to D-Bus through the pure-Rust zbus, so
building needs no `libdbus`; the System Media Transport Controls on Windows;
and Now Playing on macOS.

### Single instance

The running copy listens on a local socket named after its config folder,
through [interprocess](https://crates.io/crates/interprocess): a Unix domain
socket in `$XDG_RUNTIME_DIR` on Linux (the config folder without one), in
the user's own temporary folder on macOS, and a named pipe on Windows.

## Release packages

Publishing a release on GitHub builds every package and attaches them to it;
the steps are at the top of `.github/workflows/release.yml`. Each package
below can also be built by hand.

### Linux archive

The Release workflow builds `neoamp` and packs it with `README.md` into
`neoamp-v<version>-linux-x86_64.tar.gz`.

### Flatpak

The bundle is built from `flatpak/io.github.BlakeGardner.NeoAmp.yml`, with
the desktop file, metainfo and icon in `resources/`. To build and install it
yourself, from the top of this repository, install flatpak-builder:

```bash
flatpak install --user flathub org.flatpak.Builder
```

List the crates for it to download, with
[flatpak-cargo-generator.py](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo)
from flatpak-builder-tools (it needs Python's `aiohttp`, `tomlkit` and
`PyYAML`):

```bash
python3 flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json
```

Then build and install it:

```bash
flatpak run org.flatpak.Builder --user --install --install-deps-from=flathub --force-clean flatpak_app flatpak/io.github.BlakeGardner.NeoAmp.yml
```

The `Flatpak` workflow (`.github/workflows/flatpak.yml`) does the same on
GitHub for each release, and only then.

### macOS app

The disk image is built by `macos/build-dmg.sh`, from `macos/Info.plist` and
the icon `resources/icons/neoamp-macos.svg`. To build it yourself on a Mac,
add Rust's target for the kind of Mac to build for (`x86_64-apple-darwin`
for Intel), and resvg, which draws the icon:

```bash
rustup target add aarch64-apple-darwin
```

```bash
cargo install --locked resvg
```

Then, from the top of this repository, build for `arm64` or `x86_64`
(without either, it builds for the Mac it runs on):

```bash
macos/build-dmg.sh arm64
```

It writes `target/macos/arm64/neoamp-v<version>-macos-arm64.dmg`, and leaves
the app itself in `target/macos/arm64/stage/`. The `macOS` workflow
(`.github/workflows/macos.yml`) builds both on GitHub for each release, and
only then.

### Windows installer

The installer is made with [Inno Setup](https://jrsoftware.org/isinfo.php)
from `windows/neoamp.iss`. `build.rs` builds the icon,
`resources/icons/neoamp.ico`, and the program's version details into
`neoamp.exe`; `windows/make-icon.py` draws the icon from
`resources/icons/neoamp.svg`, and is run again whenever the SVG changes. To
build both yourself on Windows, install Rust's MSVC toolchain and Inno Setup
6 (`winget install JRSoftware.InnoSetup`), then, from the top of this
repository:

```bat
powershell -ExecutionPolicy Bypass -File windows\package.ps1
```

It writes the installer and the zip to `target\windows\`. The `Windows`
workflow (`.github/workflows/windows.yml`) builds them on GitHub for each
release, and only then. It also tries the installer there: installs it,
starts NeoAmp, takes a screenshot, and uninstalls it.
