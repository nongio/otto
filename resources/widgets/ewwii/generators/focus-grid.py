#!/usr/bin/env python3
"""Draws the Cross pad backdrop: a grid of small crosses.

Writes focus/grid.png and focus/grid@2x.png, transparent except for the crosses,
and prints the SCSS variables that place the note on it.
Usage: focus-grid.py [WIDTH HEIGHT]   (logical pixels, default 1440 960)
"""
import os
import struct
import sys
import zlib

WIDTH, HEIGHT = (int(v) for v in sys.argv[1:3]) if len(sys.argv) > 2 else (1440, 960)
CELL = 28  # distance between crosses
ARM = 3  # half-length of a cross arm
MARGIN = 24  # keep crosses this far from the edges

COLS = (WIDTH - 2 * MARGIN) // CELL
ROWS = (HEIGHT - 2 * MARGIN) // CELL
X0 = (WIDTH - COLS * CELL) // 2
Y0 = (HEIGHT - ROWS * CELL) // 2
# The note's bottom-right corner sits one cell inside the last column and row, so
# a single line of crosses runs below it and to its right. The margins printed
# at the end place .focus-note-box there.
NOTE_RIGHT = X0 + (COLS - 1) * CELL
NOTE_BOTTOM = Y0 + (ROWS - 1) * CELL
# Crossless areas behind the text, as (left, top, right, bottom).
CLEAR = [
    # "Stay Focused.", centred
    (WIDTH / 2 - 150, HEIGHT / 2 - 36, WIDTH / 2 + 150, HEIGHT / 2 + 36),
    # the note
    (NOTE_RIGHT - 330, NOTE_BOTTOM - 105, NOTE_RIGHT, NOTE_BOTTOM),
    # the date, reading up the left edge from 170px down (see .focus-date)
    (0, 160, 36, 480),
]
INK = (244, 239, 232)
CROSS_ALPHA = 0.4

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "focus")


def blend(buf, w, h, x, y, alpha):
    if 0 <= x < w and 0 <= y < h and alpha > 0:
        i = (y * w + x) * 4
        a = min(255, buf[i + 3] + int(alpha * 255))
        buf[i : i + 4] = bytes((*INK, a))


def draw(scale):
    w, h = WIDTH * scale, HEIGHT * scale
    buf = bytearray(w * h * 4)
    t = scale  # one logical pixel thick

    cols, rows, x0, y0 = COLS, ROWS, X0, Y0
    arm = ARM * scale
    for r in range(rows + 1):
        for c in range(cols + 1):
            lx, ly = x0 + c * CELL, y0 + r * CELL
            if any(
                left - ARM <= lx <= right + ARM and top - ARM <= ly <= bottom + ARM
                for left, top, right, bottom in CLEAR
            ):
                continue
            cx, cy = lx * scale, ly * scale
            for d in range(-arm, arm + t):
                for k in range(t):
                    blend(buf, w, h, cx + d, cy + k, CROSS_ALPHA)
                    if not 0 <= d < t:
                        blend(buf, w, h, cx + k, cy + d, CROSS_ALPHA)

    return w, h, buf


def write_png(path, w, h, buf):
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(
            ">I", zlib.crc32(tag + data) & 0xFFFFFFFF
        )

    stride = w * 4
    raw = b"".join(b"\0" + bytes(buf[y * stride : (y + 1) * stride]) for y in range(h))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 6)))
        f.write(chunk(b"IEND", b""))


os.makedirs(OUT, exist_ok=True)
for scale, name in ((1, "grid.png"), (2, "grid@2x.png")):
    write_png(os.path.join(OUT, name), *draw(scale))
print(f"$focus-note-right: {WIDTH - NOTE_RIGHT}px;")
print(f"$focus-note-bottom: {HEIGHT - NOTE_BOTTOM}px;")
