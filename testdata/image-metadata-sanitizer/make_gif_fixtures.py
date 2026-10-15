#!/usr/bin/env python3
"""Owned CC0 indexed artwork. Literal/reset LZW is fixture construction only."""
import struct
import sys
from pathlib import Path

PALETTE = bytes((13, 27, 41, 240, 80, 30, 30, 200, 90, 40, 60, 230))
LOCAL = bytes((17, 33, 49, 100, 20, 220, 240, 180, 50, 60, 220, 230))


def blocks(data):
    return b"".join(bytes([len(data[i:i + 255])]) + data[i:i + 255]
                    for i in range(0, len(data), 255)) + b"\0"


def literal_lzw(width, height, seed):
    codes = []
    for y in range(height):
        for x in range(width):
            codes.extend((4, (x + y + seed) % 4))
    codes.append(5)
    value = sum(code << (3 * index) for index, code in enumerate(codes))
    data = value.to_bytes((len(codes) * 3 + 7) // 8, "little")
    return b"\2" + blocks(data)


def artwork(profile=None, animated=False, loop=0, legacy=False):
    header = b"GIF87a" if legacy else b"GIF89a"
    data = header + struct.pack("<HHBBB", 12, 10, 0xf1, 1, 0) + PALETTE
    if profile is not None:
        data += b"!\xff\x0bICCRGBG1012" + blocks(profile)
        data += b"!\xfe" + blocks(b"Synthetic author/device/GPS/time/copyright/comment; not a capture")
    if animated:
        data += b"!\xff\x0bNETSCAPE2.0\3\1" + struct.pack("<H", loop) + b"\0"
    frames = [(0, 0, 12, 10, 1, 7, False)]
    if animated:
        frames += [(2, 2, 4, 3, 2, 17, True), (6, 4, 3, 4, 3, 11, False)]
    for index, (x, y, w, h, disposal, delay, local) in enumerate(frames):
        if not legacy:
            data += b"!\xf9\4" + struct.pack("<BHB", disposal * 4 + 1, delay, 0) + b"\0"
        data += b"," + struct.pack("<HHHHB", x, y, w, h, 0x81 if local else 0)
        if local:
            data += LOCAL
        data += literal_lzw(w, h, index)
    return data + b";"


if __name__ == "__main__":
    profiles, destination = map(Path, sys.argv[1:])
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "bare87.gif").write_bytes(artwork(legacy=True))
    for version in (2, 4):
        profile = (profiles / f"srgb-v{version}.icc").read_bytes()
        for mode, animated, loop in (("static", False, 0), ("finite", True, 2), ("infinite", True, 0)):
            (destination / f"{mode}-v{version}.gif").write_bytes(artwork(profile, animated, loop))
