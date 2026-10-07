#!/usr/bin/env python3
"""Canonicalize owned RGB monitor ICC fixtures; not a production sanitizer.

Envelope: ICC 2.0/4.0, XYZ PCS, matrix/TRC, both chad/chrm, packed tags,
identical shared extents, and at most two contiguous description locales.
No LUT, vendor tags, nonempty legacy ScriptCode, or arbitrary profile layout.
"""
from datetime import datetime
import struct
import sys
from pathlib import Path

HEADER_SIZE = 128
TABLE_START = 132
MAX_PROFILE_SIZE = 8 * 1024 * 1024
CANONICAL_DATE = bytes.fromhex("07d000010001000000000000")
PCS_D50 = bytes.fromhex("0000f6d6000100000000d32d")
XYZ_TAGS = {b"wtpt", b"rXYZ", b"gXYZ", b"bXYZ"}
TRC_TAGS = {b"rTRC", b"gTRC", b"bTRC"}
DESCRIPTIVE_TAGS = {b"desc", b"cprt"}
EXPECTED_TAGS = XYZ_TAGS | TRC_TAGS | DESCRIPTIVE_TAGS | {b"chad", b"chrm"}
NEUTRAL_TEXT = "Jaunder metadata-sanitized profile"


def invalid(detail):
    raise ValueError("invalid ICC " + detail)


def word_size(size):
    return (size + 3) & ~3


def complete(payload, used, detail):
    if len(payload) != used:
        invalid(detail)


def decode_text(data, encoding, detail):
    try:
        return data.decode(encoding)
    except UnicodeDecodeError:
        invalid(detail)


def multilingual(payload):
    if payload[:4] != b"mluc" or len(payload) < 16:
        invalid("multilingual type")
    count, record_size = struct.unpack_from(">II", payload, 8)
    if count not in (1, 2) or record_size != 12 or 16 + count * 12 > len(payload):
        invalid("multilingual records")
    cursor = 16 + count * 12
    locales = set()
    for index in range(count):
        locale, length, offset = struct.unpack_from(">4sII", payload, 16 + index * 12)
        if (not locale[:2].isalpha() or not locale[:2].islower()
                or not locale[2:].isalpha() or not locale[2:].isupper()
                or locale in locales or length % 2 or offset != cursor
                or offset + length > len(payload)):
            invalid("multilingual record bounds/locale")
        text = decode_text(payload[offset:offset + length], "utf-16-be", "multilingual encoding")
        if "\0" in text:
            invalid("multilingual embedded terminator")
        locales.add(locale)
        cursor += length
    complete(payload, cursor, "multilingual padding")


def legacy_description(payload):
    if payload[:4] != b"desc" or len(payload) < 12:
        invalid("legacy description type")
    ascii_count = struct.unpack_from(">I", payload, 8)[0]
    unicode_header = 12 + ascii_count
    if ascii_count < 1 or unicode_header + 8 > len(payload):
        invalid("legacy ASCII bounds")
    ascii_data = payload[12:unicode_header]
    if ascii_data[-1] or b"\0" in ascii_data[:-1]:
        invalid("legacy ASCII terminator")
    decode_text(ascii_data[:-1], "ascii", "legacy ASCII encoding")
    language, unicode_count = struct.unpack_from(">II", payload, unicode_header)
    script_start = unicode_header + 8 + 2 * unicode_count
    if language != 0 or script_start + 70 > len(payload):
        invalid("legacy Unicode bounds")
    unicode_data = payload[unicode_header + 8:script_start]
    if unicode_count:
        if unicode_data[-2:] != b"\0\0":
            invalid("legacy Unicode terminator")
        text = decode_text(unicode_data[:-2], "utf-16-be", "legacy Unicode encoding")
        if "\0" in text:
            invalid("legacy Unicode embedded terminator")
    script_code, script_count = struct.unpack_from(">HB", payload, script_start)
    if script_code or script_count or any(payload[script_start + 3:script_start + 70]):
        invalid("unsupported legacy script")
    complete(payload, script_start + 70, "legacy description padding")


def body(payload, version, signature):
    # Public only to the fixture controls; each admitted body is fully consumed.
    if len(payload) < 8 or any(payload[4:8]):
        invalid("tag reserved bytes")
    kind = payload[:4]
    if signature in XYZ_TAGS:
        if kind != b"XYZ " or len(payload) != 20:
            invalid("XYZ body")
    elif signature in TRC_TAGS:
        if kind == b"curv":
            if len(payload) < 12:
                invalid("curve body")
            count = struct.unpack_from(">I", payload, 8)[0]
            complete(payload, 12 + 2 * count, "curve body")
            if count == 1 and struct.unpack_from(">H", payload, 12)[0] == 0:
                invalid("zero curve gamma")
            if count > 1:
                samples = [item[0] for item in struct.iter_unpack(">H", payload[12:12 + 2 * count])]
                if samples[0] >= samples[-1] or any(a > b for a, b in zip(samples, samples[1:])):
                    invalid("unsupported non-increasing curve")
        elif kind == b"para" and version == 4:
            if len(payload) < 12:
                invalid("parametric body")
            function = struct.unpack_from(">H", payload, 8)[0]
            parameters = {0: 1, 1: 3, 2: 4, 3: 5, 4: 7}
            if (function not in parameters or any(payload[10:12])
                    or len(payload) != 12 + 4 * parameters[function]):
                invalid("parametric body")
            values = struct.unpack_from(">" + "i" * parameters[function], payload, 12)
            if values[0] <= 0 or (function and values[1] <= 0):
                invalid("unsupported parametric gamma/scale")
            if function in (3, 4) and (values[3] < 0 or not 0 <= values[4] <= 65536):
                invalid("unsupported parametric slope/threshold")
        else:
            invalid("TRC type")
    elif signature == b"chad":
        if kind != b"sf32" or len(payload) != 44:
            invalid("adaptation body")
        a, b, c, d, e, f, g, h, i = struct.unpack_from(">9i", payload, 8)
        if a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g) == 0:
            invalid("singular adaptation")
    elif signature == b"chrm":
        if kind != b"chrm" or len(payload) != 36 or payload[8:12] != b"\0\3\0\0":
            invalid("chromaticity body")
        coordinates = struct.unpack_from(">6I", payload, 12)
        for x, y in zip(coordinates[::2], coordinates[1::2]):
            if not (0 < x <= 65536 and 0 < y <= 65536 and x + y <= 65537):
                invalid("chromaticity coordinates")
    elif signature in DESCRIPTIVE_TAGS:
        if version == 4:
            multilingual(payload)
        elif signature == b"desc":
            legacy_description(payload)
        else:
            if kind != b"text" or b"\0" not in payload[8:]:
                invalid("copyright body")
            end = payload.index(b"\0", 8)
            decode_text(payload[8:end], "ascii", "copyright encoding")
            complete(payload, end + 1, "copyright padding")
    else:
        invalid("unsupported tag")


def parse(path):
    with Path(path).open("rb") as source:
        data = source.read(MAX_PROFILE_SIZE + 1)
    if (len(data) < TABLE_START or len(data) > MAX_PROFILE_SIZE or len(data) % 4
            or struct.unpack_from(">I", data)[0] != len(data)):
        invalid("size")
    if data[36:40] != b"acsp" or data[8:12] not in (b"\2\0\0\0", b"\4\0\0\0"):
        invalid("version/signature")
    version = data[8]
    try:
        datetime(*struct.unpack_from(">6H", data, 24))
    except ValueError:
        invalid("date")
    if (data[12:24] != b"mntrRGB XYZ " or data[68:80] != PCS_D50
            or struct.unpack_from(">I", data, 64)[0] > 3
            or struct.unpack_from(">I", data, 44)[0] & ~3
            or struct.unpack_from(">Q", data, 56)[0] & ~15
            or any(data[100:128]) or (version == 2 and any(data[84:100]))):
        invalid("header semantics")
    count = struct.unpack_from(">I", data, HEADER_SIZE)[0]
    if count != len(EXPECTED_TAGS) or TABLE_START + count * 12 > len(data):
        invalid("tag count")
    tags = []
    extents = set()
    for index in range(count):
        signature, offset, size = struct.unpack_from(">4sII", data, TABLE_START + index * 12)
        if size < 8 or offset < TABLE_START + count * 12 or offset % 4 or offset + size > len(data):
            invalid("tag bounds")
        extents.add((offset, offset + size))
        tags.append((signature, offset, size, data[offset:offset + size]))
    if len({tag[0] for tag in tags}) != count or {tag[0] for tag in tags} != EXPECTED_TAGS:
        invalid("tag set")
    cursor = TABLE_START + count * 12
    for start, end in sorted(extents):
        if start != word_size(cursor) or any(data[cursor:start]):
            invalid("overlap/gap")
        cursor = end
    if len(data) != word_size(cursor) or any(data[cursor:]):
        invalid("final padding")
    for signature, _, _, payload in tags:
        body(payload, version, signature)
    values = {signature: payload for signature, _, _, payload in tags}
    white = struct.unpack_from(">3i", values[b"wtpt"], 8)
    if min(white) <= 0 or (version == 4 and values[b"wtpt"][8:20] != PCS_D50):
        invalid("media white")
    r, g, b = (struct.unpack_from(">3i", values[signature], 8)
               for signature in (b"rXYZ", b"gXYZ", b"bXYZ"))
    determinant = (r[0] * (g[1] * b[2] - g[2] * b[1])
                   - g[0] * (r[1] * b[2] - r[2] * b[1])
                   + b[0] * (r[1] * g[2] - r[2] * g[1]))
    if determinant == 0:
        invalid("singular RGB matrix")
    return data, tags, version


def neutral(version, signature):
    if version == 4:
        text = NEUTRAL_TEXT.encode("utf-16-be")
        return (b"mluc" + b"\0" * 4 + struct.pack(">II", 1, 12) + b"enUS"
                + struct.pack(">II", len(text), 28) + text)
    text = NEUTRAL_TEXT.encode("ascii") + b"\0"
    if signature == b"cprt":
        return b"text" + b"\0" * 4 + text
    return b"desc" + b"\0" * 4 + struct.pack(">I", len(text)) + text + b"\0" * 78


def rewrite(source, destination):
    data, tags, version = parse(source)
    header = bytearray(data[:HEADER_SIZE])
    header[24:36] = CANONICAL_DATE
    for begin, end in ((4, 8), (40, 44), (48, 56), (80, 128)):
        header[begin:end] = b"\0" * (end - begin)
    output = header + struct.pack(">I", len(tags)) + bytearray(12 * len(tags))
    copied = {}
    for index, (signature, old_offset, old_size, payload) in enumerate(tags):
        if signature in DESCRIPTIVE_TAGS:
            payload = neutral(version, signature)
            key = (signature,)
        else:
            key = (old_offset, old_size)
        if key not in copied:
            output.extend(b"\0" * (word_size(len(output)) - len(output)))
            copied[key] = (len(output), len(payload))
            output.extend(payload)
        struct.pack_into(">4sII", output, TABLE_START + index * 12, signature, *copied[key])
    output.extend(b"\0" * (word_size(len(output)) - len(output)))
    struct.pack_into(">I", output, 0, len(output))
    Path(destination).write_bytes(output)


if __name__ == "__main__":
    rewrite(sys.argv[1], sys.argv[2])
