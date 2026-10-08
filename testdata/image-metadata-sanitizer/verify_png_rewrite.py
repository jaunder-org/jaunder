#!/usr/bin/env python3
"""Independent PNG/APNG fixture proof; never imports the PNG rewriter."""
import importlib.util
import json
import subprocess
import sys
from pathlib import Path


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


if __name__ == "__main__":
    root, rewrite, icc_rewrite, scanner, icc_inspector, oracle = sys.argv[1:]
    root = Path(root)
    png = load("independent_png", scanner)
    icc = load("independent_icc", icc_inspector)
    output = root / "rewritten"
    output.mkdir(exist_ok=True)
    results = []
    for version in (2, 4):
        for mode in ("rgb", "rgba", "apng-first", "apng-default"):
            stem = f"{mode}-v{version}"
            source = root / "input" / (stem + ".png")
            first, repeat, second = (output / (stem + suffix + ".png") for suffix in ("", ".repeat", ".second"))
            for before, after in ((source, first), (source, repeat), (first, second)):
                subprocess.run([sys.executable, "-B", rewrite, str(before), str(after), icc_rewrite], check=True)
            assert first.read_bytes() == repeat.read_bytes() == second.read_bytes()
            source_chunks, source_profile, _ = png.inspect(source, mode)
            chunks, profile, name = png.inspect(first, mode)
            assert name == b"sanitized"
            assert {k for k, _ in chunks} <= png.RENDERING | {b"iCCP", b"eXIf"}
            assert [(k, v) for k, v in source_chunks if k in png.RENDERING] == [(k, v) for k, v in chunks if k in png.RENDERING]
            exif = [v for k, v in chunks if k == b"eXIf"]
            assert exif == [bytes.fromhex("49492a0008000000010012010300010000000600000000000000")]
            old_icc, new_icc = output / (stem + ".input.icc"), output / (stem + ".output.icc")
            old_icc.write_bytes(source_profile)
            new_icc.write_bytes(profile)
            old_header, old_tags = icc.profile(old_icc)
            new_header, new_tags = icc.sanitized_profile(new_icc)
            for tag in icc.COLOR_TAGS:
                assert old_tags[tag][2] == new_tags[tag][2]
            expected = bytearray(old_header[:128])
            expected[:4] = new_header[:4]
            expected[24:36] = icc.CLEAN_DATE
            for start, end in ((4, 8), (40, 44), (48, 56), (80, 128)):
                expected[start:end] = b"\0" * (end - start)
            assert new_header[:128] == expected
            process = subprocess.run([oracle, str(old_icc), str(new_icc)], check=True, capture_output=True)
            assert not process.stderr
            raw, oriented, semantics = png.decode(source, mode, True)
            new_raw, new_oriented, new_semantics = png.decode(first, mode, False, sanitized=True)
            assert (raw, oriented, semantics) == (new_raw, new_oriented, new_semantics)
            results.append({"fixture": stem, "compared_canvases": len(raw),
                            "compressed_and_rendering_bytes_preserved": True,
                            "orientation_only_Exif_and_scrubbed_ICC": True,
                            "all_oriented_and_unoriented_canvases_equal": True,
                            "LittleCMS_all_intents_pass": True,
                            "independent_process_determinism_and_idempotence": True})
    print(json.dumps({"scope": "fixture-only RGB/RGBA8 PNG/APNG ordinary ICC2.0/4.0",
                      "fixtures": results}, indent=2))
