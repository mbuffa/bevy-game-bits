#!/usr/bin/env python3
"""Generate the WIP dev textures for the 010-immersive warehouse.

Stdlib only -- no PIL (it may not be installed). Writes three 128x128 RGB PNGs
into assets/textures/, one per surface family, each with a real pattern so the
geometry reads at a glance while the art is still placeholder:

    dev_floor_concrete  light grey, faint 32px dev grid + a little speckle
    dev_wall_brick      dark red brick, offset courses, grey mortar
    dev_deck_iron       dark grey iron, panel seams + corner rivets

Every pattern tiles seamlessly at 128px: gen_map.py gives each face a 0.25 UV
scale, so the 128px image repeats every 32 map units and a visible seam would
show up all over the level.

Deterministic: re-running produces byte-identical files. Any per-brick / per-
panel tonal variation comes from `_jitter` (a tiny integer hash), never from
`random`.

The ladder no longer has a texture -- its brush is `skip`-textured (invisible)
and ladder.rs builds a real rails-and-rungs mesh. The five original coloured
dev_*.png (dev_floor_a, dev_wall_a, dev_grid_128, dev_door, dev_trim) are left
untouched. The old flat-grey dev_gray_{floor,wall,deck}.png are replaced by the
three files above and can be deleted.

Run from anywhere:  python3 examples/010-immersive/tools/gen_textures.py
"""

import os
import struct
import zlib

SIZE = 128
HERE = os.path.dirname(os.path.abspath(__file__))
TEX_DIR = os.path.normpath(os.path.join(HERE, "..", "..", "..", "assets", "textures"))


def write_png(path, rows):
    """rows: list of SIZE bytearrays, each 3*SIZE bytes (RGB)."""
    raw = bytearray()
    for row in rows:
        raw.append(0)  # filter type 0 (None)
        raw.extend(row)

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 2, 0, 0, 0)  # 8-bit, colour type 2 (RGB)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", ihdr)
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def _clamp(v):
    return 0 if v < 0 else (255 if v > 255 else v)


def _shade(base, delta):
    return tuple(_clamp(c + delta) for c in base)


def _jitter(*key):
    """A small deterministic signed offset from an integer key -- stands in for
    per-brick / per-panel noise without importing `random`, so output stays
    byte-identical between runs."""
    h = 2166136261
    for k in key:
        h = ((h ^ (k & 0xFFFFFFFF)) * 16777619) & 0xFFFFFFFF
    return (h % 17) - 8  # -8..+8


def new_rows(fill):
    return [bytearray(bytes(fill) * SIZE) for _ in range(SIZE)]


def put(rows, x, y, rgb):
    i = (x % SIZE) * 3
    row = rows[y % SIZE]
    row[i], row[i + 1], row[i + 2] = rgb


# --- floor: light grey concrete -------------------------------------------

def floor_concrete():
    base = (188, 188, 192)
    grid = (170, 170, 176)
    rows = new_rows(base)
    for y in range(SIZE):
        for x in range(SIZE):
            if x % 32 == 0 or y % 32 == 0:
                put(rows, x, y, grid)
            else:
                # faint stationary speckle so a big slab isn't a dead flat
                put(rows, x, y, _shade(base, _jitter(x >> 1, y >> 1) // 2))
    return rows


# --- wall: dark red brick ------------------------------------------------

def wall_brick():
    brick = (108, 44, 38)
    mortar = (86, 80, 76)
    course = 16      # 8 courses per 128px tile
    brick_w = 32     # 4 bricks per row
    mortar_px = 2
    rows = new_rows(mortar)
    for cy in range(0, SIZE, course):
        row_idx = cy // course
        offset = (brick_w // 2) if row_idx % 2 else 0
        for bx in range(0, SIZE, brick_w):
            tone = _shade(brick, _jitter(row_idx, bx // brick_w))
            for y in range(cy + mortar_px, cy + course):
                for x in range(bx + offset + mortar_px, bx + offset + brick_w):
                    put(rows, x, y, tone)
    return rows


# --- deck: dark grey iron ----------------------------------------------

def deck_iron():
    base = (66, 68, 72)
    seam = (44, 46, 50)
    rivet = (94, 97, 103)
    panel = 64       # 2 panels per axis per tile; seams land on 0 and 64 -> wraps
    seam_px = 2
    rows = new_rows(base)
    for y in range(SIZE):
        for x in range(SIZE):
            # brushed-metal streak
            put(rows, x, y, _shade(base, (_jitter(y, 0) // 4)))
    for y in range(SIZE):
        for x in range(SIZE):
            if x % panel < seam_px or y % panel < seam_px:
                put(rows, x, y, seam)
    # four rivets per panel, inset 6px from each corner
    for py in range(0, SIZE, panel):
        for px in range(0, SIZE, panel):
            for (rx, ry) in ((6, 6), (panel - 6, 6), (6, panel - 6), (panel - 6, panel - 6)):
                cx, cy = px + rx, py + ry
                for dy in range(-1, 2):
                    for dx in range(-1, 2):
                        put(rows, cx + dx, cy + dy, rivet)
    return rows


TEXTURES = {
    "dev_floor_concrete": floor_concrete,
    "dev_wall_brick": wall_brick,
    "dev_deck_iron": deck_iron,
}


def main():
    os.makedirs(TEX_DIR, exist_ok=True)
    for name, build in TEXTURES.items():
        path = os.path.join(TEX_DIR, name + ".png")
        write_png(path, build())
        print("wrote", os.path.relpath(path))


if __name__ == "__main__":
    main()
