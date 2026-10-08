#!/usr/bin/env python3
"""Executable domain-rejection and independent-observer controls for the pilot."""
import importlib.util
import struct
import subprocess
import sys
import zlib
from pathlib import Path
from PIL import Image, ImageOps


def records(data):
    result, offset = [], 8
    while offset < len(data):
        length = int.from_bytes(data[offset:offset + 4], "big")
        result.append((data[offset + 4:offset + 8], data[offset + 8:offset + 8 + length]))
        offset += 12 + length
    return result


def packed(rows):
    return bytes((137, 80, 78, 71, 13, 10, 26, 10)) + b"".join(
        struct.pack(">I", len(value)) + kind + value + struct.pack(">I", zlib.crc32(kind + value))
        for kind, value in rows)


def replace(rows, kind, transform, occurrence=0):
    output, count = [], 0
    for key, value in rows:
        if key == kind:
            if count == occurrence:
                value = transform(value)
            count += 1
        output.append((key, value))
    return output


def exif_field(data, tag, transform):
    result = bytearray(data)
    root = int.from_bytes(result[4:8], "little")
    count = int.from_bytes(result[root:root + 2], "little")
    for index in range(count):
        position = root + 2 + index * 12
        if int.from_bytes(result[position:position + 2], "little") == tag:
            result[position:position + 12] = transform(bytes(result[position:position + 12]))
            return bytes(result)
    raise AssertionError("control tag absent")


def canvases(path):
    with Image.open(path) as image:
        frames = []
        for index in range(image.n_frames):
            image.seek(index)
            frame = ImageOps.exif_transpose(image.copy()).convert("RGBA")
            frames.append((frame.size, frame.tobytes()))
        return frames


if __name__ == "__main__":
    root, rewrite, icc, observer = sys.argv[1:]
    root = Path(root)
    work = root / "controls"
    work.mkdir(exist_ok=True)
    source = (root / "input" / "apng-default-v4.png").read_bytes()
    rows = records(source)
    cases = {"signature": b"BAD" + source[3:], "truncated": source[:-1],
             "trailer": source + b"private trailer", "CRC": source[:29] + bytes([source[29] ^ 1]) + source[30:]}
    for kind in (b"IHDR", b"IEND", b"IDAT", b"acTL", b"fcTL", b"fdAT"):
        cases["missing-" + kind.decode()] = packed([(k, v) for k, v in rows if k != kind])
    for kind in (b"IHDR", b"iCCP", b"eXIf", b"acTL", b"IEND"):
        index = next(i for i, (k, _) in enumerate(rows) if k == kind)
        cases["duplicate-" + kind.decode()] = packed(rows[:index] + [rows[index]] + rows[index:])
    cases["unknown-safe-private"] = packed(rows[:1] + [(b"prIv", b"private")] + rows[1:])
    cases["unsupported-HDR"] = packed(rows[:1] + [(b"cICP", bytes((9, 16, 0, 1)))] + rows[1:])
    for label, index, value in (("depth16", 8, 16), ("interlace", 12, 1), ("palette", 9, 3)):
        cases[label] = packed(replace(rows, b"IHDR", lambda p, i=index, v=value: p[:i] + bytes([v]) + p[i + 1:]))
    cases["zero-canvas"] = packed(replace(rows, b"IHDR", lambda p: b"\0" * 4 + p[4:]))
    for value in (0, 4, 4097):
        cases[f"frame-count-{value}"] = packed(replace(rows, b"acTL", lambda p, v=value: struct.pack(">I", v) + p[4:]))
    for label, index, value in (("sequence", 0, 99), ("zero-width", 4, 0), ("outside-x", 12, 33)):
        cases[label] = packed(replace(rows, b"fcTL", lambda p, i=index, v=value: p[:i] + struct.pack(">I", v) + p[i + 4:]))
    for label, index, value in (("disposal", 24, 3), ("blend", 25, 2)):
        cases[label] = packed(replace(rows, b"fcTL", lambda p, i=index, v=value: p[:i] + bytes([v]) + p[i + 1:]))
    cases["fdAT-sequence"] = packed(replace(rows, b"fdAT", lambda p: struct.pack(">I", 99) + p[4:]))
    cases["fdAT-empty"] = packed(replace(rows, b"fdAT", lambda p: p[:4]))
    cases["fdAT-compression"] = packed(replace(rows, b"fdAT", lambda p: p[:4] + b"invalid deflate"))
    first_control = next(i for i, (k, _) in enumerate(rows) if k == b"fcTL")
    first_data = next(i for i, (k, _) in enumerate(rows) if k == b"fdAT")
    reordered = rows.copy()
    item = reordered.pop(first_data)
    reordered.insert(first_control, item)
    cases["fdAT-before-control"] = packed(reordered)
    iccp_index = next(i for i, (k, _) in enumerate(rows) if k == b"iCCP")
    reordered = rows.copy()
    item = reordered.pop(iccp_index)
    reordered.insert(next(i for i, (k, _) in enumerate(reordered) if k == b"IDAT") + 1, item)
    cases["late-profile"] = packed(reordered)
    for label, value in (("profile-keyword", b"\0\0"), ("profile-method", b"name\0\1invalid"),
                         ("profile-deflate", b"name\0\0invalid"),
                         ("profile-bomb", b"name\0\0" + zlib.compress(b"\0" * (8 * 1024 * 1024 + 1))),
                         ("profile-trailer", next(v for k, v in rows if k == b"iCCP") + b"trailer")):
        cases[label] = packed(replace(rows, b"iCCP", lambda p, v=value: v))
    cases["text-bomb"] = packed(replace(rows, b"zTXt", lambda p: b"Comment\0\0" + zlib.compress(b"x" * (8 * 1024 * 1024 + 1))))
    cases["text-method"] = packed(replace(rows, b"zTXt", lambda p: b"Comment\0\1invalid"))
    cases["international-UTF8"] = packed(replace(rows, b"iTXt", lambda p: b"Comment\0\0\0\0\0\xff"))
    for label, language in (("non-ASCII", b"\xff"), ("control", b"en\n"),
                            ("leading-hyphen", b"-en"), ("trailing-hyphen", b"en-"),
                            ("empty-subtag", b"en--GB"), ("underscore", b"en_GB"),
                            ("numeric-primary", b"123"), ("long-subtag", b"en-abcdefghi"),
                            ("unsupported-registered", b"zh-Hans")):
        cases["language-" + label] = packed(replace(rows, b"iTXt", lambda p, v=language:
            b"Comment\0\0\0" + v + b"\0\0synthetic text"))
    # Both physical orders must reject: a late profile cannot evade checks that
    # only noticed a fallback following the profile (or the reverse).
    fallbacks = ((b"gAMA", struct.pack(">I", 100000)),
                 (b"cHRM", struct.pack(">8I", 31270, 32900, 64000, 33000,
                                      30000, 60000, 15000, 6000)))
    for kind, value in fallbacks:
        for after in (False, True):
            index = iccp_index + int(after)
            cases["ICC-fallback-" + kind.decode() + ("-after" if after else "-before")] = packed(
                rows[:index] + [(kind, value)] + rows[index:])
        without_icc = [(k, v) for k, v in rows if k != b"iCCP"]
        for reverse in (False, True):
            colors = [(b"sRGB", b"\0"), (kind, value)]
            if reverse:
                colors.reverse()
            cases["sRGB-fallback-" + kind.decode() + ("-before" if reverse else "-after")] = packed(
                without_icc[:1] + colors + without_icc[1:])
    cases["unknown-text-rendering"] = packed(replace(rows, b"tEXt", lambda p: b"HDRGainMap\0rendering data"))
    for label, xml in (("XMP-orientation", b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:Orientation="8"/></rdf:RDF></x:xmpmeta>'),
                       ("XMP-HDR", b'<hdr:GainMap xmlns:hdr="urn:unknown-hdr"/>'),
                       ("XMP-DTD", b'<!DOCTYPE x [<!ENTITY secret "fixture">]><x/>')):
        cases[label] = packed(replace(rows, b"iTXt", lambda p, x=xml: b"XML:com.adobe.xmp\0\0\0\0\0" + x))
    cases["TIFF-root"] = packed(replace(rows, b"eXIf", lambda p: p[:4] + struct.pack("<I", len(p) + 8) + p[8:]))
    cases["TIFF-cycle"] = packed(replace(rows, b"eXIf", lambda p: exif_field(p, 0x8825, lambda r: r[:8] + struct.pack("<I", 8))))
    cases["orientation-zero"] = packed(replace(rows, b"eXIf", lambda p: exif_field(p, 0x0112, lambda r: r[:8] + b"\0" * 4)))
    cases["orientation-count"] = packed(replace(rows, b"eXIf", lambda p: exif_field(p, 0x0112, lambda r: r[:4] + struct.pack("<I", 2) + r[8:])))
    cases["TIFF-next-IFD"] = packed(replace(rows, b"eXIf", lambda p: p[:106] + struct.pack("<I", 8) + p[110:]))
    # Valid zlib but invalid scanline structure, and extra bytes after a valid
    # stream. Replace the default's two contiguous IDAT chunks as one unit.
    compressed = b"".join(v for k, v in rows if k == b"IDAT")
    raw = zlib.decompress(compressed)
    for label, stream in (("row-filter", zlib.compress(b"\5" + raw[1:])),
                          ("image-short", zlib.compress(raw[:-1])),
                          ("image-expanded", zlib.compress(raw + b"x")),
                          ("image-trailer", compressed + b"trailer"),
                          ("image-truncated", compressed[:-1])):
        changed = replace(rows, b"IDAT", lambda p, s=stream: s)
        changed = replace(changed, b"IDAT", lambda p: b"", occurrence=1)
        cases[label] = packed(changed)
    for label, data in cases.items():
        before, after = work / (label + ".png"), work / (label + ".output.png")
        before.write_bytes(data)
        result = subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc], capture_output=True, text=True)
        last = result.stderr.splitlines()[-1] if result.stderr else ""
        assert result.returncode == 1 and last.startswith(("ValueError: invalid PNG ", "ValueError: invalid ICC ")), (label, result.stderr)
        assert not after.exists(), label + " left output"
        assert before.read_bytes() == data
    # Empty language and the explicit registered English subset are admitted
    # case-insensitively; this is not general BCP47/registry certification.
    languages = (b"", b"en", b"EN", b"en-GB", b"EN-gb")
    for index, language in enumerate(languages):
        data = packed(replace(rows, b"iTXt", lambda p, v=language:
            b"Comment\0\0\0" + v + b"\0\0synthetic text"))
        before, after = work / f"language-valid-{index}.png", work / f"language-valid-{index}.output.png"
        before.write_bytes(data)
        subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc], check=True)
        assert canvases(before) == canvases(after)
        assert not any(k == b"iTXt" for k, _ in records(after.read_bytes()))
    # All eight orientations, including default1 removal, retain every displayed
    # default and animation canvas. This doesn't assume orientation6 alone works.
    for value in range(1, 9):
        data = packed(replace(rows, b"eXIf", lambda p, v=value: exif_field(p, 0x0112, lambda r: r[:8] + struct.pack("<H", v) + b"\0\0")))
        before, after = work / f"orientation-{value}.png", work / f"orientation-{value}.output.png"
        before.write_bytes(data)
        subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc], check=True)
        assert canvases(before) == canvases(after)
        with Image.open(after) as image:
            assert set(image.getexif()) == (set() if value == 1 else {0x0112})
            assert image.getexif().get(0x0112, 1) == value
    # A changed LAST animation frame must not hide behind the unchanged static
    # default. Preserve controls, metadata and sequence words; alter opaque RGB.
    indices = [i for i, (k, _) in enumerate(rows) if k == b"fdAT"][-2:]
    stream = b"".join(rows[i][1][4:] for i in indices)
    decoded = bytearray(zlib.decompress(stream))
    decoded[9] ^= 128  # Last-frame row0 pixel2 has alpha255 in owned artwork.
    stream = zlib.compress(decoded)
    changed = rows.copy()
    split = len(stream) // 2
    for i, part in zip(indices, (stream[:split], stream[split:])):
        changed[i] = (b"fdAT", rows[i][1][:4] + part)
    mutant = work / "last-frame-mutant.png"
    mutant.write_bytes(packed(changed))
    baseline = canvases(root / "input" / "apng-default-v4.png")
    modified = canvases(mutant)
    assert baseline[:-1] == modified[:-1] and baseline[-1] != modified[-1]
    spec = importlib.util.spec_from_file_location("independent_png", observer)
    scanner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(scanner)
    original_rows = scanner.inspect(root / "input" / "apng-default-v4.png", "apng-default")[0]
    mutated_rows = scanner.inspect(mutant, "apng-default")[0]
    assert [(k, v) for k, v in original_rows if k in scanner.RENDERING] != [(k, v) for k, v in mutated_rows if k in scanner.RENDERING]
    print(f"{len(cases)} malformed/unsupported no-output domain rejections; {len(languages)} admitted language cases; 8 orientation cases; last-frame observer control passed")
