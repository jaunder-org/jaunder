#!/usr/bin/env python3
"""Negative controls for the deliberately narrow HEIC verifier envelope."""
import importlib.util
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("witness_verify", sys.argv[2])
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def reject(data, expected):
    try:
        verify.heif_bytes(data)
    except AssertionError as error:
        require(str(error) == expected, f"expected {expected!r}, got {str(error)!r}")
    else:
        raise AssertionError(f"expected rejection: {expected}")


def child(data, name):
    top = verify.boxes(data, 0, len(data))
    _, meta_start, meta_end, meta_header = top[1]
    children = verify.boxes(data, meta_start + meta_header + 4, meta_end)
    return next(value for value in children if value[0] == name), (meta_start, meta_end)


def main():
    source = bytearray((ROOT / "input/device-like.heic").read_bytes())
    baseline = verify.heif_bytes(source)

    # Duplicate required child is rejected before item offsets are trusted.
    (pitm_type, pitm_start, pitm_end, _), (meta_start, meta_end) = child(source, b"pitm")
    duplicate = source[pitm_start:pitm_end]
    duplicate_meta = bytearray(source[:meta_end] + duplicate + source[meta_end:])
    old_size = int.from_bytes(duplicate_meta[meta_start : meta_start + 4], "big")
    duplicate_meta[meta_start : meta_start + 4] = (old_size + len(duplicate)).to_bytes(4, "big")
    reject(duplicate_meta, "unsupported or duplicate meta children")

    # A non-zero data-reference-index is outside this fixture envelope.
    bad_reference = bytearray(source)
    (_, iloc_start, _, iloc_header), _ = child(bad_reference, b"iloc")
    item_start = iloc_start + iloc_header + 4 + 2 + 2
    bad_reference[item_start + 2 : item_start + 4] = b"\0\1"
    reject(bad_reference, "unsupported iloc data-reference-index")

    # Changing base_offset changes the independently observed primary payload;
    # this control proves the parser applies it rather than discarding it.
    shifted_base = bytearray(source)
    base_offset_at = item_start + 4
    shifted_base[base_offset_at + 3] -= 1
    shifted = verify.heif_bytes(shifted_base)
    require(shifted["primary"]["payload"] != baseline["primary"]["payload"], "base_offset was ignored")
    print("HEIC controls passed: duplicate child/data-reference rejected; base_offset applied")


if __name__ == "__main__":
    main()
