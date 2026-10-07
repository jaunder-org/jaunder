#!/usr/bin/env python3
"""Fail-closed, fixture-scoped verifier for the #1702 feasibility witness.

It independently verifies one baseline JPEG scan and a narrow HEIC BMFF shape.
It is intentionally not a universal progressive JPEG or HEIF parser.
"""
import hashlib
import json
import struct
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])


def digest(value):
    return hashlib.sha256(value).hexdigest()


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def jpeg(path):
    data = path.read_bytes()
    require(data[:2] == b"\xff\xd8", f"{path}: not JPEG")
    position, scan, markers, icc_parts = 2, None, [], []
    while position < len(data):
        require(data[position] == 0xFF, f"{path}: expected marker")
        while position < len(data) and data[position] == 0xFF:
            position += 1
        require(position < len(data), f"{path}: truncated marker")
        marker = data[position]
        position += 1
        if marker == 0xDA:
            require(position + 2 <= len(data), f"{path}: truncated SOS")
            length = int.from_bytes(data[position : position + 2], "big")
            start = position + length
            end = data.rfind(b"\xff\xd9")
            require(end >= start, f"{path}: missing EOI")
            scan = data[start:end]
            position = end
            break
        require(marker not in range(0xD0, 0xD8) and marker != 0x01, "unsupported standalone marker")
        require(position + 2 <= len(data), f"{path}: truncated segment")
        length = int.from_bytes(data[position : position + 2], "big")
        require(length >= 2 and position + length <= len(data), f"{path}: invalid segment")
        payload = data[position + 2 : position + length]
        markers.append(marker)
        if marker == 0xE2 and payload.startswith(b"ICC_PROFILE\0"):
            require(len(payload) >= 14, f"{path}: truncated ICC segment")
            icc_parts.append((payload[12], payload[13], payload[14:]))
        position += length
    require(scan is not None, f"{path}: this bounded verifier requires one scan")
    require(position + 2 == len(data), f"{path}: trailing bytes after first scan")
    if icc_parts:
        count = icc_parts[0][1]
        require(count == len(icc_parts), f"{path}: incomplete ICC segment set")
        require(sorted(index for index, _, _ in icc_parts) == list(range(1, count + 1)), f"{path}: invalid ICC sequence")
        icc = b"".join(payload for _, _, payload in sorted(icc_parts))
    else:
        icc = None
    return {"scan": digest(scan), "markers": markers, "icc": icc}


def boxes(data, start, end):
    result = []
    while start < end:
        require(start + 8 <= end, "truncated ISO-BMFF box")
        size = int.from_bytes(data[start : start + 4], "big")
        typ = data[start + 4 : start + 8]
        header = 8
        if size == 1:
            require(start + 16 <= end, "truncated extended ISO-BMFF box")
            size, header = int.from_bytes(data[start + 8 : start + 16], "big"), 16
        elif size == 0:
            size = end - start
        require(size >= header and start + size <= end, "invalid ISO-BMFF box")
        result.append((typ, start, start + size, header))
        start += size
    return result


def read_uint(data, position, width, end):
    require(position + width <= end, "truncated ISO-BMFF field")
    return int.from_bytes(data[position : position + width], "big"), position + width


def iloc_extents(data, start, end):
    require(start + 6 <= end, "truncated iloc")
    version = data[start]
    require(version == 0, "unsupported iloc version")
    position = start + 4
    offset_size, length_size = data[position] >> 4, data[position] & 15
    base_offset_size, index_size = data[position + 1] >> 4, data[position + 1] & 15
    require((offset_size, length_size, base_offset_size, index_size) == (4, 4, 4, 0), "unsupported iloc field sizes")
    position += 2
    count, position = read_uint(data, position, 2, end)
    require(count == 3, "unsupported iloc item count")
    result = {}
    for _ in range(count):
        item_id, position = read_uint(data, position, 2, end)
        require(item_id not in result, "duplicate iloc item")
        data_reference, position = read_uint(data, position, 2, end)
        require(data_reference == 0, "unsupported iloc data-reference-index")
        base_offset, position = read_uint(data, position, base_offset_size, end)
        extent_count, position = read_uint(data, position, 2, end)
        require(extent_count == 1, "unsupported iloc extent count")
        offset, position = read_uint(data, position, offset_size, end)
        length, position = read_uint(data, position, length_size, end)
        absolute_offset = base_offset + offset
        require(absolute_offset + length <= len(data), "iloc extent outside file")
        result[item_id] = {"length": length, "sha256": digest(data[absolute_offset : absolute_offset + length])}
    require(position == end, "unconsumed iloc bytes")
    return result


def pitm_item_id(data, start, end):
    require(start + 6 == end and data[start] == 0, "unsupported pitm")
    return int.from_bytes(data[start + 4 : end], "big")


def iinf_types(data, start, end):
    require(start + 6 <= end and data[start] == 0, "unsupported iinf")
    count = int.from_bytes(data[start + 4 : start + 6], "big")
    entries = boxes(data, start + 6, end)
    require(count == 3 and len(entries) == count and all(typ == b"infe" for typ, *_ in entries), "unsupported iinf entries")
    result = {}
    for _, entry_start, entry_end, header in entries:
        position = entry_start + header
        require(position + 12 <= entry_end and data[position] == 2, "unsupported infe")
        item_id = int.from_bytes(data[position + 4 : position + 6], "big")
        item_type = data[position + 8 : position + 12]
        require(item_id not in result and item_type in {b"hvc1", b"Exif", b"mime"}, "unsupported infe item")
        result[item_id] = item_type
    require(set(result.values()) == {b"hvc1", b"Exif", b"mime"}, "unsupported iinf types")
    return result


def iref_edges(data, start, end):
    require(start + 4 <= end, "truncated iref")
    version, position = data[start], start + 4
    require(version in (0, 1), "unsupported iref version")
    width, edges = (4 if version else 2), []
    for typ, child_start, child_end, header in boxes(data, position, end):
        position = child_start + header
        source, position = read_uint(data, position, width, child_end)
        count, position = read_uint(data, position, 2, child_end)
        targets = []
        for _ in range(count):
            target, position = read_uint(data, position, width, child_end)
            targets.append(target)
        require(position == child_end, "unconsumed iref child bytes")
        edges.append({"type": typ.decode("latin1"), "from": source, "to": targets})
    return edges


def heif_bytes(data):
    data = bytes(data)
    top = boxes(data, 0, len(data))
    require([typ for typ, *_ in top] == [b"ftyp", b"meta", b"mdat"], "unsupported top-level HEIC shape")
    _, meta_start, meta_end, meta_header = top[1]
    require(meta_start + meta_header + 4 <= meta_end and data[meta_start + meta_header] == 0, "unsupported meta")
    children = boxes(data, meta_start + meta_header + 4, meta_end)
    required = {b"hdlr", b"iloc", b"iinf", b"iref", b"pitm", b"iprp"}
    require(len(children) == len(required) and {typ for typ, *_ in children} == required, "unsupported or duplicate meta children")
    child_map = {typ: (start, end, header) for typ, start, end, header in children}
    iloc_start, iloc_end, iloc_header = child_map[b"iloc"]
    iinf_start, iinf_end, iinf_header = child_map[b"iinf"]
    iref_start, iref_end, iref_header = child_map[b"iref"]
    pitm_start, pitm_end, pitm_header = child_map[b"pitm"]
    locations = iloc_extents(data, iloc_start + iloc_header, iloc_end)
    types = iinf_types(data, iinf_start + iinf_header, iinf_end)
    primary = pitm_item_id(data, pitm_start + pitm_header, pitm_end)
    require(set(locations) == set(types) == {1, 2, 3} and types[primary] == b"hvc1", "unsupported HEIC item identities")
    edges = iref_edges(data, iref_start + iref_header, iref_end)
    require(edges == [{"type": "cdsc", "from": 2, "to": [primary]}, {"type": "cdsc", "from": 3, "to": [primary]}], "unsupported HEIC reference graph")
    return {"primary": {"item_id": primary, "payload": locations[primary]}, "descriptive": {item_id: locations[item_id] for item_id, item_type in types.items() if item_type in {b"Exif", b"mime"}}, "types": {item_id: item_type.decode("latin1") for item_id, item_type in types.items()}, "iref": edges, "mdat": digest(data[top[2][1] + top[2][3] : top[2][2]])}


def heif(path):
    return heif_bytes(path.read_bytes())


def icc(path):
    data = Path(path).read_bytes()
    require(len(data) >= 132 and int.from_bytes(data[:4], "big") == len(data), "invalid ICC size")
    require(data[36:40] == b"acsp", "invalid ICC signature")
    count = int.from_bytes(data[128:132], "big")
    require(132 + count * 12 <= len(data), "invalid ICC tag table")
    tags = {}
    for index in range(count):
        signature, offset, size = struct.unpack_from(">4sII", data, 132 + index * 12)
        require(offset + size <= len(data) and size >= 4, "invalid ICC tag extent")
        tags[signature] = data[offset : offset + size]
    return {"data": data, "tags": tags}


def verify():
    source_jpeg = jpeg(ROOT / "input/device-like.jpg")
    generic_jpeg = jpeg(ROOT / "candidate/device-like.jpg")
    generic_jpeg_second = jpeg(ROOT / "candidate/device-like.second-pass.jpg")
    rewritten_jpeg = jpeg(ROOT / "format-specific/device-like.jpg")
    rewritten_jpeg_second = jpeg(ROOT / "format-specific/device-like.second-pass.jpg")
    require(source_jpeg["scan"] == generic_jpeg["scan"] == rewritten_jpeg["scan"], "JPEG scan changed")
    require(all(marker in {0xE2, 0xDB, 0xC0, 0xC4} for marker in generic_jpeg["markers"]), "JPEG descriptive markers remain")
    require(generic_jpeg["icc"] == source_jpeg["icc"], "generic ICC failure fixture changed")
    require((ROOT / "candidate/device-like.jpg").read_bytes() == (ROOT / "candidate/device-like.second-pass.jpg").read_bytes(), "generic JPEG second-pass idempotence failed")
    require((ROOT / "format-specific/device-like.jpg").read_bytes() == (ROOT / "format-specific/device-like.second-pass.jpg").read_bytes(), "format-specific JPEG second-pass idempotence failed")

    source_heif = heif(ROOT / "input/device-like.heic")
    generic_heif = heif(ROOT / "candidate/device-like.heic")
    require(source_heif["primary"] == generic_heif["primary"], "HEIC primary item changed")
    require(all(payload["length"] == 0 for payload in generic_heif["descriptive"].values()), "HEIC descriptive items remain")
    require(source_heif["types"] == generic_heif["types"] and source_heif["iref"] == generic_heif["iref"], "HEIC typed graph changed")
    require((ROOT / "candidate/device-like.heic").read_bytes() == (ROOT / "candidate/device-like.second-pass.heic").read_bytes(), "HEIC second-pass idempotence failed")

    input_icc = icc(ROOT / "icc/input-v4.icc")
    scrubbed_icc = icc(ROOT / "icc/scrubbed-v4.icc")
    scrubbed_icc_second = icc(ROOT / "icc/scrubbed-v4.second-pass.icc")
    require(input_icc["data"][8] == 4 and input_icc["data"][12:24] == b"linkRGB XYZ ", "wrong supported ICC fixture")
    require(set(input_icc["tags"]) == {b"A2B0", b"wtpt", b"desc", b"cprt", b"pseq", b"psid"}, "missing ICC descriptive fixture tags")
    require(b"mluc" in input_icc["tags"][b"desc"] and b"mluc" in input_icc["tags"][b"cprt"], "missing ICC locale descriptions")
    require(b"enUS" in input_icc["tags"][b"desc"] and b"frFR" in input_icc["tags"][b"desc"], "missing ICC locale variants")
    require(scrubbed_icc["data"] == scrubbed_icc_second["data"], "ICC scrubber second-pass idempotence failed")
    require(set(scrubbed_icc["tags"]) == {b"A2B0", b"wtpt"}, "ICC descriptive tags remain")
    require(rewritten_jpeg["icc"] == scrubbed_icc["data"], "rewritten JPEG does not contain scrubbed ICC")
    require(all(marker in {0xE2, 0xDB, 0xC0, 0xC4} for marker in rewritten_jpeg["markers"]), "rewritten JPEG descriptive markers remain")
    require(scrubbed_icc["data"][24:36] == b"\0" * 12 and scrubbed_icc["data"][48:56] == b"\0" * 8 and scrubbed_icc["data"][80:128] == b"\0" * 48, "ICC descriptive header fields remain")
    require(input_icc["tags"][b"A2B0"] == scrubbed_icc["tags"][b"A2B0"], "ICC transform bytes changed")
    return {"generic_candidate": "expected-icc-retention-failure", "icc_rewrite": "passed", "jpeg_scan": source_jpeg["scan"], "heif_primary": source_heif["primary"], "icc_transform": digest(input_icc["tags"][b"A2B0"])}


if __name__ == "__main__":
    print(json.dumps(verify(), sort_keys=True))
