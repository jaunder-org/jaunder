#!/usr/bin/env python3
"""Bounded PNG/APNG fixture prototype, NOT a production sanitizer.

RGB/RGBA8, non-interlaced, known standard presentation chunks and ordinary
ICC2.0/4.0 only. Classic TIFF IFD0/GPS subset; no thumbnails or vendor graphs.
Fixture file cap is not a proposed upload limit. Task2 execution proof remains.
"""
import importlib.util
import struct
import sys
import tempfile
import zlib
import xml.etree.ElementTree as ET
from pathlib import Path

MAGIC = b"\x89PNG\r\n\x1a\n"
MAX_FILE = 32 * 1024 * 1024
MAX_META = 8 * 1024 * 1024
MAX_RECORDS = 65536
TEXT = {b"tEXt", b"zTXt", b"iTXt"}
SINGLE = {b"IHDR", b"IEND", b"iCCP", b"eXIf", b"tIME", b"sRGB", b"gAMA", b"cHRM", b"sBIT", b"pHYs", b"bKGD", b"acTL", b"PLTE", b"tRNS"}
RETAIN = {b"IHDR", b"IEND", b"IDAT", b"fdAT", b"fcTL", b"acTL", b"sRGB", b"gAMA", b"cHRM", b"sBIT", b"pHYs", b"bKGD", b"PLTE", b"tRNS"}


def invalid(detail):
    raise ValueError("invalid PNG " + detail)


def require(condition, detail):
    if not condition:
        invalid(detail)


def inflate(data, limit):
    try:
        reader = zlib.decompressobj()
        result = reader.decompress(data, limit + 1)
    except zlib.error:
        invalid("compressed metadata")
    require(len(result) <= limit and reader.eof and not reader.unused_data
            and not reader.unconsumed_tail, "metadata expansion/trailer")
    return result


def keyword(data):
    end = data.find(b"\0")
    require(1 <= end <= 79, "keyword size")
    name = data[:end]
    require(all(32 <= c <= 126 or 161 <= c <= 255 for c in name)
            and not name.startswith(b" ") and not name.endswith(b" ")
            and b"  " not in name, "keyword encoding")
    return end


def descriptive_text(name, value):
    # Application-specific XMP can carry orientation/HDR rendering requirements.
    # Admit only the known descriptive fixture schema; never discard unknown
    # namespaces/attributes merely because PNG's generic text decoder ignores it.
    if name != b"XML:com.adobe.xmp":
        require(name in {b"Title", b"Author", b"Description", b"Copyright", b"Creation Time",
                         b"Software", b"Disclaimer", b"Warning", b"Source", b"Comment"},
                "unsupported text semantics")
        return
    require(len(value) <= 4096 and b"<!DOCTYPE" not in value and b"<!ENTITY" not in value,
            "fixture XMP size/declarations")
    try:
        document = ET.fromstring(value.decode("utf-8"))
    except (ET.ParseError, UnicodeDecodeError):
        invalid("XMP syntax")
    tags = ["{adobe:ns:meta/}xmpmeta", "{http://www.w3.org/1999/02/22-rdf-syntax-ns#}RDF",
            "{http://www.w3.org/1999/02/22-rdf-syntax-ns#}Description"]
    nodes = list(document.iter())
    require([node.tag for node in nodes] == tags and len(document) == 1
            and len(document[0]) == 1 and len(document[0][0]) == 0, "unsupported XMP structure")
    for node in nodes:
        require(not (node.text or "").strip() and not (node.tail or "").strip(), "unsupported XMP text")
        require(all(key in {"{http://purl.org/dc/elements/1.1/}description",
                           "{http://purl.org/dc/elements/1.1/}creator",
                           "{http://purl.org/dc/elements/1.1/}rights"}
                    for key in node.attrib) and (node is nodes[-1] or not node.attrib),
                "unsupported XMP rendering/attribute")


def orientation(data):
    require(len(data) >= 8 and data[:2] in (b"II", b"MM"), "TIFF header")
    endian = "<" if data[:2] == b"II" else ">"
    require(struct.unpack_from(endian + "H", data, 2)[0] == 42, "TIFF version")
    root = struct.unpack_from(endian + "I", data, 4)[0]
    seen, records, value = set(), 0, 1
    pending = [(root, False)]
    while pending:
        offset, gps = pending.pop()
        require(offset >= 8 and offset % 2 == 0 and offset not in seen
                and offset + 2 <= len(data), "TIFF directory")
        seen.add(offset)
        count = struct.unpack_from(endian + "H", data, offset)[0]
        records += count
        end = offset + 2 + count * 12
        require(records <= MAX_RECORDS and end + 4 <= len(data), "TIFF records")
        require(struct.unpack_from(endian + "I", data, end)[0] == 0, "unsupported TIFF next directory")
        tags = set()
        for index in range(count):
            pos = offset + 2 + index * 12
            tag, kind, number = struct.unpack_from(endian + "HHI", data, pos)
            require(tag not in tags, "duplicate TIFF tag")
            tags.add(tag)
            widths = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8}
            require(kind in widths and number > 0, "TIFF field type/count")
            size = number * widths[kind]
            start = pos + 8 if size <= 4 else struct.unpack_from(endian + "I", data, pos + 8)[0]
            require(size <= MAX_META and start + size <= len(data)
                    and (size <= 4 or start >= 8 and start % 2 == 0), "TIFF field bounds")
            payload = data[start:start + size]
            if gps:
                shapes = {0: (1, 4), 1: (2, 2), 2: (5, 3), 3: (2, 2), 4: (5, 3)}
                require(tag in shapes and (kind, number) == shapes[tag], "unsupported GPS field")
                if kind == 5:
                    require(all(denominator for _, denominator in struct.iter_unpack(endian + "II", payload)), "GPS rational")
            elif tag == 0x0112:
                require(kind == 3 and number == 1, "orientation type")
                value = struct.unpack(endian + "H", payload)[0]
                require(1 <= value <= 8, "orientation value")
            elif tag == 0x8825:
                require(kind == 4 and number == 1, "GPS pointer")
                pending.append((struct.unpack(endian + "I", payload)[0], True))
            else:
                require(tag in {0x010e, 0x010f, 0x0110, 0x0132, 0x013b, 0x8298}
                        and kind == 2 and payload[-1] == 0, "unsupported TIFF description")
    if value == 1:
        return None
    return b"II" + struct.pack("<HIHHHIH", 42, 8, 1, 0x0112, 3, 1, value) + b"\0" * 6


def encode(kind, payload):
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))


class Scanlines:
    """Validate deflate/Adler/filter-byte structure in bounded blocks.

    Never undo filters, reconstruct pixels, render or recompress image data.
    """
    def __init__(self, width, height, channels):
        self.reader = zlib.decompressobj()
        self.stride = 1 + width * channels
        self.expected = self.stride * height
        self.count = 0

    def feed(self, compressed):
        pending = compressed
        while pending:
            require(not self.reader.eof, "image stream trailer")
            try:
                block = self.reader.decompress(pending, 32768)
            except zlib.error:
                invalid("image compression")
            require(self.count + len(block) <= self.expected, "image expansion")
            first = (-self.count) % self.stride
            require(all(block[index] <= 4 for index in range(first, len(block), self.stride)), "row filter")
            self.count += len(block)
            require(not self.reader.unused_data, "image stream trailer")
            pending = self.reader.unconsumed_tail

    def finish(self):
        require(self.reader.eof and self.count == self.expected, "image stream completion")


def rewrite(source, destination, icc_module):
    require(Path(source).resolve() != Path(destination).resolve(), "source/output alias")
    with Path(source).open("rb") as stream:
        data = stream.read(MAX_FILE + 1)
    require(len(data) <= MAX_FILE and data[:8] == MAGIC, "fixture size/signature")
    position, records, metadata, sequence, frames = 8, 0, 0, 0, 0
    seen, output = set(), bytearray(MAGIC)
    width = height = color = 0
    idat, closed, animated, frame_data, frame_idat = False, False, False, False, False
    declared = 0
    while position < len(data):
        records += 1
        require(records <= MAX_RECORDS and position + 12 <= len(data), "chunk header/count")
        length, kind = struct.unpack_from(">I4s", data, position)
        end = position + length + 12
        require(length <= 0x7fffffff and end <= len(data), "chunk length")
        require(all(65 <= c <= 90 or 97 <= c <= 122 for c in kind)
                and 65 <= kind[2] <= 90, "chunk type")
        payload = data[position + 8:end - 4]
        require(zlib.crc32(kind + payload) == struct.unpack_from(">I", data, end - 4)[0], "CRC")
        position = end
        require(kind in SINGLE | RETAIN | TEXT, "unsupported chunk")
        require(records != 1 or kind == b"IHDR", "IHDR first")
        if kind in SINGLE:
            require(kind not in seen, "duplicate chunk")
            seen.add(kind)
        if idat and kind != b"IDAT":
            closed = True
        if kind == b"IHDR":
            require(length == 13, "IHDR size")
            width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", payload)
            require(width > 0 and height > 0 and width * height <= 100000000
                    and depth == 8 and color in (2, 6) and compression == filtering == interlace == 0,
                    "unsupported IHDR")
            image_stream = Scanlines(width, height, 3 if color == 2 else 4)
        elif kind == b"IDAT":
            require(not closed and (not animated or frames == 0 or frame_idat), "IDAT association/order")
            idat = True
            image_stream.feed(payload)
            if frame_idat:
                frame_data = True
        elif kind == b"acTL":
            require(not idat and length == 8, "acTL order/size")
            declared, _ = struct.unpack(">II", payload)
            require(1 <= declared <= 4096, "frame count")
            animated = True
        elif kind == b"fcTL":
            require(animated and length == 26 and (frames == 0 or frame_data), "fcTL association")
            seq, w, h, x, y, _, _, disposal, blend = struct.unpack(">5I2H2B", payload)
            require(seq == sequence and w > 0 and h > 0 and x + w <= width and y + h <= height
                    and disposal <= 2 and blend <= 1, "frame control")
            if frames:
                frame_stream.finish()
            elif idat:
                image_stream.finish()
            sequence += 1
            frames += 1
            require(frames <= declared, "excess frames")
            frame_idat = not idat
            require(not frame_idat or (frames == 1 and (w, h, x, y) == (width, height, 0, 0)), "first-frame rectangle")
            frame_data = False
            frame_stream = image_stream if frame_idat else Scanlines(w, h, 3 if color == 2 else 4)
        elif kind == b"fdAT":
            require(animated and idat and frames > 0 and not frame_idat and length > 4, "fdAT association")
            require(struct.unpack_from(">I", payload)[0] == sequence, "frame sequence")
            sequence += 1
            frame_stream.feed(payload[4:])
            frame_data = True
        elif kind == b"IEND":
            require(length == 0 and end == len(data) and idat
                    and (not animated or frames == declared and frame_data), "IEND/frame completion")
            if animated:
                frame_stream.finish()
            else:
                image_stream.finish()
        elif kind in {b"iCCP", b"sRGB", b"gAMA", b"cHRM", b"sBIT", b"PLTE", b"tRNS", b"bKGD", b"pHYs", b"eXIf"}:
            require(not idat, "presentation chunk order")
            if kind in {b"iCCP", b"sRGB", b"gAMA", b"cHRM", b"sBIT"}:
                require(b"PLTE" not in seen, "color chunk order")
        if kind in TEXT | {b"iCCP", b"eXIf", b"tIME"}:
            metadata += length
            require(metadata <= MAX_META, "metadata aggregate")
        if kind in TEXT:
            separator = keyword(payload)
            if kind == b"tEXt":
                expanded = payload[separator + 1:]
                require(b"\0" not in expanded, "text terminator")
            elif kind == b"zTXt":
                require(separator + 2 <= length and payload[separator + 1] == 0, "text compression")
                expanded = inflate(payload[separator + 2:], MAX_META - metadata)
                metadata += len(expanded)
            else:
                rest = payload[separator + 1:]
                require(len(rest) >= 4 and rest[0] in (0, 1) and rest[1] == 0, "international text compression")
                fields = rest[2:].split(b"\0", 2)
                require(len(fields) == 3, "international text fields")
                # PNG3 uses registered BCP47 language tags, case-insensitively.
                # This fixture envelope admits only unspecified/English/English
                # UK, not an unproved general grammar or mutable registry lookup.
                require(fields[0].lower() in {b"", b"en", b"en-gb"},
                        "unsupported/invalid fixture language tag")
                expanded = inflate(fields[2], MAX_META - metadata) if rest[0] else fields[2]
                try:
                    fields[1].decode("utf-8")
                    expanded.decode("utf-8")
                except UnicodeDecodeError:
                    invalid("international text encoding")
                if rest[0]:
                    metadata += len(expanded)
            descriptive_text(payload[:separator], expanded)
            continue
        if kind == b"tIME":
            from datetime import datetime
            require(length == 7, "time size")
            try:
                year, month, day, hour, minute, second = struct.unpack(">H5B", payload)
                datetime(year, month, day, hour, minute, min(second, 59))
                require(second <= 60, "time second")
            except ValueError:
                invalid("time value")
            continue
        if kind == b"eXIf":
            payload = orientation(payload)
            if payload is None:
                continue
        elif kind == b"iCCP":
            require(not {b"sRGB", b"gAMA", b"cHRM"}.intersection(seen), "unproved color fallbacks")
            separator = keyword(payload)
            require(separator + 2 <= length and payload[separator + 1] == 0, "profile compression")
            profile = inflate(payload[separator + 2:], MAX_META - metadata)
            metadata += len(profile)
            with tempfile.TemporaryDirectory() as directory:
                old, new = Path(directory) / "input.icc", Path(directory) / "output.icc"
                old.write_bytes(profile)
                icc_module.rewrite(old, new)
                payload = b"sanitized\0\0" + zlib.compress(new.read_bytes())
        elif kind == b"sRGB":
            require(not {b"iCCP", b"gAMA", b"cHRM"}.intersection(seen)
                    and length == 1 and payload[0] <= 3, "sRGB/fallbacks")
        elif kind == b"gAMA":
            require(not {b"iCCP", b"sRGB"}.intersection(seen)
                    and length == 4 and int.from_bytes(payload, "big") > 0, "gamma/fallbacks")
        elif kind == b"cHRM":
            require(not {b"iCCP", b"sRGB"}.intersection(seen) and length == 32,
                    "chromaticity size/fallbacks")
            for x, y in struct.iter_unpack(">II", payload):
                require(0 < y <= 100000 and x <= 100000 and x + y <= 100000, "chromaticity")
        elif kind == b"sBIT":
            require(length == (3 if color == 2 else 4) and all(1 <= c <= 8 for c in payload), "significant bits")
        elif kind == b"PLTE":
            require(length > 0 and length % 3 == 0 and length <= 768 and b"tRNS" not in seen and b"bKGD" not in seen, "palette")
        elif kind in {b"tRNS", b"bKGD"}:
            require(length == 6 and (kind != b"tRNS" or color == 2)
                    and all(v[0] <= 255 for v in struct.iter_unpack(">H", payload)), "background/transparency")
        elif kind == b"pHYs":
            require(length == 9 and payload[8] <= 1, "physical dimensions")
        output.extend(encode(kind, payload))
    require(b"IEND" in seen, "missing IEND")
    require(len(output) <= MAX_FILE, "fixture output size")
    Path(destination).write_bytes(output)


if __name__ == "__main__":
    spec = importlib.util.spec_from_file_location("icc_fixture", sys.argv[3])
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    rewrite(sys.argv[1], sys.argv[2], module)
