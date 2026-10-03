#!/usr/bin/env python3
"""Draws the Grid pad backdrop: a drafting grid with rulers and a diagonal.

Writes lines/grid.png and lines/grid@2x.png, transparent except for the lines,
and prints the SCSS variables that place the text on it.
Usage: lines-grid.py [WIDTH HEIGHT]   (logical pixels, default 1440 960)

The frame holds ROWS rows of square cells filling the height, full columns in
the middle and a narrower column at each side taking the rest of the width.
Some lines stop short to merge cells for the text:
the date runs up the left side column, the title fills the middle row and the
note fills the end of the bottom row. The margins for those labels are printed
at the end as SCSS variables, which Otto puts ahead of ewwii.scss.
"""
import os
import struct
import sys
import zlib

WIDTH, HEIGHT = (int(v) for v in sys.argv[1:3]) if len(sys.argv) > 2 else (1440, 960)
ROWS = 5
MINOR = 8  # minor divisions per cell
# Room around the frame for the rulers.
MARGIN = 24
# The side of a major cell: the rows fill the height, in whole minor steps.
CELL = (HEIGHT - 2 * MARGIN) // ROWS // MINOR * MINOR
COLS = (WIDTH - 2 * MARGIN) // CELL - 1  # full columns, with a narrower one at each side
STEP = CELL // MINOR
# The side columns take what the full ones leave, in whole minor steps.
SIDE = (WIDTH - 2 * MARGIN - COLS * CELL) // 2 // STEP * STEP

FRAME_W = COLS * CELL + 2 * SIDE
FRAME_H = ROWS * CELL
X0 = (WIDTH - FRAME_W) // 2
Y0 = (HEIGHT - FRAME_H) // 2
X1, Y1 = X0 + FRAME_W, Y0 + FRAME_H

# Vertical lines between full columns, and horizontal lines between rows.
VLINES = [X0 + SIDE + c * CELL for c in range(COLS + 1)]
HLINES = [Y0 + r * CELL for r in range(1, ROWS)]

MID_ROW = ROWS // 2
TITLE_COLS = 3  # full columns the title spans, centred
NOTE_COLS = 3  # full columns the note spans, ending at the frame
DATE_ROWS = 2  # rows the date spans, from the top
# Half the height of the date's 11px line, which turns about its centre.
DATE_HALF_LINE = 7
# The note's four 13px lines, about 16px apart, centred in the height of a row.
NOTE_DROP = (CELL - 4 * 16) // 2

INK = (244, 239, 232)
MAJOR_ALPHA = 0.4
MINOR_ALPHA = 0.1
DIAGONAL_ALPHA = 0.4
TICK_ALPHA = 0.4

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lines")


def vline_gaps(x):
    """Row ranges a vertical line skips, as (top, bottom)."""
    gaps = []
    centre = WIDTH // 2
    if abs(x - centre) < TITLE_COLS * CELL / 2:
        gaps.append((Y0 + MID_ROW * CELL, Y0 + (MID_ROW + 1) * CELL))
    if x > X1 - SIDE - NOTE_COLS * CELL:
        gaps.append((Y1 - CELL, Y1))
    return gaps


def hline_gaps(y):
    """Column ranges a horizontal line skips, as (left, right)."""
    return [(X0, X0 + SIDE)] if y < Y0 + DATE_ROWS * CELL else []


def draw(scale):
    w, h = WIDTH * scale, HEIGHT * scale
    alpha = bytearray(w * h)

    def plot(x, y, a):
        if 0 <= x < w and 0 <= y < h:
            i = y * w + x
            alpha[i] = max(alpha[i], int(a * 255))

    def hline(y, left, right, a):
        for py in range(y * scale, (y + 1) * scale):
            for px in range(left * scale, right * scale + scale):
                plot(px, py, a)

    def vline(x, top, bottom, a):
        for px in range(x * scale, (x + 1) * scale):
            for py in range(top * scale, bottom * scale + scale):
                plot(px, py, a)

    def spans(start, end, gaps):
        cuts = sorted(gaps)
        for g0, g1 in cuts:
            if g0 > start:
                yield start, g0
            start = max(start, g1)
        if start < end:
            yield start, end

    # Minor grid, from the frame's left edge so the side columns get whole steps.
    for x in range(X0 + STEP, X1, STEP):
        vline(x, Y0, Y1, MINOR_ALPHA)
    for y in range(Y0 + STEP, Y1, STEP):
        hline(y, X0, X1, MINOR_ALPHA)

    # Frame and major lines.
    hline(Y0, X0, X1, MAJOR_ALPHA)
    hline(Y1, X0, X1, MAJOR_ALPHA)
    vline(X0, Y0, Y1, MAJOR_ALPHA)
    vline(X1, Y0, Y1, MAJOR_ALPHA)
    for x in VLINES:
        for top, bottom in spans(Y0, Y1, vline_gaps(x)):
            vline(x, top, bottom, MAJOR_ALPHA)
    for y in HLINES:
        for left, right in spans(X0, X1, hline_gaps(y)):
            hline(y, left, right, MAJOR_ALPHA)

    # 45° diagonal from the bottom-left corner to the top edge.
    for d in range(FRAME_H * scale):
        for k in range(scale):
            plot(X0 * scale + d + k, Y1 * scale - d, DIAGONAL_ALPHA)

    # Rulers outside the frame: a tick every minor step, longer on major lines.
    gap, short, long = 6, 3, 7
    for x in range(X0, X1 + 1, STEP):
        n = long if x in VLINES or x in (X0, X1) else short
        vline(x, Y0 - gap - n, Y0 - gap, TICK_ALPHA)
        vline(x, Y1 + gap, Y1 + gap + n, TICK_ALPHA)
    for y in range(Y0, Y1 + 1, STEP):
        n = long if y in HLINES or y in (Y0, Y1) else short
        hline(y, X0 - gap - n, X0 - gap, TICK_ALPHA)
        hline(y, X1 + gap, X1 + gap + n, TICK_ALPHA)

    buf = bytearray(w * h * 4)
    for i, a in enumerate(alpha):
        if a:
            buf[i * 4 : i * 4 + 4] = bytes((*INK, a))
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

note_left = X1 - SIDE - NOTE_COLS * CELL
date_length = DATE_ROWS * CELL - 2 * STEP
print(f"$lines-date-top: {Y0}px;")
print(f"$lines-date-length: {date_length}px;")
print(f"$lines-date-shift-x: {X0 + SIDE // 2 - date_length // 2}px;")
print(f"$lines-date-shift-y: {DATE_ROWS * CELL // 2 - DATE_HALF_LINE}px;")
print(f"$lines-note-left: {note_left + STEP}px;")
print(f"$lines-note-top: {Y1 - CELL + NOTE_DROP}px;")
