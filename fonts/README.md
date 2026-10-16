# Built-in fonts

NeoAmp draws all its text in these fonts, which are built into the program
(`include_bytes!` in `src/font.rs`). It reads no fonts from the system, so the
text looks the same on Linux, Windows and macOS. The files are unmodified
copies of the upstream fonts.

| File | Font | Used for | Licence |
| --- | --- | --- | --- |
| `LiberationSans-Regular.ttf` | Liberation Sans 2.1.5 | Arial and Helvetica (same measurements), any font not listed here, the menus and dialogs | [SIL Open Font License 1.1](https://github.com/liberationfonts/liberation-fonts/blob/main/LICENSE) |
| `LiberationMono-Regular.ttf` | Liberation Mono 2.1.5 | Courier New, Courier and Andale Mono (same measurements); Lucida Console and Fixed | [SIL Open Font License 1.1](https://github.com/liberationfonts/liberation-fonts/blob/main/LICENSE) |
| `LiberationSerif-Regular.ttf` | Liberation Serif 2.1.5 | Times New Roman and Times (same measurements) | [SIL Open Font License 1.1](https://github.com/liberationfonts/liberation-fonts/blob/main/LICENSE) |
| `tahoma.ttf` | Wine's Tahoma | Tahoma, MS Sans Serif, Microsoft Sans Serif | [GNU LGPL 2.1 or later](https://gitlab.winehq.org/wine/wine/-/blob/wine-11.19/COPYING.LIB) |
| `DejaVuSans.ttf` | DejaVu Sans 2.37 | Verdana and Lucida Sans (close in look and width); the key symbols in macOS menus (⌘ ⌥ ⇧ ← → ⌦) | [Bitstream Vera and Arev licences, DejaVu's changes public domain](https://github.com/dejavu-fonts/dejavu-fonts/blob/master/LICENSE) |

Where they come from:

- **Liberation** (<https://github.com/liberationfonts/liberation-fonts>):
  Debian's `fonts-liberation` 1:2.1.5-3.
- **DejaVu** (<https://dejavu-fonts.github.io/>): Debian's
  `fonts-dejavu-core` 2.37-8.
- **Wine's Tahoma**: `fonts/tahoma.ttf` in Wine's repository at the tag
  `wine-11.19`
  (<https://gitlab.winehq.org/wine/wine/-/blob/wine-11.19/fonts/tahoma.ttf>).
  Its source is `fonts/tahoma.sfd` at the same tag. Copyright (c) 2004 Larry
  Snyder, based on Bitstream Vera Sans, copyright (c) 2003 Bitstream, Inc.
