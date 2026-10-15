#!/usr/bin/env python3
"""Construct owned WebP candidate inputs; this is not a metadata rewriter."""
import io
import struct
import sys
from pathlib import Path

from PIL import Image
from make_png_fixtures import exif

WIDTH, HEIGHT = 32, 24
XMP = (b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF '
       b'xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
       b'<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/" '
       b'dc:description="synthetic WebP XMP description"/>'
       b'</rdf:RDF></x:xmpmeta>')


def artwork(seed):
    """Return RGBA owned artwork, including intentionally colored transparent pixels."""
    pixels = []
    for y in range(HEIGHT):
        for x in range(WIDTH):
            alpha = (0, 96, 255)[(x + y + seed) % 3]
            pixels.append(((x * 17 + seed) % 256, (y * 29 + seed) % 256,
                           ((x + y) * 11 + seed) % 256, alpha))
    image = Image.new("RGBA", (WIDTH, HEIGHT))
    image.putdata(pixels)
    return image


def save_static(path, profile, lossless):
    artwork(7).save(path, "WEBP", lossless=lossless, quality=83, method=6,
                    exact=True, icc_profile=profile, exif=exif(), xmp=XMP)


def u24(value):
    return value.to_bytes(3, "little")


def chunk(kind, payload):
    return kind + struct.pack("<I", len(payload)) + payload + (b"\0" if len(payload) & 1 else b"")


def exact_lossless_vp8l(frame):
    """Extract one owned static Pillow exact-lossless frame for ANMF assembly."""
    encoded = io.BytesIO()
    frame.save(encoded, "WEBP", lossless=True, exact=True, method=6)
    data, at = encoded.getvalue(), 12
    assert data[:4] == b"RIFF" and data[8:12] == b"WEBP"
    while at < len(data):
        kind, length = data[at:at + 4], struct.unpack_from("<I", data, at + 4)[0]
        payload = data[at + 8:at + 8 + length]
        if kind == b"VP8L":
            return payload
        at += 8 + length + (length & 1)
    raise RuntimeError("static exact-lossless Pillow frame lacked VP8L")


def save_lossless_animation(path, profile, loop, last_seed):
    """Assemble owned full-frame ANMF VP8L chunks; not a general WebP rewriter."""
    frames = [artwork(7), artwork(31), artwork(last_seed)]
    payloads = [exact_lossless_vp8l(frame) for frame in frames]
    controls = []
    for duration, payload in zip((70, 170, 110), payloads, strict=True):
        # Stored x/y are half-pixel units; full-canvas origin is zero. Bit 1
        # means no blend and bit 0 is no dispose, matching the observed corpus.
        header = u24(0) + u24(0) + u24(WIDTH - 1) + u24(HEIGHT - 1) + u24(duration) + b"\2"
        controls.append(chunk(b"ANMF", header + chunk(b"VP8L", payload)))
    body = b"".join((
        chunk(b"VP8X", b"\x3e\0\0\0" + u24(WIDTH - 1) + u24(HEIGHT - 1)),
        chunk(b"ICCP", profile),
        chunk(b"ANIM", bytes((29, 19, 9, 39)) + struct.pack("<H", loop)),
        *controls,
        chunk(b"EXIF", exif()),
        chunk(b"XMP ", XMP),
    ))
    path.write_bytes(b"RIFF" + struct.pack("<I", len(body) + 4) + b"WEBP" + body)


def save_animation(path, profile, lossless, loop, last_seed=61):
    if lossless:
        save_lossless_animation(path, profile, loop, last_seed)
        return
    # Pillow's animation encoder is retained for lossy corpus construction.
    frames = [artwork(7), artwork(31), artwork(last_seed)]
    frames[0].save(path, "WEBP", save_all=True, append_images=frames[1:],
                   duration=[70, 170, 110], loop=loop, lossless=False,
                   quality=83, method=6, exact=True, background=(9, 19, 29, 39),
                   icc_profile=profile, exif=exif(), xmp=XMP)


if __name__ == "__main__":
    profiles, destination = map(Path, sys.argv[1:])
    destination.mkdir(parents=True, exist_ok=True)
    for version in (2, 4):
        profile = (profiles / f"srgb-v{version}.icc").read_bytes()
        for lossless, label in ((False, "lossy"), (True, "lossless")):
            save_static(destination / f"static-{label}-alpha-v{version}.webp",
                        profile, lossless)
            loop = 2 if version == 2 else 0
            save_animation(destination / f"animated-{label}-alpha-v{version}.webp",
                           profile, lossless, loop)
            # Same geometry, loop, and controls but a different final canvas:
            # a valid replacement source for the independent late-frame control.
            save_animation(destination / f"animated-{label}-alpha-v{version}-alternate.webpm",
                           profile, lossless, loop, last_seed=93)
    # Plain framing controls deliberately carry neither descriptive metadata nor ICC.
    artwork(3).convert("RGB").save(destination / "plain-lossy-rgb.webp", "WEBP", quality=83)
    artwork(3).convert("RGB").save(destination / "plain-lossless-rgb.webp", "WEBP", lossless=True)
