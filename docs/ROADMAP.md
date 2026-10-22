# Roadmap: the Winamp 2.x features NeoAmp still lacks

This list came from reading the code on 2026-10-13, not from running the app.
The phases are in the suggested order of work. Tick items off as they land.

## Phase 1: Saved settings

Done: `settings.toml` and `playlist.m3u8` in the platform's config folder
([src/settings.rs](../src/settings.rs)). See [Saved settings](GUIDE.md#saved-settings) in the user guide,
and [SETTINGS.md](SETTINGS.md) for every key.

- [x] Save and restore volume, balance, EQ (on, preamp, bands), shuffle,
      repeat, time mode, visualizer mode, which windows are open, window
      positions, playlist size, skin, zoom and double size.
- [x] Save and restore the playlist and its current entry.

## Phase 2: Popup menus

Done: menus drawn like Windows 2000's in windows of their own
([src/menu.rs](../src/menu.rs), [src/menus.rs](../src/menus.rs),
[src/popup.rs](../src/popup.rs)). See [Menus](GUIDE.md#menus) in the user guide.

- [x] Decide: native menus or menus drawn in the skin canvas. Drawn, in the
      classic Windows grey, each in its own borderless window: native menus
      need GTK on Linux, and a menu drawn in the skin canvas would be cut off
      at the window's edge.
- [x] Build the popup-menu component.
- [x] Main menu, from the title-bar **O** button, right-click and Alt+F: Play
      file, folder or URL (Phase 6); Skins; Options; Playback; Visualization;
      Exit.
- [x] **O** opens the options menu.
- [x] **A** toggles always on top.
- [x] **I** opens file info (see Phase 5).
- [x] **V** opens the visualizer menu.
- [x] EQ **PRESETS**: built-in presets (Rock, Classical, …), load and save,
      `.eqf` import, Reset / Flat.
- [x] EQ **AUTO**: load a preset per file.
- [x] Visualizer options. Analyzer: fire or line style, thick or thin bars,
      peaks on/off, falloff speeds. Oscilloscope: dots, lines or solid.
- [x] Skins menu (Alt+S), listing NeoAmp's skins folder. There is no
      separate skin browser window.

## Phase 3: Playlist

Done (ADD > URL came with Phase 6): see [Playlist](GUIDE.md#playlist) in the user guide.
Editing is in [src/ui.rs](../src/ui.rs) and [src/playlist.rs](../src/playlist.rs),
playlist files in [src/playlist_file.rs](../src/playlist_file.rs) and
[src/pls.rs](../src/pls.rs), and the Jump to file dialog in
[src/jump.rs](../src/jump.rs).

- [x] Make the ADD, REM, SEL, MISC and LIST OPTS menus act: letting go over
      an item chooses it, and a click chooses the bottom item, as in Winamp.
  - [x] ADD: file, folder, and URL (Phase 6).
  - [x] REM: remove selected, crop, clear, and MISC REM > Remove all dead
        files.
  - [x] SEL: select all, none, invert.
  - [x] MISC: sort (title, file name, path), reverse, randomize, and FILE
        INFO (Phase 5); MISC OPTIONS does nothing.
  - [x] LIST OPTS: new, save and load list.
- [x] Shift- and Ctrl-click multi-select (Cmd-click on macOS).
- [x] Drag rows to reorder, and Alt+Up / Alt+Down.
- [x] Keys: Delete (Ctrl: crop, Alt: dead files), Ctrl+A, Ctrl+I, arrows,
      Enter, Home/End, PgUp/PgDn, Ctrl+Shift+1/2/3, Ctrl+R, Ctrl+Shift+R,
      Ctrl+N/O/S. They work while the playlist has the focus; Ctrl+A still
      toggles always on top in the other windows.
- [x] Mouse wheel: scroll the playlist, and set the volume over the main
      window.
- [x] Make shuffle shuffle: each entry once per round, Previous retraces.
- [x] Load and save M3U, M3U8 and PLS, from LIST OPTS, the command line and
      drag-and-drop, with relative paths, `file://` URLs, Windows-1252 text
      and Windows-style backslashes.
- [x] J: jump to file, a Windows 2000-style dialog with a search box.
- [x] Shift+L: open a folder (Play Folder... in the main menu).
- [x] A track whose entry is removed while it plays plays on, as in Winamp.

## Phase 4: Windowshade mode

Done: see [Windowshade](GUIDE.md#windowshade) in the user guide. The bars are drawn by `draw_shade` in
[src/main_window.rs](../src/main_window.rs),
[src/eq_window.rs](../src/eq_window.rs) and
[src/playlist_window.rs](../src/playlist_window.rs), from the sprites in
`main_shade`, `eq_shade` and `pl_shade` in [src/sprites.rs](../src/sprites.rs).

- [x] Collapsed main window, EQ and playlist, from each window's shade button,
      by double-clicking a title bar, and with Ctrl+W (the main window, or in
      the playlist the playlist). Windows docked below move with it, and the
      state is saved.
- [x] Load `eq_ex.bmp` (needed by the collapsed EQ).
- [x] Read the `[WindowShade]` and `[EqualizerWS]` sections of `region.txt`.

## Phase 5: Smaller items

Done except Ctrl+P, which waits for a Preferences dialog (see Optional);
Ctrl+L came with Phase 6. See [File info](GUIDE.md#file-info), [Skin fonts and
pointers](GUIDE.md#skin-fonts-and-pointers) and
[Stop with fadeout](GUIDE.md#stop-with-fadeout) in the user guide.

- [x] More formats: symphonia's `aac`, `alac`, `isomp4` (M4A), `aiff` and
      `mpa` (MP1, MP2, MP3) features, listed in `FORMATS` in
      [src/audio/mod.rs](../src/audio/mod.rs).
- [x] Show the current song in the OS window title, for the taskbar and
      Alt+Tab: "3. Artist - Title - NeoAmp [Paused]", as Winamp wrote it.
- [x] Honour `Font=` in `pledit.txt`, with built-in free fonts standing in
      for the families skins name ([src/font.rs](../src/font.rs)).
- [x] Use the skin's cursors (`.cur` files, [src/cursor.rs](../src/cursor.rs)),
      borrowing the Winamp base skin's where a skin has none.
- [x] File info dialog (the **I** button, Alt+3, MISC > FILE INFO),
      [src/info.rs](../src/info.rs). It shows the tags but cannot edit them
      (the user chose read-only over writing tags with `lofty`).
- [ ] Shortcuts:
  - [x] Ctrl+L (open URL), with Phase 6.
  - [x] Alt+3 (file info).
  - [ ] Ctrl+P (preferences) needs a Preferences dialog (see Optional).
  - [x] Shift+V (fade out and stop), and Shift-click on Stop.

## Phase 6: Internet radio

Done: see [Internet radio](GUIDE.md#internet-radio) in the user guide. Winamp's signature feature,
SHOUTcast and Icecast streams. The user chose, before any code: an HTTP client
of NeoAmp's own on rustls (SHOUTcast 1.x answers `ICY 200 OK`, which
ureq and hyper reject), the station's name as the entry's title with the song
in the scrolling title, an Open URL dialog that pastes and starts with the
clipboard's address, and reconnecting a few times before moving on.

- [x] An HTTP streaming source for symphonia ([src/http.rs](../src/http.rs),
      [src/audio/stream.rs](../src/audio/stream.rs)): HTTPS, redirects,
      chunked bodies, playlists that lead to streams, reconnection.
- [x] Open URL (Ctrl+L, Play URL..., ADD > URL), with clipboard paste
      ([src/url_dialog.rs](../src/url_dialog.rs)), and `.pls` / `.m3u` files
      that point at streams.
- [x] Show the station's song title (ICY metadata, or a chained Ogg stream's
      tags) in the scrolling title and the window title.
- [x] Draw the buffering indicator (`WORKING`) while a stream connects or
      buffers, with `[Buffering: 45%]` in the scrolling title.

Left out: HE-AAC's SBR (AAC+ plays as its AAC core only), Opus and HLS
streams, a station directory, and recording streams.

## Phase 7: Playback and platform integration

Done: see [Gapless playback](GUIDE.md#gapless-playback),
[Output device](GUIDE.md#output-device), [Media keys and "now
playing"](GUIDE.md#media-keys-and-now-playing) and [NeoAmp already
running](GUIDE.md#neoamp-already-running) in the user guide. Before any code,
the claim that Windows and macOS go silent until a restart when the device
changes was checked against cpal 0.18's code and its issue tracker
(RustAudio/cpal#1183, #1339): a stream on the default device follows the
system's default by itself on both (Windows 10's automatic stream routing,
macOS's default output unit), so what silenced playback was a *chosen* device
going away, or macOS left with no output at all. The user then chose: an
Output Device submenu under Options; a chosen device that goes away hands
over to the default and is returned to when it comes back; on Linux, the
sound server's devices through cpal's pure-Rust PulseAudio host; and files
opened in a second NeoAmp replace the playlist and play, with `--enqueue` to
add them.

- [x] Gapless playback: the decoder preloads the next file and queues it
      straight after the current one, behind a mark the device callback
      passes ([src/audio/decode.rs](../src/audio/decode.rs),
      [src/audio/output.rs](../src/audio/output.rs)).
- [x] Choose the output device (Options > Output Device), and reopen it
      when it disappears, or go back to it when it returns
      ([src/audio/device.rs](../src/audio/device.rs)).
- [x] Media keys and the OS "now playing" display through `souvlaki`: MPRIS
      on Linux, SMTC on Windows, Now Playing on macOS
      ([src/now_playing.rs](../src/now_playing.rs)).
- [x] Single instance: a file opened from the file manager while NeoAmp runs
      goes to the running copy instead of starting a second one
      ([src/instance.rs](../src/instance.rs)).

Left out: cover art in the "now playing" display, and files opened from the
macOS Finder (they arrive as an Apple Event that winit does not pass on, so
the app claims no file types). Output devices, media controls and the single
instance were tried on Linux only.

## Phase 8: Sharp drawing and NeoAmp's own skin

Done: see [Zoom](GUIDE.md#zoom) and [SVG skins](GUIDE.md#svg-skins) in the
user guide, and [SVG_SKINS.md](SVG_SKINS.md) for skin makers. Not a Winamp
feature: Winamp drew every skin at one screen pixel per skin pixel. The user
chose SVG artwork over xBRZ and Lanczos upscales of the base skin's bitmaps;
SVG sheets inside ordinary `.wsz` files, used at every zoom; sharp drawing
always on, with no setting; and a skin of NeoAmp's own, traced from the base
skin with NeoAmp's name and icon, shown by default.

- [x] Draw each window at the screen's resolution: as many image pixels per
      skin pixel as the screen shows, rounded up
      ([src/canvas.rs](../src/canvas.rs)), so the playlist text and slanted
      window shapes are sharp while bitmaps are enlarged pixel for pixel.
- [x] SVG sheets in skins (`cbuttons.svg` beside or instead of
      `cbuttons.bmp`), rasterised at each window's density
      ([src/svg.rs](../src/svg.rs)).
- [x] NeoAmp's own skin, all SVG (`skins/NeoAmp.wsz`, drawn by
      `skins/neo/make.py`), shown by default. Winamp's base skin stays built
      in, as **<Winamp Base Skin>** in the Skins menu and as the fallback
      every skin borrows from.
- [ ] 1x bitmaps in `skins/NeoAmp.wsz`, so that it shows in Winamp and
      Webamp too.
- [x] Skins > Skin Museum...: the Winamp Skin Museum's skins, searched and
      tried on NeoAmp's own windows before installing
      ([src/museum_dialog.rs](../src/museum_dialog.rs),
      [src/museum.rs](../src/museum.rs)); see [Skin
      Museum](GUIDE.md#skin-museum). In place of Winamp's skin browser, which
      listed the skins folder and linked to winamp.com for more. The user
      chose the museum's own order before a search, search over every skin
      not flagged NSFW, trying on after half a second's rest, a 100 MB cache
      and a dialog opened beside the windows, with no key of its own. The
      museum's screenshots were shown beside the list at first, then taken
      out at the user's request: the windows wearing the skin show it.
- [ ] Open a `https://.../*.wsz` address given on the command line as a
      skin, downloading it first.

Sharp drawing and the SVG sheets were run on Linux (X11) and macOS; not yet
on Windows.

## Optional

- A Preferences dialog. Most settings can live in menus instead.
- Opus and tracker modules (MOD, XM, S3M, IT). symphonia has neither, so both
  need other crates.

## Out of scope

Media Library, the minibrowser, the video window, AVS and MilkDrop
visualizers, a plugin system, modern `.wal` skins and CD ripping.
