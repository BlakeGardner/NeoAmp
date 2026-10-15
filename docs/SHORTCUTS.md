# Keyboard shortcuts

Every key NeoAmp responds to, by where it works. The mouse controls are in
[Controls](../README.md#controls) in the README.

The keys are Winamp 2.x's. On macOS, Cmd takes Ctrl's place and Option is Alt,
as in Mac apps. The macOS column writes them the way the menus do there:
⌘ Cmd, ⌥ Option, ⇧ Shift, ⌦ forward delete (fn+Delete on a laptop keyboard).
See [macOS differences](#macos-differences).

The code is `App::key_cmd` and `App::playlist_key` in
[src/app.rs](../src/app.rs), with the menus' keys in
[src/menu.rs](../src/menu.rs), the Jump to file dialog's in
[src/jump.rs](../src/jump.rs) and the File info dialog's in
[src/info_window.rs](../src/info_window.rs).

## Main, equalizer and playlist windows

These work in whichever of the three windows has the focus. The playlist has
[keys of its own](#playlist-window) that come first.

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
| J | J | Jump to file: search the playlist and play an entry |
| Alt+3 | ⌥3 | File info: the current track's tags and details |

### Windows and display

| Key | macOS | Action |
| --- | --- | --- |
| Alt+E | ⌥E | Show or hide the playlist |
| Alt+G | ⌥G | Show or hide the equalizer |
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
| Backspace | Delete | Delete the character before the caret |
| Delete | ⌦ | Delete the character after the caret |
| Enter | Return | Play the highlighted entry and close the dialog |
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
  search.
- ⌘W shades a window, as Ctrl+W does in Winamp, rather than closing it as in
  most Mac apps.

## Not implemented yet

Winamp shortcuts that NeoAmp does not have yet (see the
[roadmap](ROADMAP.md)): Ctrl+L (open a URL), which waits for internet radio
(Phase 6), and Ctrl+P (preferences), since NeoAmp has no Preferences dialog.
