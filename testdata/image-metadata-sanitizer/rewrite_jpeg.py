#!/usr/bin/env python3
"""Owned baseline JPEG byte-copy witness; NOT entropy/upload validation.

T.81 marker framing, JFIF 1.01 unitless 1:1 without thumbnails, Adobe transform 1,
8-bit sequential Huffman YCbCr, one interleaved scan; no restart admission.
Progressive, other APP schemas, previews and other codecs explicitly reject.
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
XMP = b"http://ns.adobe.com/xap/1.0/\0"


class JPEGDomainError(ValueError):
    pass


def require(condition, detail):
    if not condition:
        raise JPEGDomainError("invalid JPEG " + detail)


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def segment(marker, payload):
    return bytes((255, marker)) + struct.pack(">H", len(payload) + 2) + payload


def records(data):
    require(data[:2] == b"\xff\xd8", "signature")
    at, result, scanned = 2, [], False
    while at < len(data):
        require(len(result) < MAX_RECORDS and at + 2 <= len(data)
                and data[at] == 255 and data[at + 1] not in (0, 255), "marker/count")
        marker = data[at + 1]
        if marker == 0xd9:
            require(scanned and at + 2 == len(data), "EOI extent/order")
            return result
        require(not scanned, "multiple scans/late markers")
        require(marker not in range(0xd0, 0xd9) and marker != 1
                and at + 4 <= len(data), "standalone/header")
        length = int.from_bytes(data[at + 2:at + 4], "big")
        end = at + 2 + length
        require(length >= 2 and end <= len(data), "segment extent")
        result.append((marker, data[at + 4:end]))
        at = end
        if marker == 0xda:
            start = at
            while at < len(data):
                if data[at] != 255:
                    at += 1
                    continue
                require(at + 1 < len(data), "scan extent")
                if data[at + 1] == 0:
                    at += 2
                    continue
                require(data[at + 1] == 0xd9, "unsupported scan marker/restart")
                break
            require(at > start, "empty scan")
            result.append((0, data[start:at]))
            scanned = True
    require(False, "missing EOI")


def rewrite(source, destination, icc, metadata):
    source, destination = Path(source), Path(destination)
    require(source.resolve() != destination.resolve(), "source/output alias")
    with source.open("rb") as stream:
        data = stream.read(MAX_FILE + 1)
    require(len(data) <= MAX_FILE, "fixture file budget")
    items = records(data)
    require(items and items[0][0] == 0xe0, "JFIF must immediately follow SOI")
    require(sum(len(p) for m, p in items if 0xe0 <= m <= 0xef or m == 0xfe) <= MAX_META,
            "metadata aggregate")
    seen, quant, huffman, chunks = set(), set(), set(), []
    output = bytearray(b"\xff\xd8")
    for marker, payload in items:
        require(marker in {0, 0xe0, 0xe1, 0xe2, 0xee, 0xfe, 0xdb, 0xc4, 0xc0, 0xda},
                "unsupported marker")
        if marker in {0xe0, 0xee, 0xc0, 0xda}:
            require(marker not in seen, "duplicate singleton")
        if marker >= 0xe0 or marker == 0xfe:
            require(0xc0 not in seen, "APP/COM order")
        if marker == 0xe0:
            require(payload == b"JFIF\0\1\1\0\0\1\0\1\0\0", "unsupported JFIF/thumbnail")
        elif marker == 0xee:
            require(payload == b"Adobe\0d\0\0\0\0\1", "unsupported Adobe")
        elif marker == 0xe1:
            if payload.startswith(b"Exif\0\0"):
                require("exif" not in seen, "duplicate EXIF")
                seen.add("exif")
                try:
                    clean = metadata.orientation(payload[6:])
                except metadata.WebPDomainError as error:
                    raise JPEGDomainError("invalid JPEG EXIF") from error
                if clean is not None:
                    output.extend(segment(marker, b"Exif\0\0" + clean))
            else:
                require(payload.startswith(XMP) and "xmp" not in seen, "unsupported/duplicate APP1")
                seen.add("xmp")
                try:
                    metadata.xmp(payload[len(XMP):])
                except metadata.WebPDomainError as error:
                    raise JPEGDomainError("invalid JPEG XMP") from error
            continue
        elif marker == 0xe2:
            require(payload.startswith(b"ICC_PROFILE\0") and len(payload) > 14, "APP2 schema")
            chunks.append(payload)
            continue
        elif marker == 0xfe:
            # COM has no JPEG rendering semantics (T.81 B.2.4.5).
            continue
        elif marker == 0xdb:
            at = 0
            while at < len(payload):
                table = payload[at]
                require(table <= 3 and table not in quant and at + 65 <= len(payload), "DQT")
                require(all(payload[at + 1:at + 65]), "zero quantizer")
                quant.add(table)
                at += 65
            require(at == len(payload) and at > 0, "DQT extent")
        elif marker == 0xc4:
            at = 0
            while at < len(payload):
                require(at + 17 <= len(payload), "DHT header")
                table, counts = payload[at], payload[at + 1:at + 17]
                total, available = sum(counts), 1
                require(table in (0, 1, 0x10, 0x11) and table not in huffman
                        and total > 0 and at + 17 + total <= len(payload), "DHT extent/id")
                for count in counts:
                    available = available * 2 - count
                    require(available > 0, "DHT oversubscribed/all-ones code")
                symbols = payload[at + 17:at + 17 + total]
                require(len(set(symbols)) == total and
                        (all(v <= 11 for v in symbols) if table < 16 else
                         all(v in (0, 0xf0) or 1 <= (v & 15) <= 10 for v in symbols)), "DHT symbols")
                huffman.add(table)
                at += 17 + total
        elif marker == 0xc0:
            require(len(payload) == 15 and payload[0] == 8 and payload[5] == 3, "baseline RGB frame")
            height, width = struct.unpack_from(">HH", payload, 1)
            require(width and height and width * height <= MAX_PIXELS, "pixel budget")
            require(payload[6:] in (b"\1\x11\0\2\x11\1\3\x11\1",
                                     b"\1\x22\0\2\x11\1\3\x11\1"), "unproved components/sampling")
            require({0, 1} <= quant, "missing quantizers")
        elif marker == 0xda:
            require(0xc0 in seen and payload == b"\3\1\0\2\x11\3\x11\0?\0"
                    and {0, 1, 0x10, 0x11} <= huffman, "scan header/order/tables")
        seen.add(marker)
        output.extend(payload if marker == 0 else segment(marker, payload))
    require({0xc0, 0xda, 0xe0} <= seen, "frame/color identification")
    if chunks:
        count = chunks[0][13]
        require(count == len(chunks) and [p[12] for p in chunks] == list(range(1, count + 1))
                and all(p[13] == count for p in chunks), "ICC sequence/count")
        with tempfile.TemporaryDirectory() as directory:
            old, new = Path(directory) / "old.icc", Path(directory) / "new.icc"
            old.write_bytes(b"".join(p[14:] for p in chunks))
            try:
                icc.rewrite(old, new)
            except ValueError as error:
                # Only the helper's explicit profile-domain diagnostic is mapped.
                # Unexpected ValueError/programming failures remain infrastructure.
                if not str(error).startswith("invalid ICC "):
                    raise
                raise JPEGDomainError("invalid JPEG ICC") from error
            profile = new.read_bytes()
        parts = [profile[i:i + 65519] for i in range(0, len(profile), 65519)]
        encoded = b"".join(segment(0xe2, b"ICC_PROFILE\0" + bytes((i + 1, len(parts))) + p)
                           for i, p in enumerate(parts))
        # JFIF APP0 immediately follows SOI; canonical ICC follows APP0.
        after_jfif = 2 + len(segment(0xe0, items[0][1]))
        output[after_jfif:after_jfif] = encoded
    output.extend(b"\xff\xd9")
    require(len(output) <= MAX_FILE, "output budget")
    destination.write_bytes(output)


if __name__ == "__main__":
    icc = load(sys.argv[3], "jpeg_icc")
    metadata = load(sys.argv[4], "jpeg_metadata")
    metadata.PNG_METADATA = load(sys.argv[5], "jpeg_png_metadata")
    try:
        rewrite(sys.argv[1], sys.argv[2], icc, metadata)
    except JPEGDomainError as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
