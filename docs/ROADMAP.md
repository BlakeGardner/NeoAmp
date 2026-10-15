# Roadmap: the Winamp 2.x features NeoAmp still lacks

This list came from reading the code on 2026-10-13, not from running the app.
The phases are in the suggested order of work. Tick items off as they land.

## Phase 1: Saved settings

Done: `settings.toml` and `playlist.m3u8` in the platform's config folder
([src/settings.rs](../src/settings.rs)). See "Saved settings" in the README,
and [SETTINGS.md](SETTINGS.md) for every key.

- [x] Save and restore volume, balance, EQ (on, preamp, bands), shuffle,
      repeat, time mode, visualizer mode, which windows are open, window
      positions, playlist size, skin, zoom and double size.
- [x] Save and restore the playlist and its current entry.

## Phase 2: Popup menus

Done: menus drawn like Windows 2000's in windows of their own
([src/menu.rs](../src/menu.rs), [src/menus.rs](../src/menus.rs),
[src/popup.rs](../src/popup.rs)). See "Menus" in the README.

- [x] Decide: native menus or menus drawn in the skin canvas. Drawn, in the
      classic Windows grey, each in its own borderless window: native menus
      need GTK on Linux, and a menu drawn in the skin canvas would be cut off
      at the window's edge.
- [x] Build the popup-menu component.
- [x] Main menu, from the title-bar **O** button, right-click and Alt+F: Play
      file or folder; Skins; Options; Playback; Visualization; Exit. Play URL
      waits for Phase 6.
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

Done except ADD > URL, which waits for Phase 6: see "Playlist" in the README.
Editing is in [src/ui.rs](../src/ui.rs) and [src/playlist.rs](../src/playlist.rs),
playlist files in [src/playlist_file.rs](../src/playlist_file.rs) and
[src/pls.rs](../src/pls.rs), and the Jump to file dialog in
[src/jump.rs](../src/jump.rs).

- [x] Make the ADD, REM, SEL, MISC and LIST OPTS menus act: letting go over
      an item chooses it, and a click chooses the bottom item, as in Winamp.
  - [x] ADD: file, folder. URL waits for Phase 6.
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

Done: see "Windowshade" in the README. The bars are drawn by `draw_shade` in
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

Done except Ctrl+L, which waits for Phase 6, and Ctrl+P, which waits for a
Preferences dialog (see Optional). See "File info", "Skin fonts and
pointers" and "Stop with fadeout" in the README.

- [x] More formats: symphonia's `aac`, `alac`, `isomp4` (M4A), `aiff` and
      `mpa` (MP1, MP2, MP3) features, listed in `FORMATS` in
      [src/audio/mod.rs](../src/audio/mod.rs).
- [x] Show the current song in the OS window title, for the taskbar and
      Alt+Tab: "3. Artist - Title - NeoAmp [Paused]", as Winamp wrote it.
- [x] Honour `Font=` in `pledit.txt`, looking the family up among the
      system's fonts with fontdb ([src/font.rs](../src/font.rs)).
- [x] Use the skin's cursors (`.cur` files, [src/cursor.rs](../src/cursor.rs)),
      borrowing the base skin's where a skin has none.
- [x] File info dialog (the **I** button, Alt+3, MISC > FILE INFO),
      [src/info.rs](../src/info.rs). It shows the tags but cannot edit them
      (the user chose read-only over writing tags with `lofty`).
- [ ] Shortcuts:
  - [ ] Ctrl+L (open URL) needs Phase 6.
  - [x] Alt+3 (file info).
  - [ ] Ctrl+P (preferences) needs a Preferences dialog (see Optional).
  - [x] Shift+V (fade out and stop), and Shift-click on Stop.

## Phase 6: Internet radio

Winamp's signature feature: SHOUTcast and Icecast streams.

- [ ] An HTTP streaming source for symphonia.
- [ ] Open URL (Ctrl+L, ADD > URL), and `.pls` / `.m3u` files that point at
      streams.
- [ ] Show the station's song title (ICY metadata) in the scrolling title.
- [ ] Draw the buffering indicator while a stream buffers. Its sprite,
      `WORKING`, is already in [src/sprites.rs](../src/sprites.rs) but never
      drawn.

## Phase 7: Playback and platform integration

- [ ] Gapless playback (the README lists the gap as a limitation).
- [ ] Choose the output device, and reopen it when it disappears. On Windows
      and macOS, playback goes silent until a restart today.
- [ ] Media keys and the OS "now playing" display: MPRIS on Linux, SMTC on
      Windows, Now Playing on macOS. The `souvlaki` crate covers all three.
- [ ] Single instance: a file opened from the file manager while NeoAmp runs
      goes to the running copy instead of starting a second one.

## Optional

- A Preferences dialog. Most settings can live in menus instead.
- Opus and tracker modules (MOD, XM, S3M, IT). symphonia has neither, so both
  need other crates.

## Out of scope

Media Library, the minibrowser, the video window, AVS and MilkDrop
visualizers, a plugin system, modern `.wal` skins and CD ripping.
