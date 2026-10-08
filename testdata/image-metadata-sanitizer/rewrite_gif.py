#!/usr/bin/env python3
"""Fixture-only GIF87/89 metadata rewrite. No production execution guarantee.

Known NETSCAPE looping/comments/ordinaryICC only; unknown applications, XMP,
plain-text rendering and user-input controls reject until independently proved.
Validates LZW dictionary/indices/count without constructing a pixel raster.
"""
import importlib.util
import struct
import sys
import tempfile
from pathlib import Path

MAX_FILE = 32 * 1024 * 1024  # Fixture cap, not an upload-policy change.
MAX_META = 8 * 1024 * 1024
MAX_RECORDS = 65536


def require(condition, message):
    if not condition:
        raise ValueError("invalid GIF " + message)


class Reader:
    def __init__(self, data):
        self.data, self.offset, self.records = data, 0, 0

    def take(self, count):
        require(count >= 0 and self.offset + count <= len(self.data), "bounds")
        value = self.data[self.offset:self.offset + count]
        self.offset += count
        return value

    def byte(self):
        return self.take(1)[0]

    def record(self):
        self.records += 1
        require(self.records <= MAX_RECORDS, "record count")

    def blocks(self):
        value = bytearray()
        while True:
            self.record()
            size = self.byte()
            if not size:
                return bytes(value)
            value.extend(self.take(size))


def encoded_blocks(data):
    return b"".join(bytes([len(data[i:i + 255])]) + data[i:i + 255]
                    for i in range(0, len(data), 255)) + b"\0"


def lzw(data, minimum, palette, expected):
    require(minimum == 2, "unproved LZW minimum size")
    clear, eoi = 1 << minimum, (1 << minimum) + 1
    lengths, firsts, maxima = [0] * 16, [0] * 16, [0] * 16
    for index in range(clear):
        lengths[index], firsts[index], maxima[index] = 1, index, index
    next_code, width, previous = clear + 2, minimum + 1, None
    position, emitted, started = 0, 0, False
    while True:
        require(position + width <= len(data) * 8, "missing LZW end")
        byte_offset, shift = divmod(position, 8)
        code = (int.from_bytes(data[byte_offset:byte_offset + 3], "little") >> shift) & ((1 << width) - 1)
        position += width
        require(started or code == clear, "missing initial LZW clear")
        started = True
        if code == clear:
            next_code, width, previous = clear + 2, minimum + 1, None
            continue
        if code == eoi:
            require(emitted == expected, "LZW expansion count")
            # Narrow fixture envelope: no post-EOI bytes, zero residual bits.
            remaining = len(data) * 8 - position
            require(remaining < 8 and (not remaining or data[-1] >> (8 - remaining) == 0), "LZW trailer/padding")
            return
        if code < next_code:
            require(code < clear or code >= clear + 2, "LZW reserved code")
            length, first, maximum = lengths[code], firsts[code], maxima[code]
        else:
            require(code == next_code and previous is not None and next_code < 16, "LZW dictionary code")
            length, first, maximum = lengths[previous] + 1, firsts[previous], maxima[previous]
        require(maximum < palette and emitted + length <= expected, "LZW palette/expansion")
        emitted += length
        if previous is not None and next_code < 16:
            lengths[next_code] = lengths[previous] + 1
            firsts[next_code] = firsts[previous]
            maxima[next_code] = max(maxima[previous], first)
            next_code += 1
            if next_code == 1 << width:
                require(width < 4, "unproved LZW width/saturation")
                width += 1
        previous = code


def rewrite(source, destination, icc):
    require(Path(source).resolve() != Path(destination).resolve(), "source/output alias")
    with Path(source).open("rb") as stream:
        data = stream.read(MAX_FILE + 1)
    require(len(data) <= MAX_FILE, "fixture input size")
    reader = Reader(data)
    version = reader.take(6)
    require(version in (b"GIF87a", b"GIF89a"), "signature")
    screen = reader.take(7)
    width, height, flags, background, _ = struct.unpack("<HHBBB", screen)
    require(width > 0 and height > 0 and width * height <= 100000000, "canvas")
    global_count = 1 << ((flags & 7) + 1) if flags & 128 else 0
    global_palette = reader.take(3 * global_count)
    require(not global_count or background < global_count, "background index")
    output = bytearray(version + screen + global_palette)
    pending, frames, metadata = None, 0, 0
    profile_seen, loop_seen, trailer_seen = False, False, False
    while reader.offset < len(data):
        reader.record()
        start = reader.offset
        marker = reader.byte()
        if marker == 0x3b:
            require(reader.offset == len(data) and frames > 0 and pending is None, "trailer/control completion")
            output.extend(b";")
            trailer_seen = True
            break
        if marker == 0x21:
            require(version == b"GIF89a", "87a extension")
            label = reader.byte()
            if label == 0xf9:
                require(pending is None and reader.byte() == 4, "GCE scope/size")
                gce = reader.take(4)
                require(reader.byte() == 0 and not gce[0] & 0xe0
                        and ((gce[0] >> 2) & 7) <= 3 and not gce[0] & 2, "unsupported GCE")
                pending = gce
                output.extend(data[start:reader.offset])
            elif label == 0xfe:
                comment = reader.blocks()
                metadata += len(comment)
                require(metadata <= MAX_META, "metadata aggregate")
            elif label == 0xff:
                require(reader.byte() == 11, "application size")
                application = reader.take(11)
                value = reader.blocks()
                if application == b"NETSCAPE2.0":
                    require(not loop_seen and frames == 0 and pending is None
                            and len(value) == 3 and value[0] == 1, "loop scope/payload")
                    # Preserve one canonical-size loop sub-block; split loop
                    # blocks are unsupported even if concatenation looks valid.
                    require(data[reader.offset - 5:reader.offset] == b"\3" + value + b"\0", "loop block shape")
                    loop_seen = True
                    output.extend(data[start:reader.offset])
                elif application == b"ICCRGBG1012":
                    require(not profile_seen and frames == 0 and pending is None, "profile scope/duplicate")
                    profile_seen = True
                    metadata += len(value)
                    require(metadata <= MAX_META, "metadata aggregate")
                    with tempfile.TemporaryDirectory() as directory:
                        old, new = Path(directory) / "input.icc", Path(directory) / "output.icc"
                        old.write_bytes(value)
                        icc.rewrite(old, new)
                        value = new.read_bytes()
                    output.extend(b"!\xff\x0bICCRGBG1012" + encoded_blocks(value))
                else:
                    require(False, "unsupported application")
            else:
                require(False, "unsupported rendering/extension")
        elif marker == 0x2c:
            descriptor = reader.take(9)
            x, y, w, h, image_flags = struct.unpack("<HHHHB", descriptor)
            require(w > 0 and h > 0 and x + w <= width and y + h <= height
                    and not image_flags & 0x18, "image rectangle/reserved")
            # Interlace is not yet independently exercised in the owned slice.
            require(not image_flags & 0x40, "unproved interlace")
            local_count = 1 << ((image_flags & 7) + 1) if image_flags & 128 else 0
            reader.take(3 * local_count)
            colors = local_count or global_count
            require(colors > 0 and (pending is None or not pending[0] & 1 or pending[3] < colors), "palette/transparency")
            minimum = reader.byte()
            compressed = reader.blocks()
            lzw(compressed, minimum, colors, w * h)
            frames += 1
            require(frames <= 4096, "frame limit")
            pending = None
            output.extend(data[start:reader.offset])
        else:
            require(False, "unknown block")
    require(trailer_seen and reader.offset == len(data), "missing trailer")
    require(len(output) <= MAX_FILE, "fixture output size")
    Path(destination).write_bytes(output)


if __name__ == "__main__":
    spec = importlib.util.spec_from_file_location("ordinary_icc", sys.argv[3])
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    rewrite(sys.argv[1], sys.argv[2], module)
