# NeoAmp

NeoAmp is a music player in Rust with a pixel-exact reproduction of the
classic Winamp 2.x interface (main window, equalizer and playlist editor),
drawn with classic Winamp skins, that plays MP3, MP2, AAC, M4A (AAC and
Apple Lossless), WAV, AIFF, FLAC and Ogg Vorbis files, and SHOUTcast and
Icecast internet radio. The transport buttons, position bar, volume, balance and ten-band
equalizer all act on the audio, and the visualizer shows what is actually
playing.

NeoAmp is an independent project, not affiliated with or endorsed by the
makers of Winamp. Winamp is a trademark of its owner.

![NeoAmp's main window, equalizer and playlist](docs/screenshot@2x.png)

## How it works

Classic Winamp skins (`.wsz` files) are zip archives of fixed-size sprite
sheets (`main.bmp`, `cbuttons.bmp`, `titlebar.bmp`, ...) plus a few text
files. Winamp itself just blitted rectangles out of those sheets, so this
program does the same:

- `src/canvas.rs` is a small RGBA software framebuffer with clipped blits.
- `src/sprites.rs` holds the sprite coordinates and window layout, which are
  identical for every classic skin.
- `src/skin.rs` reads a `.wsz` (zip + BMP/PNG), `viscolor.txt`,
  `pledit.txt`, `region.txt` and the mouse pointers (`src/cursor.rs`), and
  falls back per file to the embedded base skin for anything a skin leaves
  out (see [Skin fonts and pointers](#skin-fonts-and-pointers)).
- `src/main_window.rs`, `src/eq_window.rs`, `src/playlist_window.rs` draw the
  three windows and their windowshade bars; `src/ui.rs` holds state, hit
  testing and input.
- `src/app.rs` runs one borderless `winit` window per NeoAmp window, presents
  each framebuffer through `pixels` (wgpu) and drives the title-bar drags and
  docking. `src/upscale.rs` draws the framebuffer over the whole window (see
  [Scaling](#scaling)), and `src/zoom.rs` picks the zoom.
- `src/menu.rs` draws popup menus and handles the pointer and keyboard in
  them; `src/menus.rs` holds NeoAmp's own menus, and `src/popup.rs` gives each
  open menu its window (see [Menus](#menus)). `src/eqf.rs` reads and writes
  equalizer presets, and `src/library.rs` lists the skins and presets folders.
- `src/file_dialog.rs` shows the platform's file dialogs (see
  [Opening files](#opening-files)).
- `src/playlist.rs` sorts, moves and shuffles playlist entries;
  `src/playlist_file.rs` loads and saves playlist files through `src/m3u.rs`
  and `src/pls.rs` (see [Playlist](#playlist)).
- `src/jump.rs` is the Jump to file dialog, `src/info.rs` the File info
  dialog and `src/url_dialog.rs` the Open URL dialog; `src/jump_window.rs`,
  `src/info_window.rs` and `src/url_window.rs` give them windows through
  `src/dialog_window.rs`. `src/text_box.rs` is their edit box,
  `src/clipboard.rs` reads the clipboard for it, and `src/classic.rs` holds
  the Windows 2000 colours, bevels and buttons they share with the menus.
- `src/audio/` plays the music (see [Audio](#audio)), internet radio included
  (`src/audio/stream.rs`, fetched by `src/http.rs`; see
  [Internet radio](#internet-radio)), and `src/vis.rs` turns the samples being
  played into the spectrum analyzer and oscilloscope.
- `src/settings.rs` remembers the settings and the playlist between runs (see
  [Saved settings](#saved-settings)); `src/m3u.rs` reads and writes the
  playlist file.
- `src/lib.rs` exposes all of the above as a library; `src/main.rs` only
  parses the command line. `cargo test` covers layout, hit testing, input,
  playlist editing and playlist files, the skin parsers, the blitter, the
  dialogs and menus, the saved settings and playlist, the audio pipeline
  (decoding, resampling, the equalizer, and the decoder and mixer threads
  working together), and the HTTP client and internet radio, against small
  servers the tests run on the computer itself.

Each NeoAmp window is its own borderless OS window. Grab a title bar to
drag one away; the main window carries whatever is docked to it, while the
equalizer and playlist leave on their own. Windows snap to each other's
edges and to the screen edges when they come within 10 pixels, like the
original. Resizing the playlist from its grip leaves the other windows where
they are, even ones docked to it, as Webamp does (how Winamp itself behaved
here is undocumented). Shading a window (see [Windowshade](#windowshade)) is
different: the windows docked below it move up or down with its bottom edge.
`src/dock.rs` holds that geometry.

Docking needs to read and set window positions, which Wayland forbids, so
the app prefers the X11 backend (XWayland on a Wayland desktop) whenever
`DISPLAY` is set. Some compositors, COSMIC among them, also ignore moves of
managed X11 windows; the app notices this at startup and recreates its
windows as override-redirect windows, which are always placed exactly where
asked. Such windows have no taskbar entry, stay above other windows (like
Winamp's "always on top") and take keyboard focus when clicked.

- `NEOAMP_BACKEND=x11|wayland` forces a backend. On pure Wayland, title-bar
  drags are handed to the compositor and nothing snaps.
- `NEOAMP_UNMANAGED=1|0` forces or forbids override-redirect windows instead
  of probing.

## Audio

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

**Gapless playback.** Tracks follow one another without a gap, as with
Winamp's gapless output plugins. While a file plays, the interface tells the
decoder which file comes next (the next entry, the next one shuffle has
already picked, or the first again with repeat on). Once the current file is
decoded to its end, the decoder opens the next one and queues its audio
straight after, with a mark that tells the callback where it begins; as its
first sample plays, the time display starts again from zero and the playlist
moves on. MP3s with an encoder's gapless header (LAME's) and Ogg Vorbis files
lose their encoder padding, so an album that was cut into tracks plays
seamlessly. A seek, or a change of device, just before the end goes back to
the file still playing. Internet streams, and a file that follows one, start
the usual way, as does the next track during a fade-out (which stops at the
end of the track). A file shorter than half a second ends the usual way too,
with a short gap.

**Output device.** Options > Output Device lists the system's default and
every output device by name; the choice is saved. Where the platforms
differ:

- **Windows and macOS:** the list is the system's own (WASAPI, CoreAudio).
  On the system's default, NeoAmp follows the system when it switches outputs
  (Windows 10's automatic stream routing, macOS's default output unit), say
  when headphones are plugged in.
- **Linux:** the list is the sound server's outputs, Bluetooth ones included,
  reached through cpal's PulseAudio host: a pure-Rust client of the
  PulseAudio protocol, which PipeWire speaks too, so building needs nothing
  more. Where no sound server runs, it lists ALSA's hardware devices
  (`plughw:`, one per device). The system's default stays ALSA's `default`
  device, which the sound server routes to the desktop's chosen output.

When a chosen device goes away (headphones switched off, a USB DAC
unplugged), playback moves to the system's default, and back to the chosen
device when it returns; NeoAmp looks for it every two seconds. A device that
fails, or none at all, is opened again as soon as one can be. Either way the
track goes on from where it was: the decoder starts again at that point, at
the new device's rate.

A background thread reads tags ("Artist - Title") and lengths for the
playlist as files are added.

**Stop with fadeout.** Shift+V, Shift-click on Stop, or Playback > Stop w/
fadeout lowers the volume evenly to nothing over 3 seconds, then stops, as
Winamp's Shift+V did (how long Winamp's own fade took is not documented; 3
seconds is NeoAmp's choice). The volume slider stays where it is. Playing,
pausing, stopping or changing track cancels the fade, and a track that ends
while fading stops there rather than starting the next (if the next was
already queued to follow without a gap, the fade carries on over it and stops
when it is done). The output glides
from one volume to the next over each block of audio it plays, so the fade,
like dragging the volume slider, never clicks.

## Media keys and "now playing"

NeoAmp joins the system's media controls through
[souvlaki](https://crates.io/crates/souvlaki): the keyboard's media keys
(play/pause, next, previous, stop) control it whichever window has the focus,
and the system's "now playing" display shows the track's title, artist,
album and length, from its tags, and offers the same buttons and seeking. An
internet stream shows the song its station names, with the station as the
artist. Play/pause never starts a playing track over, as Winamp's Play button
(X) does.

- **Linux:** MPRIS over D-Bus, which GNOME, KDE, COSMIC and most desktops use
  for their media keys and media widgets. souvlaki talks to D-Bus through the
  pure-Rust zbus, so building needs no `libdbus`. The volume can be set from
  there too. Without a D-Bus session (a bare console, say), NeoAmp runs
  without media controls. Each NeoAmp has its own MPRIS name,
  `org.mpris.MediaPlayer2.neoamp.instance<process id>`.
- **Windows:** the System Media Transport Controls, the media overlay that
  the volume keys bring up, tied to the main window.
- **macOS:** Now Playing in Control Center, and the media keys (F7 to F9).

## Scaling

Classic skins were drawn for one screen pixel per skin pixel on 800x600 and
1024x768 screens, which looks tiny on a modern monitor. The windows are zoomed
in half steps from 1x to 3x:

- **Default:** chosen from the height of the screen (in the desktop's logical
  pixels). 1080 lines stay at 1x, 1440 get 1.5x, 2160 get 2x. On a 27"
  1440p monitor, 1.5x is about the physical size Winamp had on a 15" CRT.
- **Ctrl+=** / **Ctrl+-** step the zoom, **Ctrl+0** goes back to the default,
  and `--scale 1|1.5|2|2.5|3` overrides it at startup. A zoom picked either
  way is remembered. At the default none is, so the next start picks one for
  whatever screen it finds.
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

## Windowshade

Each window can collapse to a bar 14 pixels high, as in Winamp: click the
shade button beside its close button, or double-click its title bar. In the
bar, the same button or a double-click anywhere that is not a control opens
it out again. **Ctrl+W** shades the main window, or the playlist while it has
the focus. The equalizer has no key, as in Winamp.

- **Main window:** the options, minimize and close buttons, a small
  visualizer (click it to change the mode, right-click for its menu), the
  time (click it for elapsed or remaining), previous, play, pause, stop, next
  and eject, and a position bar that seeks when let go. The time and the
  position bar are blank while stopped, and the time blinks while paused.
- **Equalizer:** the volume and balance sliders.
- **Playlist:** the current entry's number, title and length; a title too
  long for the bar ends in "…". The grip at its right changes only the width,
  in the same 25-pixel steps, and the playlist opens out to the height it had.

Windows docked below a window follow its bottom edge as it is shaded or
opened out, so a stack of windows stays a stack, as in Webamp. A closed
window that was docked below reopens docked to the bar. Which windows are
shaded is saved with the other settings.

The bars come from the skin's `titlebar.bmp`, `eq_ex.bmp` and `pledit.bmp`.
A skin's `region.txt` can shape the main window's bar (`[WindowShade]`) and
the equalizer's (`[EqualizerWS]`); `[Normal]` and `[Equalizer]` apply only to
the full windows. A skin without `eq_ex.bmp` gets the built-in skin's
equalizer bar, but keeps its own shade button when pressed in the full window,
as Webamp does, since the built-in one would not match.

Where the platforms differ: on pure Wayland, which lets no application move
its windows, the windows docked below stay where they are and nothing
follows. On macOS, ⌘W shades a window rather than closing it.

## Menus

Winamp 2.x used the system's own menus, so NeoAmp draws its menus the way
Windows 2000 did, grey and bevelled, whatever skin is loaded:

- **Main menu**: the options button at the left of the main title bar,
  **Alt+F**, or a right-click on any window. It plays a file, a folder or a
  URL (see [Internet radio](#internet-radio)),
  shows or hides the equalizer and playlist, and holds the Skins, Options,
  Playback and Visualization submenus, and Exit.
- **Options**: the `O` in the clutter bar (also in the main menu). Time
  elapsed or remaining, always on top, double size, the zoom, repeat and
  shuffle, and the output device (see [Audio](#audio)).
- **Visualization**: the `V` in the clutter bar, or a right-click on the
  visualizer. The mode, and Winamp's analyzer and oscilloscope styles and
  falloff speeds.
- **EQ presets**: the equalizer's PRESETS button. Winamp's built-in presets,
  your own, `.eqf` files, auto-load presets and Reset (see
  [Saved settings](#saved-settings)).
- **Skins**: **Alt+S** (also in the main menu). The skins in NeoAmp's skins
  folder, the built-in base skin, and Open Skin... for any other.
- **Sort** and **Remove misc**: the playlist's MISC > SORT LIST and
  REM > MISC REM (see [Playlist](#playlist)).

The Playback submenu also holds Stop w/ fadeout (**Shift+V**) and Jump to
file... (**J**).

The `A` in the clutter bar toggles always on top, and stays down while it is
on, as `D` does for double size. The `I` opens the File info dialog (see
[File info](#file-info)).

Menus open where Windows opened them: dropping down from the button that opens
them, or at the pointer for a right-click, turned the other way at the screen's
edge. Submenus open when the pointer rests on their item for a moment, or at
once on a click. The keyboard works as in Windows: the arrow keys move and open
and close submenus, Enter chooses, Escape closes, and a letter picks the item
it is underlined in (underlines appear once a key has been used). A click
anywhere else, or switching to another application, closes the menu. Menus grow
with the zoom, as Windows' grow with the desktop's scale, but not with double
size, which Winamp's never did; their text is drawn at that size, so it stays
sharp.

The text is Liberation Sans, built into NeoAmp (see
[Skin fonts and pointers](#skin-fonts-and-pointers)), on every system, in place
of Windows 2000's menu font, Tahoma, which most Linux systems lack.

The menus list each item's keyboard shortcut the way the platform writes them:
"Ctrl+Shift+1" on Windows and Linux, "⇧⌘1" on macOS. Liberation Sans has no ⌘,
⌥ or ⇧, so those come from DejaVu Sans, also built in.

Each open menu is a small window of its own, since a menu reaches past the
NeoAmp window it opens from. These windows are drawn in software with
[softbuffer](https://crates.io/crates/softbuffer), from the people behind
winit, because they must appear at once: setting up a GPU surface, as the main
windows use, can take a tenth of a second. softbuffer needs no system
libraries to build. Where the platforms differ:

- **X11** (including XWayland): menus are override-redirect windows, as other
  toolkits' menus are, so they land exactly where they are put. COSMIC's
  compositor redraws override-redirect XWayland windows only about once a
  second, so there the highlight may lag behind the pointer; this has not
  been tried on COSMIC yet.
- **Windows and macOS:** menus are borderless windows kept above the others;
  on Windows they stay off the taskbar. Neither has been tried yet.
- **Pure Wayland** lets no window choose its position, so menus open wherever
  the compositor puts them.

## Opening files

Eject (in the main window, or the small one in the playlist's bottom bar),
**L** or Play File... in the main menu opens the platform's own file dialog,
through [rfd](https://crates.io/crates/rfd). Pick one file or several (or a
playlist file): as with Winamp's "Open file(s)", they replace the playlist and
the first one plays. **Shift+L** or Play Folder... picks a folder instead,
whose playable files (sorted by name, subfolders included) replace the
playlist in the same way. The playlist's ADD menu adds files or a folder to
the end of the playlist instead, without interrupting what is playing, and
its LIST OPTS menu loads and saves playlist files (see [Playlist](#playlist)).
Cancelling changes nothing. The windows keep drawing while the dialog is open.
The same dialogs open skins and equalizer presets from the menus, starting in
NeoAmp's skins and presets folders.

The dialog starts on "All supported types" and also offers each format (MP3,
MP1 and MP2, AAC, MPEG-4 audio, WAV, AIFF, FLAC, Ogg Vorbis) and playlists
one at a time. MPEG-4 audio covers `.m4a`, `.m4b` and `.mp4`, so a folder
added to the playlist brings its `.mp4` videos too, and plays their sound,
as Winamp did. Saving a playlist offers M3U8, M3U and
PLS, and starts on `Playlist.m3u8`. The platforms differ a little:

- **Windows:** the standard Open dialog, with the file types in its drop-down.
- **macOS:** an open panel that slides down from the main window as a sheet.
  macOS panels have no file type drop-down, so it always shows every
  supported type.
- **Linux:** the desktop's own file chooser, reached through the XDG desktop
  portal (GNOME, KDE, COSMIC and most other desktops run one). The portal is
  reached over D-Bus with the system's `libdbus`, which is loaded at runtime,
  so building needs no extra packages. Some choosers match file types
  case-sensitively, so both `*.mp3` and `*.MP3` are listed; a mixed-case name
  such as `song.Mp3` may be hidden there. Without a portal, rfd runs `zenity`
  instead if it is installed. With neither, eject does nothing and rfd logs
  why.

Saving goes by the extension of the name given: `.pls` writes PLS, anything
else extended M3U. A name without an extension gets `.m3u8`, since not every
platform's save dialog adds the chosen type's extension.

**NeoAmp already running.** Only one NeoAmp runs at a time, as with Winamp:
a file opened from the file manager, or `neoamp song.mp3` in a terminal,
while NeoAmp runs goes to the running copy, where it replaces the playlist
and plays, and the new start ends. With `--enqueue` (Winamp's `/ADD`) the
files join the end of the playlist instead, without interrupting what plays,
for an "Add to NeoAmp" action in a file manager. A skin is shown, and
starting NeoAmp again with nothing to open brings the running one to the
front. The running copy listens on a local socket named after its config
folder, through [interprocess](https://crates.io/crates/interprocess): a
Unix domain socket in `$XDG_RUNTIME_DIR` on Linux (the config folder without
one), in the user's own temporary folder on macOS, and a named pipe on
Windows. NeoAmps with different config folders (`NEOAMP_CONFIG_DIR`) run side
by side. If the running copy does not answer within five seconds, the new one
starts anyway. File names travel byte for byte, so names that are not valid
Unicode arrive intact.

## Internet radio

NeoAmp plays SHOUTcast and Icecast stations, as Winamp did, and files on the
web.

**Opening a stream.** **Ctrl+L** (Cmd+L on macOS) or Play URL... in the main
menu opens a dialog drawn like Windows 2000's: paste or type the address and
press Enter or Open. The stream replaces the playlist and plays, as with
Winamp's Open URL. The playlist's ADD > URL opens the same dialog as Add URL,
which adds the address to the end of the playlist instead. When the clipboard
holds a web address, the dialog starts with it, selected, so that Enter opens
it and typing replaces it; Ctrl+V pastes and Ctrl+A selects the whole
address. An address without `http://` gets it (`radio.example:8000`), and a
file path or `file://` URL opens that file. Stream URLs also work on the
command line and in `.m3u`, `.m3u8` and `.pls` playlist files, and are saved
as they were written.

**Station playlists.** Stations usually link a playlist (`listen.pls`,
`stream.m3u`) rather than the stream itself. NeoAmp reads it when the entry
plays, and plays the first stream in it that answers; SHOUTcast's playlists
list several servers for one station, so the others are fallbacks. The entry
stays one entry, the playlist's address, so the station's servers are looked
up afresh each time it plays. A playlist is recognised by its media type, or
else by `.pls`, `.m3u` or `.m3u8` at the end of its address.

**What it shows.** While a stream connects the scrolling title says
`[Connecting]`, then `[Buffering: 45%]` until a couple of seconds of audio
have arrived, and the small light left of the play indicator is lit (the
skin's "working" light, which Winamp lit while buffering). Then the entry
takes the name the station gives itself, unless a playlist file named it
already, and the scrolling title and the window title show the song the
station says is playing (its SHOUTcast/Icecast metadata), or the station's
name when it names none. The song changes as its audio reaches the speakers.
Whenever the scrolling title changes (a new song or a new status, or for a
file a new track) it starts again from its first character, as Winamp's did.
The time counts up from the start; the position bar has no thumb and the
arrow keys do not seek, since a stream has no length. The bitrate and sample
rate show as for files. Equalizer auto-load presets go by file name, so a
stream has none.

**Dropouts.** If the connection drops, or nothing arrives for 15 seconds,
NeoAmp connects again, showing `[Reconnecting]`. It tries three more times,
after 1, 2 and 4 seconds; if those fail too, it moves on to the next entry,
as for a file that cannot be played. A connection that lasted half a minute
counts as good, so a station that drops now and then is followed for as long
as it plays. If the audio runs out while the connection is slow, the title
shows `[Buffering: ...]` again until a couple of seconds are back. A paused
stream keeps what arrives, up to about half a minute at 128 kbit/s; if the
station drops the connection meanwhile, NeoAmp reconnects when play resumes,
skipping ahead to what the station plays by then.

**Formats.** Streams are decoded like files, so MP3, AAC, Ogg Vorbis (whose
song titles come in each song's own tags, as Icecast sends them), FLAC and WAV
play. HE-AAC ("AAC+"), common at low bitrates, plays only as its AAC core: at
half the sample rate and without its high frequencies, since symphonia has no
decoder for the rest. Opus streams and HLS (`.m3u8` playlists of short
segments, which many broadcasters' own web players use) do not play; NeoAmp
logs why and moves on. A file on the web plays to its end, without seeking.

**How it works.** `src/http.rs` is a small HTTP/1.1 client written for this,
because SHOUTcast 1.x servers answer `ICY 200 OK` instead of
`HTTP/1.0 200 OK`, which general-purpose clients reject. HTTPS uses
[rustls](https://crates.io/crates/rustls) with Mozilla's root certificates
built in ([webpki-roots](https://crates.io/crates/webpki-roots)), so every
platform checks certificates the same way. A certificate that only the
system trusts, such as one from a company's own authority, is refused.
`src/audio/stream.rs` connects on a thread of its own, follows playlists,
takes the song titles out of the audio and passes the audio to the decoder
thread through a buffer, which symphonia reads like a file that cannot seek.

Everything here works the same on Linux, Windows and macOS. Only the
clipboard differs, through [arboard](https://crates.io/crates/arboard): on
Linux it is X11's, which XWayland shares with Wayland applications, so under
pure Wayland with no XWayland the dialog cannot paste. Internet radio has
been tried on Linux only.

## Playlist

The playlist editor works as Winamp 2.x's did.

**Selecting.** A click selects one entry, Shift-click selects everything from
the last entry clicked, and Ctrl-click (Cmd-click on macOS, where Ctrl-click
means right-click) adds or removes one entry. A double-click or Enter plays
an entry. The keyboard moves a cursor through the list: Up, Down, Page Up,
Page Down, Home and End, with Shift to select as it goes. These keys work
while the playlist window has the focus; in the other windows Up and Down
still set the volume.

**Moving.** Drag selected entries to move them together; dragging past the
top or bottom of the list scrolls it. Alt+Up and Alt+Down move the selection
one place. The mouse wheel scrolls the list (over the main window it sets the
volume).

**The bottom buttons.** ADD, REM, SEL, MISC and LIST OPTS each show a menu
drawn from the skin while held; let go over an item to choose it. Each menu's
bottom item sits over its button, so a plain click chooses that one, as in
Winamp: ADD adds files, REM removes the selection, SEL selects everything,
MISC does nothing (it is Winamp's misc options) and LIST OPTS loads a list.

| Menu | Items |
| --- | --- |
| ADD | URL (see [Internet radio](#internet-radio)), folder, files |
| REM | misc (opens a menu with Remove all dead files), all, crop, selected |
| SEL | invert, none, all |
| MISC | sort (opens a menu: by title, file name, or path; reverse; randomize), file info (for the first selected entry, see [File info](#file-info)) and misc options (does nothing yet) |
| LIST OPTS | new (empties the list), save, load |

**Removing the track that plays.** As in Winamp, it plays on; when it ends,
the entry that followed it plays next (or playback stops if it was the last
and repeat is off).

**Shuffle.** With shuffle on, every entry plays once, in random order, before
any plays again. At the end of that round playback stops unless repeat is on,
in which case a new round starts. Next always moves on; Previous goes back
through the entries in the order they played.

**Playlist files.** NeoAmp reads and writes extended M3U (`.m3u`, `.m3u8`) and
Winamp's PLS (`.pls`), from LIST OPTS, the file dialogs, the command line and
drag and drop. Loading one from LIST OPTS or the eject dialog replaces the
playlist and plays it; dropping one adds its entries. Entries whose length the
file gives keep its title and length; the others have their tags read. Since
playlists often come from Windows:

- Lines may end in `\r\n` or `\n`.
- Text that is not UTF-8 is read as Windows-1252, the codepage Winamp wrote
  on Western systems. On Linux, a non-UTF-8 name is first tried as the file
  system's own bytes, then as Windows-1252.
- Relative paths are taken from the playlist's folder. Off Windows, a
  relative path with backslashes and no forward slashes (`Rock\01.mp3`) has
  them turned into separators. `.` and `..` are resolved by name, as Winamp
  did, without following symbolic links. Absolute Windows paths (`C:\...`)
  only work on Windows.
- `file://` URLs are read. A `file://server/share/...` URL names a network
  share, which only Windows can open; elsewhere it is skipped. `http://` and
  `https://` URLs are internet radio streams (see
  [Internet radio](#internet-radio)), and are saved as they were written.
  Other kinds of URL (`rtsp://`, `mms://`) are skipped and logged.

Saved lists are UTF-8 whatever their extension, with `\r\n` line ends for PLS
and `\n` for M3U. Files inside the playlist's folder (or below it) are written
relative to it, so the folder can move as a whole; other files are written
with their full paths.

**Jump to file.** **J** (in any window, or Playback > Jump to file...) opens a
dialog drawn like Windows 2000's, as Winamp's was: type, and the list narrows
to the entries whose title or file name (for a stream, its URL) holds every
word typed, ignoring case.
Up, Down, Page Up and Page Down move the highlight, Enter, the Jump button or a
double-click plays it, and Escape or Close closes the dialog. Left, Right,
Home and End move the caret in the search box; Ctrl+V (Cmd+V on macOS) pastes,
and Ctrl+A selects all of it, so that typing replaces it. The dialog is an
ordinary window with the system's own title bar,
so the window manager places and moves it; it is drawn with softbuffer like
the menus, at the same size, in the same font.

## File info

The `I` in the clutter bar, **Alt+3** or the playlist's MISC > FILE INFO
opens a dialog drawn like Windows 2000's, as Winamp's was, about one file:
the current track, or from the playlist (MISC > FILE INFO, or Alt+3 while the
playlist has the focus) the first selected entry. It shows the file's path,
its tags (title, artist, album, year, track, genre and comment) and what it
holds (format, codec, length, average bitrate, sample rate, channels, bits
per sample for PCM and lossless audio, and its size). Winamp's dialog could
also edit the tags; NeoAmp's only shows them, in read-only boxes, and never
writes to music files. Enter, Escape, Space or Close closes it; asking for
another file while it is open shows that one instead.

The file is read on a thread of its own, so the dialog opens at once and
fills in a moment later; a file that cannot be read says why. For an internet
stream it shows the address, and while the stream plays, what its station
says: the song (as the title), the genre, its description (as the comment),
its name and web site, and the stream's format, bitrate, sample rate and
channels. It follows the stream as the songs change. Tags come from
whatever the file holds (ID3v1 and ID3v2, APE, Vorbis comments, FLAC, MP4 and
RIFF INFO tags); a Vorbis DESCRIPTION counts as the comment. Like the Jump
to file dialog, it is an ordinary window with the system's title bar, drawn
with softbuffer in the menus' font, and it has no clipboard:
its text cannot be selected or copied.

## Skin fonts and pointers

**The playlist's font** is the one the skin's `pledit.txt` names (`Font=`),
as in Winamp, at the size Winamp used, or rather a free font that stands in
for it. NeoAmp has its fonts built in and reads none from the system, so the
playlist, the menus and the dialogs look the same on Linux, Windows and macOS
whatever fonts each has installed. It draws all its text itself, with
[fontdue](https://crates.io/crates/fontdue), not through the system, so the
same font gives the same pixels everywhere. Microsoft's own fonts cannot be
built in: their licence does not allow it.

| The skin names | NeoAmp uses |
| --- | --- |
| Arial (nearly every skin), Helvetica | Liberation Sans, made to their measurements |
| Courier New, Courier, Andale Mono | Liberation Mono, made to their measurements |
| Lucida Console, Lucida Sans Typewriter, Fixed | Liberation Mono, within a pixel in 50 characters of Lucida Console |
| Times New Roman, Times | Liberation Serif, made to their measurements |
| Tahoma, MS Sans Serif, Microsoft Sans Serif | Wine's Tahoma |
| Verdana, Lucida Sans, Lucida Sans Unicode | DejaVu Sans, close in look and width |
| Any other font | Liberation Sans |

Family names are matched ignoring case, as Windows matched them. A font made
to another's measurements takes the same room, letter for letter, and differs
only in the shapes of a few letters. Windows' bitmap fonts (`.fon`, such as
MS Sans Serif) get Liberation Sans too, as with Webamp's fallback to Arial.

The fonts are in `fonts/`, which lists where each comes from and its licence
(`fonts/README.md`); they add 2 MB to the program.

**Mouse pointers.** Winamp skins can carry their own pointers as `.cur` files,
one for each part of the windows: `TITLEBAR.CUR` over the main title bar,
`VOLBAL.CUR` over the volume and balance sliders, `PSIZE.CUR` over the
playlist's grip, and so on through the 24 Winamp read (`NORMAL.CUR`,
`EQSLID.CUR`, `PTBAR.CUR`, ...). NeoAmp shows them where Winamp did, following
Webamp's map of them and Winamp's Skinner's Atlas. A skin without one of them
borrows the built-in skin's, as with its bitmaps; the built-in Winamp 2.91
skin has eight (`CLOSE`, `EQSLID`, `EQTITLE`, `MAINMENU`, `POSBAR`, `PSIZE`,
`TITLEBAR` and `VOLBAL`). Where neither has one, the system's arrow shows, or
over the playlist's grip a resize arrow. While a slider, a window or the grip
is dragged, the pointer stays as it was when the drag began, as Windows kept
it.

- A `.cur` file holding an animated cursor (`.ani`), which Winamp allowed,
  shows its first frame, still.
- Pixels that invert the screen behind them, which only Windows cursors can
  have, are left transparent.
- On a desktop scaled up (200%, say), Windows, X11 and Wayland would draw a
  skin's pointer at its own pixel size, smaller than the system's; NeoAmp
  enlarges it by the nearest whole number. macOS enlarges it itself.

**The window title.** The main window's title names the current track, as
Winamp's did in the taskbar: "3. Artist - Title - NeoAmp", with "[Paused]"
or "[Stopped]" added unless it plays, and just "NeoAmp" with an empty
playlist. The taskbar, the window switcher and the like show it; on X11 with
override-redirect windows (see above) there is no taskbar entry to show it
in.

## Saved settings

NeoAmp starts the way it was left: volume, balance, the equalizer, shuffle,
repeat, the time display, the visualizer and its options, which windows are
open, where and which are shaded, the playlist's size, the skin, the zoom,
double size and always on top, and the playlist with its current entry. The
playlist comes back stopped, as in Winamp. NeoAmp saves within a couple of
seconds of a change and again on exit, so a crash loses little.

Two files share one folder, with the skins and presets the menus list:

| Platform | Folder |
| --- | --- |
| Linux | `~/.config/neoamp/` (or `$XDG_CONFIG_HOME/neoamp/`) |
| Windows | `%APPDATA%\NeoAmp\` |
| macOS | `~/Library/Application Support/NeoAmp/` |

- `settings.toml` holds the settings. Edit it while NeoAmp is closed, since a
  running NeoAmp rewrites it (without comments). A missing key keeps its
  default, and an unusable value (a volume out of range, a zoom between the
  steps) is brought into range or falls back on its own. A file that cannot
  be read as settings is renamed `settings.toml.bad`, and NeoAmp starts with
  the defaults.
- `playlist.m3u8` is the playlist as an extended M3U file, which other players
  can open too. Entries whose length it records are not scanned again at
  startup. Streams are kept as their URLs, titled with their station's name.
- `skins/` (`Skins\` on Windows and macOS) holds the skins the Skins menu
  lists. A skin opened from anywhere else, dropped onto a window or given on
  the command line is copied in, so it stays listed.
- `presets/` (`Presets\`) holds equalizer presets saved from EQ presets >
  Save > Preset..., as Winamp `.eqf` files, and lists them under Load.
- `autoload.eqf` holds the auto-load presets: equalizer settings kept for one
  file name each (EQ presets > Save > Auto-load preset). With AUTO on, a
  file's preset loads as it starts.

[docs/SETTINGS.md](docs/SETTINGS.md) describes every setting: what it
controls, the values it takes and its default.

`NEOAMP_CONFIG_DIR=<folder>` keeps all of it somewhere else. `--screenshot`
neither reads nor writes any of it. Files, a skin or `--scale` given on the command
line take precedence over what was saved and are remembered in turn, so files
replace the saved playlist (`--enqueue` adds them to it instead). The output
device chosen in the Options menu is remembered too; one that is not connected
when NeoAmp starts stays chosen, and plays once it is.

The equalizer and playlist positions are kept relative to the main window, so
docked windows stay docked, and they are converted when the zoom or the
desktop's scale has changed since. Closing either window also remembers where
it was, and it reopens there, as in Winamp. A window whose title bar would land
off every screen (a monitor unplugged since, say) is placed the default way
instead. Each file is written beside the old one under a temporary name and
then renamed over it, so a crash during a save leaves the old file whole.

Where the platforms differ:

- **Pure Wayland:** window positions can be neither read nor set, so they are
  not saved. Whatever an X11 session saved stays for the next one.
- **Minimized windows** have no position worth keeping (Windows reports one far
  off the screen), so the last one from before is kept.
- **File names that are not valid Unicode**, which Linux allows, are written
  to the playlist as raw bytes and come back exactly. A skin at such a path is
  not remembered, because TOML holds only Unicode text. On Windows, the rare
  path that is not valid Unicode (an unpaired surrogate) is written with
  replacement characters, and that entry will not play after a restart.

## Prebuilt binaries

Each [GitHub release](https://github.com/BlakeGardner/NeoAmp/releases) has
NeoAmp built for Linux (x86-64), Windows (x86-64) and macOS (Apple silicon and
Intel): an archive holding the program and this README. The base skin is built
into the program, so nothing else needs installing alongside it. Linux also
gets a Flatpak bundle (see [Flatpak](#flatpak)).

- **Linux** needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12 and anything
  later) and ALSA's library, `libasound2`, which desktop systems already have.
- **Windows** needs nothing else; the C runtime is built in.
- **macOS** refuses to open the program at first, because it is not signed or
  notarized by Apple. After unpacking it, clear the quarantine flag the browser
  set on it:

```bash
xattr -d com.apple.quarantine neoamp
```

Publishing a release on GitHub builds these and attaches them to it; the steps
are at the top of `.github/workflows/release.yml`.

## Flatpak

Each release also has NeoAmp as a Flatpak bundle for Linux (x86-64),
`neoamp-v<version>-linux-x86_64.flatpak`. Install it with:

```bash
flatpak install --user neoamp-v0.1.0-linux-x86_64.flatpak
```

It runs on the Freedesktop 26.08 runtime, which Flatpak fetches from
Flathub. Add Flathub first if the system does not have it:

```bash
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
```

NeoAmp then shows in the desktop's applications, with its icon, and the file
manager offers it for audio files and playlists. A later release's bundle
installs over it. From a terminal:

```bash
flatpak run io.github.BlakeGardner.NeoAmp
```

Inside Flatpak's sandbox NeoAmp works as it does otherwise (media keys, the
output device list, handing files to the running NeoAmp, the built-in fonts),
with these differences:

- **X11 only.** Docking needs X11, so the Flatpak runs on X11, or XWayland on
  a Wayland desktop. `NEOAMP_BACKEND=wayland` does nothing there, and a
  desktop without XWayland cannot run it.
- **Files.** NeoAmp can read files anywhere on the computer, outside system
  folders, but writes only into its own folder (below). It writes a playlist
  elsewhere through the file chooser, which hands it that one file, so a
  playlist saved from the Flatpak names its entries by their full paths
  rather than relative to the playlist.
- **Settings** are kept in `~/.var/app/io.github.BlakeGardner.NeoAmp/config/neoamp/`
  instead of `~/.config/neoamp/` (see `docs/SETTINGS.md`). A Flatpak NeoAmp and
  one installed otherwise keep separate settings, skins and presets, and each
  runs a copy of its own.

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

## Building and running

The stock `base-2.91.wsz` skin must be present at `skins/base-2.91.wsz`; it is
embedded into the binary at build time.

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
extra either.

```bash
cargo run --release
```

Play files, whole folders or playlist files (folders add their MP3, MP2,
AAC, M4A, WAV, AIFF, FLAC and Ogg Vorbis files, sorted by name; `.m3u`,
`.m3u8` and `.pls` files add their entries); they replace the saved playlist and the first one starts
playing. Without any, the playlist from last time comes back. Dropping files,
folders or playlists onto any window adds them to the playlist too, and starts
them if nothing is playing. Eject picks files in a dialog (see
[Opening files](#opening-files)):

```bash
cargo run --release -- ~/Music/some-album
```

Stream URLs work the same way (see [Internet radio](#internet-radio)):

```bash
cargo run --release -- http://radio.example:8000/listen.pls
```

Load another classic skin from the Skins menu (**Alt+S**), by passing it on
the command line or by dropping a `.wsz` onto the window. It is copied into
the skins folder and stays until another one replaces it; **<Base Skin>** in
the Skins menu goes back to the built-in skin:

```bash
cargo run --release -- ~/Downloads/Classic.wsz
```

With NeoAmp already running, this plays the album in it; `--enqueue` adds it to
the end of its playlist instead:

```bash
cargo run --release -- --enqueue ~/Music/another-album
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
| Play/pause, next, previous, stop from anywhere | the system's "now playing" display | the keyboard's media keys |
| Fade out, then stop | Shift-click Stop; Playback > Stop w/ fadeout | Shift+V |
| Open files (they replace the playlist) | either eject button | L |
| Open a folder (it replaces the playlist) | Play Folder... in the main menu | Shift+L |
| Open a URL: internet radio, or a file on the web (it replaces the playlist) | Play URL... in the main menu | Ctrl+L |
| Play a playlist entry | double-click it | Enter (playlist) |
| Jump to file (search the playlist) | Playback > Jump to file... | J |
| File info (tags and details) | the `I` in the clutter bar; MISC > FILE INFO (selected entry) | Alt+3 (in the playlist: the selected entry) |
| Select playlist entries | click, Shift-click, Ctrl-click (Cmd on macOS); SEL menu | Up / Down / PgUp / PgDn / Home / End, with Shift to extend; Ctrl+A all, Ctrl+I invert (playlist) |
| Move the selected entries | drag them | Alt+Up / Alt+Down (playlist) |
| Remove entries | REM menu | Delete: selected, Ctrl+Delete: crop, Alt+Delete: dead files (playlist) |
| Sort, reverse, randomize | MISC > SORT LIST | Ctrl+Shift+1 / 2 / 3: by title / file name / path, Ctrl+R, Ctrl+Shift+R (playlist) |
| Add files, a folder or a URL to the playlist | ADD menu | |
| New, load and save playlist files | LIST OPTS menu | Ctrl+N / Ctrl+O / Ctrl+S (playlist) |
| Scroll the playlist | the scroll bar, its arrows, or the mouse wheel | |
| Shuffle / Repeat | toggle buttons | S / R |
| Elapsed vs. remaining time | click the time display | Ctrl+T |
| Equalizer / Playlist windows | EQ / PL buttons, window close buttons | Alt+G / Alt+E |
| Windowshade (collapse a window to a bar, or open it out) | the shade button beside a window's close button, or double-click its title bar | Ctrl+W: the main window, or the playlist (playlist) |
| Main menu | the options button at the left of the title bar, or right-click | Alt+F |
| Options menu | the `O` in the clutter bar | |
| Output device | Options > Output Device | |
| Always on top | the `A` in the clutter bar | Ctrl+A (but in the playlist, select all) |
| Double size (doubles the zoom) | the `D` in the clutter bar | Ctrl+D |
| Zoom in / out / back to default | Options > Zoom | Ctrl+= / Ctrl+- / Ctrl+0 |
| Volume, balance, EQ bands and preamp | drag the sliders (the marquee shows the value); the wheel over the main window sets the volume | Up / Down: volume (outside the playlist) |
| Seek | drag the position bar; it seeks when you let go | Left / Right: 5 seconds |
| Equalizer on / off | the ON button | |
| EQ presets, and auto-load presets with AUTO on | the PRESETS button | |
| Visualizer mode (analyzer, scope, off) | click the visualizer | |
| Visualizer styles and falloff | the `V` in the clutter bar, or right-click the visualizer | |
| Skins | Skins in the main menu | Alt+S |
| Move a window (docked ones follow the main window) | drag its title bar | |
| Resize the playlist | drag the grip in its bottom-right corner (in its windowshade bar, the grip beside the shade button changes the width) | |
| Playlist menus | hold ADD / REM / SEL / MISC / LIST OPTS, let go over an item | |

Keys marked "(playlist)" work while the playlist window has the focus.
[docs/SHORTCUTS.md](docs/SHORTCUTS.md) lists every key, including those of the
menus and the dialogs.

On macOS, Cmd takes Ctrl's place in every shortcut above (Cmd+A, Cmd+Shift+1,
...) and Option is Alt; the menus show them as ⌘A, ⇧⌘1, ⌥E. There are two
differences on macOS:

- macOS keeps Cmd+Shift+3 for screenshots, so sorting the playlist by path is
  only in the MISC > SORT LIST menu there.
- Delete is the forward-delete key (fn+Delete on a laptop keyboard). The key
  labelled "delete" on a Mac is Backspace, which removes nothing.

Cmd+W shades a window, as Ctrl+W does in Winamp, rather than closing it as in
most Mac apps. The media keys are F7 to F9 on a Mac keyboard, which has no
Stop key.

## Scope and limitations

- Classic 2.x skins (`.wsz`) only. Modern `.wal` skins are a different format.
- The minibrowser window and a Preferences dialog (Winamp's Ctrl+P) are not
  implemented. The menus leave out what NeoAmp cannot do yet
  (Preferences and the like) rather than grey it.
- The File info dialog shows tags but cannot edit them.
- Internet radio plays HE-AAC ("AAC+") streams only as their AAC core,
  without their high frequencies, and does not play Opus or HLS streams (see
  [Internet radio](#internet-radio)). There is no directory of stations, no
  recording of streams, and no seeking in a stream or a file on the web.
- The playlist's MISC OPTIONS item does nothing, and there is no right-click
  menu for the playlist's entries (a right-click there opens the main menu).
- The dialogs' edit boxes paste (Ctrl+V) and select all (Ctrl+A), but cannot
  copy, or select part of their text. The Jump to file dialog keeps its
  highlight by position, not by entry, if the playlist changes while it is
  open.
- Gapless playback leaves out streams and files shorter than half a second
  (see [Audio](#audio)).
- On macOS, a file opened from the Finder does not reach NeoAmp: macOS sends
  it to the running application as an Apple Event, which winit does not pass
  on, and NeoAmp is not built as an app bundle that the Finder could open
  files with. Opening files from the command line works.
- The "now playing" display shows no cover art.
- Output devices, media controls and handing files to the running NeoAmp
  have been tried on Linux only. On Windows and macOS they follow what cpal
  and souvlaki document.
- The playlist shows a skin's font only where NeoAmp has a stand-in for it
  built in; any other font gives way to Liberation Sans (see
  [Skin fonts and pointers](#skin-fonts-and-pointers)). Skins' animated
  pointers stand still.
