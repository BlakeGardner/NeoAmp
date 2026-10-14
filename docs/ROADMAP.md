# Roadmap: what's missing from a mostly complete Winamp 2.x

This list came from reading the code on 2026-10-13, not from running the app.
The phases are in the suggested order of work. Tick items off as they land.

## Phase 1: Saved settings

Nothing survives a restart today.

- [ ] Save and restore volume, balance, EQ (on, preamp, bands), shuffle,
      repeat, time mode, visualizer mode, which windows are open, window
      positions, playlist size, skin, zoom and double size.
- [ ] Save and restore the playlist and its current entry.

## Phase 2: Popup menus

These controls show a pressed state and nothing else: the title-bar **O**
button, right-click on any window, **O**, **A**, **I** and **V** in the strip
left of the time display, and the EQ's **PRESETS** button. See the no-op arm
at the end of `Ui::mouse_up` in [src/ui.rs](../src/ui.rs).

- [ ] Decide: native menus or menus drawn in the skin canvas.
      Recommendation: draw our own, since they fit the borderless windows.
- [ ] Build the popup-menu component.
- [ ] Main menu, from the title-bar **O** button and right-click: Play file,
      folder or URL; Skins; Options; Playback; Visualization; Exit.
- [ ] **O** opens the options menu.
- [ ] **A** toggles always on top.
- [ ] **I** opens file info (see Phase 5).
- [ ] **V** opens the visualizer menu.
- [ ] EQ **PRESETS**: built-in presets (Rock, Classical, …), load and save,
      `.eqf` import, Reset / Flat.
- [ ] EQ **AUTO**: load a preset per file. It toggles today but does nothing.
- [ ] Visualizer options. Analyzer: fire or line style, thick or thin bars,
      peaks on/off, falloff speeds. Oscilloscope: dots, lines or solid. The
      style is fixed today ([src/vis.rs](../src/vis.rs)).
- [ ] Skins menu and skin browser (Alt+S). Dropping a `.wsz` is the only way
      to change skins today.

## Phase 3: Playlist

- [ ] Make the ADD, REM, SEL, MISC and LIST OPTS menus act. They pop up, but
      releasing on an item does nothing.
  - [ ] ADD: file, folder, URL.
  - [ ] REM: remove selected, crop, clear.
  - [ ] SEL: select all, none, invert.
  - [ ] MISC: sort (title, file name, path), reverse, randomize.
  - [ ] LIST OPTS: new, save and load list.
- [ ] Shift- and Ctrl-click multi-select. A click always selects just one row
      today (the `PlTrack` arm of `Ui::mouse_down`).
- [ ] Drag rows to reorder.
- [ ] Keys: Delete, Ctrl+A, arrows, Enter, Home/End, PgUp/PgDn.
- [ ] Mouse wheel: scroll the playlist, and set the volume over the main
      window. [src/app.rs](../src/app.rs) handles no wheel events yet.
- [ ] Make shuffle shuffle. The button toggles, but `Ui::next` and
      `Ui::advance` always move to the following entry.
- [ ] Load and save M3U, M3U8 and PLS, from LIST OPTS, the command line and
      drag-and-drop. Resolve relative paths against the playlist's folder.
- [ ] J: jump to file (a search box over the playlist).
- [ ] Shift+L: open a folder.

## Phase 4: Windowshade mode

- [ ] Collapsed main window, EQ and playlist, from each window's shade button
      and by double-clicking a title bar.
- [ ] Load `eq_ex.bmp` (needed by the collapsed EQ). `SHEET_FILES` in
      [src/skin.rs](../src/skin.rs) doesn't list it.
- [ ] Read the `[WindowShade]` and `[EqualizerWS]` sections of `region.txt`.
      `parse_region` only keeps `[Normal]` and `[Equalizer]`.

## Phase 5: Smaller items

- [ ] More formats. symphonia 0.6 has `aac`, `alac`, `isomp4` (M4A), `aiff`
      and `mp1`/`mp2` as cargo features: enable them in `Cargo.toml` and add
      them to `FORMATS` in [src/audio/mod.rs](../src/audio/mod.rs), which also
      feeds the open dialog's filters.
- [ ] Show the current song in the OS window title, for the taskbar and
      Alt+Tab. `App::open_pane` always uses "Winamp".
- [ ] Honour `Font=` in `pledit.txt`. It is parsed but ignored, and the
      playlist always uses the first font in `CANDIDATES` it finds
      ([src/font.rs](../src/font.rs)).
- [ ] Use the skin's cursors (`.cur` files).
- [ ] File info dialog (the **I** button, Alt+3).
- [ ] Shortcuts: Ctrl+L (open URL, needs Phase 6), Alt+3 (file info), Ctrl+P
      (preferences, if there is a Preferences dialog), Shift+V (fade out and
      stop).

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
- [ ] Single instance: a file opened from the file manager while Winamp runs
      goes to the running copy instead of starting a second one.

## Optional

- A Preferences dialog. Most settings can live in menus instead.
- Opus and tracker modules (MOD, XM, S3M, IT). symphonia has neither, so both
  need other crates.

## Out of scope

Media Library, the minibrowser, the video window, AVS and MilkDrop
visualizers, a plugin system, modern `.wal` skins and CD ripping.
