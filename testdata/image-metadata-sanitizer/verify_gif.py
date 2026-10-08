#!/usr/bin/env python3
"""Independent owned GIF byte/consumer proof, not a production parser."""
import hashlib
import importlib.util
import json
import shutil
import struct
import subprocess
import sys
from pathlib import Path
from PIL import Image


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def inspect(path):
    blob = path.read_bytes()
    assert blob[:6] in (b"GIF87a", b"GIF89a") and len(blob) >= 13
    x, y = struct.unpack_from("<HH", blob, 6)
    assert (x, y) == (12, 10)
    at = 13 + (3 * (1 << ((blob[10] & 7) + 1)) if blob[10] & 128 else 0)
    core = [blob[:at]]
    profiles, comments, controls, rectangles, loops = [], [], [], [], []

    def take_blocks(position):
        payload = bytearray()
        while True:
            assert position < len(blob)
            count = blob[position]
            position += 1
            if not count:
                return position, bytes(payload)
            assert position + count <= len(blob)
            payload.extend(blob[position:position + count])
            position += count

    while at < len(blob):
        start = at
        if blob[at] == 59:
            assert at + 1 == len(blob)
            core.append(b";")
            break
        if blob[at] == 44:
            assert at + 10 <= len(blob)
            left, top, w, h, flags = struct.unpack_from("<HHHHB", blob, at + 1)
            assert w > 0 and h > 0 and left + w <= x and top + h <= y and flags & 0x58 == 0
            rectangles.append((left, top, w, h, flags))
            at += 10
            if flags & 128:
                at += 3 * (1 << ((flags & 7) + 1))
            assert at < len(blob) and 2 <= blob[at] <= 8
            at, _ = take_blocks(at + 1)
            core.append(blob[start:at])
        else:
            assert blob[at] == 33 and at + 2 <= len(blob)
            label = blob[at + 1]
            if label == 249:
                assert at + 8 <= len(blob) and blob[at + 2] == 4 and blob[at + 7] == 0
                controls.append(blob[at + 3:at + 7])
                at += 8
                core.append(blob[start:at])
            elif label == 254:
                at, value = take_blocks(at + 2)
                comments.append(value)
            else:
                assert label == 255 and at + 14 <= len(blob) and blob[at + 2] == 11
                application = blob[at + 3:at + 14]
                at, value = take_blocks(at + 14)
                if application == b"ICCRGBG1012":
                    profiles.append(value)
                else:
                    assert application == b"NETSCAPE2.0" and len(value) == 3 and value[0] == 1
                    loops.append(struct.unpack_from("<H", value, 1)[0])
                    core.append(blob[start:at])
    assert core[-1] == b";" and rectangles
    assert len(profiles) <= 1 and len(loops) <= 1
    return core, profiles, comments, controls, rectangles, loops


def decode(path):
    with Image.open(path) as image:
        frames, timings, disposals = [], [], []
        loop = image.info.get("loop")
        for index in range(image.n_frames):
            image.seek(index)
            rgba = image.convert("RGBA")
            frames.append((rgba.size, hashlib.sha256(rgba.tobytes()).hexdigest()))
            timings.append(image.info.get("duration", 0))
            disposals.append(getattr(image, "disposal_method", 0))
        return frames, timings, disposals, loop


if __name__ == "__main__":
    root, rewrite, icc_rewrite, icc_inspector, oracle, exiftool, magick = sys.argv[1:]
    root = Path(root)
    inspector = load("independent_icc", icc_inspector)
    for directory in ("output", "candidate", "decoded"):
        (root / directory).mkdir(exist_ok=True)
    results = []
    for source in sorted((root / "input").glob("*.gif")):
        name = source.stem
        target, repeat, second = (root / "output" / (name + suffix + ".gif") for suffix in ("", ".repeat", ".second"))
        for before, after in ((source, target), (source, repeat), (target, second)):
            subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc_rewrite], check=True)
        assert target.read_bytes() == repeat.read_bytes() == second.read_bytes()
        old = inspect(source)
        new = inspect(target)
        assert old[0] == new[0] and old[3:] == new[3:]
        assert not new[2]
        assert decode(source) == decode(target)
        for label, path in (("input", source), ("output", target)):
            raw = root / "decoded" / (name + "." + label + ".rgba")
            process = subprocess.run([magick, str(path), "-coalesce", "-depth", "8", "rgba:" + str(raw)], check=True, capture_output=True)
            assert not process.stderr
        assert (root / "decoded" / (name + ".input.rgba")).read_bytes() == (root / "decoded" / (name + ".output.rgba")).read_bytes()
        assert len(old[1]) == len(new[1])
        if old[1]:
            before, after = root / "output" / (name + ".input.icc"), root / "output" / (name + ".output.icc")
            before.write_bytes(old[1][0])
            after.write_bytes(new[1][0])
            old_header, old_tags = inspector.profile(before)
            new_header, new_tags = inspector.sanitized_profile(after)
            expected = bytearray(old_header[:128])
            expected[:4] = new_header[:4]
            expected[24:36] = inspector.CLEAN_DATE
            for begin, end in ((4, 8), (40, 44), (48, 56), (80, 128)):
                expected[begin:end] = b"\0" * (end - begin)
            assert new_header[:128] == expected
            assert all(old_tags[tag][2] == new_tags[tag][2] for tag in inspector.COLOR_TAGS)
            process = subprocess.run([oracle, str(before), str(after)], check=True, capture_output=True)
            assert not process.stderr
            candidate = root / "candidate" / source.name
            shutil.copyfile(source, candidate)
            process = subprocess.run([exiftool, "-overwrite_original", "-all=", "--icc_profile:all", str(candidate)], check=True, capture_output=True)
            assert not process.stderr
            candidate_info = inspect(candidate)
            assert candidate_info[1] == old[1]  # Privacy gap in retained original ICC.
            assert not candidate_info[2]
        else:
            assert source.read_bytes() == target.read_bytes()
        frames, timing, disposal, loop = decode(target)
        animated = name.startswith(("finite", "infinite"))
        assert len(frames) == (3 if animated else 1)
        assert loop == (2 if name.startswith("finite") else 0 if name.startswith("infinite") else None)
        if name != "bare87":
            assert timing == ([70, 170, 110] if animated else [70])
            assert disposal == ([1, 2, 3] if animated else [1])
        results.append({"fixture": name, "compared_canvases": len(frames),
                        "exact_rendering_palette_LZW_controls": True,
                        "Pillow_and_ImageMagick_all_frames_equal": True,
                        "ICC_scrub_and_LittleCMS_intents": bool(old[1]),
                        "deterministic_and_idempotent": True})
    assert len(results) == 7
    print(json.dumps({"scope": "owned GIF87/89, known NETSCAPE/comments/ordinaryICC",
                      "fixtures": results}, indent=2))
