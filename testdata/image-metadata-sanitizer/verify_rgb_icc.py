#!/usr/bin/env python3
"""Independent ICC fixture inspector: consumes fields, not sanitizer reports.

This reader deliberately does not import the ICC canonicalizer. It validates
only the declared 2.0/4.0 packed RGB matrix/TRC fixture envelope, not arbitrary
ICC profiles or certification of a device's calibration.
"""
from datetime import datetime
import importlib.util
import struct
import sys
from pathlib import Path

TEXT = "Jaunder metadata-sanitized profile"
XYZ_TAGS = {b"wtpt", b"rXYZ", b"gXYZ", b"bXYZ"}
TRC_TAGS = {b"rTRC", b"gTRC", b"bTRC"}
COLOR_TAGS = XYZ_TAGS | TRC_TAGS | {b"chad", b"chrm"}
ALL_TAGS = COLOR_TAGS | {b"desc", b"cprt"}
SOURCE_DATE = bytes.fromhex("07ea000a0007000c00220038")
CLEAN_DATE = bytes.fromhex("07d000010001000000000000")


def check(condition, detail):
    if not condition:
        raise AssertionError("invalid inspected ICC " + detail)


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Cursor:
    """A bounded, advancing reader; finishing must consume the entire body."""
    def __init__(self, data):
        self.data = data
        self.position = 0

    def take(self, count):
        check(0 <= count <= len(self.data) - self.position, "truncated field")
        start = self.position
        self.position += count
        return self.data[start:self.position]

    def number(self, count, signed=False):
        return int.from_bytes(self.take(count), "big", signed=signed)

    def zeros(self, count):
        check(not any(self.take(count)), "reserved/padding bytes")

    def finish(self):
        check(self.position == len(self.data), "surplus body bytes")


def text(data, encoding):
    try:
        return data.decode(encoding)
    except UnicodeDecodeError:
        check(False, "text encoding")


def inspect_body(payload, version, signature):
    reader = Cursor(payload)
    kind = reader.take(4)
    reader.zeros(4)
    description = None
    if signature in XYZ_TAGS:
        check(kind == b"XYZ ", "XYZ type")
        reader.take(12)  # Exactly one signed 16.16 XYZ triple.
    elif signature in TRC_TAGS:
        if kind == b"curv":
            count = reader.number(4)
            check(2 * count <= len(payload) - reader.position, "curve sample bounds")
            samples = [reader.number(2) for _ in range(count)]
            if count == 1:
                check(samples[0] > 0, "curve gamma")
            elif count > 1:
                check(samples[0] < samples[-1] and samples == sorted(samples), "curve monotonicity")
            reader.finish()
            return None
        check(kind == b"para" and version == 4, "TRC type/version")
        function = reader.number(2)
        reader.zeros(2)
        check(function in (0, 1, 2, 3, 4), "parametric function")
        parameters = [reader.number(4, signed=True) for _ in range((1, 3, 4, 5, 7)[function])]
        check(parameters[0] > 0 and (function == 0 or parameters[1] > 0), "parametric gamma/scale")
        if function >= 3:
            check(parameters[3] >= 0 and parameters[4] in range(65537), "parametric slope/threshold")
    elif signature == b"chad":
        check(kind == b"sf32", "adaptation type")
        values = [reader.number(4, signed=True) for _ in range(9)]
        determinant = (values[0] * values[4] * values[8] + values[1] * values[5] * values[6]
                       + values[2] * values[3] * values[7] - values[2] * values[4] * values[6]
                       - values[1] * values[3] * values[8] - values[0] * values[5] * values[7])
        check(determinant != 0, "singular adaptation")
    elif signature == b"chrm":
        check(kind == b"chrm" and reader.number(2) == 3 and reader.number(2) == 0, "chromaticity header")
        for _ in range(3):
            x, y = reader.number(4), reader.number(4)
            check(x > 0 and y > 0 and max(x, y) <= 65536 and x + y <= 65537, "chromaticity values")
    elif version == 2 and signature == b"desc":
        check(kind == b"desc", "legacy description type")
        count = reader.number(4)
        check(count >= 1, "legacy ASCII count")
        ascii_data = reader.take(count)
        check(ascii_data[-1] == 0 and b"\0" not in ascii_data[:-1], "legacy ASCII terminator")
        ascii_text = text(ascii_data[:-1], "ascii")
        check(reader.number(4) == 0, "unsupported legacy language")
        unicode_count = reader.number(4)
        unicode_data = reader.take(2 * unicode_count)
        unicode_text = ""
        if unicode_count:
            check(unicode_data[-2:] == b"\0\0", "legacy Unicode terminator")
            unicode_text = text(unicode_data[:-2], "utf-16-be")
            check("\0" not in unicode_text, "legacy Unicode interior terminator")
        check(reader.number(2) == 0 and reader.number(1) == 0, "unsupported legacy script")
        reader.zeros(67)
        reader.finish()
        return (ascii_text, unicode_text)
    elif version == 2 and signature == b"cprt":
        check(kind == b"text", "legacy copyright type")
        end = payload.find(b"\0", reader.position)
        check(end >= reader.position, "copyright terminator")
        description = text(reader.take(end - reader.position), "ascii")
        reader.zeros(1)
        reader.finish()
        return description
    else:
        check(signature in (b"desc", b"cprt") and version == 4 and kind == b"mluc", "description type")
        count, record_size = reader.number(4), reader.number(4)
        check(count in (1, 2) and record_size == 12, "mluc table")
        records = [(reader.take(4), reader.number(4), reader.number(4)) for _ in range(count)]
        descriptions = []
        locales = set()
        for locale, size, offset in records:
            check(locale[:2].isalpha() and locale[:2].islower()
                  and locale[2:].isalpha() and locale[2:].isupper()
                  and locale not in locales and offset == reader.position and size % 2 == 0, "mluc record")
            value = text(reader.take(size), "utf-16-be")
            check("\0" not in value, "mluc terminator")
            descriptions.append((locale, value))
            locales.add(locale)
        reader.finish()
        return descriptions
    reader.finish()
    return description


def profile(path):
    with Path(path).open("rb") as source:
        data = source.read(8 * 1024 * 1024 + 1)
    check(132 <= len(data) <= 8 * 1024 * 1024 and len(data) % 4 == 0, "size/alignment")
    header = Cursor(data[:128])
    check(header.number(4) == len(data), "declared size")
    header.take(4)  # Source CMM identifier is scrubbed, not trusted as syntax.
    version_bytes = header.take(4)
    check(version_bytes in (bytes([2, 0, 0, 0]), bytes([4, 0, 0, 0])), "version")
    version = version_bytes[0]
    check(header.take(12) == b"mntrRGB XYZ ", "class/spaces")
    calendar = [header.number(2) for _ in range(6)]
    try:
        datetime(*calendar)
    except ValueError:
        check(False, "date")
    check(header.take(4) == b"acsp", "signature")
    header.take(4)  # Primary platform is anonymized in output.
    check(header.number(4) & ~3 == 0, "flags")
    header.take(8)  # Manufacturer/model are anonymized in output.
    check(header.number(8) & ~15 == 0, "attributes")
    check(header.number(4) <= 3, "intent")
    illuminant = tuple(header.number(4, signed=True) for _ in range(3))
    check(illuminant == (63190, 65536, 54061), "PCS illuminant")
    header.take(4)
    identifier = header.take(16)
    check(version == 4 or not any(identifier), "v2 reserved ID")
    header.zeros(28)
    header.finish()
    table_reader = Cursor(data[128:])
    count = table_reader.number(4)
    check(count == 11, "tag count")
    tags = {}
    for _ in range(count):
        signature = table_reader.take(4)
        offset, size = table_reader.number(4), table_reader.number(4)
        check(signature not in tags and offset >= 132 + 12 * count and offset % 4 == 0
              and size >= 8 and offset + size <= len(data), "tag bounds/duplicate")
        tags[signature] = (offset, size, data[offset:offset + size])
    check(set(tags) == ALL_TAGS, "required tag set")
    position = 132 + 12 * count
    for start, end in sorted(set((offset, offset + size) for offset, size, _ in tags.values())):
        padding = (-position) % 4
        check(start == position + padding and not any(data[position:start]), "extent overlap/gap")
        position = end
    check(len(data) == position + (-position) % 4 and not any(data[position:]), "final padding")
    for signature, (_, _, payload) in tags.items():
        inspect_body(payload, version, signature)
    white = tuple(int.from_bytes(tags[b"wtpt"][2][index:index + 4], "big", signed=True)
                  for index in (8, 12, 16))
    check(min(white) > 0 and (version == 2 or white == illuminant), "media white")
    columns = [tuple(int.from_bytes(tags[signature][2][index:index + 4], "big", signed=True)
                     for index in (8, 12, 16)) for signature in (b"rXYZ", b"gXYZ", b"bXYZ")]
    a, d, g = columns[0]
    b, e, h = columns[1]
    c, f, i = columns[2]
    check(a * e * i + b * f * g + c * d * h != c * e * g + b * d * i + a * f * h,
          "singular RGB matrix")
    return data, tags


def sanitized_profile(path):
    data, tags = profile(path)
    check(data[24:36] == CLEAN_DATE, "canonical date")
    for start, end in ((4, 8), (40, 44), (48, 56), (80, 128)):
        check(not any(data[start:end]), "unscrubbed identifier")
    for signature in (b"desc", b"cprt"):
        payload = tags[signature][2]
        value = inspect_body(payload, data[8], signature)
        if data[8] == 4:
            check(value == [(b"enUS", TEXT)] and len(payload) == 28 + 2 * len(TEXT), "canonical mluc")
        elif signature == b"desc":
            check(value == (TEXT, "") and len(payload) == 12 + len(TEXT) + 1 + 8 + 70, "canonical legacy description")
        else:
            check(value == TEXT and len(payload) == 8 + len(TEXT) + 1, "canonical copyright")
    return data, tags


def main():
    root = Path(sys.argv[1])
    spec = importlib.util.spec_from_file_location("jpeg_inspector", sys.argv[2])
    jpeg = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(jpeg)
    for space in ("srgb", "display-p3"):
        for version in (2, 4):
            stem = f"{space}-v{version}"
            source_data, source_tags = profile(root / "ordinary" / f"{stem}.icc")
            output_data, output_tags = sanitized_profile(root / "ordinary" / f"{stem}.scrubbed.icc")
            second_data, _ = sanitized_profile(root / "ordinary" / f"{stem}.scrubbed-second.icc")
            require(source_data[24:36] == SOURCE_DATE and source_data[48:56] == b"JAUDTEST" and source_data[80:84] == b"OWND", "source identifiers absent")
            expected_header = bytearray(source_data[:128])
            expected_header[:4] = output_data[:4]
            expected_header[24:36] = CLEAN_DATE
            for start, end in ((4, 8), (40, 44), (48, 56), (80, 128)):
                expected_header[start:end] = b"\0" * (end - start)
            require(output_data[:128] == expected_header, "unrequested header semantics changed")
            for signature in COLOR_TAGS:
                require(source_tags[signature][2] == output_tags[signature][2], "color payload changed")
            require(output_data == second_data, "ICC idempotence failed")
            paths = [root / "ordinary" / f"{stem}.{suffix}.jpg" for suffix in ("input", "output", "output-second", "repeat")]
            images = [jpeg.jpeg(path) for path in paths]
            require(len({image["scan"] for image in images}) == 1, "JPEG scan changed")
            require(all(image["icc"] == output_data for image in images[1:]), "JPEG output profile mismatch")
            require(paths[1].read_bytes() == paths[2].read_bytes() == paths[3].read_bytes(), "JPEG determinism/idempotence failed")
    print("ordinary RGB ICC 2.0/4.0 sRGB/P3-primary gamma-2.2 structure and JPEG checks passed")


if __name__ == "__main__":
    main()
