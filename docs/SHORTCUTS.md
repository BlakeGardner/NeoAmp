# Keyboard shortcuts

Every key NeoAmp responds to, by where it works. The mouse controls are in
[Controls](GUIDE.md#controls) in the user guide.

The keys are Winamp 2.x's. On macOS, Cmd takes Ctrl's place and Option is Alt,
as in Mac apps. The macOS column writes them the way the menus do there:
⌘ Cmd, ⌥ Option, ⇧ Shift, ⌦ forward delete (fn+Delete on a laptop keyboard),
⌫ Delete (the key Windows calls Backspace) and ↩ Return.
See [macOS differences](#macos-differences).

The code is `App::key_cmd`, `App::playlist_key` and `App::milkdrop_key` in
[src/app.rs](../src/app.rs), with the menus' keys in
[src/menu.rs](../src/menu.rs), the Jump to file dialog's in
[src/jump.rs](../src/jump.rs) and [src/jump_window.rs](../src/jump_window.rs),
the Skin Museum dialog's in [src/museum_dialog.rs](../src/museum_dialog.rs)
and [src/museum_window.rs](../src/museum_window.rs),
the Open URL dialog's in [src/url_window.rs](../src/url_window.rs), the edit
boxes' in [src/text_box.rs](../src/text_box.rs) and the File info dialog's in
[src/info_window.rs](../src/info_window.rs).

## Main, equalizer and playlist windows

These work in whichever of NeoAmp's windows has the focus. The playlist and
MilkDrop have keys of their own ([playlist](#playlist-window),
[MilkDrop](#milkdrop-window)) that come first.

### Playback

| Key | macOS | Action |
| --- | --- | --- |
| Z | Z | Previous track |
| X | X | Play; resumes when paused, and starts the track again when it is already playing |
| C | C | Pause, or resume when paused |
| V | V | Stop |
| Shift+V | ⇧V | Fade out over 3 seconds, then stop (also Shift-click on Stop) |
| B | B | Next track |
| Left | ← | Back 5 seconds (while playing or paused) |
| Right | → | Forward 5 seconds (while playing or paused) |
| Up | ↑ | Volume up 2% (not in the playlist) |
| Down | ↓ | Volume down 2% (not in the playlist) |
| R | R | Repeat on or off |
| S | S | Shuffle on or off |

### Files

| Key | macOS | Action |
| --- | --- | --- |
| L | L | Open files; they replace the playlist |
| Shift+L | ⇧L | Open a folder; its files replace the playlist |
| Ctrl+L | ⌘L | Open a URL (internet radio, or a file on the web); it replaces the playlist |
| J | J | Jump to file: search the playlist and play an entry |
| Alt+3 | ⌥3 | File info: the current track's tags and details |

### Windows and display

| Key | macOS | Action |
| --- | --- | --- |
| Alt+E | ⌥E | Show or hide the playlist |
| Alt+G | ⌥G | Show or hide the equalizer |
| Ctrl+Shift+K | ⇧⌘K | Open or close MilkDrop's window |
| Ctrl+T | ⌘T | Time display: elapsed or remaining |
| Ctrl+W | ⌘W | Windowshade the main window: collapse it to a bar, or open it out again (in the playlist, the playlist) |
| Ctrl+A | ⌘A | Always on top on or off (not in the playlist, where it selects every entry) |
| Ctrl+D | ⌘D | Double size on or off |
| Ctrl+= (or Ctrl++) | ⌘= | Zoom in a half step, up to 3x |
| Ctrl+- | ⌘- | Zoom out a half step, down to 1x |
| Ctrl+0 | ⌘0 | Zoom back to the default for the screen |

### Menus

| Key | macOS | Action |
| --- | --- | --- |
| Alt+F | ⌥F | Open the main menu |
| Alt+S | ⌥S | Open the Skins menu |

## Media keys

The keyboard's media keys reach NeoAmp through the system's media controls
([src/now_playing.rs](../src/now_playing.rs)), so they work whichever
application has the focus, as do the buttons of the system's "now playing"
display: MPRIS on Linux (GNOME, KDE, COSMIC and most desktops pass the keys
on), the media overlay on Windows, Now Playing on macOS. With several players
running, the system decides which one the keys go to.

| Key | macOS | Action |
| --- | --- | --- |
| Play/Pause | ⏯ (F8) | Pause while playing, else play; unlike X, never starts a playing track over |
| Next track | ⏭ (F9) | Next track |
| Previous track | ⏮ (F7) | Previous track |
| Stop | (none on Mac keyboards) | Stop |

The display can also seek (by 5 seconds, by any amount, or to a spot) and, on
Linux, set the volume.

## Playlist window

While the playlist has the focus, these come before the keys above. The other
keys above still work there, except Up and Down (which move through the list
instead of setting the volume), Ctrl+A (which selects every entry instead of
keeping the windows on top), Ctrl+W (which shades the playlist instead of the
main window) and Alt+3 (which shows the selected entry instead of the current
track).

| Key | macOS | Action |
| --- | --- | --- |
| Up / Down | ↑ / ↓ | Move to the previous or next entry |
| Page Up / Page Down | Page Up / Page Down | Move a page up or down |
| Home / End | Home / End | Move to the first or last entry |
| Shift with any of the above | ⇧ with any of the above | Select as the cursor moves |
| Enter | Return | Play the entry at the cursor |
| Alt+Up / Alt+Down | ⌥↑ / ⌥↓ | Move the selected entries up or down a place |
| Ctrl+A | ⌘A | Select every entry |
| Ctrl+I | ⌘I | Invert the selection |
| Delete | ⌦ | Remove the selected entries |
| Ctrl+Delete | ⌘⌦ | Crop: remove every entry that is not selected |
| Alt+Delete | ⌥⌦ | Remove dead entries (files that no longer exist) |
| Ctrl+Shift+1 | ⇧⌘1 | Sort by title |
| Ctrl+Shift+2 | ⇧⌘2 | Sort by file name |
| Ctrl+Shift+3 | ⇧⌘3 (taken by macOS) | Sort by path and file name |
| Ctrl+R | ⌘R | Reverse the list |
| Ctrl+Shift+R | ⇧⌘R | Randomize the list |
| Ctrl+N | ⌘N | New list: remove every entry |
| Ctrl+O | ⌘O | Load a playlist file, replacing the list |
| Ctrl+S | ⌘S | Save the list to a playlist file |
| Ctrl+W | ⌘W | Windowshade the playlist: collapse it to a bar, or open it out again |
| Alt+3 | ⌥3 | File info for the first selected entry (or the one at the cursor) |

The sort keys and Alt+3 go by the digit keys' position, so they work on
layouts where Shift turns 1 into another character or 3 needs Shift.

### Clicks with modifiers

| Click | macOS | Action |
| --- | --- | --- |
| Shift-click | ⇧-click | Select everything from the last entry clicked |
| Ctrl-click | ⌘-click | Add or remove one entry from the selection |
| Ctrl+Shift-click | ⇧⌘-click | Add a range to the selection |

On macOS, Ctrl-click means right-click, so Cmd-click is the one that selects.

## MilkDrop window

While MilkDrop's window has the focus, these come before the keys above. The
other keys above still work there, except R (which changes the order of the
presets instead of turning repeat on or off) and L (which lists the presets
instead of opening files). They are MilkDrop's own keys.

| Key | macOS | Action |
| --- | --- | --- |
| Space | Space | Blend into the next preset |
| H | H | Cut to the next preset at once |
| Backspace | ⌫ | Go back to the preset before, at once |
| R | R | Presets in random order, or in the order listed |
| Scroll Lock | F14 | Lock the preset: it stays until you change it |
| L | L | List the presets, to choose one |
| Alt+Enter | ⌥↩ | Fill the screen with MilkDrop, or go back to the window (as a double-click on the picture does) |
| Escape | Esc | Go back to the window from filling the screen |

## In an open menu

Menus take the keyboard while they are open, as Windows' do.

| Key | macOS | Action |
| --- | --- | --- |
| Up / Down | ↑ / ↓ | Move the highlight |
| Right | → | Open the highlighted submenu |
| Left | ← | Close the submenu |
| Enter | Return | Choose the highlighted item, or open its submenu |
| Escape | Esc | Close the submenu, or the menu at the top level |
| A letter | A letter | Choose the item with that letter underlined; when several share it, each press moves to the next |

## Jump to file dialog

Typing searches the playlist. A key typed with Ctrl, Alt or Cmd held is not
added to the search.

| Key | macOS | Action |
| --- | --- | --- |
| Up / Down | ↑ / ↓ | Move the highlight in the list |
| Page Up / Page Down | Page Up / Page Down | Move the highlight a page |
| Left / Right | ← / → | Move the caret in the search box |
| Home / End | Home / End | Move the caret to the start or end of the search box |
| Backspace | Delete | Delete the character before the caret (or everything, when all is selected) |
| Delete | ⌦ | Delete the character after the caret (or everything, when all is selected) |
| Ctrl+V | ⌘V | Paste the clipboard's text into the search box |
| Ctrl+A | ⌘A | Select everything in the search box, so that typing replaces it |
| Enter | Return | Play the highlighted entry and close the dialog |
| Escape | Esc | Close the dialog |

## Skin Museum dialog

Skins > Skin Museum... opens it. Typing searches the museum once you pause. A
key typed with Ctrl, Alt or Cmd held is not added to the search.

| Key | macOS | Action |
| --- | --- | --- |
| Up / Down | ↑ / ↓ | Move the highlight in the list; NeoAmp tries the skin on a moment later |
| Page Up / Page Down | Page Up / Page Down | Move the highlight a page |
| Left / Right | ← / → | Move the caret in the search box |
| Home / End | Home / End | Move the caret to the start or end of the search box |
| Backspace | Delete | Delete the character before the caret (or everything, when all is selected) |
| Delete | ⌦ | Delete the character after the caret (or everything, when all is selected) |
| Ctrl+V | ⌘V | Paste the clipboard's text into the search box |
| Ctrl+A | ⌘A | Select everything in the search box, so that typing replaces it |
| Enter | Return | Search at once, if what is typed is not listed yet; otherwise install the highlighted skin |
| Escape | Esc | Close the dialog, putting back the skin you had |

## Open URL dialog

Ctrl+L or Play URL... in the main menu opens it as "Open URL"; the
playlist's ADD > URL opens it as "Add URL". It starts with the address on the
clipboard, selected, if the clipboard holds one, so that Enter opens it and
typing replaces it. A key typed with Ctrl, Alt or Cmd held is not added to
the address.

| Key | macOS | Action |
| --- | --- | --- |
| Left / Right | ← / → | Move the caret (with all selected: to its start or end) |
| Home / End | Home / End | Move the caret to the start or end |
| Backspace | Delete | Delete the character before the caret (or everything, when all is selected) |
| Delete | ⌦ | Delete the character after the caret (or everything, when all is selected) |
| Ctrl+V | ⌘V | Paste the clipboard's text |
| Ctrl+A | ⌘A | Select the whole address |
| Enter | Return | Open (or Add) the address and close the dialog |
| Escape | Esc | Close the dialog |

## File info dialog

The dialog only shows a file's tags and details; its one button is Close.

| Key | macOS | Action |
| --- | --- | --- |
| Enter, Escape or Space | Return, Esc or Space | Close the dialog |

## macOS differences

- Cmd replaces Ctrl, and Option replaces Alt, in every shortcut.
- macOS keeps ⇧⌘3 for screenshots, so sorting the playlist by path is only in
  the MISC > SORT LIST menu there.
- Delete is the forward-delete key, fn+Delete on a laptop keyboard. The key
  labelled "delete" on a Mac is Backspace, which only edits the Jump to file
  search and the Open URL address.
- ⌘W shades a window, as Ctrl+W does in Winamp, rather than closing it as in
  most Mac apps.
- Mac keyboards have no Scroll Lock. F14, which sits where Scroll Lock does
  on Apple's full-size keyboards, locks MilkDrop's preset instead; on other
  keyboards, use Lock preset in MilkDrop's menu.

## Not implemented yet

Winamp shortcuts that NeoAmp does not have yet (see the
[roadmap](ROADMAP.md)): Ctrl+P (preferences), since NeoAmp has no Preferences
dialog.
