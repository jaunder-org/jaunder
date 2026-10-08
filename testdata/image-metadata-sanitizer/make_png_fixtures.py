#!/usr/bin/env python3
"""Owned CC0 PNG/APNG artwork for a candidate probe, not device evidence.

Eight-bit non-interlaced RGB/RGBA only. Compression occurs during fixture
construction; a sanitizer must copy the resulting image/frame streams.
"""
import struct
import sys
import zlib
from pathlib import Path

SIGNATURE = b"\x89PNG\r\n\x1a\n"
WIDTH, HEIGHT = 32, 24


def chunk(kind, data):
    return (struct.pack(">I", len(data)) + kind + data
            + struct.pack(">I", zlib.crc32(kind + data)))


def scanlines(width, height, seed, channels):
    rows = bytearray()
    for y in range(height):
        rows.append(0)  # No PNG row filter, chosen by this artwork generator.
        for x in range(width):
            rows.extend(((x * 7 + seed) % 256, (y * 11 + seed) % 256,
                         ((x + y) * 13 + seed) % 256))
            if channels == 4:
                rows.append((0, 127, 255)[(x + y) % 3])
    return zlib.compress(rows)


def exif():
    # Classic little-endian TIFF with main orientation and descriptive IFD0,
    # plus a real GPS IFD. Values describe no device/person; landmark coordinates
    # are synthetic test metadata, not a capture location.
    entries = [
        (0x010e, 2, b"Synthetic PNG description\0"),
        (0x010f, 2, b"Synthetic camera company\0"),
        (0x0110, 2, b"Fixture camera model\0"),
        (0x0112, 3, struct.pack("<H", 6)),
        (0x0132, 2, b"2026:10:07 12:34:56\0"),
        (0x013b, 2, b"Fixture PNG author\0"),
        (0x8298, 2, b"CC0 fixture description\0"),
        (0x8825, 4, b"\0" * 4),
    ]
    start = 8 + 2 + 12 * len(entries) + 4
    external = bytearray()
    records = bytearray()
    for tag, kind, data in entries:
        count = len(data) // {2: 1, 3: 2, 4: 4}[kind]
        value = data.ljust(4, b"\0") if len(data) <= 4 else struct.pack("<I", start + len(external))
        records.extend(struct.pack("<HHI", tag, kind, count) + value)
        if len(data) > 4:
            external.extend(data)
            external.extend(b"\0" * (len(external) % 2))
    gps_offset = start + len(external)
    # The GPS pointer is the final sorted IFD0 record.
    records[-4:] = struct.pack("<I", gps_offset)
    gps = [
        (0, 1, bytes((2, 3, 0, 0))),
        (1, 2, b"N\0"),
        (2, 5, struct.pack("<6I", 51, 1, 30, 1, 4, 1)),
        (3, 2, b"W\0"),
        (4, 5, struct.pack("<6I", 0, 1, 7, 1, 28, 1)),
    ]
    gps_start = gps_offset + 2 + 12 * len(gps) + 4
    gps_records, gps_external = bytearray(), bytearray()
    for tag, kind, data in gps:
        count = len(data) // {1: 1, 2: 1, 5: 8}[kind]
        value = data.ljust(4, b"\0") if len(data) <= 4 else struct.pack("<I", gps_start + len(gps_external))
        gps_records.extend(struct.pack("<HHI", tag, kind, count) + value)
        if len(data) > 4:
            gps_external.extend(data)
    return (b"II" + struct.pack("<HIH", 42, 8, len(entries)) + records
            + b"\0" * 4 + external + struct.pack("<H", len(gps))
            + gps_records + b"\0" * 4 + gps_external)


def metadata(profile):
    xmp = (b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF '
           b'xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
           b'<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/" '
           b'dc:description="synthetic PNG XMP description"/>'
           b'</rdf:RDF></x:xmpmeta>')
    return b"".join((
        chunk(b"iCCP", b"Fixture author camera profile\0\0" + zlib.compress(profile)),
        chunk(b"tEXt", b"Author\0Fixture PNG author"),
        chunk(b"zTXt", b"Comment\0\0" + zlib.compress(b"Synthetic private comment")),
        chunk(b"iTXt", b"XML:com.adobe.xmp\0\1\0en\0Description\0" + zlib.compress(xmp)),
        chunk(b"tIME", struct.pack(">H5B", 2026, 10, 7, 12, 34, 56)),
        chunk(b"eXIf", exif()),
    ))


def artwork(profile, mode):
    channels = 3 if mode == "rgb" else 4
    data = (SIGNATURE + chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8,
                                                2 if channels == 3 else 6, 0, 0, 0))
            + metadata(profile))
    if not mode.startswith("apng"):
        compressed = scanlines(WIDTH, HEIGHT, 5, channels)
        split = len(compressed) // 2
        return data + chunk(b"IDAT", compressed[:split]) + chunk(b"IDAT", compressed[split:]) + chunk(b"IEND", b"")
    separate_default = mode == "apng-default"
    data += chunk(b"acTL", struct.pack(">II", 3, 2 if separate_default else 0))
    if separate_default:
        # A different static fallback image, explicitly NOT animation frame 0.
        compressed = scanlines(WIDTH, HEIGHT, 101, 4)
        split = len(compressed) // 2
        data += chunk(b"IDAT", compressed[:split]) + chunk(b"IDAT", compressed[split:])
    sequence = 0
    frames = ((WIDTH, HEIGHT, 0, 0, 7, 100, 0, 0),
              (8, 7, 2, 3, 17, 0, 1, 1),
              (5, 4, 10, 6, 11, 60, 2, 1))
    for index, (width, height, x, y, numerator, denominator, disposal, blend) in enumerate(frames):
        data += chunk(b"fcTL", struct.pack(">5I2H2B", sequence, width, height, x, y,
                                           numerator, denominator, disposal, blend))
        sequence += 1
        compressed = scanlines(width, height, 5 + index * 31, 4)
        split = len(compressed) // 2
        for part in (compressed[:split], compressed[split:]):
            if index == 0 and not separate_default:
                data += chunk(b"IDAT", part)
            else:
                data += chunk(b"fdAT", struct.pack(">I", sequence) + part)
                sequence += 1
    return data + chunk(b"IEND", b"")


if __name__ == "__main__":
    profiles, destination = map(Path, sys.argv[1:])
    destination.mkdir(parents=True, exist_ok=True)
    for version in (2, 4):
        profile = (profiles / f"srgb-v{version}.icc").read_bytes()
        for mode in ("rgb", "rgba", "apng-first", "apng-default"):
            (destination / f"{mode}-v{version}.png").write_bytes(artwork(profile, mode))
