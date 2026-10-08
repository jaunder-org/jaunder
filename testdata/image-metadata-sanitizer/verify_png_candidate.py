#!/usr/bin/env python3
"""Independent trusted-fixture PNG/APNG probe of ExifTool, NOT a sanitizer.

Does not import the artwork generator. Pillow renders every default/animation
canvas for test evidence only; it does not automatically apply ICC transforms.
"""
import hashlib
import json
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path
from PIL import Image, ImageOps

MAX_PROFILE = 8 * 1024 * 1024
RENDERING = {b"IHDR", b"PLTE", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"sBIT",
             b"pHYs", b"bKGD", b"acTL", b"fcTL", b"IDAT", b"fdAT", b"IEND"}
DESCRIPTIVE = {b"tEXt", b"zTXt", b"iTXt", b"tIME", b"eXIf"}


def inspect(path, mode):
    data = path.read_bytes()
    assert data[:8] == bytes((137, 80, 78, 71, 13, 10, 26, 10))
    position, records, sequences, controls = 8, [], [], []
    profile, profile_name = None, None
    while position < len(data):
        assert len(records) < 65536 and position + 12 <= len(data)
        length = int.from_bytes(data[position:position + 4], "big")
        end = position + 12 + length
        assert end <= len(data)
        kind = data[position + 4:position + 8]
        assert len(kind) == 4 and all(65 <= c <= 90 or 97 <= c <= 122 for c in kind)
        assert 65 <= kind[2] <= 90
        payload = data[position + 8:end - 4]
        assert zlib.crc32(kind + payload) == int.from_bytes(data[end - 4:end], "big")
        records.append((kind, payload))
        if kind == b"iCCP":
            assert profile is None and len(payload) <= MAX_PROFILE
            separator = payload.index(b"\0")
            assert 1 <= separator <= 79 and payload[separator + 1] == 0
            profile_name = payload[:separator]
            inflater = zlib.decompressobj()
            profile = inflater.decompress(payload[separator + 2:], MAX_PROFILE + 1)
            assert len(profile) <= MAX_PROFILE and inflater.eof
            assert not inflater.unused_data and not inflater.unconsumed_tail
        if kind == b"fcTL":
            assert len(payload) == 26
            values = struct.unpack(">5I2H2B", payload)
            sequences.append(values[0])
            controls.append(values[1:])
        if kind == b"fdAT":
            assert len(payload) > 4
            sequences.append(int.from_bytes(payload[:4], "big"))
        if kind == b"IEND":
            assert length == 0 and end == len(data)
        position = end
    names = [kind for kind, _ in records]
    assert names[0] == b"IHDR" and names[-1] == b"IEND"
    assert names.count(b"IHDR") == names.count(b"IEND") == 1
    assert records[0][1] == struct.pack(">IIBBBBB", 32, 24, 8, 2 if mode == "rgb" else 6, 0, 0, 0)
    idat_positions = [i for i, kind in enumerate(names) if kind == b"IDAT"]
    assert len(idat_positions) == 2
    assert idat_positions == list(range(idat_positions[0], idat_positions[-1] + 1))
    if mode.startswith("apng"):
        assert names.count(b"acTL") == 1
        assert names.index(b"acTL") < idat_positions[0]
        actl = next(payload for kind, payload in records if kind == b"acTL")
        assert actl == struct.pack(">II", 3, 2 if mode == "apng-default" else 0)
        assert controls == [(32, 24, 0, 0, 7, 100, 0, 0),
                            (8, 7, 2, 3, 17, 0, 1, 1),
                            (5, 4, 10, 6, 11, 60, 2, 1)]
        assert sequences == list(range(9 if mode == "apng-default" else 7))
        assert (names.index(b"fcTL") > idat_positions[-1]) == (mode == "apng-default")
        # Exact fixture association, not merely fcTL/fdAT counts.
        frame = -1
        frame_parts = [0, 0, 0]
        for kind, payload in records:
            if kind == b"fcTL":
                frame += 1
            elif kind == b"fdAT":
                assert frame >= 0
                frame_parts[frame] += 1
            elif kind == b"IDAT" and mode == "apng-first":
                assert frame == 0
                frame_parts[0] += 1
        assert frame_parts == [2, 2, 2]
    else:
        assert not controls and not sequences and b"acTL" not in names and b"fdAT" not in names
    assert profile is not None
    return records, profile, profile_name


def digest(data):
    return hashlib.sha256(data).hexdigest()


def decode(path, mode, original, sanitized=False):
    with Image.open(path) as image:
        image.load()
        assert image.size == (32, 24)
        exif = image.getexif()
        assert exif.get(0x0112, 1) == (6 if original or sanitized else 1)
        if original:
            assert {0x010f, 0x0110, 0x0132, 0x013b, 0x8298, 0x8825} <= set(exif)
            assert {0, 1, 2, 3, 4} <= set(exif.get_ifd(0x8825))
        elif sanitized:
            assert set(exif) == {0x0112}
        else:
            assert len(exif) == 0
        separate = mode == "apng-default"
        expected_count = 4 if separate else 3 if mode.startswith("apng") else 1
        assert image.n_frames == expected_count
        assert bool(image.info.get("default_image", False)) == separate
        loop = image.info.get("loop")
        if mode.startswith("apng"):
            assert loop == (2 if separate else 0)
        frames = []
        oriented = []
        information = []
        for index in range(image.n_frames):
            image.seek(index)
            frame = image.copy()
            rgba = frame.convert("RGBA")
            frames.append((rgba.size, digest(rgba.tobytes())))
            displayed = ImageOps.exif_transpose(frame).convert("RGBA")
            oriented.append((displayed.size, digest(displayed.tobytes())))
            information.append(tuple(image.info.get(key) for key in ("duration", "disposal", "blend")))
        return frames, oriented, (separate, loop, information)


if __name__ == "__main__":
    root = Path(sys.argv[1])
    exiftool = sys.argv[2]
    assert Path(exiftool).is_absolute()
    (root / "candidate").mkdir(parents=True, exist_ok=True)
    results = []
    for version in (2, 4):
        for mode in ("rgb", "rgba", "apng-first", "apng-default"):
            name = f"{mode}-v{version}.png"
            before = root / "input" / name
            after = root / "candidate" / name
            shutil.copyfile(before, after)
            process = subprocess.run([exiftool, "-overwrite_original", "-all=",
                                      "--icc_profile:all", str(after)],
                                     check=True, capture_output=True)
            assert not process.stderr, "candidate emitted diagnostics"
            source, source_profile, source_name = inspect(before, mode)
            candidate, candidate_profile, candidate_name = inspect(after, mode)
            assert DESCRIPTIVE <= {kind for kind, _ in source}
            assert not DESCRIPTIVE.intersection(kind for kind, _ in candidate)
            assert [(k, v) for k, v in source if k in RENDERING] == [(k, v) for k, v in candidate if k in RENDERING]
            assert source_profile == candidate_profile
            assert source_name == b"Fixture author camera profile"
            raw, oriented, semantics = decode(before, mode, True)
            new_raw, new_oriented, new_semantics = decode(after, mode, False)
            assert raw == new_raw and semantics == new_semantics
            assert oriented != new_oriented
            results.append({"fixture": name, "decoded_canvases": len(raw),
                            "rendering_chunks_and_compressed_payloads_identical": True,
                            "all_unoriented_RGBA_canvases_identical": True,
                            "orientation_lost": True,
                            "descriptive_ICC_body_retained_unchanged": True,
                            "profile_name_retained_unchanged": source_name == candidate_name})
    print(json.dumps({"candidate": "ExifTool13.59 -all= --icc_profile:all",
                      "sanitizer_policy_satisfied": False, "fixtures": results}, indent=2))
