#!/usr/bin/env python3
"""Independent input/output proof for the owned WebP rewrite corpus.

No rewriting code is imported. Container observation, Pillow canvases, ICC
inspection and LittleCMS transforms are checked through separate consumers.
"""
import importlib.util
import json
import struct
import subprocess
import sys
from pathlib import Path


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def outer(path):
    data, at, result = Path(path).read_bytes(), 12, []
    assert len(data) >= 12 and data[:4] == b"RIFF" and data[8:12] == b"WEBP"
    assert struct.unpack_from("<I", data, 4)[0] + 8 == len(data)
    while at < len(data):
        assert at + 8 <= len(data)
        kind, size = data[at:at + 4], struct.unpack_from("<I", data, at + 4)[0]
        end = at + 8 + size
        assert end + (size & 1) <= len(data) and (not size & 1 or data[end] == 0)
        result.append((kind, data[at + 8:end]))
        at = end + (size & 1)
    assert at == len(data)
    return result


def minimal_orientation(blob):
    assert len(blob) == 26 and blob[:8] == b"II\x2a\0\x08\0\0\0"
    assert struct.unpack_from("<H", blob, 8)[0] == 1
    tag, kind, count, value = struct.unpack_from("<HHIH", blob, 10)
    assert tag == 0x0112 and kind == 3 and count == 1 and 1 <= value <= 8
    assert blob[20:] == b"\0" * 6
    return value


def verify_pair(source, target, observe, icc, lcms, artifacts):
    """Assert the complete published output contract, also used by sensitivity tests."""
    before, after = observe.inspect(source), observe.inspect(target)
    assert observe.compressed(before) == observe.compressed(after)
    for key in ("canvas", "animation", "frames"):
        assert before[key] == after[key], key
    original, rewritten = observe.decode(source), observe.decode(target)
    orientation = original["orientation"]
    assert orientation in range(1, 9)
    retained = orientation != 1
    assert after["kinds"] == [kind for kind in before["kinds"]
                              if kind != "XMP " and (retained or kind != "EXIF")]
    assert after["flags"] == before["flags"] & ~(4 | (0 if retained else 8))
    assert set(after["metadata"]) == ({"ICCP", "EXIF"} if retained else {"ICCP"})
    for key in ("frames", "displayed", "timings", "loop", "transparent_colored"):
        assert original[key] == rewritten[key], key
    assert all(original["transparent_colored"])
    assert rewritten["orientation"] == (orientation if retained else None)
    assert rewritten["exif_tags"] == ([274] if retained else [])
    assert rewritten["icc"] and not rewritten["xmp"]
    source_records, target_records = outer(source), outer(target)
    exifs = [payload for kind, payload in target_records if kind == b"EXIF"]
    assert len(exifs) == int(retained)
    if retained:
        assert minimal_orientation(exifs[0]) == orientation

    # Extract by record walking: byte sequences inside compressed/profile data
    # never become top-level metadata. Keep extracted profiles as private proof.
    profiles = []
    for label, entries in (("input", source_records), ("output", target_records)):
        values = [value for kind, value in entries if kind == b"ICCP"]
        assert len(values) == 1
        path = Path(artifacts) / (Path(target).stem + "." + label + ".icc")
        path.write_bytes(values[0])
        profiles.append(path)
    old_header, old_tags = icc.profile(profiles[0])
    new_header, new_tags = icc.sanitized_profile(profiles[1])
    assert all(old_tags[tag][2] == new_tags[tag][2] for tag in icc.COLOR_TAGS)
    expected = bytearray(old_header[:128])
    expected[:4] = new_header[:4]
    expected[24:36] = icc.CLEAN_DATE
    for start, end in ((4, 8), (40, 44), (48, 56), (80, 128)):
        expected[start:end] = b"\0" * (end - start)
    assert new_header[:128] == expected
    process = subprocess.run([str(lcms), *(str(path) for path in profiles)], check=True, capture_output=True)
    assert not process.stderr
    return {"fixture": Path(source).name, "canvases": len(original["frames"]),
            "compressed_frame_control_bytes_equal": True, "raw_and_oriented_canvases_equal": True,
            "colored_transparent_pixels_preserved": True, "minimal_orientation_and_private_metadata_absent": True,
            "ICC_LittleCMS_all_intents_equal": True}


def main():
    root, rewrite, icc_rewrite, png_rewrite, observer_path, icc_path, lcms = sys.argv[1:]
    root = Path(root)
    observe, icc = load("webp_oracle", observer_path), load("webp_icc_inspector", icc_path)
    output = root / "rewritten"
    output.mkdir(exist_ok=True)
    results = []
    sources = sorted(path for path in (root / "input").glob("*.webp") if path.name.startswith(("static-", "animated-")))
    for source in sources:
        first, repeat, second = (output / (source.stem + suffix + ".webp") for suffix in ("", ".repeat", ".second"))
        for before, after in ((source, first), (source, repeat), (first, second)):
            subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc_rewrite, png_rewrite], check=True)
        assert first.read_bytes() == repeat.read_bytes() == second.read_bytes()
        result = verify_pair(source, first, observe, icc, lcms, output)
        result["separate_process_deterministic_and_idempotent"] = True
        results.append(result)
    assert len(results) == 8
    print(json.dumps({"scope": "owned static/three-frame alpha WebP, ordinary RGB ICC2/4, raw TIFF orientation",
                      "fixtures": results}, indent=2))


if __name__ == "__main__":
    main()
