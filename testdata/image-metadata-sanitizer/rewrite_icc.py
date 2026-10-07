#!/usr/bin/env python3
"""Fixture-scoped ICC v4 device-link descriptive-field rewriter.

This is a feasibility probe, not a production sanitizer. It accepts only the
owned LittleCMS-generated RGB-to-XYZ v4 device-link profile used by probe.nix.
All other profile versions/classes/tag layouts fail closed.
"""
import hashlib
import struct
import sys
from pathlib import Path

HEADER_SIZE = 128
TAG_TABLE_OFFSET = 132
RETAINED = {b"A2B0": b"mAB ", b"wtpt": b"XYZ "}
REMOVED = {b"desc", b"cprt", b"pseq", b"psid"}


def fail(message):
    raise ValueError(message)


def read_profile(path):
    data = Path(path).read_bytes()
    if len(data) < TAG_TABLE_OFFSET:
        fail("profile is shorter than its tag table")
    if struct.unpack_from(">I", data, 0)[0] != len(data):
        fail("declared profile size does not equal file size")
    if data[36:40] != b"acsp":
        fail("missing ICC signature")
    if data[8] != 4:
        fail("only ICC v4 profiles are supported")
    if data[12:16] != b"link":
        fail("only device-link profiles are supported")
    if data[16:20] != b"RGB " or data[20:24] != b"XYZ ":
        fail("only RGB-to-XYZ device-link profiles are supported")
    count = struct.unpack_from(">I", data, HEADER_SIZE)[0]
    if count > 6 or TAG_TABLE_OFFSET + count * 12 > len(data):
        fail("invalid or unsupported tag count")
    tags = []
    occupied = []
    for index in range(count):
        signature, offset, size = struct.unpack_from(">4sII", data, TAG_TABLE_OFFSET + index * 12)
        if size < 4 or offset < TAG_TABLE_OFFSET + count * 12 or offset + size > len(data):
            fail("tag extent is outside the profile")
        if offset % 4:
            fail("tag extent is not four-byte aligned")
        for begin, end in occupied:
            if not (offset + size <= begin or end <= offset):
                fail("overlapping or shared tag extents are unsupported")
        occupied.append((offset, offset + size))
        payload = data[offset : offset + size]
        tags.append((signature, payload))
    signatures = {signature for signature, _ in tags}
    if len(signatures) != len(tags):
        fail("duplicate ICC tag signature")
    if signatures not in (set(RETAINED) | REMOVED, set(RETAINED)):
        fail("unsupported tag signature set")
    for signature, payload in tags:
        if signature in RETAINED and payload[:4] != RETAINED[signature]:
            fail("retained tag has an unsupported type")
    return data, tags


def scrub(source, destination):
    data, tags = read_profile(source)
    header = bytearray(data[:HEADER_SIZE])
    # Keep colour-space semantics but remove timestamp, implementation/device
    # identifiers, creator, profile ID, and reserved bytes.
    for begin, end in ((4, 8), (24, 36), (48, 56), (80, 128)):
        header[begin:end] = b"\0" * (end - begin)
    retained = [(signature, payload) for signature, payload in tags if signature in RETAINED]
    output = bytearray(header)
    output.extend(struct.pack(">I", len(retained)))
    table_at = len(output)
    output.extend(b"\0" * (12 * len(retained)))
    entries = []
    for signature, payload in retained:
        while len(output) % 4:
            output.append(0)
        offset = len(output)
        output.extend(payload)
        entries.append((signature, offset, len(payload)))
    for index, entry in enumerate(entries):
        struct.pack_into(">4sII", output, table_at + index * 12, *entry)
    struct.pack_into(">I", output, 0, len(output))
    Path(destination).write_bytes(output)


def transform_hash(path):
    _, tags = read_profile(path)
    payload = next(payload for signature, payload in tags if signature == b"A2B0")
    return hashlib.sha256(payload).hexdigest()


if __name__ == "__main__":
    scrub(sys.argv[1], sys.argv[2])
