#!/usr/bin/env python3
"""Bounded owned-WebP metadata rewrite prototype, not a production sanitizer.

It copies compressed VP8/VP8L/ALPH and ANIM/ANMF bytes; it never decodes or
re-encodes pixels.  The closed envelope is the eight owned alpha fixtures and
plain RGB controls: known RIFF chunks, ordinary RGB ICC 2/4, and raw TIFF EXIF.
"""
import importlib.util
import struct
import sys
import tempfile
from pathlib import Path

MAX_FILE = 32 * 1024 * 1024
MAX_META = 8 * 1024 * 1024
MAX_RECORDS = 65536
MAX_PIXELS = 100000000
PNG_METADATA = None


class WebPDomainError(ValueError):
    pass


def bad(detail):
    raise WebPDomainError("invalid WebP " + detail)


def require(condition, detail):
    if not condition:
        bad(detail)


def u24(value):
    return int.from_bytes(value, "little")


def chunk(kind, payload):
    return kind + struct.pack("<I", len(payload)) + payload + (b"\0" if len(payload) & 1 else b"")


def header(kind, payload):
    if kind == b"VP8 ":
        require(len(payload) >= 10, "VP8 header")
        tag = int.from_bytes(payload[:3], "little")
        require(not tag & 1 and ((tag >> 1) & 7) <= 3 and tag & 16, "VP8 frame tag")
        require(tag >> 5 and 10 + (tag >> 5) <= len(payload) and payload[3:6] == b"\x9d\x01\x2a", "VP8 partition")
        width, height = struct.unpack_from("<HH", payload, 6)
        require(not (width | height) & 0xc000 and width and height, "VP8 dimensions")
        return width & 0x3fff, height & 0x3fff
    require(kind == b"VP8L" and len(payload) >= 5 and payload[0] == 0x2f, "VP8L header")
    bits = int.from_bytes(payload[1:5], "little")
    require(not bits >> 29, "VP8L reserved bits")
    return (bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1


def alpha(payload):
    require(payload and not payload[0] & 0xc0 and (payload[0] & 3) <= 1 and ((payload[0] >> 4) & 3) <= 1, "ALPH header")


def records(data, start=12, end=None):
    end = len(data) if end is None else end
    out, at = [], start
    while at < end:
        require(len(out) < MAX_RECORDS and at + 8 <= end, "chunk header/count")
        kind, size = data[at:at + 4], struct.unpack_from("<I", data, at + 4)[0]
        stop = at + 8 + size
        padded = stop + (size & 1)
        require(stop <= end and padded <= end and (not size & 1 or data[stop] == 0), "chunk extent/padding")
        out.append((kind, data[at + 8:stop]))
        at = padded
    require(at == end, "chunk boundary")
    return out


def orientation(payload):
    # Reuse the proven PNG fixture TIFF parser: this deliberately admits only
    # raw classic TIFF IFD0/GPS fixture schemas, not Exif-prefixed or arbitrary
    # TIFF. Its ValueError is its documented malformed-metadata domain result.
    if PNG_METADATA is None:
        raise RuntimeError("WebP metadata parser unavailable")
    try:
        result = PNG_METADATA.orientation(payload)
    except ValueError as error:
        bad(str(error))
    # The shared parser rejects invalid offsets/types/counts, duplicate tags,
    # nested GPS errors and next-IFD thumbnails. Reject covert trailing bytes
    # beyond the last referenced TIFF field as well.
    endian = "<" if payload[:2] == b"II" else ">"
    widths, seen = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8}, set()
    extents, external, structural = [], [], []
    def visit(offset):
        require(offset not in seen, "EXIF IFD overlap")
        seen.add(offset)
        count = struct.unpack_from(endian + "H", payload, offset)[0]
        directory = (offset, offset + 2 + count * 12 + 4)
        extents.append(directory)
        structural.append(directory)
        for index in range(count):
            at = offset + 2 + index * 12
            tag, kind, number = struct.unpack_from(endian + "HHI", payload, at)
            size = widths[kind] * number
            start = at + 8 if size <= 4 else struct.unpack_from(endian + "I", payload, at + 8)[0]
            extents.append((start, start + size))
            if size > 4:
                external.append((start, start + size))
            if tag == 0x8825:
                visit(struct.unpack_from(endian + "I", payload, at + 8)[0])
    visit(struct.unpack_from(endian + "I", payload, 4)[0])
    # Every byte has a validated structural or value extent; overlap is covert
    # structure, and uncovered bytes are an unsupported private trailer.
    for start, end in sorted(extents):
        require(start >= 8 and end <= len(payload), "EXIF extent")
    # Inline values live inside their own IFD entry; only out-of-line values
    # must be disjoint from IFD structures and from one another.
    for index, (start, end) in enumerate(structural):
        require(all(end <= left or right <= start for left, right in structural[index + 1:]),
                "EXIF IFD structural overlap")
    for index, (start, end) in enumerate(external):
        require(all(end <= left or right <= start for left, right in structural), "EXIF external/IFD overlap")
        require(all(index == other or end <= left or right <= start
                    for other, (left, right) in enumerate(external)), "EXIF external value overlap")
    covered = bytearray(len(payload))
    covered[:8] = b"\1" * 8
    for start, end in extents: covered[start:end] = b"\1" * (end - start)
    require(all(value == 0 for value, used in zip(payload, covered) if not used), "EXIF trailing data")
    return result


def xmp(payload):
    if PNG_METADATA is None:
        raise RuntimeError("WebP metadata parser unavailable")
    try:
        PNG_METADATA.descriptive_text(b"XML:com.adobe.xmp", payload)
    except ValueError as error:
        bad(str(error))


def rewrite(source, destination, icc_module):
    source, destination = Path(source), Path(destination)
    require(source.resolve() != destination.resolve(), "source/output alias")
    data = source.read_bytes()
    require(len(data) <= MAX_FILE and len(data) >= 12 and data[:4] == b"RIFF" and data[8:12] == b"WEBP", "RIFF/WebP")
    require(struct.unpack_from("<I", data, 4)[0] + 8 == len(data), "RIFF extent")
    original = records(data)
    kinds = [kind for kind, _ in original]
    known = {b"VP8 ", b"VP8L", b"ALPH", b"VP8X", b"ICCP", b"EXIF", b"XMP ", b"ANIM", b"ANMF"}
    require(kinds and set(kinds) <= known, "unknown/empty chunk")
    require(any(kind in {b"VP8 ", b"VP8L", b"ANMF"} for kind in kinds), "missing rendering image")
    require(all(kinds.count(kind) <= 1 for kind in (b"VP8X", b"ICCP", b"EXIF", b"XMP ", b"ANIM")), "duplicate singleton")
    require(b"VP8X" not in kinds or kinds[0] == b"VP8X", "VP8X order")
    first_render = min(index for index, kind in enumerate(kinds)
                       if kind in {b"ALPH", b"VP8 ", b"VP8L", b"ANIM", b"ANMF"})
    if b"ICCP" in kinds:
        require(b"VP8X" in kinds and kinds.index(b"VP8X") < kinds.index(b"ICCP") < first_render, "ICCP order")
    if b"EXIF" in kinds:
        require(kinds.index(b"EXIF") > max(index for index, kind in enumerate(kinds) if kind in {b"VP8 ", b"VP8L", b"ANMF"}), "EXIF order")
    if b"XMP " in kinds:
        require(kinds.index(b"XMP ") == len(kinds) - 1 and b"EXIF" in kinds and kinds.index(b"EXIF") < kinds.index(b"XMP "), "XMP order")
    animated = b"ANMF" in kinds
    require((b"ANIM" in kinds) == animated and (not animated or kinds.index(b"ANIM") < kinds.index(b"ANMF")), "animation pairing/order")
    require(not animated or not any(kind in {b"VP8 ", b"VP8L", b"ALPH"} for kind in kinds), "animated top-level image")
    metadata = sum(len(value) for kind, value in original if kind in {b"ICCP", b"EXIF", b"XMP "})
    require(metadata <= MAX_META, "metadata aggregate")
    canvas = None
    flags = 0
    if b"VP8X" in kinds:
        vp8x = original[kinds.index(b"VP8X")][1]
        require(len(vp8x) == 10 and not any(vp8x[1:4]) and not vp8x[0] & 0xc1, "VP8X reserved")
        flags, canvas = vp8x[0], (u24(vp8x[4:7]) + 1, u24(vp8x[7:10]) + 1)
    nested_alpha = False
    nested_lossless_alpha = False
    nested_records = 0
    for kind, value in original:
        if kind == b"ALPH": alpha(value)
        elif kind in (b"VP8 ", b"VP8L"): header(kind, value)
        elif kind == b"ANIM":
            require(len(value) == 6, "ANIM size")
        elif kind == b"ANMF":
            require(canvas is not None and len(value) >= 16, "ANMF canvas/header")
            x, y = u24(value[:3]) * 2, u24(value[3:6]) * 2
            width, height = u24(value[6:9]) + 1, u24(value[9:12]) + 1
            require(value[15] == 2 and (x, y, width, height) == (0, 0, *canvas), "ANMF closed frame control")
            inner = records(value, 16, len(value))
            nested_records += len(inner)
            require(len(original) + nested_records <= MAX_RECORDS, "total record count")
            inner_kinds = [name for name, _ in inner]
            require(inner_kinds in ([b"VP8 "], [b"VP8L"], [b"ALPH", b"VP8 "]), "ANMF subchunks")
            if inner_kinds[0] == b"ALPH": alpha(inner[0][1]); nested_alpha = True
            if inner[-1][0] == b"VP8L":
                nested_lossless_alpha |= bool(int.from_bytes(inner[-1][1][1:5], "little") & (1 << 28))
            require(header(*inner[-1]) == (width, height), "ANMF dimensions")
    require(not animated or kinds.count(b"ANMF") == 3, "animated frame count")
    direct = [(kind, value) for kind, value in original if kind in {b"VP8 ", b"VP8L", b"ALPH"}]
    if not animated:
        image = [(kind, value) for kind, value in direct if kind != b"ALPH"]
        require(len(image) == 1 and [kind for kind, _ in direct] in ([image[0][0]], [b"ALPH", b"VP8 "]), "static image")
        require(image[0][0] != b"VP8L" or not any(kind == b"ALPH" for kind, _ in direct), "ALPH VP8L")
        encoded = header(*image[0])
        require(canvas is None or canvas == encoded, "static canvas")
        canvas = encoded
    require(canvas is not None and canvas[0] * canvas[1] <= MAX_PIXELS, "canvas bounds")
    expected = (0x20 if b"ICCP" in kinds else 0) | (0x08 if b"EXIF" in kinds else 0) | (0x04 if b"XMP " in kinds else 0) | (0x02 if animated else 0)
    has_alpha = nested_alpha or nested_lossless_alpha or any(kind == b"ALPH" for kind, _ in direct) or any(kind == b"VP8L" and int.from_bytes(value[1:5], "little") & (1 << 28) for kind, value in direct)
    expected |= 0x10 if has_alpha else 0
    if not ((flags == expected) if b"VP8X" in kinds else not expected):
        bad(f"VP8X feature flags {flags:#x}/{expected:#x}")
    # Admit the original witnessed metadata layout and its canonical outputs,
    # not the power set of individually known optional chunks. Canonical forms
    # must already have scrubbed ICC and, if present, minimal orientation TIFF.
    plain = kinds in ([b"VP8 "], [b"VP8L"])
    core = ([b"VP8X", b"ICCP", b"ANIM", b"ANMF", b"ANMF", b"ANMF"] if animated
            else [b"VP8X", b"ICCP"] + [kind for kind, _ in direct])
    suffix = kinds[len(core):]
    require(plain or (has_alpha and kinds[:len(core)] == core
                     and suffix in ([], [b"EXIF"], [b"EXIF", b"XMP "])),
            "unsupported chunk layout")
    canonical_input = not plain and b"XMP " not in kinds
    if canonical_input and b"EXIF" in kinds:
        payload = next(value for kind, value in original if kind == b"EXIF")
        require(orientation(payload) == payload, "unsupported partial metadata layout")
    retained_orientation = (b"EXIF" in kinds
                            and orientation(next(value for kind, value in original if kind == b"EXIF")) is not None)
    output = []
    for kind, value in original:
        if kind == b"XMP ":
            xmp(value)
            continue
        if kind == b"EXIF":
            value = orientation(value)
            if value is None: continue
        elif kind == b"ICCP":
            with tempfile.TemporaryDirectory() as directory:
                old, new = Path(directory) / "input.icc", Path(directory) / "output.icc"
                old.write_bytes(value)
                try:
                    icc_module.rewrite(old, new)
                except ValueError as error:
                    # The shared ordinary-ICC helper uses ValueError solely for
                    # its documented rejected-profile domain.
                    bad(str(error))
                rewritten = new.read_bytes()
                require(not canonical_input or rewritten == value, "unsupported partial metadata layout")
                value = rewritten
        if kind == b"VP8X":
            new_flags = flags & ~0x04
            if not retained_orientation: new_flags &= ~0x08
            value = bytes([new_flags]) + value[1:]
        output.append((kind, value))
    body = b"".join(chunk(kind, value) for kind, value in output)
    result = b"RIFF" + struct.pack("<I", len(body) + 4) + b"WEBP" + body
    require(len(result) <= MAX_FILE, "output size")
    destination.write_bytes(result)


if __name__ == "__main__":
    spec = importlib.util.spec_from_file_location("webp_icc", sys.argv[3])
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    png_spec = importlib.util.spec_from_file_location("webp_png_metadata", sys.argv[4])
    PNG_METADATA = importlib.util.module_from_spec(png_spec)
    png_spec.loader.exec_module(PNG_METADATA)
    rewrite(sys.argv[1], sys.argv[2], module)
