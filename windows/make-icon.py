#!/usr/bin/env python3
"""Draws resources/icons/neoamp.ico, NeoAmp's Windows icon, from resources/icons/neoamp.svg.

    python3 windows/make-icon.py

The .ico is kept in the repository because build.rs builds it into every Windows build of
neoamp.exe; run this again after changing the SVG, and commit the result. It runs anywhere,
from anywhere in this repository, and needs resvg, which draws each size:

    cargo install --locked resvg
"""

import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

# The sizes Windows asks for at the usual display scales: 16 to 64 pixels for title bars, the
# taskbar, Alt+Tab and Explorer's smaller views, 256 for its largest.
SIZES = [16, 20, 24, 32, 40, 48, 64, 256]


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    svg = root / "resources" / "icons" / "neoamp.svg"
    ico = root / "resources" / "icons" / "neoamp.ico"
    if shutil.which("resvg") is None:
        sys.exit("make-icon.py: resvg draws the icon: install it with 'cargo install --locked resvg'")

    with tempfile.TemporaryDirectory() as tmp:
        images = []
        for size in SIZES:
            png = Path(tmp) / f"{size}.png"
            subprocess.run(
                ["resvg", "-w", str(size), "-h", str(size), str(svg), str(png)], check=True
            )
            images.append((size, png.read_bytes()))

    # An ICONDIR header, then a directory entry for each image, then the images themselves,
    # each a whole PNG file, which Windows reads at every size since Vista.
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries = b""
    for size, data in images:
        # Width and height are a byte each; 0 means 256.
        side = size % 256
        entries += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    ico.write_bytes(header + entries + b"".join(data for _, data in images))
    print(f"Wrote {ico.relative_to(root)}")


if __name__ == "__main__":
    main()
