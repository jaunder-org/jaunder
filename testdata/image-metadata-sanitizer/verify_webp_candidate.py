#!/usr/bin/env python3
"""Independent WebP/ExifTool candidate observer, not a sanitizer or entropy validator.

The admitted envelope is the owned Pillow/libwebp corpus: VP8 key frames with
versions 0--3 and no scaling, VP8L version 0, and ALPH headers with documented
reserved bits clear.  Payload entropy is deliberately not parsed here.
"""
import hashlib
import json
import shutil
import struct
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageOps, UnidentifiedImageError

IMAGE_CHUNKS = {b"VP8 ", b"VP8L", b"ALPH"}
METADATA_CHUNKS = {b"ICCP", b"EXIF", b"XMP "}
KNOWN = IMAGE_CHUNKS | METADATA_CHUNKS | {b"VP8X", b"ANIM", b"ANMF"}


class WebPContainerError(ValueError):
    """The independent observer's deliberate RIFF/WebP structural rejection."""


def require(condition, reason):
    if not condition:
        raise WebPContainerError(reason)


def u24(data):
    return int.from_bytes(data, "little")


def riff_chunks(data, start=12, end=None):
    end = len(data) if end is None else end
    records, at = [], start
    while at < end:
        require(at + 8 <= end, "truncated chunk header")
        kind, length = data[at:at + 4], struct.unpack_from("<I", data, at + 4)[0]
        payload_end = at + 8 + length
        padded_end = payload_end + (length & 1)
        require(payload_end <= end and padded_end <= end, "chunk extent/padding")
        if length & 1:
            require(data[payload_end] == 0, "nonzero RIFF padding")
        records.append((kind, data[at + 8:payload_end], at, padded_end))
        at = padded_end
    require(at == end, "nested boundary")
    return records


def image_header(kind, payload):
    if kind == b"VP8 ":
        require(len(payload) >= 10, "VP8 header")
        tag = int.from_bytes(payload[:3], "little")
        require(not (tag & 1), "VP8 non-key frame")
        require(((tag >> 1) & 7) <= 3, "VP8 version")
        require(bool(tag & 16), "VP8 hidden frame")
        partition = tag >> 5
        require(partition and 10 + partition <= len(payload), "VP8 partition bounds")
        require(payload[3:6] == b"\x9d\x01\x2a", "VP8 start code")
        raw_width, raw_height = struct.unpack_from("<HH", payload, 6)
        require(not (raw_width & 0xc000) and not (raw_height & 0xc000), "VP8 scaling")
        width, height = raw_width & 0x3fff, raw_height & 0x3fff
    elif kind == b"VP8L":
        require(len(payload) >= 5 and payload[0] == 0x2f, "VP8L signature")
        bits = int.from_bytes(payload[1:5], "little")
        require(not (bits >> 29), "VP8L version/reserved bits")
        width, height = (bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1
    else:
        raise WebPContainerError("not an image chunk")
    require(width and height, "zero image dimensions")
    return width, height


def alpha_header(payload):
    require(payload, "ALPH header")
    value = payload[0]
    require(not (value & 0xc0), "ALPH reserved bits")
    require((value & 3) <= 1, "ALPH compression")
    require(((value >> 2) & 3) <= 3, "ALPH filter")
    require(((value >> 4) & 3) <= 1, "ALPH preprocessing")


def inspect(path):
    data = path.read_bytes()
    require(len(data) >= 12 and data[:4] == b"RIFF" and data[8:12] == b"WEBP", "RIFF/WebP")
    require(struct.unpack_from("<I", data, 4)[0] + 8 == len(data), "RIFF declared extent/trailer")
    records = riff_chunks(data)
    kinds = [kind for kind, _, _, _ in records]
    require(bool(kinds) and set(kinds) <= KNOWN, "unknown/empty chunk")
    for kind in (b"VP8X", b"ICCP", b"EXIF", b"XMP ", b"ANIM"):
        require(kinds.count(kind) <= 1, f"duplicate {kind.decode('ascii')}")
    require(b"VP8X" not in kinds or kinds[0] == b"VP8X", "VP8X ordering")
    animated = b"ANMF" in kinds
    require((b"ANIM" in kinds) == animated, "ANIM/ANMF pairing")
    if animated:
        require(kinds.index(b"ANIM") < kinds.index(b"ANMF"), "ANIM ordering")
        require(not any(kind in IMAGE_CHUNKS for kind in kinds), "top-level image in animation")
    if b"ICCP" in kinds:
        first_image = min((index for index, kind in enumerate(kinds) if kind in IMAGE_CHUNKS or kind == b"ANMF"), default=len(kinds))
        require(kinds.index(b"ICCP") < first_image, "ICCP ordering")
    flags, canvas = 0, None
    if b"VP8X" in kinds:
        payload = records[kinds.index(b"VP8X")][1]
        require(len(payload) == 10 and payload[1:4] == b"\0\0\0", "VP8X reserved bytes")
        flags = payload[0]
        require(not (flags & 0xc1), "VP8X reserved flags")
        canvas = (u24(payload[4:7]) + 1, u24(payload[7:10]) + 1)
    animation, frames = None, []
    if animated:
        payload = records[kinds.index(b"ANIM")][1]
        require(len(payload) == 6 and flags & 2, "ANIM header/flag")
        animation = {"background_bgra": payload[:4].hex(), "loop_count": struct.unpack_from("<H", payload, 4)[0]}
        require(canvas is not None, "animated VP8X canvas")
    for kind, payload, _, _ in records:
        if kind != b"ANMF":
            continue
        require(len(payload) >= 16, "ANMF header")
        x, y = u24(payload[:3]) * 2, u24(payload[3:6]) * 2
        width, height, duration, control = u24(payload[6:9]) + 1, u24(payload[9:12]) + 1, u24(payload[12:15]), payload[15]
        require(not (control & 0xfc) and x + width <= canvas[0] and y + height <= canvas[1], "ANMF flags/rectangle")
        nested = riff_chunks(payload, 16)
        nested_kinds = [entry[0] for entry in nested]
        require(nested_kinds in ([b"VP8 "], [b"VP8L"], [b"ALPH", b"VP8 "]), "ANMF subchunks")
        if nested_kinds[0] == b"ALPH":
            alpha_header(nested[0][1])
        require(image_header(nested[-1][0], nested[-1][1]) == (width, height), "ANMF encoded dimensions")
        frames.append({"rect": [x, y, width, height], "duration_ms": duration,
                       "blend": not bool(control & 2), "dispose_to_background": bool(control & 1),
                       "subchunks": [(name.decode("ascii"), hashlib.sha256(value).hexdigest()) for name, value, _, _ in nested]})
    direct = [(kind, payload) for kind, payload, _, _ in records if kind in IMAGE_CHUNKS]
    if not animated:
        image = [(kind, payload) for kind, payload in direct if kind in (b"VP8 ", b"VP8L")]
        require(len(image) == 1, "static image count")
        if image[0][0] == b"VP8L":
            require(not any(kind == b"ALPH" for kind, _ in direct), "ALPH with VP8L")
        else:
            require([kind for kind, _ in direct] in ([b"VP8 "], [b"ALPH", b"VP8 "]), "static ALPH/VP8 ordering")
            if direct[0][0] == b"ALPH":
                alpha_header(direct[0][1])
        encoded_canvas = image_header(*image[0])
        if canvas is not None:
            require(canvas == encoded_canvas, "VP8X/static canvas mismatch")
        canvas = encoded_canvas
    require(canvas and canvas[0] <= 16384 and canvas[1] <= 16384, "canvas bounds")
    all_images = [(kind, payload) for kind, payload in direct if kind in (b"VP8 ", b"VP8L")]
    lossless_alpha = any(kind == b"VP8L" and bool(int.from_bytes(payload[1:5], "little") & (1 << 28)) for kind, payload in all_images)
    # Nested VP8L alpha bit is read from bytes, not inferred from frame count.
    for kind, payload, _, _ in records:
        if kind == b"ANMF":
            lossless_alpha |= any(name == b"VP8L" and bool(int.from_bytes(value[1:5], "little") & (1 << 28)) for name, value, _, _ in riff_chunks(payload, 16))
    expected_flags = ((0x20 if b"ICCP" in kinds else 0) | (0x08 if b"EXIF" in kinds else 0) |
                      (0x04 if b"XMP " in kinds else 0) | (0x10 if any(kind == b"ALPH" for kind, _ in direct) or any(frame["subchunks"][0][0] == "ALPH" for frame in frames) or lossless_alpha else 0) |
                      (0x02 if animated else 0))
    require((flags == expected_flags) if b"VP8X" in kinds else not expected_flags, "VP8X feature flags")
    metadata = {kind.decode("ascii"): hashlib.sha256(payload).hexdigest() for kind, payload, _, _ in records if kind in METADATA_CHUNKS}
    return {"canvas": canvas, "kinds": [kind.decode("ascii") for kind in kinds], "metadata": metadata,
            "image_payloads": [(kind.decode("ascii"), hashlib.sha256(payload).hexdigest()) for kind, payload in direct],
            "animation": animation, "frames": frames, "flags": flags}


def decode(path):
    with Image.open(path) as image:
        frames, oriented, timings, transparent_colored = [], [], [], []
        exif = image.getexif()
        for index in range(image.n_frames):
            image.seek(index)
            frame = image.copy()
            rgba = frame.convert("RGBA")
            pixels = rgba.tobytes()
            frames.append(hashlib.sha256(pixels).hexdigest())
            transparent_colored.append(any(pixels[offset + 3] == 0 and any(pixels[offset:offset + 3]) for offset in range(0, len(pixels), 4)))
            shown = ImageOps.exif_transpose(frame).convert("RGBA")
            oriented.append((shown.size, hashlib.sha256(shown.tobytes()).hexdigest()))
            timings.append(image.info.get("duration"))
        return {"frames": frames, "orientation": exif.get(0x0112), "displayed": oriented,
                "timings": timings, "loop": image.info.get("loop"), "icc": bool(image.info.get("icc_profile")),
                "xmp": bool(image.info.get("xmp")), "exif_tags": sorted(exif.keys()),
                "transparent_colored": transparent_colored}


def compressed(view):
    return view["image_payloads"], [frame["subchunks"] for frame in view["frames"]]


def candidate_decode(path):
    try:
        return decode(path), None
    except UnidentifiedImageError as error:
        return None, f"UnidentifiedImageError: {error}"
    except OSError as error:
        if error.errno is not None:
            raise
        return None, f"Pillow decode OSError: {error}"


def assert_candidate_observations(result):
    """Pin measured ExifTool candidate behavior; this is not sanitizer approval."""
    source, candidate = result["source"], result["candidate"]
    source_decode, candidate_decode = result["source_decode"], result["candidate_decode"]
    assert result["candidate_structural_error"] is None
    assert result["candidate_decode_error"] is None
    assert candidate is not None and candidate_decode is not None
    assert result["diagnostics"] == {"stdout": "1 image files updated", "stderr": ""}
    assert candidate["metadata"] == {"ICCP": source["metadata"]["ICCP"]}
    assert candidate["flags"] == source["flags"] & ~0x0c
    assert candidate["kinds"] == [kind for kind in source["kinds"] if kind not in ("EXIF", "XMP ")]
    assert candidate["canvas"] == source["canvas"]
    assert candidate["animation"] == source["animation"]
    assert candidate["frames"] == source["frames"]
    assert result["compressed_payloads_identical"] and compressed(source) == compressed(candidate)
    assert result["raw_canvases_equal"] and source_decode["frames"] == candidate_decode["frames"]
    assert source_decode["timings"] == candidate_decode["timings"]
    assert source_decode["loop"] == candidate_decode["loop"]
    assert source_decode["orientation"] == 6 and candidate_decode["orientation"] is None
    assert not result["orientation_retained"] and not result["oriented_canvases_equal"]
    assert all(tuple(size) == (24, 32) for size, _ in source_decode["displayed"])
    assert all(tuple(size) == (32, 24) for size, _ in candidate_decode["displayed"])
    assert candidate_decode["icc"] and not candidate_decode["xmp"] and candidate_decode["exif_tags"] == []


def source_requirements(name, view, decoded):
    animated = name.startswith("animated-")
    expected_loop = 2 if name.endswith("-v2.webp") else 0
    assert set(view["metadata"]) == {"ICCP", "EXIF", "XMP "}, "fixture metadata"
    assert decoded["orientation"] == 6 and decoded["icc"] and decoded["xmp"], "fixture decode metadata"
    assert {270, 271, 272, 274, 306, 315, 33432, 34853} <= set(decoded["exif_tags"]), "fixture EXIF privacy fields"
    assert all(size == (24, 32) for size, _ in decoded["displayed"]), "fixture orientation display"
    if animated:
        assert len(view["frames"]) == len(decoded["frames"]) == 3, "fixture frame count"
        assert view["animation"]["loop_count"] == decoded["loop"] == expected_loop, "fixture loop"
        assert decoded["timings"] == [70, 170, 110], "fixture timing"
        assert all(frame["rect"][2:] == [32, 24] for frame in view["frames"]), "fixture frame dimensions"
    else:
        assert len(decoded["frames"]) == 1, "fixture static frame count"
    return decoded["transparent_colored"]


if __name__ == "__main__":
    root, exiftool = map(Path, sys.argv[1:])
    assert exiftool.is_absolute(), "ExifTool path"
    (root / "candidate").mkdir(parents=True, exist_ok=True)
    results, transparent_measurements = [], {}
    sources = sorted(path for path in (root / "input").glob("*.webp") if path.name.startswith(("static-", "animated-")))
    for source in sources:
        target = root / "candidate" / source.name
        shutil.copyfile(source, target)
        process = subprocess.run([str(exiftool), "-overwrite_original", "-all=", "--icc_profile:all", str(target)], capture_output=True, text=True)
        assert process.returncode == 0, f"ExifTool failure: {process.stderr}"
        before, source_decode = inspect(source), decode(source)
        transparent_measurements[source.name] = source_requirements(source.name, before, source_decode)
        try:
            after, structural_error = inspect(target), None
        except WebPContainerError as error:
            after, structural_error = None, str(error)
        decoded_candidate, decode_error = candidate_decode(target)
        result = {"fixture": source.name, "source": before, "candidate": after,
                  "candidate_structural_error": structural_error, "decoded_canvases": len(source_decode["frames"]),
                  "raw_canvases_equal": decoded_candidate is not None and source_decode["frames"] == decoded_candidate["frames"],
                  "oriented_canvases_equal": decoded_candidate is not None and source_decode["displayed"] == decoded_candidate["displayed"],
                  "orientation_retained": decoded_candidate is not None and source_decode["orientation"] == decoded_candidate["orientation"],
                  "source_decode": source_decode, "candidate_decode": decoded_candidate,
                  "candidate_decode_error": decode_error,
                  "compressed_payloads_identical": after is not None and compressed(before) == compressed(after),
                  "diagnostics": {"stdout": process.stdout.strip(), "stderr": process.stderr.strip()}}
        assert_candidate_observations(result)
        results.append(result)
    assert len(results) == 8, "fixture count"
    assert all(all(value) for value in transparent_measurements.values()), f"fixture decoded colored transparent pixels: {transparent_measurements}"
    print(json.dumps({"candidate": "ExifTool 13.59 -overwrite_original -all= --icc_profile:all",
                      "scope": "owned WebP candidate observation; not sanitization proof", "fixtures": results}, indent=2))
