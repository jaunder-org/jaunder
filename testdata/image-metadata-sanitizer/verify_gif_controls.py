#!/usr/bin/env python3
"""Owned GIF rejection, dictionary-code and late-frame observer controls."""
import importlib.util
import struct
import subprocess
import sys
from pathlib import Path
from PIL import Image


def split(data):
    at = 13 + 3 * (1 << ((data[10] & 7) + 1))
    header, items = data[:at], []

    def end_blocks(position):
        while data[position]:
            position += 1 + data[position]
        return position + 1

    while at < len(data):
        start = at
        if data[at] == 59:
            items.append(data[at:at + 1])
            break
        if data[at] == 44:
            flags = data[at + 9]
            at += 10 + (3 * (1 << ((flags & 7) + 1)) if flags & 128 else 0)
            at = end_blocks(at + 1)
        elif data[at + 1] == 249:
            at += 8
        else:
            at = end_blocks(at + (14 if data[at + 1] == 255 else 2))
        items.append(data[start:at])
    return header, items


def code_stream(codes, minimum=2):
    result, bits = 0, 0
    for code, width in codes:
        result |= code << bits
        bits += width
    value = result.to_bytes((bits + 7) // 8, "little")
    return bytes([minimum, len(value)]) + value + b"\0"


if __name__ == "__main__":
    root, rewrite, icc, observer, magick = sys.argv[1:]
    root = Path(root)
    work = root / "controls"
    work.mkdir(exist_ok=True)
    original = (root / "input" / "finite-v4.gif").read_bytes()
    header, items = split(original)
    gce = next(i for i, x in enumerate(items) if x.startswith(b"!\xf9"))
    image = next(i for i, x in enumerate(items) if x.startswith(b","))
    profile = next(i for i, x in enumerate(items) if x.startswith(b"!\xff\x0bICCRGBG1"))
    loop = next(i for i, x in enumerate(items) if x.startswith(b"!\xff\x0bNETSCAPE"))
    cases = {"signature": b"bad" + original[3:], "missing-trailer": original[:-1],
             "trailer": original + b"private", "palette-truncated": header[:-1]}

    def change(index, value):
        return header + b"".join(items[:index] + [value] + items[index + 1:])

    cases["zero-canvas"] = header[:6] + b"\0\0" + original[8:]
    cases["background-index"] = header[:11] + b"\4" + original[12:]
    cases["87a-extensions"] = b"GIF87a" + original[6:]
    for label, packed in (("GCE-reserved", 0xe5), ("disposal", 17), ("user-input", 7)):
        cases[label] = change(gce, items[gce][:3] + bytes([packed]) + items[gce][4:])
    cases["GCE-size"] = change(gce, items[gce][:2] + b"\3" + items[gce][3:])
    cases["GCE-terminator"] = change(gce, items[gce][:-1] + b"\1")
    cases["GCE-duplicate"] = change(gce, items[gce] * 2)
    cases["GCE-dangling"] = header + b"".join(items[:-1]) + items[gce] + b";"
    for label, flags in (("image-reserved", 8), ("interlace", 64)):
        cases[label] = change(image, items[image][:9] + bytes([flags]) + items[image][10:])
    cases["outside-rectangle"] = change(image, b"," + struct.pack("<H", 12) + items[image][3:])
    cases["zero-image-width"] = change(image, items[image][:5] + b"\0\0" + items[image][7:])
    cases["transparent-index"] = change(gce, items[gce][:6] + b"\4" + items[gce][7:])
    cases["plain-text-rendering"] = header + b"!\1\x0c" + b"\0" * 12 + b"\0" + b"".join(items)
    for label, identifier in (("unknown-app", b"PRIVATE1001"), ("XMP", b"XMP DataXMP"), ("ANIMEXTS", b"ANIMEXTS1.0")):
        cases[label] = header + b"!\xff\x0b" + identifier + b"\0" + b"".join(items)
    cases["loop-duplicate"] = change(loop, items[loop] * 2)
    cases["loop-shape"] = change(loop, b"!\xff\x0bNETSCAPE2.0\2\1\0\1\0\0")
    cases["loop-value"] = change(loop, b"!\xff\x0bNETSCAPE2.0\3\2\0\0\0")
    cases["late-loop"] = header + b"".join(x for i, x in enumerate(items[:-1]) if i != loop) + items[loop] + b";"
    cases["profile-duplicate"] = change(profile, items[profile] * 2)
    cases["late-profile"] = header + b"".join(x for i, x in enumerate(items[:-1]) if i != profile) + items[profile] + b";"
    cases["invalid-profile"] = change(profile, b"!\xff\x0bICCRGBG1012\3bad\0")
    cases["subblock-truncated"] = header + b"!\xfe\xffshort"
    base = (root / "input" / "bare87.gif").read_bytes()
    bare_header, bare_items = split(base)
    descriptor = b"," + struct.pack("<HHHHB", 0, 0, 3, 1, 0)
    for label, codes in (("no-clear", [(0, 3), (5, 3)]),
                         ("dictionary-code", [(4, 3), (7, 3), (5, 3)]),
                         ("expansion-short", [(4, 3), (0, 3), (5, 3)]),
                         ("missing-end", [(4, 3), (0, 3)])):
        cases[label] = bare_header + descriptor + code_stream(codes) + b";"
    cases["LZW-minimum"] = bare_header + descriptor + b"\1\1\0\0;"
    cases["LZW-extra-bytes"] = bare_header + descriptor + b"\2\3\x84\x0b\0\0;"
    small_palette = bare_header[:10] + bytes([bare_header[10] & ~7]) + bare_header[11:19]
    cases["LZW-palette-index"] = small_palette + b"," + struct.pack("<HHHHB", 0, 0, 1, 1, 0) + code_stream([(4, 3), (3, 3), (5, 3)]) + b";"
    for minimum in range(3, 9):
        codes = [(1 << minimum, minimum + 1), (0, minimum + 1),
                 (1, minimum + 1), (2, minimum + 1), ((1 << minimum) + 1, minimum + 1)]
        cases[f"unproved-minimum-{minimum}"] = bare_header + descriptor + code_stream(codes, minimum) + b";"
    # This valid stream reaches the FIRST unproved 4->5-bit transition. It is
    # independently decoded below before the prototype's rejection is checked.
    crossing = [(4, 3)] + [(i % 4, 3 if i < 3 else 4) for i in range(11)] + [(5, 5)]
    cases["unproved-width5"] = bare_header + b"," + struct.pack("<HHHHB", 0, 0, 11, 1, 0) + code_stream(crossing) + b";"
    for label, data in cases.items():
        before, after = work / (label + ".gif"), work / (label + ".output.gif")
        before.write_bytes(data)
        if label.startswith("unproved-"):
            with Image.open(before) as image:
                image.load()
                assert image.n_frames == 1
            consumer = subprocess.run([magick, str(before), "-coalesce", "-depth", "8", "rgba:" + str(before) + ".rgba"], capture_output=True, check=True)
            assert not consumer.stderr
            assert len(Path(str(before) + ".rgba").read_bytes()) == 12 * 10 * 4
        result = subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc], capture_output=True, text=True)
        last = result.stderr.splitlines()[-1] if result.stderr else ""
        assert result.returncode == 1 and last.startswith(("ValueError: invalid GIF ", "ValueError: invalid ICC ")), (label, result.stderr)
        assert not after.exists() and before.read_bytes() == data
    spec = importlib.util.spec_from_file_location("independent_gif", observer)
    proof = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(proof)
    # Positive dictionary special case and code-width growth, independently
    # decoded by Pillow and ImageMagick, with exact rendering bytes retained.
    for label, width, codes in (("KwKwK", 3, [(4, 3), (0, 3), (6, 3), (5, 3)]),
                               ("growth", 4, [(4, 3), (0, 3), (1, 3), (2, 3), (3, 4), (5, 4)]),
                               ("width4-boundary", 10, [(4, 3)] + [(i % 4, 3 if i < 3 else 4) for i in range(10)] + [(5, 4)]),
                               ("clear-width4", 11, [(4, 3)] + [(i % 4, 3 if i < 3 else 4) for i in range(10)] + [(4, 4), (2, 3), (5, 3)])):
        data = bare_header + b"," + struct.pack("<HHHHB", 0, 2, width, 1, 0) + code_stream(codes) + b";"
        before, after = work / (label + ".gif"), work / (label + ".output.gif")
        before.write_bytes(data)
        subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc], check=True)
        assert data == after.read_bytes() and proof.decode(before) == proof.decode(after)
        for path in (before, after):
            result = subprocess.run([magick, str(path), "-coalesce", "-depth", "8", "rgba:" + str(path) + ".rgba"], capture_output=True, check=True)
            assert not result.stderr
        assert Path(str(before) + ".rgba").read_bytes() == Path(str(after) + ".rgba").read_bytes()
    # Opaque first pixel of the final image changes; earlier frames stay equal.
    last_image = max(i for i, x in enumerate(items) if x.startswith(b","))
    mutated = bytearray(items[last_image])
    assert mutated[9] == 0 and mutated[10] == 2
    mutated[12] ^= 1 << 3
    mutant = work / "last-frame-mutant.gif"
    mutant.write_bytes(change(last_image, bytes(mutated)))
    old_frames = proof.decode(root / "input" / "finite-v4.gif")[0]
    new_frames = proof.decode(mutant)[0]
    assert old_frames[:-1] == new_frames[:-1] and old_frames[-1] != new_frames[-1]
    assert proof.inspect(root / "input" / "finite-v4.gif")[0] != proof.inspect(mutant)[0]
    print(f"{len(cases)} no-output domain rejections; KwKwK/code-width growth, 4-bit boundary/reset, independently decoded unproved-state rejection and late-frame controls pass")
