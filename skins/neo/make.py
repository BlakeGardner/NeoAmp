#!/usr/bin/env python3
"""Draws the NeoAmp skin's sheets as SVG and packs them into a .wsz.

Every sheet is drawn at its classic size, one SVG unit per skin pixel, with each sprite where
src/sprites.rs expects it (see docs/SVG_SKINS.md). The layout and look follow Winamp 2.91's
base skin; text and icons are drawn as outlines so that they stay sharp at any zoom.

Usage: python3 make.py [OUT.wsz]
Writes the SVGs next to this script and the skin to ../NeoAmp.wsz (or OUT.wsz).
Needs only Python 3's standard library.
"""

import math
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


# --- SVG building ------------------------------------------------------------------------------


def num(v):
    """A coordinate, without trailing zeros."""
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


class Svg:
    def __init__(self, w, h):
        self.w, self.h = w, h
        self.defs = []
        self.body = []
        self.next_id = 0

    def new_id(self, prefix):
        self.next_id += 1
        return f"{prefix}{self.next_id}"

    def add(self, element):
        self.body.append(element)

    def comment(self, text):
        self.body.append(f"<!-- {text} -->")

    def rect(self, x, y, w, h, fill, extra=""):
        if w <= 0 or h <= 0:
            return
        self.add(
            f'<rect x="{num(x)}" y="{num(y)}" width="{num(w)}" height="{num(h)}" fill="{fill}"{extra}/>'
        )

    def px(self, x, y, fill):
        self.rect(x, y, 1, 1, fill)

    def hline(self, x, y, w, fill):
        self.rect(x, y, w, 1, fill)

    def vline(self, x, y, h, fill):
        self.rect(x, y, 1, h, fill)

    def poly(self, pts, fill, extra=""):
        p = " ".join(f"{num(x)},{num(y)}" for x, y in pts)
        self.add(f'<polygon points="{p}" fill="{fill}"{extra}/>')

    def path(self, d, fill="none", stroke=None, width=None, extra=""):
        s = f'<path d="{d}" fill="{fill}"'
        if stroke:
            s += f' stroke="{stroke}" stroke-width="{num(width)}"'
        self.add(s + extra + "/>")

    def linear(self, x1, y1, x2, y2, stops):
        """A gradient in user space; `stops` is a list of (offset, colour)."""
        gid = self.new_id("g")
        st = "".join(f'<stop offset="{num(o)}" stop-color="{c}"/>' for o, c in stops)
        self.defs.append(
            f'<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" '
            f'x1="{num(x1)}" y1="{num(y1)}" x2="{num(x2)}" y2="{num(y2)}">{st}</linearGradient>'
        )
        return f"url(#{gid})"

    def clip(self, inner):
        """A clip path made of the given elements; returns its id."""
        cid = self.new_id("c")
        self.defs.append(f'<clipPath id="{cid}">{inner}</clipPath>')
        return cid

    def open(self, transform=None, clip=None):
        a = ""
        if transform:
            a += f' transform="{transform}"'
        if clip:
            a += f' clip-path="url(#{clip})"'
        self.add(f"<g{a}>")

    def close(self):
        self.add("</g>")

    def text(self):
        out = [
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w}" height="{self.h}" '
            f'viewBox="0 0 {self.w} {self.h}">'
        ]
        if self.defs:
            out.append("<defs>")
            out.extend(self.defs)
            out.append("</defs>")
        out.extend(self.body)
        out.append("</svg>")
        return "\n".join(out) + "\n"


# --- Geometry ----------------------------------------------------------------------------------


def inside(pts, x, y):
    """Whether (x, y) is inside the polygon (even-odd)."""
    n = len(pts)
    c = False
    for i in range(n):
        (x1, y1), (x2, y2) = pts[i], pts[(i + 1) % n]
        if (y1 > y) != (y2 > y) and x < (x2 - x1) * (y - y1) / (y2 - y1) + x1:
            c = not c
    return c


def edge_normals(pts):
    """Each edge's outward unit normal."""
    out = []
    n = len(pts)
    for i in range(n):
        (x1, y1), (x2, y2) = pts[i], pts[(i + 1) % n]
        ex, ey = x2 - x1, y2 - y1
        length = math.hypot(ex, ey)
        nx, ny = -ey / length, ex / length
        mx, my = (x1 + x2) / 2, (y1 + y2) / 2
        if inside(pts, mx + nx * 0.01, my + ny * 0.01):
            nx, ny = -nx, -ny
        out.append((nx, ny))
    return out


def inset(pts, d):
    """The polygon with every edge moved inwards by `d`."""
    normals = edge_normals(pts)
    n = len(pts)
    lines = []
    for i in range(n):
        (x1, y1) = pts[i]
        nx, ny = normals[i]
        (x2, y2) = pts[(i + 1) % n]
        lines.append(((x1 - nx * d, y1 - ny * d), (x2 - nx * d, y2 - ny * d)))
    out = []
    for i in range(n):
        (a1, a2), (b1, b2) = lines[i - 1], lines[i]
        out.append(intersect(a1, a2, b1, b2))
    return out


def intersect(a1, a2, b1, b2):
    (x1, y1), (x2, y2), (x3, y3), (x4, y4) = a1, a2, b1, b2
    den = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4)
    if abs(den) < 1e-9:
        return a2
    t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / den
    return (x1 + t * (x2 - x1), y1 + t * (y2 - y1))


LIGHT_FROM = (-0.5, -1.0)


def engraved(svg, pts, dark, light, face, d=1.0):
    """A shape cut into the surface: its top and left walls in shadow, its bottom and right
    walls lit, as the classic skin draws its button icons."""
    normals = edge_normals(pts)
    svg.poly(pts, dark)
    deep = inset(pts, d + 0.5)
    n = len(pts)
    for i in range(n):
        nx, ny = normals[i]
        if nx * LIGHT_FROM[0] + ny * LIGHT_FROM[1] <= 0:
            j = (i + 1) % n
            svg.poly([pts[i], pts[j], deep[j], deep[i]], light)
    svg.poly(inset(pts, d), face)



def shift(pts, dx, dy):
    return [(x + dx, y + dy) for x, y in pts]


# --- The main window's background --------------------------------------------------------------

# Measured from the base skin: the main window's background is a band of light, brightest a
# little right of the middle, tilted 21 degrees.
MAIN_BG_ANGLE = math.radians(21)
MAIN_BG_LEN = 300
MAIN_BG_STOPS = [
    (0, "#141420"), (12, "#161621"), (24, "#181724"), (36, "#1b1b2a"), (48, "#1f1f30"),
    (60, "#222235"), (72, "#26263b"), (84, "#292940"), (96, "#2d2c46"), (108, "#30304c"),
    (120, "#333351"), (132, "#363554"), (144, "#373757"), (156, "#383859"), (168, "#39395a"),
    (180, "#39395a"), (192, "#373757"), (204, "#363554"), (216, "#333350"), (228, "#30304b"),
    (240, "#2d2d46"), (252, "#292940"), (264, "#242439"), (276, "#201f31"), (300, "#1a1a29"),
]


def main_bg_fill(svg):
    """The main window's background gradient, in main-window coordinates."""
    dx, dy = math.cos(MAIN_BG_ANGLE), math.sin(MAIN_BG_ANGLE)
    return svg.linear(
        0, 0, dx * MAIN_BG_LEN, dy * MAIN_BG_LEN,
        [(t / MAIN_BG_LEN, c) for t, c in MAIN_BG_STOPS],
    )


def main_bg_patch(svg, sx, sy, w, h, mx, my):
    """Paints the sheet rectangle (sx, sy, w, h) with the main window's background as it is
    at (mx, my) in the main window, for sprites drawn over it."""
    cid = svg.clip(f'<rect x="{sx}" y="{sy}" width="{w}" height="{h}"/>')
    svg.open(transform=None, clip=cid)
    svg.open(transform=f"translate({sx - mx} {sy - my})")
    svg.rect(mx, my, w, h, main_bg_fill(svg))
    svg.close()
    svg.close()


# --- cbuttons: the transport buttons -----------------------------------------------------------

# Button greys, light to dark.
BTN_HI = "#efffff"
BTN_RIM = "#adb5c6"
BTN_FACE = "#bdced6"
BTN_ICON = "#97a8b9"
BTN_CORNER = "#8494a5"
BTN_SHADE = "#7b8494"
BTN_CORNER_DARK = "#526373"
BTN_DARK = "#4a5a6b"
# Pressed.
HOLE = "#080810"
PBTN_HI = "#adbdc6"
PBTN_FACE = "#7b8c9c"
PBTN_CORNER = "#637b84"
PBTN_CORNER_DARK = "#526b73"


BTN = {"hi": "#efffff", "rim": "#adb5c6", "face": "#bdced6", "corner": "#8494a5",
       "shade": "#7b8494", "corner_dark": "#526373", "dark": "#4a5a6b"}
PL_BTN = {"hi": "#edffff", "rim": "#adb4c4", "face": "#bfcdd3", "corner": "#8192a5",
          "shade": "#798296", "corner_dark": "#576476", "dark": "#4e586e"}


def raised_button(svg, x, y, w, h, shadow_row=True, corner_steps=False, pal=None):
    """A raised grey button `w` x `h` with its rim, highlight and shadow. Without
    `shadow_row`, the bottom row is left unpainted."""
    if pal:
        return _raised(svg, x, y, w, h, shadow_row, corner_steps, pal)
    return _raised(svg, x, y, w, h, shadow_row, corner_steps, BTN)


def _raised(svg, x, y, w, h, shadow_row, corner_steps, c):
    BTN_FACE, BTN_RIM, BTN_HI, BTN_SHADE = c["face"], c["rim"], c["hi"], c["shade"]
    BTN_CORNER, BTN_CORNER_DARK, BTN_DARK = c["corner"], c["corner_dark"], c["dark"]
    hh = h if shadow_row else h - 1
    svg.rect(x, y, w, hh, BTN_FACE)
    # Outer rim, light, on the top and left; a highlight inside it.
    svg.hline(x, y, w - 1, BTN_RIM)
    svg.vline(x, y, h - 1, BTN_RIM)
    svg.hline(x + 1, y + 1, w - 3, BTN_HI)
    svg.vline(x + 1, y + 1, h - 3, BTN_HI)
    svg.px(x + w - 2, y + 1, BTN_RIM)
    # Shadows: an inner rim, then two darker steps on the right and bottom.
    svg.vline(x + w - 3, y + 2, h - 4, BTN_RIM)
    svg.hline(x + 2, y + h - 3, w - 4, BTN_RIM)
    svg.vline(x + w - 2, y + 2, h - 3, BTN_SHADE)
    if corner_steps:
        svg.vline(x + w - 2, y + 2, 2, BTN_CORNER)
    svg.hline(x + 3, y + h - 2, w - 4, BTN_SHADE)
    svg.px(x + 2, y + h - 2, BTN_CORNER)
    svg.vline(x + w - 1, y + 2, hh - 2, BTN_DARK)
    svg.px(x + w - 1, y, BTN_CORNER)
    svg.px(x + w - 1, y + 1, BTN_CORNER_DARK)
    if shadow_row:
        svg.hline(x + 2, y + h - 1, w - 2, BTN_DARK)
        svg.px(x, y + h - 1, BTN_CORNER)
        svg.px(x + 1, y + h - 1, BTN_CORNER_DARK)


def pressed_button(svg, x, y, w, h):
    """A pushed-in button: a dark hole around a flat face lit along its top and left."""
    svg.rect(x, y, w, h, HOLE)
    fx, fy, fw, fh = x + 2, y + 2, w - 3, h - 2
    svg.rect(fx, fy, fw, fh - 1, PBTN_FACE)
    svg.hline(fx + 1, fy + 1, fw - 2, PBTN_HI)
    svg.vline(fx + 1, fy + 1, fh - 2, PBTN_HI)
    svg.px(fx, fy, PBTN_CORNER)
    svg.px(fx + fw - 2, fy, PBTN_CORNER)
    svg.px(fx + fw - 1, fy, PBTN_CORNER_DARK)
    svg.px(fx, fy + fh - 2, PBTN_CORNER)
    svg.px(fx, fy + fh - 1, PBTN_CORNER_DARK)


def transport_icons():
    """Each button's icon shapes, relative to the button's top left, unpressed."""
    tri_left = [(14.5, 4), (16, 4), (16, 14), (14.5, 14), (9.5, 9)]

    def bar(x):
        return [(x, 4), (x + 3, 4), (x + 3, 14), (x, 14)]

    return {
        "prev": [bar(6), tri_left],
        "play": [[(6, 4), (16.5, 9), (6, 14)]],
        "pause": [[(5, 4), (9, 4), (9, 14), (5, 14)], [(12, 4), (16, 4), (16, 14), (12, 14)]],
        "stop": [[(6, 4), (16, 4), (16, 14), (6, 14)]],
        "next": [[(6, 4), (7.5, 4), (12.5, 9), (7.5, 14), (6, 14)], bar(13)],
        "eject": [[(10.5, 3), (16.5, 9), (4.5, 9)], [(5, 10), (16, 10), (16, 13), (5, 13)]],
    }


def sheet_cbuttons():
    svg = Svg(136, 36)
    icons = transport_icons()
    # Each sprite is 23 x 18, the next button's 22 x 18; the last column (but the next
    # button's) and row show the main window's background beside the button.
    buttons = [("prev", 0, 22), ("play", 23, 22), ("pause", 46, 22), ("stop", 69, 22),
               ("next", 92, 22)]
    for name, x, w in buttons:
        svg.comment(f"{name}, normal and pressed")
        main_bg_patch(svg, x, 0, 23, 18, 16 + x, 88)
        raised_button(svg, x, 0, w, 17, corner_steps=True)
        for shape in icons[name]:
            engraved(svg, shift(shape, x, 0), BTN_DARK, BTN_HI, BTN_ICON)
        pressed_button(svg, x, 18, 23 if name != "next" else 22, 18)
        for shape in icons[name]:
            engraved(svg, shift(shape, x + 2, 20), HOLE, PBTN_HI, BTN_DARK)
    svg.comment("eject, normal and pressed")
    main_bg_patch(svg, 114, 0, 22, 16, 136, 89)
    raised_button(svg, 114, 0, 22, 16, shadow_row=False)
    for shape in icons["eject"]:
        engraved(svg, shift(shape, 114, 0), BTN_DARK, BTN_HI, BTN_ICON)
    pressed_button(svg, 114, 16, 22, 16)
    for shape in icons["eject"]:
        engraved(svg, shift(shape, 116, 17), HOLE, PBTN_HI, BTN_DARK)
    return svg


# --- Pixel art as outlines ---------------------------------------------------------------------


def pixel_art(svg, rows, x, y, fill, on="#"):
    """Draws pixel art (`rows` of text, `on` characters lit) as outlines: straight runs as
    exact rectangles, and pixels that only touch diagonally joined by smooth strokes, so
    diagonals and rounded corners stay smooth when enlarged."""
    lit = {(i, j) for j, row in enumerate(rows) for i, ch in enumerate(row) if ch in on}
    covered = set()
    for j, row in enumerate(rows):
        i = 0
        while i < len(row):
            if (i, j) in lit and (i + 1, j) in lit:
                k = i
                while (k, j) in lit:
                    k += 1
                svg.rect(x + i, y + j, k - i, 1, fill)
                covered.update((m, j) for m in range(i, k))
                i = k
            else:
                i += 1
    width = max(len(r) for r in rows)
    for i in range(width):
        j = 0
        while j < len(rows):
            if (i, j) in lit and (i, j + 1) in lit:
                k = j
                while (i, k) in lit:
                    k += 1
                svg.rect(x + i, y + j, 1, k - j, fill)
                covered.update((i, m) for m in range(j, k))
                j = k
            else:
                j += 1
    d = []
    diag = set()
    for (i, j) in sorted(lit):
        for di in (-1, 1):
            if (i + di, j + 1) in lit and (i + di, j) not in lit and (i, j + 1) not in lit:
                d.append(f"M{num(x + i + 0.5)} {num(y + j + 0.5)}l{di} 1")
                diag.update({(i, j), (i + di, j + 1)})
    if d:
        svg.add(
            f'<path d="{"".join(d)}" fill="none" stroke="{fill}" stroke-width="1" '
            'stroke-linecap="round"/>'
        )
    for p in sorted(lit - covered - diag):
        svg.rect(x + p[0], y + p[1], 1, 1, fill)


def pixel_font(spec):
    """Glyphs from rows of pixel art: `spec` maps a string of characters to rows in which
    the characters' glyphs sit side by side, separated by a column of spaces."""
    font = {}
    for chars, rows in spec.items():
        cols = max(len(r) for r in rows)
        rows = [r.ljust(cols) for r in rows]
        starts, i = [], 0
        while i < cols:
            if any(r[i] != " " for r in rows):
                j = i
                while j < cols and any(r[j] != " " for r in rows):
                    j += 1
                starts.append((i, j))
                i = j
            else:
                i += 1
        assert len(starts) == len(chars), (chars, starts)
        for ch, (a, b) in zip(chars, starts):
            font[ch] = [r[a:b] for r in rows]
    return font


def draw_text(svg, font, text, x, y, fill, gap=1):
    """Writes `text` in a pixel font; returns the x after it."""
    for ch in text:
        if ch == " ":
            x += 3
            continue
        g = font[ch]
        pixel_art(svg, g, x, y, fill)
        x += len(g[0]) + gap
    return x



# The 5-pixel capitals on the shuffle, EQ and PL buttons. The E of EQ is round.
SMALL_CAPS = pixel_font({
    "SHUFLE": [
        "### #  # #  # ### #   ###",
        "#   #  # #  # #   #   #  ",
        "### #### #  # ### #   ###",
        "  # #  # #  # #   #   #  ",
        "### #  # .##. #   ### ###",
    ],
    "eQPL": [
        ".## .##. ### #  ",
        "#   #  # # # #  ",
        "### #  # ### #  ",
        "#   # #. #   #  ",
        ".## .# # #   ###",
    ],
})


# --- shufrep: shuffle, repeat, EQ and PL --------------------------------------------------------

LABEL = "#2f374d"
LABEL_PRESSED = "#000000"
LED_OFF = "#105a00"
LED_ON = "#00d600"
LED_EDGE = "#d5def2"
LED_EDGE_PRESSED = "#8f9fa8"


def led(svg, x, y, w, h, lit, pressed):
    """A small square lamp set into a button."""
    edge = LED_EDGE_PRESSED if pressed else LED_EDGE
    svg.rect(x, y, w, h, edge)
    svg.rect(x, y, w - 1, h - 1, HOLE if pressed else BTN_DARK)
    svg.rect(x + 1, y + 1, w - 2, h - 2, LED_ON if lit else LED_OFF)


def repeat_icon(svg, x, y, fill):
    """The repeat button's loop arrow, its top left at (x, y)."""
    svg.path(
        f"M{num(x + 2)} {num(y + 4.5)}H{num(x + 8.5)}V{num(y + 0.5)}H{num(x + 1.5)}"
        f"Q{num(x + 0.5)} {num(y + 0.5)} {num(x + 0.5)} {num(y + 1.5)}V{num(y + 3)}",
        stroke=fill, width=1,
    )
    svg.poly([(x, y + 2), (x + 3, y + 2), (x, y + 5)], fill)


def sheet_shufrep():
    svg = Svg(92, 85)
    # Repeat (28 x 15) and shuffle (47 x 15): off, off pressed, on, on pressed, each over the
    # main window's background at (210, 89) and (164, 89).
    for row, (lit, pressed) in enumerate([(False, False), (False, True), (True, False),
                                          (True, True)]):
        y = row * 15
        # The buttons are 27 and 45 wide, inside a column of background on each side.
        for x, w, bw, mx, label in [(0, 28, 27, 210, None), (28, 47, 45, 164, "SHUFFLE")]:
            if pressed:
                svg.rect(x, y, w, 15, HOLE)
                pressed_button(svg, x, y, bw + 1, 15)
                ox, oy = x + 1, y + 1
            else:
                main_bg_patch(svg, x, y, w, 15, mx, 89)
                raised_button(svg, x + 1, y + 1, bw, 13)
                ox, oy = x, y
            led(svg, ox + 5, oy + 4, 5, 4, lit, pressed)
            fill = LABEL_PRESSED if pressed else LABEL
            if label:
                draw_text(svg, SMALL_CAPS, label, ox + 12, oy + 5, fill)
            else:
                repeat_icon(svg, ox + 12, oy + 5, fill)
    # EQ and PL (23 x 12): off and on in rows, normal then pressed across.
    for row, lit in enumerate([False, True]):
        y = 61 + row * 12
        for col, label in enumerate(["EQ", "PL"]):
            x = col * 23
            main_bg_patch(svg, x, y, 23, 12, 219 + col * 23, 58)
            raised_button(svg, x, y, 22, 11)
            led(svg, x + 4, y + 3, 4, 4, lit, False)
            draw_text(svg, SMALL_CAPS, label.replace("E", "e"), x + 10, y + 3, LABEL)
            px = 46 + col * 23
            pressed_button(svg, px, y, 23, 12)
            led(svg, px + 6, y + 5, 4, 4, lit, True)
            draw_text(svg, SMALL_CAPS, label.replace("E", "e"), px + 12, y + 5,
                      LABEL_PRESSED)
    return svg


# --- The display: digits, status and the text font ---------------------------------------------

LCD_BG = "#000000"
LCD_DOT = "#182129"
LCD_GREEN = "#00f800"


def lcd_dots(svg, x, y, w, h, x0=0, y0=0):
    """The faint dot grid of the display: a dot on every even column and row, counted from
    (x0, y0)."""
    d = []
    for j in range(y, y + h):
        if (j - y0) % 2:
            continue
        for i in range(x, x + w):
            if (i - x0) % 2 == 0:
                d.append(f"M{i} {j}h1v1h-1z")
    if d:
        svg.path("".join(d), fill=LCD_DOT)


# The time's digits, 0 to 9, then a blank, each 9 x 13: segments one pixel thick.
DIGITS = [
    ".#######.........#.#######..#######.#.......#.#######..#######.########..#######..#######..",
    "#.......#........#........#........##.......##........#................##.......##.......#.",
    "#.......#........#........#........##.......##........#................##.......##.......#.",
    "#.......#........#........#........##.......##........#................##.......##.......#.",
    "#.......#........#........#........##.......##........#................##.......##.......#.",
    "#.......#........#........#........##.......##........#................##.......##.......#.",
    "#.......#........#.#######..#######..########.#######.########.........#.#######..########.",
    "#.......#........##................#........#........##.......#........##.......#........#.",
    "#.......#........##................#........#........##.......#........##.......#........#.",
    "#.......#........##................#........#........##.......#........##.......#........#.",
    "#.......#........##................#........#........##.......#........##.......#........#.",
    "#.......#........##................#........#........##.......#........##.......#........#.",
    ".#######.........#.#######..#######.........#.#######..#######.........#.#######..#######..",
]


def sheet_numbers():
    svg = Svg(99, 13)
    svg.rect(0, 0, 99, 13, LCD_BG)
    for cell in range(11):
        cx = cell * 9
        rows = [r[cx:cx + 9] if cell < 10 else "" for r in DIGITS]
        lit = {(i, j) for j, r in enumerate(rows) for i, ch in enumerate(r) if ch == "#"}
        d = []
        for j in range(0, 13, 2):
            for i in range(0, 9, 2):
                if (i, j) not in lit:
                    d.append(f"M{cx + i} {j}h1v1h-1z")
        svg.path("".join(d), fill=LCD_DOT)
        if cell < 10:
            pixel_art(svg, rows, cx, 0, LCD_GREEN)
    return svg


def sheet_playpaus():
    svg = Svg(42, 9)
    svg.rect(0, 0, 42, 9, LCD_BG)
    lcd_dots(svg, 0, 0, 36, 9)
    green = "#00e800"
    svg.comment("play, pause and stop")
    svg.poly([(4, 0), (9, 4.5), (4, 9)], green)
    svg.rect(9, 2, 3, 5, green)
    svg.rect(15, 2, 3, 5, green)
    svg.rect(20, 2, 5, 5, green)
    svg.comment("not working, then working: the lamps beside the status")
    for x, top, bottom in [(36, green, "#4e0f00"), (39, "#114033", "#ff2833")]:
        svg.rect(x, 0, 3, 3, top)
        svg.rect(x, 6, 3, 3, bottom)
        lcd_dots(svg, x, 3, 3, 3, x0=0)
    return svg


# The text font: 31 columns and 3 rows of 5 x 6 cells, glyphs 4 pixels wide (src/font.rs maps
# characters to cells).
TEXT_FONT = [
    ".##..###...##..###..####.####..###.#..#..###....#.#..#.#....#..#.#..#..##..###...##..###...###..###.#..#.#..#.#..#.#..#.#..#.####..#.#....##...............",
    "#..#.#..#.#..#.#..#.#....#....#....#..#...#.....#.#.#..#....####.#..#.#..#.#..#.#..#.#..#.#......#..#..#.#..#.#..#.#..#.#..#....#..#.#...#..#..............",
    "####.###..#....#..#.###..###..#.##.####...#.....#.##...#....####.##.#.#..#.###..#..#.###...##....#..#..#.#..#.#..#..##...##....#........#.##.#.............",
    "#..#.#..#.#....#..#.#....#....#..#.#..#...#..#..#.##...#....#..#.#.##.#..#.#....##.#.##......#...#..#..#.#..#.####..##....#...#.........#.####.............",
    "#..#.#..#.#..#.#..#.#....#....#..#.#..#...#..#..#.#.#..#....#..#.#..#.#..#.#....#.##.#.#.....#...#..#..#..##..####.#..#...#..#...........#.................",
    "#..#.###...##..###..####.#.....##..#..#..###..##..#..#.####.#..#.#..#..##..#.....###.#..#.###....#...##...##..#..#.#..#...#..####.........##...............",
    ".##....#..###..####...#..####..##..####..##...##..................................#...#..........#.............##....##...#....#...##..............#...##..",
    "#..#..##.....#...#...##..#....#.......#.#..#.#..#...................#.#...........#...#..........#..#........#.#......#..#.#..#.#..##.#......###..###.####.",
    "#.##...#.....#..##..#.#..###..###....#...##..#..#............##....#...#..............#........#####.#......#..#......#.#...#..#.....#............##...##..",
    "##.#...#...##.....#.####....#.#..#..#...#..#..###..................#...#...####.......#..........#....#....#...#......#.......#.##..#........###...##.####.",
    "#..#...#..#....#..#...#.....#.#..#..#...#..#....#............##.....#.#..........................#.....#..#....#......#.......#.#..#.##..#........###..##..",
    ".##....#..####..##....#..###...##...#....##...##..#.#.###.............................#...#####.........##.....##....##........#.#...##..#.........#.......",
    ".##..#..#.#..#..##.....",
    "#..#..##.......#..#...#",
    ".##..#..#..##.....#.###",
    "#..#.#..#.#..#..##...##",
    "####.#..#.####.......#.",
    "#..#..##..#..#..##.....",
]
TEXT_GREEN = "#00e200"


def sheet_text():
    svg = Svg(155, 18)
    svg.rect(0, 0, 155, 18, "#000000")
    # One row of glyphs at a time, so that no stroke reaches into the row below.
    for row in range(3):
        pixel_art(svg, TEXT_FONT[row * 6:row * 6 + 6], 0, row * 6, TEXT_GREEN)
    return svg


# --- monoster: mono and stereo --------------------------------------------------------------

def lowercase(name, x, y):
    """Path data for the few lowercase letters of the mono and stereo lamps, x-height 4 from
    `y`, the letter's left edge at `x`. Returns (path, advance)."""
    t, b, m = y + 0.5, y + 3.5, y + 2
    if name == "o":
        return (f"M{num(x + 1.5)} {num(t)}h1a1 1 0 0 1 1 1v1a1 1 0 0 1 -1 1h-1"
                f"a1 1 0 0 1 -1 -1v-1a1 1 0 0 1 1 -1z", 4.5)
    if name == "e":
        return (f"M{num(x + 0.5)} {num(m)}h3v-0.5a1 1 0 0 0 -1 -1h-1a1 1 0 0 0 -1 1v1"
                f"a1 1 0 0 0 1 1h2", 4.3)
    if name == "s":
        return (f"M{num(x + 3.2)} {num(t)}h-2a0.75 0.75 0 0 0 0 1.5h1.4a0.75 0.75 0 0 1 0 1.5"
                f"h-2.1", 4.0)
    if name == "t":
        return (f"M{num(x + 1.5)} {num(y - 1)}v3.5a1 1 0 0 0 1 1h0.8M{num(x)} {num(t)}h3", 3.7)
    if name == "r":
        return (f"M{num(x + 0.5)} {num(y)}v4M{num(x + 0.5)} {num(t + 1)}a1.2 1.2 0 0 1 1.2 -1"
                f"h1.3", 3.5)
    if name == "m":
        return (f"M{num(x + 0.5)} {num(y)}v4M{num(x + 0.5)} {num(t + 0.6)}a1 1 0 0 1 1 -0.6"
                f"h0.5a1 1 0 0 1 1 1v3M{num(x + 3)} {num(t + 0.4)}a1 1 0 0 1 1 -0.4h0.5"
                f"a1 1 0 0 1 1 1v3", 6.5)
    if name == "n":
        return (f"M{num(x + 0.5)} {num(y)}v4M{num(x + 0.5)} {num(t + 0.6)}a1 1 0 0 1 1 -0.6"
                f"h1a1 1 0 0 1 1 1v3", 4.5)
    raise ValueError(name)


def word(text, x, y, spacing):
    d = []
    for ch in text:
        path, adv = lowercase(ch, x, y)
        d.append(path)
        x += adv + spacing
    return "".join(d)


def sheet_monoster():
    svg = Svg(58, 24)
    glow = svg.new_id("f")
    svg.defs.append(
        f'<filter id="{glow}" x="-30%" y="-80%" width="160%" height="260%">'
        '<feGaussianBlur stdDeviation="1.4"/></filter>'
    )
    # Stereo (29 x 12) over the main window at (239, 41), mono (27 x 12) at (212, 41); lit,
    # then unlit below.
    for x, w, mx, text, tx, spacing in [(0, 29, 239, "stereo", 3, -0.25),
                                        (29, 27, 212, "mono", 34, 0.2)]:
        for row, lit in enumerate([True, False]):
            y = row * 12
            main_bg_patch(svg, x, y, w, 12, mx, 41)
            d = word(text, tx, y + 4, spacing)
            cid = svg.clip(f'<rect x="{x}" y="{y}" width="{w}" height="12"/>')
            svg.open(clip=cid)
            if lit:
                svg.path(d, stroke="#00c020", width=2.4,
                         extra=f' stroke-linecap="round" opacity="0.55" filter="url(#{glow})"')
                svg.path(d, stroke="#00ff00", width=1.0, extra=' stroke-linecap="round"')
            else:
                svg.path(d, stroke="#a2a2ae", width=1.0, extra=' stroke-linecap="round"')
            svg.close()
    return svg


# --- posbar: the position bar ------------------------------------------------------------------

# The groove is lit from the middle: (x, colour) stops for its top two rows, its body and its
# lit bottom edge.
POSBAR_TOP = [(0, "#11111a"), (24, "#151521"), (48, "#191926"), (72, "#1c1b2c"), (96, "#1e1e2f"),
              (120, "#1f1f31"), (156, "#1f1f31"), (180, "#1e1d2e"), (204, "#1a1a29"),
              (228, "#171723"), (248, "#141420")]
POSBAR_BODY = [(0, "#1b1b2a"), (12, "#1e1e2f"), (24, "#202033"), (36, "#242337"), (48, "#26263c"),
               (60, "#292940"), (72, "#2b2b44"), (84, "#2d2d47"), (96, "#2f2f4a"),
               (108, "#30304c"), (126, "#31314d"), (150, "#30304c"), (168, "#2f2f4a"),
               (180, "#2d2d47"), (192, "#2b2b44"), (204, "#28283f"), (216, "#26253b"),
               (228, "#232337"), (240, "#1f1f31"), (248, "#1e1e30")]
POSBAR_EDGE = [(0, "#636370"), (24, "#686877"), (48, "#6c6c7f"), (72, "#717186"), (96, "#73738a"),
               (126, "#75758c"), (156, "#74748b"), (180, "#717286"), (204, "#6f6e81"),
               (228, "#69697a"), (248, "#666574")]


def h_gradient(svg, stops):
    return svg.linear(0, 0, 248, 0, [(x / 248, c) for x, c in stops])


def gold_thumb(svg, x, c):
    """The position bar's thumb, 29 x 10 at (x, 0): a gold bar with a slot. `c` holds its
    colours, from light to dark."""
    svg.rect(x, 0, 29, 10, c["face"])
    svg.vline(x, 0, 9, c["edge"])
    svg.vline(x, 9, 1, c["edge2"])
    svg.hline(x + 1, 0, 27, c["rim"])
    svg.vline(x + 1, 0, 9, c["rim"])
    svg.hline(x + 1, 9, 28, c["shadow"])
    svg.vline(x + 28, 1, 9, c["shadow"])
    svg.vline(x + 27, 1, 8, c["dark"])
    svg.hline(x + 2, 8, 26, c["dark"])
    svg.px(x + 28, 0, c["dark"])
    # The slot: dark along its top and left, its bottom and right lit.
    svg.hline(x + 2, 1, 24, c["slot"])
    svg.vline(x + 2, 1, 7, c["slot"])
    svg.px(x + 26, 1, c["glint"])
    svg.rect(x + 3, 2, 23, 1, c["shine"])
    svg.rect(x + 3, 3, 23, 1, c["glint"])
    svg.rect(x + 3, 4, 23, 1, c["groove"])
    svg.rect(x + 3, 5, 23, 1, c["face"])
    svg.rect(x + 3, 6, 23, 1, c["shine"])
    svg.rect(x + 3, 7, 24, 1, c["face"])
    svg.vline(x + 3, 3, 4, c["shine"])
    svg.vline(x + 25, 2, 5, c["shine"])
    svg.vline(x + 26, 2, 5, c["face"])
    svg.px(x + 26, 7, c["groove"])
    svg.px(x + 2, 7, c["glint"])
    svg.px(x + 3, 2, c["mid"])


def sheet_posbar():
    svg = Svg(307, 10)
    svg.comment("the groove, 248 x 10")
    svg.rect(0, 0, 248, 2, h_gradient(svg, POSBAR_TOP))
    svg.rect(0, 2, 248, 7, h_gradient(svg, POSBAR_BODY))
    svg.rect(0, 9, 248, 1, h_gradient(svg, POSBAR_EDGE))
    svg.rect(0, 0, 1, 9, "#0f0f18")
    svg.comment("the thumb, normal and pressed")
    gold_thumb(svg, 248, {
        "edge": "#656573", "edge2": "#656573", "rim": "#dbcb9e", "face": "#af9861",
        "shine": "#f4eac7", "glint": "#f5f5f5", "groove": "#755b22", "mid": "#c5b27e",
        "slot": "#836930", "dark": "#8d753a", "shadow": "#090202",
    })
    svg.rect(277, 0, 1, 10, "#424a5a")
    gold_thumb(svg, 278, {
        "edge": "#6c5017", "edge2": "#614610", "rim": "#b0985e", "face": "#8f7539",
        "shine": "#c3ae75", "glint": "#d8c593", "groove": "#664b13", "mid": "#a1864b",
        "slot": "#70551b", "dark": "#785d21", "shadow": "#200c00",
    })
    return svg


# --- volume and balance -------------------------------------------------------------------------

# Each of the 28 frames' bar, top row to bottom row, green at the left (quiet, or centred)
# to red at the right.
SLIDER_BARS = [
    ("#1c6c14", "#15800a", "#18920b", "#28991c"), ("#2f811b", "#2e9c12", "#34b215", "#42b626"),
    ("#478627", "#4fa423", "#5aba28", "#65be37"), ("#519b2c", "#5cc02a", "#69da30", "#73dc3e"),
    ("#5c9b2c", "#6bc02a", "#7ada30", "#82dc3e"), ("#61a02c", "#72c72a", "#81e230", "#8ae33e"),
    ("#6da02c", "#81c72a", "#92e230", "#9ae33e"), ("#77a031", "#90c731", "#a4e238", "#a9e346"),
    ("#77a031", "#90c731", "#a4e238", "#a9e346"), ("#88a031", "#a6c731", "#bde238", "#c1e346"),
    ("#8e9b2c", "#aec02a", "#c6da30", "#c9dc3e"), ("#8e9b2c", "#aec02a", "#c6da30", "#c9dc3e"),
    ("#8e9b2c", "#aec02a", "#c6da30", "#c9dc3e"), ("#8e9b2c", "#aec02a", "#c6da30", "#c9dc3e"),
    ("#9f9b2c", "#c5c02a", "#e0da30", "#e1dc3e"), ("#9f902c", "#c5b12a", "#e0c930", "#e1cc3e"),
    ("#9f902c", "#c5b12a", "#e0c930", "#e1cc3e"), ("#9f902c", "#c5b12a", "#e0c930", "#e1cc3e"),
    ("#9f8127", "#c59c23", "#e0b228", "#e1b637"), ("#9f6c27", "#c58023", "#e09228", "#e19937"),
    ("#9f6c27", "#c58023", "#e09228", "#e19937"), ("#9b5d21", "#bf6d1b", "#d97c1f", "#db842f"),
    ("#8e5b17", "#ae6a0d", "#c6780f", "#c98120"), ("#9f5721", "#c5651b", "#e0721e", "#e17c2e"),
    ("#9f4921", "#c5511b", "#e05c1e", "#e1672e"), ("#9f3e21", "#c5431b", "#e04c1e", "#e1582e"),
    ("#9f2a1b", "#c52712", "#e02c15", "#e13c26"), ("#9f161b", "#c50c12", "#e00e15", "#e11f26"),
]


def slider_thumb(svg, x, y, pressed):
    """A volume or balance thumb, 14 x 11: grey with three grip lines, or dark when held."""
    if pressed:
        face, hi, grip, right = "#000000", "#dae7ea", "#ffffff", "#4a5a6b"
    else:
        face, hi, grip, right = "#adbcc4", "#dae7ea", "#0b0f16", "#7b8494"
    svg.rect(x, y, 14, 11, "#000000")
    svg.rect(x + 1, y + 1, 12, 9, face)
    svg.hline(x, y, 14, "#0b0f16")
    svg.hline(x + 1, y + 1, 11, hi)
    svg.vline(x + 1, y + 1, 9, hi)
    svg.vline(x + 11, y + 2, 7, right)
    svg.hline(x + 1, y + 9, 11, "#4a5a6b" if not pressed else "#7b8494")
    svg.hline(x + 1, y + 8, 10, face if pressed else "#7b8494")
    if pressed:
        svg.hline(x + 1, y + 9, 11, "#4a5a6b")
    for gx in (4, 6, 8):
        svg.rect(x + gx, y + 4 - (0 if pressed else 1) + 0, 1, 3 if pressed else 4, grip)


def slider_sheet(x0, groove_w, bg_mx, outside):
    svg = Svg(68, 433)
    svg.rect(0, 0, 68, 433, outside)
    for i, bar in enumerate(SLIDER_BARS):
        y = i * 15
        main_bg_patch(svg, x0, y, 68 if outside == "none" else 38, 14, bg_mx, 57)
        svg.add(f'<rect x="{x0}" y="{y + 3}" width="{groove_w}" height="6" rx="2.5" '
                f'fill="#7e7e92"/>')
        svg.add(f'<rect x="{x0}" y="{y + 3}" width="{groove_w}" height="5" rx="2.5" '
                f'fill="#171724"/>')
        fill = svg.linear(0, y + 4, 0, y + 8, [(0.125, bar[0]), (0.375, bar[1]),
                                              (0.625, bar[2]), (0.875, bar[3])])
        svg.add(f'<rect x="{x0 + 1}" y="{y + 4}" width="{groove_w - 2}" height="4" rx="2" '
                f'fill="{fill}"/>')
        svg.hline(x0, y + 14, 68 if outside == "none" else 38, "#c8c5d3")
    svg.comment("the thumbs: pressed, then normal")
    svg.rect(0, 419, 68, 1, "#c8c5d3")
    svg.rect(0, 420, 31, 13, "#2f2f44")
    svg.rect(31, 421, 37, 12, "#19202a")
    slider_thumb(svg, 0, 422, True)
    slider_thumb(svg, 15, 422, False)
    return svg


def sheet_volume():
    return slider_sheet(0, 65, 107, "none")


def sheet_balance():
    return slider_sheet(9, 37, 177, "#000000")


# --- The title bars' lettering ----------------------------------------------------------------

# Bold capitals 6 high: 2-wide stems, 1.2-high bars. Each glyph is (advance, [path, ...]) in
# its own units, each path filled even-odd.
TITLE_FONT = {
    "N": (6.4, ["M0 0H2L4.4 3.9V0H6.4V6H4.4L2 2.1V6H0Z"]),
    "E": (4.6, ["M0 0H4.6V1.2H2V2.4H4.2V3.6H2V4.8H4.6V6H0Z"]),
    "O": (6.4, ["M2 0H4.4A2 2 0 0 1 6.4 2V4A2 2 0 0 1 4.4 6H2A2 2 0 0 1 0 4V2A2 2 0 0 1 2 0Z"
                "M2.6 1.2A0.6 0.6 0 0 0 2 1.8V4.2A0.6 0.6 0 0 0 2.6 4.8H3.8A0.6 0.6 0 0 0 4.4 4.2"
                "V1.8A0.6 0.6 0 0 0 3.8 1.2Z"]),
    "Q": (6.6, ["M2 0H4.4A2 2 0 0 1 6.4 2V4A2 2 0 0 1 4.4 6H2A2 2 0 0 1 0 4V2A2 2 0 0 1 2 0Z"
                "M2.6 1.2A0.6 0.6 0 0 0 2 1.8V4.2A0.6 0.6 0 0 0 2.6 4.8H3.8A0.6 0.6 0 0 0 4.4 4.2"
                "V1.8A0.6 0.6 0 0 0 3.8 1.2Z", "M3.4 4L4.9 3.6L6.6 6H5Z"]),
    "A": (6.6, ["M2.2 0H4.4L6.6 6H4.6L4.2 4.8H2.4L2 6H0ZM2.8 3.6H3.8L3.3 2Z"]),
    "M": (8.4, ["M0 0H2.3L4.2 3.5L6.1 0H8.4V6H6.4V2.8L4.8 5.4H3.6L2 2.8V6H0Z"]),
    "P": (5.8, ["M0 0H3.8A2 2 0 0 1 5.8 2V2.2A2 2 0 0 1 3.8 4.2H2V6H0Z"
                "M2 1.2V3H3.4A0.6 0.6 0 0 0 4 2.4V1.8A0.6 0.6 0 0 0 3.4 1.2Z"]),
    "R": (6.0, ["M0 0H3.8A2 2 0 0 1 5.8 2V2.2A2 2 0 0 1 4.5 4L6 6H3.8L2.7 4.2H2V6H0Z"
                "M2 1.2V3H3.4A0.6 0.6 0 0 0 4 2.4V1.8A0.6 0.6 0 0 0 3.4 1.2Z"]),
    "U": (6.2, ["M0 0H2V4.2A0.6 0.6 0 0 0 2.6 4.8H3.6A0.6 0.6 0 0 0 4.2 4.2V0H6.2V4"
                "A2 2 0 0 1 4.2 6H2A2 2 0 0 1 0 4Z"]),
    "L": (4.6, ["M0 0H2V4.8H4.6V6H0Z"]),
    "I": (2.0, ["M0 0H2V6H0Z"]),
    "Z": (5.4, ["M0 0H5.4V1.2L2.4 4.8H5.4V6H0V4.8L3 1.2H0Z"]),
    "Y": (6.4, ["M0 0H2.1L3.2 2.3L4.3 0H6.4L4.2 3.8V6H2.2V3.8Z"]),
    "S": (5.4, ["M1.9 0H5.2V1.2H2.2A0.6 0.6 0 0 0 2.2 2.4H3.5A1.8 1.8 0 0 1 3.5 6H0.2V4.8H3.2"
                "A0.6 0.6 0 0 0 3.2 3.6H1.9A1.8 1.8 0 0 1 1.9 0Z"]),
    "T": (6.0, ["M0 0H6V1.2H4V6H2V1.2H0Z"]),
    # The rest of the alphabet, for the titles of generic windows (gen.svg).
    "B": (5.8, ["M0 0H3.6A1.8 1.8 0 0 1 5.4 1.8A1.5 1.5 0 0 1 4.7 3A1.6 1.6 0 0 1 5.8 4.4"
                "A1.6 1.6 0 0 1 4.2 6H0Z"
                "M2 1.2V2.4H3.2A0.6 0.6 0 0 0 3.2 1.2Z"
                "M2 3.6V4.8H3.6A0.6 0.6 0 0 0 3.6 3.6Z"]),
    "C": (5.6, ["M2 0H5.6V1.2H2.6A0.6 0.6 0 0 0 2 1.8V4.2A0.6 0.6 0 0 0 2.6 4.8H5.6V6H2"
                "A2 2 0 0 1 0 4V2A2 2 0 0 1 2 0Z"]),
    "D": (6.2, ["M0 0H4A2.2 2.2 0 0 1 6.2 2.2V3.8A2.2 2.2 0 0 1 4 6H0Z"
                "M2 1.2V4.8H3.6A0.6 0.6 0 0 0 4.2 4.2V1.8A0.6 0.6 0 0 0 3.6 1.2Z"]),
    "F": (4.6, ["M0 0H4.6V1.2H2V2.4H4.2V3.6H2V6H0Z"]),
    "G": (6.2, ["M2 0H6V1.2H2.6A0.6 0.6 0 0 0 2 1.8V4.2A0.6 0.6 0 0 0 2.6 4.8H4.2V3.8H3.2"
                "V2.6H6.2V6H2A2 2 0 0 1 0 4V2A2 2 0 0 1 2 0Z"]),
    "H": (6.2, ["M0 0H2V2.4H4.2V0H6.2V6H4.2V3.6H2V6H0Z"]),
    "J": (5.2, ["M3.2 0H5.2V4A2 2 0 0 1 3.2 6H2A2 2 0 0 1 0 4V3.4H2V4.2A0.6 0.6 0 0 0 2.6 4.8"
                "A0.6 0.6 0 0 0 3.2 4.2Z"]),
    "K": (6.2, ["M0 0H2V2.3L4 0H6.2L3.5 3L6.2 6H4L2 3.7V6H0Z"]),
    "V": (6.6, ["M0 0H2.1L3.3 3.9L4.5 0H6.6L4.4 6H2.2Z"]),
    "W": (8.8, ["M0 0H2L2.6 3.6L3.5 0H5.3L6.2 3.6L6.8 0H8.8L7.5 6H5.4L4.4 2.6L3.4 6H1.3Z"]),
    "X": (6.4, ["M0 0H2.2L3.2 1.8L4.2 0H6.4L4.3 3L6.4 6H4.2L3.2 4.2L2.2 6H0L2.1 3Z"]),
}
TITLE_GAP = 1.3


def title_width(text, gap=TITLE_GAP, sx=1.0):
    adv = sum(TITLE_FONT[ch][0] if ch != " " else 2.4 for ch in text)
    return (adv + gap * (len(text) - 1)) * sx


def title_text(svg, text, x, y, fill, gap=TITLE_GAP, sx=1.0):
    """Writes `text` in the title bars' capitals, its top left at (x, y), narrowed to `sx`
    of its width."""
    for ch in text:
        if ch == " ":
            x += (2.4 + gap) * sx
            continue
        adv, paths = TITLE_FONT[ch]
        scale = f" scale({num(sx)} 1)" if sx != 1 else ""
        for d in paths:
            svg.add(f'<path transform="translate({num(x)} {num(y)}){scale}" d="{d}" '
                    f'fill="{fill}" fill-rule="evenodd"/>')
        x += (adv + gap) * sx


# --- titlebar: title bars, their buttons, the clutter bar and the windowshade bar ----------------

TITLE_LINE_STOPS = [(0, "#5b5b62"), (16, "#5d5d67"), (32, "#60606a"), (48, "#62626e"),
                    (64, "#646472"), (80, "#696877"), (96, "#6b6b7c"), (112, "#6e6e81"),
                    (128, "#717186"), (144, "#737388"), (160, "#74748b"), (176, "#75758c"),
                    (192, "#75758c"), (208, "#73738a"), (224, "#717186"), (240, "#707083"),
                    (256, "#6c6c7e"), (274, "#6a6a7a")]
GOLD = {
    True: ("#1e1f25", "#ecce7a", "#ffffff", "#45413c", "#a3946a", "#ecce7a", "#25262c"),
    False: ("#1f1f22", "#86774d", "#909090", "#34302c", "#625a45", "#86774d", "#222326"),
}


def bar_background(svg, x, y, w, mx=None):
    """A title bar's background, 14 rows from (x, y), as over the main window's top with its
    left edge at main-window x `mx` (by default, as titlebar.bmp places it)."""
    main_bg_patch(svg, x, y, w, 14, x - 27 if mx is None else mx, 0)
    svg.rect(x, y, w, 1, "#000000", ' opacity="0.42"')
    svg.rect(x, y + 12, w, 2, "#000000", ' opacity="0.3"')
    line = svg.linear(x, 0, x + 274, 0, [(t / 274, c) for t, c in TITLE_LINE_STOPS])
    svg.rect(x + 1, y + 1, w - 1, 1, line)
    svg.rect(x + 1, y + 1, 1, 13, "#5b5b62")
    svg.rect(x, y, 1, 14, "#0d0d14")


def stripes(svg, x, y, w, active):
    """The gold stripes beside a title, 7 rows from (x, y), with rounded ends."""
    c = GOLD[active]
    svg.rect(x - 1, y, w + 2, 7, c[0])
    svg.rect(x, y, w, 1, c[0])
    svg.rect(x, y + 6, w, 1, c[6])
    cid = svg.clip(f'<rect x="{x}" y="{y + 1}" width="{w}" height="5" rx="1.2"/>')
    svg.open(clip=cid)
    for i in range(5):
        svg.rect(x, y + 1 + i, w, 1, c[1 + i])
    svg.close()


def options_icon(svg, x, y, gold, inside, base):
    """The options menu's squiggle, in a 9 x 9 cell at (x, y)."""
    svg.poly([(x + 0.5, y + 4.5), (x + 2, y + 1.3), (x + 3.5, y + 4.5)], inside,
             f' stroke="{gold}" stroke-width="1" stroke-linejoin="round"')
    svg.poly([(x + 4.5, y + 4.5), (x + 6, y + 7.7), (x + 7.5, y + 4.5)], inside,
             f' stroke="{gold}" stroke-width="1" stroke-linejoin="round"')
    svg.path(f"M{num(x + 1)} {num(y + 4.5)}H{num(x + 3)}M{num(x + 5)} {num(y + 4.5)}H{num(x + 7)}",
             stroke=base, width=1)


def dome(svg, x, y, stops, flip=False):
    """A round title-bar button, 7 x 7 at (x, y), shaded top to bottom."""
    if flip:
        stops = [(1 - o, c) for o, c in reversed(stops)]
    fill = svg.linear(0, y, 0, y + 7, stops)
    svg.add(f'<rect x="{x}" y="{y}" width="7" height="7" rx="2.2" fill="{fill}"/>')


MIN_DOME = {
    True: [(0, "#45413c"), (0.2, "#524a3d"), (0.38, "#6b5d43"), (0.52, "#887749"),
           (0.68, "#bfb97e"), (0.82, "#cee1ce"), (1, "#bbd6d9")],
    False: [(0, "#1e110a"), (0.2, "#241b09"), (0.38, "#352d10"), (0.52, "#513e1b"),
            (0.68, "#86774d"), (0.82, "#81978d"), (1, "#73939a")],
    "pressed": [(0, "#45413c"), (0.2, "#524a3d"), (0.38, "#6b5d43"), (0.55, "#887749"),
                (0.75, "#bfb97e"), (1, "#9c9175")],
}
SHADE_DOME = {
    True: [(0, "#b3b6b8"), (0.22, "#cee2d1"), (0.36, "#6b5d43"), (0.7, "#5a573a"),
           (1, "#45413c")],
    False: [(0, "#73939a"), (0.22, "#89959a"), (0.36, "#352d10"), (0.8, "#352d10"),
            (1, "#141414")],
    "pressed": [(0, "#bfb97e"), (0.25, "#bfb97e"), (0.36, "#6b5d43"), (0.7, "#524a3d"),
                (1, "#45413c")],
}


def close_box(svg, x, y, active, pressed=False):
    """The close button: an X in a small dark box, 7 x 7 at (x, y)."""
    edge = "#6b5d43"
    svg.add(f'<rect x="{x}" y="{y}" width="7" height="7" rx="1" fill="{edge}"/>')
    svg.add(f'<rect x="{x + 0.6}" y="{y + 0.6}" width="5.8" height="5.8" rx="0.6" '
            f'fill="{"#352d10" if active else "#0c0c0c"}"/>')
    cross = "#b69c54" if active and not pressed else ("#a3946a" if pressed else "#685935")
    svg.path(f"M{num(x + 1.3)} {num(y + 1.3)}L{num(x + 5.7)} {num(y + 5.7)}"
             f"M{num(x + 5.7)} {num(y + 1.3)}L{num(x + 1.3)} {num(y + 5.7)}",
             stroke=cross, width=1.1, extra=' stroke-linecap="round"')
    if active:
        svg.add(f'<circle cx="{num(x + 3.5)}" cy="{num(y + 3.5)}" r="0.75" fill="#ffffff"/>')


def disc(svg, x, y, centre_stops, rim):
    """The windowshade bar's shade button: a disc, 7 x 7 at (x, y)."""
    svg.add(f'<circle cx="{num(x + 3.5)}" cy="{num(y + 3.5)}" r="3.5" fill="{rim}"/>')
    fill = svg.linear(x + 5, y + 1, x + 2, y + 6, centre_stops)
    svg.add(f'<circle cx="{num(x + 3.6)}" cy="{num(y + 3.4)}" r="2.45" fill="{fill}"/>')


DISC = {
    True: ([(0, "#aabab1"), (0.5, "#aabab1"), (1, "#cee1ce")], "#4e4740"),
    False: ([(0, "#73939a"), (0.5, "#73939a"), (1, "#81978d")], "#2a2210"),
    "pressed": ([(0, "#bfb97e"), (0.5, "#bfb97e"), (1, "#c2b161")], "#4e4740"),
}


def title_buttons(svg, x, y, active, shaded=False):
    """Minimize, shade and close, as the bar draws them, from the minimize button at (x, y)."""
    dome(svg, x + 1, y + 1, MIN_DOME[active])
    if shaded:
        stops, rim = DISC[active]
        disc(svg, x + 11, y + 1, stops, rim)
    else:
        dome(svg, x + 11, y + 1, SHADE_DOME[active])
    close_box(svg, x + 21, y + 1, active)


def pressed_cell(svg, x, y):
    """A pressed title-bar button's gold 9 x 9 square."""
    svg.rect(x, y, 9, 9, "#a3946a")


def title_bar(svg, y, active, text="NEOAMP"):
    """The main window's title bar, 275 x 14 at (27, y)."""
    bar_background(svg, 27, y, 275)
    icon = ("#efbe66", "#34302c", "#9a875d") if active else ("#a08134", "#2c231f", "#6f6340")
    options_icon(svg, 33, y + 3, *icon)
    if text:
        w = title_width(text)
        cx = 164.5
        tx = round(cx - w / 2, 1)
        stripes(svg, 47, y + 4, round(tx - 3 - 47), active)
        right = round(tx + w + 3)
        stripes(svg, right, y + 4, 269 - right, active)
        title_text(svg, text, tx, y + 4.5, "#ffffff" if active else "#909090")
    else:
        stripes(svg, 47, y + 4, 222, active)
    title_buttons(svg, 271, y + 3, active)


def mirrored(pts, axis):
    return [(2 * axis - x, y) for x, y in reversed(pts)]


_PREV_TRI = [(196.8, 36.5), (202, 32.6), (202, 40.4)]
_PREV_BAR = [(195.5, 33), (197.3, 33), (197.3, 40), (195.5, 40)]
# The windowshade bar's transport icons, as convex outlines in the active bar (y 29 to 42):
# gold with a white core and a muted edge.
SHADE_ICONS = [
    _PREV_TRI, _PREV_BAR,
    [(204, 32.6), (212.2, 36.5), (204, 40.4)],
    [(214, 33), (218, 33), (218, 40), (214, 40)],
    [(218, 33), (222, 33), (222, 40), (218, 40)],
    [(224, 33), (231, 33), (231, 40), (224, 40)],
    mirrored(_PREV_TRI, 217.5), mirrored(_PREV_BAR, 217.5),
    [(247, 33.2), (251.2, 37.6), (242.8, 37.6)],
    [(244, 38.4), (251, 38.4), (251, 40.2), (244, 40.2)],
]


def shade_bar(svg, y, active):
    """The main window in windowshade mode, 275 x 14 at (27, y)."""
    bar_background(svg, 27, y, 275)
    icon = ("#efbe66", "#34302c", "#9a875d") if active else ("#a08134", "#2c231f", "#6f6340")
    options_icon(svg, 33, y + 3, *icon)
    # Narrower here, to fit before the stripes as Winamp's own name does.
    title_text(svg, "NEOAMP", 47, y + 4.8, "#ffffff" if active else "#909090", gap=1.0,
               sx=0.9)
    for x, w in [(91, 14), (145, 7), (185, 5)]:
        stripes(svg, x, y + 4, w, active)
    for x, w in [(105, 40), (152, 33)]:
        svg.rect(x, y + 4, w, 7, "#0a0d16")
        svg.rect(x, y + 5, w, 5, "#000000")
    svg.rect(171, y + 6, 2, 1, "#00f800")
    svg.rect(171, y + 8, 2, 1, "#00f800")
    gold, white, edge = (("#ecce7a", "#ffffff", "#736c52") if active
                         else ("#86774d", "#909090", "#625a45"))
    dy = y - 29
    for shape in SHADE_ICONS:
        pts = shift(shape, 0, dy)
        svg.poly(pts, edge)
    for shape in SHADE_ICONS:
        pts = shift(shape, 0, dy)
        svg.poly(inset(pts, 0.6), gold)
        core = inset(pts, 1.4)
        if signed_area(core) * signed_area(pts) > 0 and polygon_area(core) > 0.3:
            svg.poly(core, white)
    svg.comment("the position bar's slot")
    svg.rect(252, y + 4, 18, 7, "#242439")
    svg.rect(253, y + 5, 17, 5, "#2d2d47")
    svg.rect(253, y + 10, 17, 1, "#717186")
    title_buttons(svg, 271, y + 3, active, shaded=True)


def signed_area(pts):
    a = 0
    for i in range(len(pts)):
        (x1, y1), (x2, y2) = pts[i], pts[(i + 1) % len(pts)]
        a += x1 * y2 - x2 * y1
    return a / 2


def polygon_area(pts):
    return abs(signed_area(pts))


CLUTTER_LETTERS = {
    "O": (3, [".##.", "#..#", "#..#", "#..#", "#..#", ".##."]),
    "A": (11, [".##.", "#..#", "#..#", "####", "#..#", "#..#"]),
    "I": (19, ["###", ".#.", ".#.", ".#.", ".#.", "###"]),
    "D": (27, ["###.", "#..#", "#..#", "#..#", "#..#", "###."]),
    "V": (34, ["#..#", "#..#", "#..#", "#..#", "#..#", ".##."]),
}


def clutter_strip(svg, x, y, h, letters, lit=None):
    """A strip of the clutter bar, 8 wide: a dark line, the display's dots, and its letters
    (`letters` maps a letter to its top row); the `lit` letter is drawn lit."""
    svg.rect(x, y, 8, h, "#000000")
    svg.rect(x + 1, y, 7, 1, "#000008")
    svg.rect(x, y, 1, h, "#181829")
    lcd_dots(svg, x + 1, y + 2, 7, h - 3, x0=x, y0=y)
    svg.rect(x + 3, y + h - 1, 5, 1, "#60606a")
    for letter, top in letters.items():
        rows = CLUTTER_LETTERS[letter][1]
        pixel_art(svg, rows, x + 4, y + top, "#70818f" if letter == lit else "#333c49")


def third_thumbs(svg, x, y, dark, shade):
    """A windowshade slider's thumbs, 3 x 7 each, for the left, middle and right thirds of
    its travel."""
    gold, rim = "#ecce7a", "#a3946a"
    for i in range(3):
        tx = x + 3 * i
        svg.rect(tx, y, 3, 7, gold)
        left, right = (rim, dark) if i == 0 else ((rim, rim) if i == 1 else (dark, rim))
        for row in (0, 6):
            svg.px(tx, y + row, left)
            svg.px(tx + 2, y + row, right)
        if i != 1:
            sx = tx + 2 if i == 0 else tx
            svg.rect(sx, y + 1, 1, 5, rim)
            svg.px(sx, y + 1, shade)
            svg.px(sx, y + 5, shade)


def sheet_titlebar():
    svg = Svg(344, 87)
    svg.rect(0, 0, 344, 87, "#2f2f44")
    svg.comment("title bars: active, inactive")
    title_bar(svg, 0, True)
    svg.rect(27, 14, 275, 1, "#0d0d14")
    title_bar(svg, 15, False)
    svg.comment("windowshade bars: active, inactive (they share a row)")
    shade_bar(svg, 29, True)
    shade_bar(svg, 42, False)
    svg.comment("spare title bars (Winamp's easter egg), without lettering")
    title_bar(svg, 57, True, text=None)
    title_bar(svg, 72, False, text=None)
    svg.comment("the buttons on their own: normal in row 0, pressed in rows 9 and 18")
    bar_background(svg, 0, 0, 27)
    svg.rect(0, 0, 27, 9, "#14141f")
    options_icon(svg, 0, 0, "#ecce7a", "#34302c", "#9a875d")
    dome(svg, 10, 1, MIN_DOME[True])
    close_box(svg, 19, 1, True)
    pressed_cell(svg, 0, 9)
    options_icon(svg, 0, 9, "#ecce7a", "#212835", "#9a875d")
    pressed_cell(svg, 9, 9)
    dome(svg, 10, 10, MIN_DOME["pressed"])
    pressed_cell(svg, 18, 9)
    close_box(svg, 19, 10, True, pressed=True)
    svg.rect(0, 18, 18, 18, "#30304c")
    dome(svg, 1, 19, SHADE_DOME[True])
    pressed_cell(svg, 9, 18)
    dome(svg, 10, 19, SHADE_DOME["pressed"])
    stops, rim = DISC[True]
    disc(svg, 1, 28, stops, rim)
    pressed_cell(svg, 9, 27)
    stops, rim = DISC["pressed"]
    disc(svg, 10, 28, stops, rim)
    svg.comment("the windowshade position bar and its thumb in each third")
    svg.rect(0, 36, 17, 7, "#5a6b7b")
    svg.rect(0, 36, 16, 6, "#000010")
    svg.rect(1, 37, 15, 5, "#30304b")
    svg.rect(1, 37, 15, 1, "#313142")
    third_thumbs(svg, 17, 36, "#3d3f4c", "#685935")
    svg.comment("the clutter bar, then each of its letters pressed")
    svg.rect(302, 0, 1, 87, "#0a0d16")
    letters = {k: v[0] for k, v in CLUTTER_LETTERS.items()}
    clutter_strip(svg, 304, 0, 43, letters)
    clutter_strip(svg, 312, 0, 43, {})
    pressed_rows = {"O": 47, "A": 55, "I": 63, "D": 71, "V": 78}
    for i, lit in enumerate("OAIDV"):
        clutter_strip(svg, 304 + 8 * i, 44, 43, {k: v - 44 for k, v in pressed_rows.items()},
                      lit)
    return svg


# --- main: the main window ---------------------------------------------------------------------

# The lit edges of grooves and panels: grey, brighter where the background is.
EDGE_STOPS = [(0, "#5b5b62"), (20, "#5f5f68"), (50, "#62626d"), (80, "#666675"), (110, "#6c6c7e"),
              (140, "#717186"), (170, "#74748b"), (200, "#75758c"), (225, "#73738a"),
              (250, "#6d6d7f"), (275, "#666675")]


def edge_fill(svg):
    """The lit-edge grey, in main-window coordinates."""
    return svg.linear(0, 0, 275, 0, [(x / 275, c) for x, c in EDGE_STOPS])


def shadow(svg, x, y, w, h, depth=0.4):
    """Darkens the background, for the shaded side of a groove."""
    svg.rect(x, y, w, h, "#000000", f' opacity="{depth}"')


def sunken(svg, x, y, w, h, fill="#000000"):
    """A panel set into the surface, (x, y, w, h) inside: shadow above and to the left, a lit
    edge below and to the right."""
    shadow(svg, x - 1, y - 1, w + 1, 1, 0.45)
    shadow(svg, x - 1, y, 1, w and h + 1, 0.45)
    svg.rect(x, y, w, h, fill)
    edge = edge_fill(svg)
    svg.rect(x, y + h, w + 1, 1, edge)
    svg.rect(x + w, y, 1, h + 1, edge)


# Lettering beside the bit rate and sample rate boxes.
KBPS = ["#...#..........",
        "#...#..........",
        "#.#.###..##..##",
        "##..#..#.#.#.#.",
        "##..#..#.#.#..#",
        "#.#.###..##..##",
        ".........#....."]
KHZ = ["#............",
       "#...#..#.....",
       "#.#.#..#.####",
       "##..####...#.",
       "##..#..#..#..",
       "#.#.#..#..###"]

# NeoAmp's icon (resources/icons/neoamp.svg): three slanted bars.
ICON_BAR = ("M480,470 L1305,152 C1370,128 1410,190 1410,250 C1410,320 1380,395 1310,412 "
            "L470,738 C420,752 382,700 382,640 C380,560 420,492 480,470 Z")


def neoamp_icon(svg, cx, cy, size):
    """NeoAmp's icon, `size` high, centred on (cx, cy), over a faint copy of itself a little
    below and to the right."""
    s = size / 860
    tx, ty = cx - 510.5 * s, cy - 507 * s
    clip = svg.clip(f'<path d="{ICON_BAR}"/>')
    bar = svg.new_id("bar")
    svg.defs.append(
        f'<g id="{bar}"><path fill="#FCB316" d="{ICON_BAR}"/>'
        f'<polygon fill="#F26B1D" clip-path="url(#{clip})" '
        'points="300,708 1450,348 1450,800 300,800"/></g>'
    )
    inner = (f'<use href="#{bar}"/><use href="#{bar}" transform="translate(-95 352)"/>'
             f'<use href="#{bar}" transform="translate(-185 710)"/>')
    body = f'<g transform="translate(512 512) scale(0.66) translate(-805 -805)">{inner}</g>'
    svg.add(f'<g transform="translate({num(tx + 0.6)} {num(ty + 0.7)}) scale({s:.6f})" '
            f'opacity="0.55"><g>{body}</g></g>')
    svg.add(f'<g transform="translate({num(tx)} {num(ty)}) scale({s:.6f})">{body}</g>')


def sheet_main():
    svg = Svg(275, 116)
    svg.rect(0, 0, 275, 116, main_bg_fill(svg))
    edge = edge_fill(svg)
    svg.comment("the window's rim: lit on the top and left, dark on the right and bottom")
    shadow(svg, 0, 0, 275, 1, 0.3)
    shadow(svg, 274, 0, 1, 116, 0.2)
    shadow(svg, 0, 115, 275, 1, 0.3)
    svg.rect(0, 0, 1, 116, "#0d0d14")
    svg.rect(1, 1, 273, 1, edge)
    svg.rect(1, 1, 1, 114, "#5c5c63")
    svg.comment("the groove around the panel")
    shadow(svg, 4, 12, 267, 2)
    shadow(svg, 4, 12, 2, 100)
    shadow(svg, 269, 14, 2, 98)
    shadow(svg, 6, 110, 263, 2)
    svg.rect(6, 14, 263, 1, edge)
    svg.rect(6, 14, 1, 96, "#5c5c63")
    svg.rect(271, 12, 1, 101, edge)
    svg.rect(5, 112, 267, 1, edge)
    svg.comment("the display")
    sunken(svg, 12, 23, 91, 41)
    lcd_dots(svg, 12, 24, 91, 39)
    for y in (30, 34):
        svg.rect(72, y, 3, 1, "#03f605")
    blue = "#5e95ea"
    for y in range(44, 61, 4):
        svg.px(22, y, blue)
    for x in range(26, 99, 4):
        svg.px(x, 60, blue)
    svg.comment("the song title, bit rate and sample rate")
    sunken(svg, 109, 24, 157, 12)
    sunken(svg, 109, 41, 18, 10)
    sunken(svg, 154, 41, 13, 10)
    pixel_art(svg, KBPS, 131, 43, "#ffffff")
    pixel_art(svg, KHZ, 171, 43, "#ffffff")
    svg.comment("the position bar's groove (the position bar is drawn over it)")
    shadow(svg, 16, 72, 248, 2, 0.45)
    shadow(svg, 16, 74, 248, 7, 0.15)
    svg.rect(18, 81, 247, 1, edge)
    svg.rect(264, 72, 1, 10, edge)
    svg.comment("NeoAmp's icon where Winamp has its logo")
    neoamp_icon(svg, 258.5, 97.5, 16)
    return svg


# --- eqmain: the equalizer ---------------------------------------------------------------------

DB_LABELS = {
    (45, 37): ["....##..##.....#.#..",
               ".#...#....#....#.#..",
               "###..#..###..###.###",
               ".#...#..#....#.#.#.#",
               "....###.###..###.###"],
    (46, 66): ["......##.....#.#..",
               ".#...#..#....#.#..",
               "###..#..#..###.###",
               ".#...#..#..#.#.#.#",
               "......##...###.###"],
    (45, 96): ["....##..##.....#.#..",
               ".....#....#....#.#..",
               "###..#..###..###.###",
               ".....#..#....#.#.#.#",
               "....###.###..###.###"],
}
BAND_LABELS = {
    (18, 103): ["##..##..###..#..#.#.##.",
                "#.#.#.#.#...#.#.###.#.#",
                "##..##..##..###.###.##.",
                "#...#.#.#...#.#.#.#.#..",
                "#...#.#.###.#.#.#.#.#.."],
    (81, 102): [".##...##.", "#....#..#", "###..#..#", "#..#.#..#", "#..#.#..#", ".##...##."],
    (99, 102): ["#.###..##.", "#...#.#..#", "#..#..#..#", "#..#..#..#", "#.#...#..#",
                "#.#....##."],
    (117, 102): ["##..#..##.", "..#.#.#..#", "##..#.#..#", "..#.#.#..#", "..#.#.#..#",
                 "##..#..##."],
    (133, 102): [".##...##...##.", "#....#..#.#..#", "###..#..#.#..#", "#..#.#..#.#..#",
                 "#..#.#..#.#..#", ".##...##...##."],
    (155, 102): ["#.#.#", "#.#.#", "#.##.", "#.##.", "#.#.#", "#.#.#"],
    (172, 102): ["##..#.#", "..#.#.#", "##..##.", "..#.##.", "..#.#.#", "##..#.#"],
    (190, 102): [".##..#.#", "#....#.#", "###..##.", "#..#.##.", "#..#.#.#", ".##..#.#"],
    (208, 102): ["#.##..#.#", "#...#.#.#", "#...#.##.", "#..#..##.", "#.#...#.#", "#.###.#.#"],
    (226, 102): ["#...#.#.#", "#..##.#.#", "#.#.#.##.", "#.###.##.", "#...#.#.#", "#...#.#.#"],
    (244, 102): ["#..##..#.#", "#.#....#.#", "#.###..##.", "#.#..#.##.", "#.#..#.#.#",
                 "#..##..#.#"],
}
# The 5-pixel capitals on the equalizer's buttons.
EQ_CAPS = pixel_font({
    "ONAUTPRES": [
        ".##. #..# .##. #..# ### ###  ###  .### ###",
        "#..# ##.# #..# #..# .#. #..# #..# #... #..",
        "#..# #### #### #..# .#. ###. ###. #### .##",
        "#..# #.## #..# #..# .#. #... #..# #... ...#",
        ".##. #..# #..# .##. .#. #... #..# .### ###",
    ],
})
# Each slider frame's bar: its outer and inner columns, quiet (green) to loud (red).
EQ_BARS = [
    ("#2a9a16", "#2a9a16"), ("#2a9a16", "#2a9a16"), ("#5ab02c", "#5ab02c"), ("#71cd34", "#71cd34"),
    ("#71cd34", "#71cd34"), ("#71cd34", "#89e230"), ("#90c73d", "#89e230"), ("#90c73d", "#a4e238"),
    ("#90c73d", "#a4e238"), ("#afc338", "#c4db32"), ("#afc338", "#c4db32"), ("#afc338", "#c4db32"),
    ("#afc338", "#c4db32"), ("#afc338", "#c4db32"), ("#c6b335", "#e0cd30"), ("#c6b335", "#e0cd30"),
    ("#c6b335", "#e0cd30"), ("#c6b335", "#e0cd30"), ("#c6b335", "#e0b228"), ("#c58730", "#e09228"),
    ("#c58730", "#e09228"), ("#c3702a", "#dc771f"), ("#b0711d", "#c6780f"), ("#c3702a", "#dc771f"),
    ("#c5542a", "#e0541e"), ("#c5542a", "#e0541e"), ("#d3221b", "#d3221b"), ("#d3221b", "#d3221b"),
]
EQ_DASHES = [(16, 5), (35, 6), (73, 7)] + [(91 + 18 * i, 7) for i in range(9)] + [(253, 5)]


def glow(svg, x0, x1, y0, y1, fade, colour):
    """A soft-edged patch of light over (x0, y0)-(x1, y1), fading out over `fade` units."""
    fx = fade / (x1 - x0)
    fy = fade / (y1 - y0)
    gid = svg.new_id("g")
    svg.defs.append(
        f'<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" x1="{x0}" y1="0" '
        f'x2="{x1}" y2="0"><stop offset="0" stop-color="{colour}" stop-opacity="0"/>'
        f'<stop offset="{num(fx)}" stop-color="{colour}"/>'
        f'<stop offset="{num(1 - fx)}" stop-color="{colour}"/>'
        f'<stop offset="1" stop-color="{colour}" stop-opacity="0"/></linearGradient>'
    )
    mid = svg.new_id("g")
    svg.defs.append(
        f'<linearGradient id="{mid}" gradientUnits="userSpaceOnUse" x1="0" y1="{y0}" '
        f'x2="0" y2="{y1}"><stop offset="0" stop-color="#fff" stop-opacity="0"/>'
        f'<stop offset="{num(fy)}" stop-color="#fff"/>'
        f'<stop offset="{num(1 - fy)}" stop-color="#fff"/>'
        f'<stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>'
    )
    mask = svg.new_id("m")
    svg.defs.append(f'<mask id="{mask}" maskUnits="userSpaceOnUse" x="{x0}" y="{y0}" '
                    f'width="{x1 - x0}" height="{y1 - y0}"><rect x="{x0}" y="{y0}" '
                    f'width="{x1 - x0}" height="{y1 - y0}" fill="url(#{mid})"/></mask>')
    svg.add(f'<rect x="{x0}" y="{y0}" width="{x1 - x0}" height="{y1 - y0}" '
            f'fill="url(#{gid})" mask="url(#{mask})"/>')


def window_frame(svg, edge):
    """The rim and groove of the main and equalizer windows, 275 x 116."""
    shadow(svg, 0, 0, 275, 1, 0.3)
    shadow(svg, 274, 0, 1, 116, 0.2)
    shadow(svg, 0, 115, 275, 1, 0.3)
    svg.rect(0, 0, 1, 116, "#0d0d14")
    svg.rect(1, 1, 273, 1, edge)
    svg.rect(1, 1, 1, 114, "#5c5c63")
    shadow(svg, 4, 12, 267, 2)
    shadow(svg, 4, 12, 2, 100)
    shadow(svg, 269, 14, 2, 98)
    shadow(svg, 6, 110, 263, 2)
    svg.rect(6, 14, 263, 1, edge)
    svg.rect(6, 14, 1, 96, "#5c5c63")
    svg.rect(271, 12, 1, 101, edge)
    svg.rect(5, 112, 267, 1, edge)


def eq_button(svg, x, y, w, label, lit, pressed, text_dx, font=None):
    """An ON or AUTO button (`w` wide, 12 high) with its lamp, or PRESETS without one."""
    font = font or EQ_CAPS
    if pressed:
        pressed_button(svg, x, y, w, 12)
        ox, oy = x + 2, y + 2
    else:
        raised_button(svg, x, y, w, 12)
        ox, oy = x, y
    if lit is not None:
        led(svg, ox + 3, oy + 3, 5, 4, lit, pressed)
    draw_text(svg, font, label, ox + text_dx, oy + 3, LABEL_PRESSED if pressed else LABEL)


def eq_thumb(svg, x, y, pressed):
    """An equalizer slider's thumb, 11 x 11."""
    svg.rect(x, y, 11, 11, "#000000")
    for cx, cy in [(0, 0), (10, 0), (0, 10), (10, 10)]:
        svg.px(x + cx, y + cy, "#292942")
    face = "#000000" if pressed else "#adbdc6"
    svg.rect(x + 1, y + 1, 9, 9, "#4a5a6b")
    svg.rect(x + 1, y + 1, 8, 8, "#dde6e9")
    svg.rect(x + 2, y + 2, 7, 7, face)
    if not pressed:
        svg.rect(x + 8, y + 2, 1, 7, "#7b8494")
        svg.rect(x + 2, y + 8, 7, 1, "#7b8494")
        svg.px(x + 2, y + 8, "#8b9ba7")
    grip = "#f9feff" if pressed else "#080810"
    svg.rect(x + 4, y + 4, 3, 1, grip)
    svg.rect(x + 4, y + 6, 3, 1, grip)


def eq_slider_frame(svg, x, y, bar):
    """One of the 28 slider frames, 14 x 63: the bar in its groove."""
    bg = svg.linear(0, y, 0, y + 63, [(0, "#2e2e48"), (0.12, "#343453"), (0.3, "#373757"),
                                      (0.7, "#383858"), (0.88, "#363655"), (1, "#31314d")])
    svg.rect(x, y, 14, 63, bg)
    svg.add(f'<rect x="{x + 3}" y="{y + 1}" width="6" height="61" rx="2.5" fill="#8d8ea0"/>')
    svg.add(f'<rect x="{x + 3}" y="{y + 1}" width="5" height="60" rx="2.5" fill="#13131e"/>')
    fill = svg.linear(x + 4, 0, x + 8, 0, [(0.125, bar[0]), (0.375, bar[1]), (0.625, bar[1]),
                                          (0.875, bar[0])])
    svg.add(f'<rect x="{x + 4}" y="{y + 2}" width="4" height="59" rx="2" fill="{fill}"/>')
    svg.add(f'<rect x="{x + 4}" y="{y + 2}" width="4" height="3" rx="1.5" fill="#000000" '
            'opacity="0.45"/>')


def eq_title_bar(svg, y, active):
    """The equalizer's title bar, 275 x 14 at (0, y)."""
    bar_background(svg, 0, y, 275, mx=0)
    text = "NEOAMP EQUALIZER"
    room = 194 - 88 - 6
    sx = min(1.0, room / title_width(text))
    w = title_width(text, sx=sx)
    tx = round(141 - w / 2, 1)
    stripes(svg, 8, y + 4, round(tx - 3 - 8), active)
    right = round(tx + w + 3)
    stripes(svg, right, y + 4, 251 - right, active)
    title_text(svg, text, tx, y + 4.5, "#ffffff" if active else "#909090", sx=sx)
    dome(svg, 255, y + 4, SHADE_DOME[active])
    close_box(svg, 265, y + 4, active)


def sheet_eqmain():
    svg = Svg(275, 315)
    svg.rect(0, 0, 275, 315, "#00c6ff")
    svg.comment("the window, 275 x 116")
    svg.rect(0, 0, 275, 116, main_bg_fill(svg))
    cid = svg.clip('<rect x="7" y="15" width="262" height="95"/>')
    svg.open(clip=cid)
    glow(svg, 12, 41, 22, 110, 8, "#39395b")
    glow(svg, 66, 262, 22, 110, 9, "#39395a")
    svg.close()
    edge = edge_fill(svg)
    window_frame(svg, edge)
    svg.comment("the graph's grid (the graph is drawn over it)")
    svg.rect(86, 26, 113, 1, edge)
    for i in range(10):
        svg.rect(88 + 12 * i, 17, 1, 19, edge)
    svg.comment("level marks and labels")
    for y in (39, 68, 98):
        for x, w in EQ_DASHES:
            svg.rect(x, y, w, 1, "#d7d7dd")
    for (x, y), rows in DB_LABELS.items():
        pixel_art(svg, rows, x, y, "#d4a80c")
    for (x, y), rows in BAND_LABELS.items():
        pixel_art(svg, rows, x, y, "#d7d7dd")
    svg.comment("the close button, normal and pressed")
    svg.rect(0, 116, 9, 9, "#292942")
    close_box(svg, 1, 117, True)
    pressed_cell(svg, 0, 125)
    close_box(svg, 1, 126, True, pressed=True)
    svg.comment("ON and AUTO: off, on, off pressed, on pressed")
    for x, lit, pressed in [(10, False, False), (69, True, False), (128, False, True),
                            (187, True, True)]:
        svg.rect(x + 25, 119, 1, 12, "#000000")
        eq_button(svg, x, 119, 25 if not pressed else 26, "ON", lit, pressed, 10)
        eq_button(svg, x + 26, 119, 32, "AUTO", lit, pressed, 10)
    svg.comment("title bars: active, inactive")
    eq_title_bar(svg, 134, True)
    eq_title_bar(svg, 149, False)
    svg.comment("slider thumbs, the 28 slider frames, and PRESETS")
    eq_thumb(svg, 0, 164, False)
    eq_thumb(svg, 0, 176, True)
    for i, bar in enumerate(EQ_BARS):
        x, y = 13 + (i % 14) * 15, 164 + (i // 14) * 65
        eq_slider_frame(svg, x, y, bar)
        svg.rect(x + 14, y, 1, 63, "#cec6d6")
    eq_button(svg, 224, 164, 44, "PRESETS", None, False, 5)
    eq_button(svg, 224, 176, 44, "PRESETS", None, True, 5)
    svg.comment("the graph's background, its colour scale and the preamp line")
    gbg = svg.linear(0, 0, 113, 0, [(0, "#2c2b43"), (0.25, "#30304b"), (0.5, "#363554"),
                                    (0.75, "#39395a"), (1, "#383858")])
    svg.rect(0, 294, 113, 19, gbg)
    gedge = svg.linear(0, 0, 113, 0, [(0, "#6c6c7e"), (0.4, "#717186"), (0.75, "#75758c"),
                                      (1, "#73738a")])
    for i in range(10):
        svg.rect(2 + 12 * i, 294, 1, 19, gedge)
    scale = ["#d3221b", "#ef5221", "#ef7b21", "#e09228", "#e09228", "#e09228", "#e0b228",
             "#efdc31", "#efdc31", "#efdc31", "#d2eb35", "#d2eb35", "#a4e238", "#a4e238",
             "#89e230", "#71cd34", "#5ab02c", "#2a9a16", "#2a9a16"]
    for i, c in enumerate(scale):
        svg.rect(115, 294 + i, 1, 1, c)
    svg.rect(0, 314, 113, 1, "#bacbdd")
    return svg


# --- eq_ex: the equalizer's windowshade bar ---------------------------------------------------


def eq_shade_bar(svg, y, active):
    """The equalizer in windowshade mode, 275 x 14 at (0, y): stripes, the volume and balance
    slots, and the shade and close buttons."""
    bar_background(svg, 0, y, 275, mx=0)
    svg.rect(0, y + 13, 275, 1, "#000000")
    stripes(svg, 7, y + 4, 51, active)
    stripes(svg, 210, y + 4, 42, active)
    for x, w in [(61, 97), (164, 43)]:
        shadow(svg, x, y + 4, w, 1, 0.35)
        shadow(svg, x, y + 4, 1, 6, 0.35)
        edge = edge_fill(svg)
        svg.rect(x, y + 10, w, 1, edge)
        svg.rect(x + w - 1, y + 4, 1, 7, edge)
    stops, rim = DISC[active]
    disc(svg, 255, y + 4, stops, rim)
    close_box(svg, 265, y + 4, active)


def sheet_eq_ex():
    svg = Svg(275, 82)
    svg.rect(0, 0, 275, 82, "#00c6ff")
    svg.comment("windowshade bars: active, inactive")
    eq_shade_bar(svg, 0, True)
    eq_shade_bar(svg, 15, False)
    svg.comment("volume and balance thumbs, for each third of their slots")
    svg.rect(0, 29, 22, 9, "#00c6ff")
    third_thumbs(svg, 1, 30, "#39404d", "#635e3b")
    third_thumbs(svg, 11, 30, "#39404d", "#635e3b")
    svg.comment("shade (normal window) pressed; close; unshade and close pressed")
    gold = "#b49163"
    svg.rect(1, 38, 9, 9, gold)
    dome(svg, 2, 39, SHADE_DOME["pressed"])
    main_bg_patch(svg, 11, 38, 9, 9, 264, 3)
    close_box(svg, 12, 39, True)
    svg.rect(1, 47, 9, 9, gold)
    stops, rim = DISC["pressed"]
    disc(svg, 2, 48, stops, rim)
    svg.rect(11, 47, 9, 9, gold)
    close_box(svg, 12, 48, True, pressed=True)
    return svg


# --- pledit: the playlist editor --------------------------------------------------------------

PL_DARK = "#292940"
PL_LIGHT = "#3e3c60"
PL_EDGE_DARK = "#1d1d2d"
PL_EDGE_LIGHT = "#6a6a7a"
# The title piece's background: light at its ends, dark behind the lettering.
PL_TITLE_STOPS = [(0, "#3e3c60"), (4, "#3e3c60"), (8, "#3d3b5e"), (12, "#3b3a5c"),
                  (16, "#393859"), (20, "#373556"), (24, "#343351"), (28, "#32314d"),
                  (32, "#302f4b"), (36, "#2d2d46"), (40, "#2c2b43"), (44, "#2a2a41"),
                  (48, "#292940"), (52, "#292940"), (56, "#2a2a41"), (60, "#2b2a43"),
                  (64, "#2d2d46"), (68, "#2e2e49"), (72, "#31304c"), (76, "#32324f"),
                  (80, "#363554"), (84, "#383757"), (88, "#3b3a5c"), (92, "#3d3b5e"),
                  (96, "#3e3c60"), (100, "#3e3c60")]
# The bottom pieces' backgrounds, dark towards the window's corners.
PL_BOTTOM_LEFT_STOPS = [(0, "#141420"), (8, "#141520"), (20, "#181926"), (32, "#1c1d2c"),
                        (44, "#212134"), (56, "#26263b"), (68, "#2a2a42"), (80, "#2e2e49"),
                        (92, "#32324f"), (104, "#363554"), (116, "#383858"), (125, "#383858")]
PL_BOTTOM_RIGHT_STOPS = [(0, "#383859"), (12, "#39395a"), (24, "#383858"), (36, "#363655"),
                         (48, "#32324f"), (60, "#2e2e49"), (72, "#2a2a42"), (84, "#26253b"),
                         (96, "#201f32"), (108, "#1c1d2c"), (114, "#201f32"), (120, "#242439"),
                         (126, "#28273e"), (132, "#292940"), (150, "#292940")]

PL_MENU_ITEMS = {0: 3, 54: 4, 104: 3, 154: 3, 204: 3}
MENU_TEXT = {
    (0, 0): (4, 3, ['.##..###..###.', '#..#.#..#.#..#', '####.#..#.#..#', '#..#.#..#.#..#', '#..#.###..###.', '..............', '.#.#.##..#....', '.#.#.#.#.#....', '.#.#.##..#....', '.#.#.#.#.#....', '.###.#.#.###..']),
    (0, 1): (4, 3, ['.##..###..###.', '#..#.#..#.#..#', '####.#..#.#..#', '#..#.#..#.#..#', '#..#.###..###.', '..............', '.##..###.##...', '.#.#..#..#.#..', '.#.#..#..##...', '.#.#..#..#.#..', '.##..###.#.#..']),
    (0, 2): (3, 3, ['..##..###..###.', '.#..#.#..#.#..#', '.####.#..#.#..#', '.#..#.#..#.#..#', '.#..#.###..###.', '...............', '###.###.#...###', '#....#..#...#..', '##...#..#...##.', '#....#..#...#..', '#...###.###.###']),
    (54, 0): (4, 3, ['##..###.#...#', '#.#.#...##.##', '##..##..#####', '#.#.#...#.#.#', '#.#.###.#...#', '.............', '..#..#...#...', '.#.#.#...#...', '.###.#...#...', '.#.#.#...#...', '.#.#.###.###.']),
    (54, 1): (3, 6, ['.##.##..###.##.', '#...#.#.#.#.#.#', '#...##..#.#.##.', '#...#.#.#.#.#..', '.##.#.#.###.#..']),
    (54, 2): (4, 3, ['##..###.#...#', '#.#.#...##.##', '##..##..#####', '#.#.#...#.#.#', '#.#.###.#...#', '.............', '..##.###.#...', '.#...#...#...', '..#..##..#...', '...#.#...#...', '.##..###.###.']),
    (54, 3): (3, 3, ['.##..###.#...#.', '.#.#.#...##.##.', '.##..##..#####.', '.#.#.#...#.#.#.', '.#.#.###.#...#.', '...............', '#...#.#..##..##', '##.##...#...#..', '#####.#..#..#..', '#.#.#.#...#.#..', '#...#.#.##...##']),
    (104, 0): (4, 3, ['###.#..#.#..#', '.#..##.#.#..#', '.#..####.#..#', '.#..#.##..##.', '###.#..#..##.', '.............', '...##.###.#..', '..#...#...#..', '...#..##..#..', '....#.#...#..', '..##..###.###']),
    (104, 1): (3, 3, ['...##.###.#....', '..#...#...#....', '...#..##..#....', '....#.#...#....', '..##..###.###..', '...............', '###.###.##...#.', '..#.#...#.#.#.#', '.#..##..##..#.#', '#...#...#.#.#.#', '###.###.#.#..#.']),
    (104, 2): (5, 3, ['.##.###.#..', '#...#...#..', '.#..##..#..', '..#.#...#..', '##..###.###', '...........', '.#..#...#..', '#.#.#...#..', '###.#...#..', '#.#.#...#..', '#.#.###.###']),
    (154, 0): (3, 3, ['.##.###.##..###', '#...#.#.#.#..#.', '.#..#.#.##...#.', '..#.#.#.#.#..#.', '##..###.#.#..#.', '...............', '#...###..##.###', '#....#..#....#.', '#....#...#...#.', '#....#....#..#.', '###.###.##...#.']),
    (154, 1): (3, 3, ['###.###.#...###', '#....#..#...#..', '##...#..#...##.', '#....#..#...#..', '#...###.###.###', '...............', '..###.#..#.###.', '...#..##.#.#...', '...#..####.##..', '...#..#.##.#...', '..###.#..#.#...']),
    (154, 2): (3, 3, ['#...#.#..##..##', '##.##...#...#..', '#####.#..#..#..', '#.#.#.#...#.#..', '#...#.#.##...##', '...............', '###.##..###..##', '#.#.#.#..#..#..', '#.#.##...#...#.', '#.#.#....#....#', '###.#....#..##.']),
    (204, 0): (3, 3, ['#..#.###.#...#.', '##.#.#...#.#.#.', '####.##..#####.', '#.##.#...##.##.', '#..#.###.#...#.', '...............', '#...###..##.###', '#....#..#....#.', '#....#...#...#.', '#....#....#..#.', '###.###.##...#.']),
    (204, 1): (3, 3, ['.##.###.#.#.###', '#...#.#.#.#.#..', '.#..###.#.#.##.', '..#.#.#.###.#..', '##..#.#..#..###', '...............', '#...###..##.###', '#....#..#....#.', '#....#...#...#.', '#....#....#..#.', '###.###.##...#.']),
    (204, 2): (3, 3, ['#...###.###.##.', '#...#.#.#.#.#.#', '#...#.#.###.#.#', '#...#.#.#.#.#.#', '###.###.#.#.##.', '...............', '#...###..##.###', '#....#..#....#.', '#....#...#...#.', '#....#....#..#.', '###.###.##...#.']),
}
BUTTON_TEXT = {
    'ADD': (3, 6, ['.##..###..###.', '#..#.#..#.#..#', '####.#..#.#..#', '#..#.#..#.#..#', '#..#.###..###.']),
    'REM': (4, 6, ['##..###.#...#', '#.#.#...##.##', '##..##..#####', '#.#.#...#.#.#', '#.#.###.#...#']),
    'SEL': (5, 6, ['.##.###.#..', '#...#...#..', '.#..##..#..', '..#.#...#..', '##..###.###']),
    'MISC': (3, 6, ['#...#.#..##..##', '##.##...#...#..', '#####.#..#..#..', '#.#.#.#...#.#..', '#...#.#.##...##']),
    'LIST': (3, 3, ['#...###..##.###', '#....#..#....#.', '#....#...#...#.', '#....#....#..#.', '###.###.##...#.', '...............', '###.##..###..##', '#.#.#.#..#..#..', '#.#.##...#...#.', '#.#.#....#....#', '###.#....#..##.']),
}


def pl_piece(svg, x, y, w, h, stops):
    """A playlist piece's background: a horizontal gradient from (offset, colour) stops
    measured from the piece's left edge."""
    fill = svg.linear(x, 0, x + stops[-1][0], 0, [(o / stops[-1][0], c) for o, c in stops])
    svg.rect(x, y, w, h, fill)


def tint(svg, x, y, w, h, light):
    """Lightens or darkens a strip of a playlist piece, for its grooves and edges."""
    svg.rect(x, y, w, h, "#ffffff" if light else "#000000", ' opacity="0.3"')


def pl_stripes(svg, x, y, w, active, left_cap, right_cap):
    """Gold stripes in a playlist piece; an end without a cap runs off the piece's edge, to
    join the next piece's."""
    c = GOLD[active]
    cid = svg.clip(f'<rect x="{x}" y="{y}" width="{w}" height="7"/>')
    svg.open(clip=cid)
    x0 = x - (1 if left_cap else 3)
    x1 = x + w + (1 if right_cap else 3)
    svg.rect(x0, y, x1 - x0, 7, c[0])
    svg.rect(x0, y + 6, x1 - x0, 1, c[6])
    ix0, ix1 = (x if left_cap else x - 3), (x + w if right_cap else x + w + 3)
    inner = svg.clip(f'<rect x="{ix0}" y="{y + 1}" width="{ix1 - ix0}" height="5" rx="1.2"/>')
    svg.open(clip=inner)
    for i in range(5):
        svg.rect(ix0, y + 1 + i, ix1 - ix0, 1, c[1 + i])
    svg.close()
    svg.close()


def pl_top(svg, y, active):
    """The playlist's four top pieces, 20 high, at row `y`."""
    # top left (0..24)
    pl_piece(svg, 0, y, 25, 20, [(0, PL_DARK), (2, PL_DARK), (25, PL_LIGHT)])
    # title (26..125)
    pl_piece(svg, 26, y, 100, 20, PL_TITLE_STOPS)
    # fill (127..151)
    svg.rect(127, y, 25, 20, PL_LIGHT)
    # top right (153..177)
    pl_piece(svg, 153, y, 25, 20, [(0, PL_LIGHT), (24, "#2a2a41"), (25, "#2a2a41")])
    for x, w in [(0, 25), (26, 100), (127, 25), (153, 25)]:
        tint(svg, x, y, w, 1, False)
        tint(svg, x, y + 12, w, 2, False)
        tint(svg, x, y + 19, w, 1, False)
        tint(svg, x, y + 1, w, 1, True)
        tint(svg, x, y + 14, w, 1, True)
    # The left edge: rim, then the groove.
    svg.rect(0, y, 1, 20, PL_EDGE_DARK)
    svg.rect(1, y + 1, 1, 19, PL_EDGE_LIGHT)
    svg.rect(2, y + 14, 4, 1, PL_DARK)
    svg.rect(4, y + 12, 2, 8, PL_EDGE_DARK)
    svg.rect(6, y + 14, 1, 6, PL_EDGE_LIGHT)
    svg.rect(6, y + 19, 4, 1, PL_DARK)
    # The right edge.
    svg.rect(172, y + 14, 6, 1, "#2a2a41")
    svg.rect(172, y + 12, 2, 8, PL_EDGE_DARK)
    svg.rect(174, y + 12, 1, 8, PL_EDGE_LIGHT)
    svg.rect(175, y + 12, 2, 8, PL_DARK)
    svg.rect(177, y, 1, 20, PL_EDGE_DARK)
    # Stripes, the title, and the shade and close buttons.
    pl_stripes(svg, 7, y + 4, 18, active, True, False)
    pl_stripes(svg, 26, y + 4, 3, active, False, True)
    pl_stripes(svg, 123, y + 4, 3, active, True, False)
    pl_stripes(svg, 127, y + 4, 25, active, False, False)
    pl_stripes(svg, 153, y + 4, 2, active, False, True)
    text = "NEOAMP PLAYLIST"
    room = 121 - 33
    sx = min(1.0, room / title_width(text, gap=1.0))
    w = title_width(text, gap=1.0, sx=sx)
    title_text(svg, text, round(77 - w / 2, 1), y + 4.5, "#ffffff" if active else "#909090",
               gap=1.0, sx=sx)
    dome(svg, 158, y + 4, SHADE_DOME[active])
    close_box(svg, 168, y + 4, active)


def scroll_thumb(svg, x, y, pressed):
    """The playlist's scroll bar thumb, 8 x 18: a gold bar with a slot down it."""
    if pressed:
        rim, slot_edge, hi, slot, mid, shadow_ = ("#b0995e", "#6c5118", "#c3ae75", "#6c5118",
                                                  "#8d7138", "#200c00")
    else:
        rim, slot_edge, hi, slot, mid, shadow_ = ("#daca9c", "#8d7138", "#f4eac6", "#755c22",
                                                  "#b0995e", "#090202")
    svg.rect(x, y, 8, 18, mid)
    svg.rect(x, y, 8, 1, rim)
    svg.rect(x, y, 1, 17, rim)
    svg.rect(x + 7, y + 1, 1, 17, shadow_)
    svg.rect(x + 1, y + 17, 7, 1, shadow_)
    svg.px(x, y + 17, slot_edge)
    svg.rect(x + 1, y + 1, 5, 1, slot_edge)
    svg.rect(x + 1, y + 1, 1, 15, slot_edge)
    svg.rect(x + 2, y + 2, 1, 14, hi)
    svg.rect(x + 3, y + 3, 1, 12, slot)
    svg.rect(x + 5, y + 2, 1, 14, hi)
    svg.rect(x + 2, y + 15, 4, 1, hi)
    svg.rect(x + 1, y + 16, 1, 1, "#f2f2f4" if not pressed else hi)


def grip(svg, x, y, colour):
    """The windowshade playlist's width grip: four lines, each shorter than the last."""
    for i, (top, h) in enumerate([(1, 6), (2, 4), (3, 2), (3.5, 1)]):
        svg.add(f'<rect x="{x + 2 * i}" y="{num(y + top)}" width="1" height="{num(h)}" '
                f'fill="{colour}"/>')
        svg.add(f'<rect x="{x + 2 * i}" y="{num(y + top - 1)}" width="1" height="{num(h + 2)}" '
                f'fill="{colour}" opacity="0.5"/>')


def pl_shade(svg):
    """The windowshade playlist's pieces: left end, fill, and right end active and
    inactive."""
    for x, y, w in [(72, 42, 25), (72, 57, 25), (99, 42, 50), (99, 57, 50)]:
        svg.rect(x, y, w, 14, PL_DARK)
        svg.rect(x, y, w, 1, PL_EDGE_DARK)
        svg.rect(x, y + 1, w, 1, PL_EDGE_LIGHT)
        svg.rect(x, y + 13, w, 1, PL_EDGE_DARK)
    svg.rect(72, 42, 1, 14, PL_EDGE_DARK)
    svg.rect(73, 43, 1, 12, PL_EDGE_LIGHT)
    svg.rect(76, 45, 21, 9, "#000000")
    svg.rect(72, 60, 25, 9, "#000000")
    for y, active in [(42, True), (57, False)]:
        svg.rect(99, y + 3, 21, 9, "#000000")
        grip(svg, 121, y + 3, "#ffffff" if active else "#798a9e")
        stops, rim = DISC[active]
        disc(svg, 129, y + 4, stops, rim)
        close_box(svg, 139, y + 4, active)
        svg.rect(148, y, 1, 14, PL_EDGE_DARK)


def side_button(svg, x, y, art):
    """A playlist button, 22 x 18, with the lit sliver to its left."""
    svg.rect(x - 3, y + 2, 1, 14, "#8192a5")
    svg.rect(x - 2, y + 1, 1, 16, "#edffff")
    svg.rect(x - 1, y, 1, 18, "#46505d")
    raised_button(svg, x, y, 22, 18, pal=PL_BTN)
    if art:
        dx, dy, rows = art
        pixel_art(svg, rows, x + dx, y + dy, LABEL)


def mini_icons(svg):
    """The playlist's small transport icons, in its bottom right piece."""
    for shape in SHADE_ICONS[:-2]:
        pts = shift(shape, -64, 62)
        svg.poly(pts, "#736c52")
    eject = [[(181.5, 95.4), (185.6, 100), (177.4, 100)], [(178, 100.5), (185, 100.5),
                                                          (185, 102), (178, 102)]]
    for pts in eject:
        svg.poly(pts, "#736c52")
    for shape in SHADE_ICONS[:-2]:
        pts = shift(shape, -64, 62)
        svg.poly(inset(pts, 0.6), "#ecce7a")
        core = inset(pts, 1.4)
        if signed_area(core) * signed_area(pts) > 0 and polygon_area(core) > 0.3:
            svg.poly(core, "#ffffff")
    svg.poly(inset(eject[0], 0.6), "#ecce7a")
    svg.poly(inset(eject[0], 1.4), "#ffffff")
    svg.rect(178, 100.6, 7, 1.4, "#ecce7a")
    svg.rect(179, 100.8, 5, 1, "#ffffff")


def pl_bottom(svg):
    """The bottom pieces: left with ADD, REM, SEL and MISC; right with the display, the
    small transport, LIST OPTS, the scroll buttons and the resize grip; and the fill."""
    y = 72
    pl_piece(svg, 0, y, 125, 38, PL_BOTTOM_LEFT_STOPS)
    pl_piece(svg, 126, y, 150, 38, PL_BOTTOM_RIGHT_STOPS)
    svg.rect(179, 0, 25, 38, "#383858")
    for x, w in [(0, 125), (126, 150), (179, 25)]:
        yy = 0 if x == 179 else y
        tint(svg, x, yy + 32, w, 2, False)
        tint(svg, x, yy + 34, w, 1, True)
        tint(svg, x, yy + 37, w, 1, False)
    edge = svg.linear(0, 0, 150, 0, [(0, "#75758c"), (0.5, "#6e6e81"), (0.85, "#656674"),
                                     (1, "#636370")])
    svg.rect(11, y, 114, 1, "#636370")
    svg.rect(126, y, 130, 1, edge)
    svg.rect(179, 0, 25, 1, "#74748a")
    # Left edge, darkening towards the corner.
    svg.rect(0, y, 1, 38, PL_EDGE_DARK)
    svg.rect(1, y, 1, 37, PL_EDGE_LIGHT)
    svg.rect(4, y, 2, 34, PL_EDGE_DARK)
    svg.rect(6, y, 1, 32, PL_EDGE_LIGHT)
    gid = svg.new_id("g")
    svg.defs.append(
        f'<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" x1="0" y1="{y + 4}" '
        f'x2="0" y2="{y + 18}"><stop offset="0" stop-color="#000" stop-opacity="0"/>'
        '<stop offset="1" stop-color="#000" stop-opacity="0.42"/></linearGradient>'
    )
    svg.rect(0, y, 7, 38, f"url(#{gid})")
    # Right edge.
    svg.rect(270, y, 2, 34, PL_EDGE_DARK)
    svg.rect(272, y, 1, 34, PL_EDGE_LIGHT)
    svg.rect(273, y, 2, 38, PL_DARK)
    svg.rect(275, y, 1, 38, PL_EDGE_DARK)
    tint(svg, 273, y + 34, 2, 1, True)
    tint(svg, 273, y + 37, 2, 1, False)
    # ADD, REM, SEL, MISC and LIST OPTS.
    for name, bx in [("ADD", 14), ("REM", 43), ("SEL", 72), ("MISC", 101)]:
        side_button(svg, bx, y + 8, art=BUTTON_TEXT[name])
    side_button(svg, 232, y + 8, art=BUTTON_TEXT["LIST"])
    # The song display and the time.
    sunken(svg, 131, 80, 92, 10)
    sunken(svg, 189, 94, 34, 8)
    svg.rect(209, 96, 2, 1, "#00f800")
    svg.rect(209, 99, 2, 1, "#00f800")
    mini_icons(svg)
    # Scroll up and down.
    svg.poly([(264, 74), (266, 74), (269, 78), (261, 78)], PL_EDGE_LIGHT)
    svg.poly([(261, 80), (269, 80), (266, 84), (264, 84)], PL_EDGE_LIGHT)
    # The resize grip: diagonal lines in the corner.
    cid = svg.clip('<polygon points="272,90 272,104 258,104"/>')
    svg.open(clip=cid)
    for i, c in enumerate(["#5f606a", "#565663", "#53535d", "#505059", "#4d4d56"]):
        o = 3 * i
        svg.path(f"M{258 + o} {105 + 0.5}L{273 + o} {90.5 - 0}", stroke=c, width=1)
    svg.close()


def menu_item(svg, x, y, art, selected):
    """A playlist menu item, 22 x 18, normal or chosen."""
    dx, dy, rows = art
    if selected:
        svg.rect(x, y, 22, 18, "#0b0f16")
        svg.rect(x, y, 22, 1, "#19202a")
        svg.rect(x + 1, y + 2, 20, 15, "#798a9e")
        svg.rect(x + 2, y + 3, 19, 1, "#adbcc4")
        svg.rect(x + 2, y + 3, 1, 14, "#adbcc4")
        svg.rect(x + 3, y + 4, 17, 1, "#899aad")
        svg.rect(x + 3, y + 4, 1, 13, "#899aad")
        for cx, cy in [(1, 2), (19, 2), (1, 16)]:
            svg.px(x + cx, y + cy, "#677986")
        svg.px(x + 20, y + 2, "#576476")
        pixel_art(svg, rows, x + dx + 2, y + dy + 2, "#000000")
    else:
        raised_button(svg, x, y, 22, 18, pal=PL_BTN)
        pixel_art(svg, rows, x + dx, y + dy, LABEL)


def menu_bar(svg, x, items):
    """The strip beside a playlist menu."""
    svg.rect(x, 111, 3, items * 18, "#46505d")
    svg.rect(x, 111, 2, 1, "#2f2f44")
    svg.rect(x, 112, 1, items * 18 - 1, "#8192a5")
    svg.rect(x + 1, 112, 1, items * 18 - 1, "#edffff")


def sheet_pledit():
    svg = Svg(280, 186)
    svg.rect(0, 0, 280, 186, "#00c6ff")
    svg.comment("top pieces: active, inactive")
    pl_top(svg, 0, True)
    pl_top(svg, 21, False)
    svg.comment("left and right sides")
    svg.rect(0, 42, 31, 29, "#000000")
    for dx, c in enumerate([PL_EDGE_DARK, PL_EDGE_LIGHT, PL_DARK, PL_DARK, PL_EDGE_DARK,
                            PL_EDGE_DARK, PL_EDGE_LIGHT, PL_DARK, PL_DARK, PL_DARK,
                            PL_EDGE_DARK]):
        svg.rect(dx, 42, 1, 29, c)
    for dx, c in enumerate(["#000000", PL_EDGE_LIGHT, PL_DARK, PL_DARK, PL_DARK, PL_EDGE_DARK,
                            PL_DARK, PL_DARK, PL_DARK, PL_DARK, PL_DARK, PL_DARK,
                            PL_EDGE_LIGHT, PL_DARK, PL_DARK, PL_DARK, PL_EDGE_LIGHT, PL_DARK,
                            PL_DARK, PL_EDGE_DARK]):
        svg.rect(31 + dx, 42, 1, 29, c)
    svg.comment("close and shade pressed; the scroll thumb, normal and pressed")
    pl_gold = "#ae9265"
    svg.rect(52, 42, 9, 9, pl_gold)
    close_box(svg, 53, 43, True, pressed=True)
    svg.rect(62, 42, 9, 9, pl_gold)
    dome(svg, 63, 43, SHADE_DOME["pressed"])
    scroll_thumb(svg, 52, 53, False)
    scroll_thumb(svg, 61, 53, True)
    svg.comment("the windowshade playlist, and its unshade button pressed")
    pl_shade(svg)
    svg.rect(150, 42, 9, 9, pl_gold)
    stops, rim = DISC["pressed"]
    disc(svg, 151, 43, stops, rim)
    svg.comment("bottom pieces")
    pl_bottom(svg)
    svg.comment("the spare visualizer piece")
    svg.rect(205, 0, 75, 38, "#383858")
    svg.rect(205, 0, 75, 1, "#74748a")
    svg.rect(205, 7, 74, 1, "#27273d")
    svg.rect(205, 7, 1, 27, "#27273d")
    svg.rect(206, 8, 73, 23, "#000000")
    lcd_dots(svg, 206, 9, 73, 22, x0=0, y0=1)
    svg.rect(206, 31, 74, 1, "#74748a")
    svg.rect(279, 7, 1, 25, "#74748a")
    tint(svg, 205, 32, 75, 2, False)
    svg.rect(205, 34, 75, 1, "#74748a")
    svg.rect(205, 37, 75, 1, "#27273d")
    svg.comment("the menus: each item normal, then chosen; the strips beside them")
    for x, n in PL_MENU_ITEMS.items():
        for i in range(n):
            art = MENU_TEXT[(x, i)]
            menu_item(svg, x, 111 + 19 * i, art, False)
            menu_item(svg, x + 23, 111 + 19 * i, art, True)
        menu_bar(svg, 48 if x == 0 else x + 46, n)
    return svg


# --- gen: the frame of generic windows, which MilkDrop's window wears ----------------------------

# Like the playlist's frame: a rim, a groove around the picture, light pieces at the ends of the
# title bar shading to dark behind the title.


def gen_top(svg, y, active):
    """The six title bar pieces, 25 x 20 each, at row `y`."""
    pieces = [
        (0, [(0, PL_DARK), (2, PL_DARK), (25, PL_LIGHT)]),  # top left
        (26, [(0, PL_LIGHT), (25, PL_DARK)]),  # the left fill's end, before the title
        (52, [(0, PL_DARK), (25, PL_DARK)]),  # behind the title
        (78, [(0, PL_DARK), (25, PL_LIGHT)]),  # the right fill's end, after the title
        (104, [(0, PL_LIGHT), (25, PL_LIGHT)]),  # the fills
        (130, [(0, PL_LIGHT), (25, PL_DARK)]),  # top right, with the close button
    ]
    for x, stops in pieces:
        pl_piece(svg, x, y, 25, 20, stops)
        tint(svg, x, y, 25, 1, False)
        tint(svg, x, y + 12, 25, 2, False)
        tint(svg, x, y + 19, 25, 1, False)
        tint(svg, x, y + 1, 25, 1, True)
        tint(svg, x, y + 14, 25, 1, True)
    # The left edge: rim, then the groove turning down the side.
    svg.rect(0, y, 1, 20, PL_EDGE_DARK)
    svg.rect(1, y + 1, 1, 19, PL_EDGE_LIGHT)
    svg.rect(2, y + 12, 2, 8, PL_DARK)
    svg.rect(4, y + 12, 2, 8, PL_EDGE_DARK)
    svg.rect(6, y + 14, 1, 6, PL_EDGE_LIGHT)
    svg.rect(7, y + 19, 2, 1, PL_DARK)
    # The right edge: the groove's light line turning down, a ridge, the rim.
    svg.rect(147, y + 14, 1, 6, PL_EDGE_LIGHT)
    svg.rect(148, y + 15, 1, 5, PL_DARK)
    svg.rect(149, y + 12, 2, 8, PL_DARK)
    svg.rect(151, y + 12, 1, 8, PL_EDGE_LIGHT)
    svg.rect(152, y + 12, 2, 8, PL_DARK)
    svg.rect(154, y, 1, 20, PL_EDGE_DARK)
    # Stripes either side of the title, and the close button.
    pl_stripes(svg, 7, y + 4, 18, active, True, False)
    pl_stripes(svg, 26, y + 4, 22, active, False, True)
    pl_stripes(svg, 80, y + 4, 23, active, True, False)
    pl_stripes(svg, 104, y + 4, 25, active, False, False)
    pl_stripes(svg, 130, y + 4, 10, active, False, True)
    close_box(svg, 145, y + 4, active)


def gen_side(svg, x, y, w, h, columns):
    """A side piece: one colour per column, top to bottom."""
    for dx, c in enumerate(columns):
        svg.rect(x + dx, y, 1, h, c)


GEN_LEFT = [PL_EDGE_DARK, PL_EDGE_LIGHT, PL_DARK, PL_DARK, PL_EDGE_DARK, PL_EDGE_DARK,
            PL_EDGE_LIGHT, PL_DARK, PL_DARK, PL_DARK, PL_DARK]
GEN_RIGHT = [PL_EDGE_LIGHT, PL_DARK, PL_DARK, PL_DARK, PL_EDGE_LIGHT, PL_DARK, PL_DARK,
             PL_EDGE_DARK]


def gen_bottom_rows(svg, x, y, w):
    """A bottom piece's bevels, 14 rows from (x, y)."""
    tint(svg, x, y, w, 1, True)
    tint(svg, x, y + 8, w, 2, False)
    tint(svg, x, y + 10, w, 1, True)
    tint(svg, x, y + 13, w, 1, False)


def gen_bottom(svg):
    """The bottom pieces: left, right with the resize grip, and the fill between them."""
    # Bottom left, dark towards the corner.
    y = 42
    pl_piece(svg, 0, y, 125, 14, [(0, PL_DARK), (12, PL_DARK), (120, PL_LIGHT), (125, PL_LIGHT)])
    gen_bottom_rows(svg, 0, y, 125)
    svg.rect(0, y, 9, 14, PL_DARK)
    gen_side(svg, 0, y, 11, 8, GEN_LEFT[:9])
    svg.rect(4, y + 8, 5, 2, PL_EDGE_DARK)
    svg.rect(5, y + 10, 4, 1, PL_EDGE_LIGHT)
    svg.rect(0, y, 1, 14, PL_EDGE_DARK)
    svg.rect(1, y, 1, 13, PL_EDGE_LIGHT)
    svg.rect(0, y + 13, 9, 1, PL_EDGE_DARK)
    # Bottom right, dark towards the corner, with the grip.
    y = 57
    pl_piece(svg, 0, y, 125, 14, [(0, PL_LIGHT), (5, PL_LIGHT), (113, PL_DARK), (125, PL_DARK)])
    gen_bottom_rows(svg, 0, y, 125)
    svg.rect(117, y, 8, 14, PL_DARK)
    svg.rect(117, y, 4, 1, PL_EDGE_LIGHT)
    svg.rect(117, y + 8, 4, 2, PL_EDGE_DARK)
    svg.rect(117, y + 10, 4, 1, PL_EDGE_LIGHT)
    svg.rect(121, y, 1, 11, PL_EDGE_LIGHT)
    svg.rect(124, y, 1, 14, PL_EDGE_DARK)
    svg.rect(117, y + 13, 8, 1, PL_EDGE_DARK)
    # The grip: diagonal ridges, light, then shaded, then dark.
    cid = svg.clip(f'<polygon points="118,{y + 1} 119,{y + 1} 119,{y + 8} 111,{y + 8}"/>')
    svg.open(clip=cid)
    for o in (0, 3, 6):
        for d, c in ((0, "#5f5f71"), (1, "#403f50"), (2, "#1f1f30")):
            svg.path(f"M{num(111.5 + o + d)} {num(y + 8.5)}L{num(120 + o + d)} {y}",
                     stroke=c, width=1)
    svg.close()
    # The fill between them.
    pl_piece(svg, 127, 72, 25, 14, [(0, PL_LIGHT), (25, PL_LIGHT)])
    gen_bottom_rows(svg, 127, 72, 25)


GEN_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"


def gen_letter_cells(sheet_w=194, gap=1.0):
    """Each letter's cell, left edge and width, from x = 1 with a column of background
    between cells, and the narrowing that makes the alphabet fit the sheet."""
    sx = 1.0
    while True:
        widths = [round(TITLE_FONT[ch][0] * sx + gap) for ch in GEN_ALPHABET]
        if 1 + sum(widths) + len(widths) <= sheet_w:
            break
        sx -= 0.01
    cells, x = [], 1
    for w in widths:
        cells.append((x, w))
        x += w + 1
    return cells, sx


def gen_letters(svg, y, fill):
    """The alphabet in the title capitals, one cell each, on the title's background."""
    cells, sx = gen_letter_cells()
    for ch, (x, w) in zip(GEN_ALPHABET, cells):
        svg.rect(x, y, w, 7, PL_DARK)
        adv = TITLE_FONT[ch][0] * sx
        title_text(svg, ch, round(x + (w - adv) / 2, 2), y + 0.5, fill, sx=sx)


def sheet_gen():
    svg = Svg(194, 109)
    svg.rect(0, 0, 194, 109, "#00c6ff")
    svg.comment("title bar pieces: active, inactive")
    gen_top(svg, 0, True)
    gen_top(svg, 21, False)
    svg.comment("sides, and their lowest pieces")
    gen_side(svg, 127, 42, 11, 29, GEN_LEFT)
    gen_side(svg, 139, 42, 8, 29, GEN_RIGHT)
    gen_side(svg, 158, 42, 11, 24, GEN_LEFT)
    gen_side(svg, 170, 42, 8, 24, GEN_RIGHT)
    svg.comment("the close button pressed")
    svg.rect(148, 42, 9, 9, "#ae9265")
    close_box(svg, 149, 43, True, pressed=True)
    svg.comment("bottom pieces")
    gen_bottom(svg)
    svg.comment("the title's letters: active, inactive")
    gen_letters(svg, 88, "#ffffff")
    gen_letters(svg, 96, "#909090")
    return svg


# --- Packing -----------------------------------------------------------------------------------

SHEETS = {
    "cbuttons": sheet_cbuttons,
    "shufrep": sheet_shufrep,
    "numbers": sheet_numbers,
    "playpaus": sheet_playpaus,
    "text": sheet_text,
    "monoster": sheet_monoster,
    "posbar": sheet_posbar,
    "volume": sheet_volume,
    "balance": sheet_balance,
    "titlebar": sheet_titlebar,
    "main": sheet_main,
    "eqmain": sheet_eqmain,
    "eq_ex": sheet_eq_ex,
    "pledit": sheet_pledit,
    "gen": sheet_gen,
}


# The visualizer's colours (viscolor.txt): background, dots, the analyzer from top to bottom,
# the oscilloscope from brightest to dimmest, and the analyzer's peaks.
VIS_COLOURS = [
    ("0,0,0", "background"), ("24,33,41", "dots"),
    ("239,49,16", "analyzer, top"), ("206,41,16", ""), ("214,90,0", ""), ("214,102,0", ""),
    ("214,115,0", ""), ("198,123,8", ""), ("222,165,24", ""), ("214,181,33", ""),
    ("189,222,41", ""), ("148,222,33", ""), ("41,206,16", ""), ("50,190,16", ""),
    ("57,181,16", ""), ("49,156,8", ""), ("41,148,0", ""), ("24,132,8", "analyzer, bottom"),
    ("255,255,255", "oscilloscope, brightest"), ("214,214,222", ""), ("181,189,189", ""),
    ("160,170,175", ""), ("148,156,165", "oscilloscope, dimmest"),
    ("150,150,150", "analyzer peaks"),
]
PLEDIT_TXT = """[Text]
Normal=#00FF00
Current=#FFFFFF
NormalBG=#000000
SelectedBG=#0000C6
Font=Arial
"""


def text_files():
    """The skin's text files, with Windows line endings as classic skins have."""
    vis = []
    for i, (rgb, what) in enumerate(VIS_COLOURS):
        line = f"{rgb}, // {i}" + (f" = {what}" if what else "")
        vis.append(line)
    return {
        "viscolor.txt": "\r\n".join(vis) + "\r\n",
        "pledit.txt": PLEDIT_TXT.replace("\n", "\r\n"),
    }


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE.parent / "NeoAmp.wsz"
    for name, draw in SHEETS.items():
        (HERE / f"{name}.svg").write_text(draw().text(), encoding="utf-8", newline="\n")
    for name, text in text_files().items():
        (HERE / name).write_bytes(text.encode("ascii"))
    names = [f"{name}.svg" for name in SHEETS] + list(text_files())
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name in names:
            # A fixed date, so that the same sources always make the same archive.
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, (HERE / name).read_bytes())
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
