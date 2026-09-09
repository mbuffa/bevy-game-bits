#!/usr/bin/env python3
"""Generate the neutral-grey dev textures for the 010-immersive warehouse.

Stdlib only -- no PIL (the original dev_*.png needed it, and it may not be
installed). Writes three 128x128 RGB PNGs into assets/textures/:

    dev_gray_floor  flat mid-grey, faint 32px grid
    dev_gray_wall   slightly lighter grey, faint 32px grid
    dev_gray_deck   darker grey for the raised platforms

The ladder no longer has a texture -- its brush is `skip`-textured (invisible)
and ladder.rs builds a real rails-and-rungs mesh. The five original coloured
dev_*.png (dev_floor_a, dev_wall_a, dev_grid_128, dev_door, dev_trim) are left
untouched.

Run from anywhere:  python3 examples/010-immersive/tools/gen_textures.py
Deterministic: re-running produces byte-identical files.
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


def solid_with_grid(base, line, spacing=32):
    rows = []
    for y in range(SIZE):
        row = bytearray()
        for x in range(SIZE):
            on_line = (x % spacing == 0) or (y % spacing == 0)
            r, g, b = line if on_line else base
            row += bytes((r, g, b))
        rows.append(row)
    return rows


TEXTURES = {
    "dev_gray_floor": lambda: solid_with_grid((104, 104, 110), (92, 92, 98)),
    "dev_gray_wall": lambda: solid_with_grid((126, 126, 132), (112, 112, 118)),
    "dev_gray_deck": lambda: solid_with_grid((88, 88, 94), (76, 76, 82)),
}


def main():
    os.makedirs(TEX_DIR, exist_ok=True)
    for name, build in TEXTURES.items():
        path = os.path.join(TEX_DIR, name + ".png")
        write_png(path, build())
        print("wrote", os.path.relpath(path))


if __name__ == "__main__":
    main()
