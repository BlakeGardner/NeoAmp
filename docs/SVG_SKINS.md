# SVG skins

A classic skin's pictures are bitmaps, drawn for one screen pixel per skin
pixel. Zoomed in, they stay as blocky as they were drawn. A skin can also carry
its pictures as SVG, which NeoAmp draws as sharply as the screen allows at any
zoom. This page is for people making such a skin.

## How it works

An SVG skin is an ordinary `.wsz` file: a zip archive of the usual bitmaps and
text files. For any sheet, it can hold an SVG file named after the bitmap,
such as `cbuttons.svg` beside or instead of `cbuttons.bmp`.

- NeoAmp draws each sheet from its SVG if the skin has one, from its bitmap
  otherwise, and from the Winamp base skin's bitmap, built into NeoAmp, if it
  has neither, the way
  Winamp falls back for a missing bitmap. A skin can convert one sheet at a
  time.
- Winamp, Webamp and other players ignore the SVG files. A skin that keeps its
  bitmaps works there unchanged.
- The SVG is used at every zoom, 1x included. NeoAmp draws each window at as
  many pixels per skin pixel as the screen shows it, rounded up (see
  [Zoom](GUIDE.md#zoom) in the user guide), and draws each SVG at that size
  the first time a window needs it.
- Everything else comes from the skin's text files and pointers, as in any
  classic skin: `viscolor.txt`, `pledit.txt` (the playlist's colours and
  font), `region.txt` (the window shapes) and the `.cur` files.

## The sheets

Each SVG is the same size as the bitmap it replaces, one SVG unit per skin
pixel, and every sprite sits where it does in the bitmap. The easiest
template is the bitmap itself: these are the sizes of the Winamp base skin's.

| File | Size | What it holds |
| --- | --- | --- |
| `main.svg` | 275 x 116 | The main window's background |
| `titlebar.svg` | 344 x 87 | Title bars, the clutter bar and the windowshade bar |
| `cbuttons.svg` | 136 x 36 | Previous, play, pause, stop, next and eject, normal and pressed |
| `shufrep.svg` | 92 x 85 | Shuffle, repeat, EQ and PL buttons |
| `posbar.svg` | 307 x 10 | The position bar and its thumb |
| `volume.svg` | 68 x 433 | The volume slider; its thumb is at the bottom |
| `balance.svg` | 68 x 433 | The balance slider (without it, the volume's is used) |
| `monoster.svg` | 58 x 24 | Mono and stereo |
| `playpaus.svg` | 42 x 9 | The play, pause and stop indicator |
| `numbers.svg` | 99 x 13 | The time's digits |
| `nums_ex.svg` | 108 x 13 | The time's digits with a minus sign (optional) |
| `text.svg` | 155 x 74 | The 5 x 6 bitmap font (classic skins' is often 155 x 18) |
| `eqmain.svg` | 275 x 315 | The equalizer |
| `eq_ex.svg` | 275 x 82 | The equalizer's windowshade bar |
| `pledit.svg` | 280 x 186 | The playlist editor |

NeoAmp draws nothing from `gen`, `genex`, `mb` or `video`, in either form.

## Rules

- **Size.** The SVG's width and height (or its `viewBox`, when they are
  missing) give the sheet's size in skin pixels, from 1 to 1024 along each
  side. A fractional size is rounded and the drawing stretched to fit.
- **Opaque.** Paint the whole sheet. Anything left transparent is drawn black,
  as a bitmap's would be; window shapes come from `region.txt`.
- **Keep each sprite in its rectangle.** Whatever spills over a sprite's edge
  shows up in the sprite beside it. Put sprite edges on whole units, so that
  they fall between pixels at every zoom.
- **Text as outlines.** SVG text is not drawn, since NeoAmp reads no fonts.
  Convert text to paths before saving.
- **No pictures inside.** Bitmaps embedded in the SVG (PNG, JPEG, GIF, WebP)
  are not drawn, and files it links to are not loaded. Draw with shapes.
- **Plain SVG.** Up to 4 MB per file. Compressed `.svgz` files are not read.
- **Filters are slow.** Blurs and drop shadows work, but take longer to draw
  at large zooms. Prefer flat shapes and gradients.

An SVG that NeoAmp cannot use (not SVG, too large, or an impossible size) is
skipped with a warning on the terminal NeoAmp was started from, and the
sheet's bitmap is drawn instead. A sheet that would take more than 16.7
million pixels at a window's zoom, which only sheets far larger than the
classic ones reach, is drawn less finely and enlarged the rest of the way.

## Making one

1. Open the bitmap you are replacing in a vector editor such as Inkscape,
   with the page exactly the bitmap's size in pixels and one user unit per
   pixel. Lock it on a layer of its own as a guide.
2. Draw over it, keeping each sprite inside its rectangle.
3. Delete the bitmap layer, convert any text to paths, and save as plain SVG
   named after the bitmap (`cbuttons.svg`).
4. Add the SVG to the skin's zip and give the zip a `.wsz` name. Drop it onto
   NeoAmp and zoom in with **Ctrl+=** (**⌘=** on macOS) to see it.

To keep the skin working in Winamp, leave the bitmaps in the archive, or
export each SVG at its own size and save it as the bitmap.

NeoAmp's own skin, the one it shows by default (`skins/NeoAmp.wsz` in the
source repository), is an example with every sheet as SVG and no bitmaps. A script draws it rather than an
editor; see [The NeoAmp skin](DEVELOPMENT.md#the-neoamp-skin).
