#!/usr/bin/env python3
"""Generate the placeholder app icon (a flat disc on dark ground) as a 1024x1024 PNG.

Pure standard library (no Pillow). Then run `cargo tauri icon frontend/icon-src.png
-o frontend/src-tauri/icons` to derive every platform size.
"""
import struct
import sys
import zlib

SIZE = 1024
BG = (20, 22, 26)
FG = (79, 163, 255)


def px(x, y):
    dx, dy = x - SIZE / 2, y - SIZE / 2
    r = (dx * dx + dy * dy) ** 0.5
    if r < SIZE * 0.36:
        return FG + (255,)
    if r < SIZE * 0.42 and abs(dx) < SIZE * 0.03:
        return FG + (255,)
    return BG + (255,)


def main(path):
    raw = bytearray()
    for y in range(SIZE):
        raw.append(0)
        for x in range(SIZE):
            raw.extend(px(x, y))

    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "icon-src.png")
