#!/usr/bin/env python3
"""Diagnostic counterexample, NOT HEIF admission or a metadata rewriter.

For the existing owned libheif witness only, enumerate NAL framing and append
synthetic private bytes *inside* its declared VCL NAL/item extent. This tests
whether a real consumer can certify coded-data ownership. H.265 (09/2023)
7.3.8.1 locates end_of_slice_segment_flag through arithmetic-coded syntax;
a length/header probe does not locate that boundary. No entropy validity or
privacy claim follows from this inspector or from successful decoding.
"""
import hashlib
import json
import struct
import sys
from pathlib import Path

MAX_FILE = 32 * 1024 * 1024
PRIVATE_SUFFIX = b"Owned synthetic private VCL trailer; not pixels\x80"


def require(condition, detail):
    if not condition:
        raise AssertionError("HEIF checkpoint fixture mismatch: " + detail)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def boxes(data, start, end):
    result = []
    while start < end:
        require(len(result) < 65536 and start + 8 <= end, "box header/count")
        size, kind = struct.unpack_from(">I4s", data, start)
        require(8 <= size <= end - start, "box extent (32-bit only)")
        result.append((kind, start + 8, start + size))
        start += size
    require(start == end, "box boundary")
    return result


def nals(payload, length_width):
    at, result = 0, []
    while at < len(payload):
        require(len(result) < 65536 and at + length_width <= len(payload), "NAL length/count")
        size = int.from_bytes(payload[at:at + length_width], "big")
        start = at + length_width
        end = start + size
        require(size >= 2 and end <= len(payload), "NAL extent")
        nal = payload[start:end]
        require(not nal[0] & 128 and nal[1] & 7, "NAL forbidden/temporal header")
        result.append({"offset": start, "length": size, "type": (nal[0] >> 1) & 63,
                       "layer": ((nal[0] & 1) << 5) | (nal[1] >> 3),
                       "temporal_id_plus1": nal[1] & 7, "sha256": sha(nal)})
        at = end
    return result


def inspect(data):
    require(len(data) <= MAX_FILE, "file budget")
    top = boxes(data, 0, len(data))
    require([kind for kind, _, _ in top] == [b"ftyp", b"meta", b"mdat"], "top shape")
    _, fstart, fend = top[0]
    require(data[fstart:fend] == b"heic\0\0\0\0mif1heicmiaf", "brands/version")
    _, mstart, mend = top[1]
    require(data[mstart:mstart + 4] == bytes(4), "meta fullbox")
    children = boxes(data, mstart + 4, mend)
    require(len(children) == 6 and {kind for kind, _, _ in children} ==
            {b"hdlr", b"iloc", b"iinf", b"iref", b"pitm", b"iprp"}, "meta child set")
    mapping = {kind: (start, end) for kind, start, end in children}
    pstart, pend = mapping[b"pitm"]
    require(data[pstart:pstart + 4] == bytes(4) and pend == pstart + 6, "pitm")
    primary = int.from_bytes(data[pstart + 4:pend], "big")
    istart, iend = mapping[b"iinf"]
    require(data[istart:istart + 6] == b"\0\0\0\0\0\3", "iinf version/count")
    types = {}
    for kind, start, end in boxes(data, istart + 6, iend):
        require(kind == b"infe" and end - start >= 13 and data[start] == 2,
                "infe version/header")
        item_id, protection = struct.unpack_from(">HH", data, start + 4)
        require(item_id not in types and protection == 0, "infe ID/protection")
        types[item_id] = data[start + 8:start + 12].decode("ascii")
    require(len(types) == 3 and sorted(types.values()) == ["Exif", "hvc1", "mime"]
            and types.get(primary) == "hvc1", "typed primary")
    lstart, lend = mapping[b"iloc"]
    require(data[lstart:lstart + 8] == b"\0\0\0\0\x44\x40\0\3", "iloc version/sizes/count")
    at, extents, length_fields = lstart + 8, {}, {}
    _, dstart, dend = top[2]
    for _ in range(3):
        require(at + 18 <= lend, "iloc record bounds")
        item_id, reference, base, count, offset, length = struct.unpack_from(">HHIHII", data, at)
        require(item_id in types and item_id not in extents and reference == 0 and count == 1,
                "iloc ID/reference/count")
        absolute = base + offset
        require(dstart <= absolute <= dend and length <= dend - absolute, "iloc mdat ownership")
        extents[item_id] = (absolute, length)
        length_fields[item_id] = at + 14
        at += 18
    require(at == lend and set(extents) == set(types), "iloc consumption/ID set")
    owned = sorted((start, start + length) for start, length in extents.values() if length)
    require(all(right <= following for (_, right), (following, _) in zip(owned, owned[1:])),
            "iloc overlap")
    require(owned[0][0] == dstart and owned[-1][1] == dend
            and all(right == following for (_, right), (following, _) in zip(owned, owned[1:])),
            "unowned mdat bytes")
    rstart, rend = mapping[b"iprp"]
    props = boxes(data, rstart, rend)
    require([kind for kind, _, _ in props] == [b"ipco", b"ipma"], "property containers")
    _, cstart, cend = props[0]
    properties = boxes(data, cstart, cend)
    require([kind for kind, _, _ in properties] == [b"hvcC", b"colr", b"ispe", b"clap", b"pixi"],
            "observed property order")
    _, hstart, hend = properties[0]
    hvcc = data[hstart:hend]
    require(len(hvcc) >= 23 and hvcc[0] == 1 and hvcc[21] & 3 == 3 and hvcc[22] == 3,
            "hvcC configuration/length/count")
    at, config = 23, []
    for expected in (32, 33, 34):
        require(at + 5 <= len(hvcc), "hvcC array bounds")
        kind, count, length = struct.unpack_from(">BHH", hvcc, at)
        at += 5
        require(kind & 63 == expected and count == 1 and at + length <= len(hvcc), "hvcC observed arrays")
        nal = hvcc[at:at + length]
        framing = nals(len(nal).to_bytes(4, "big") + nal, 4)
        require(framing[0]["type"] == expected, "hvcC typed NAL")
        config.append({**framing[0], "array_header": kind, "hex": nal.hex()})
        at += length
    require(at == len(hvcc), "hvcC complete framing")
    start, length = extents[primary]
    payload = data[start:start + length]
    coded = nals(payload, 4)
    return {"sha256": sha(data), "length": len(data), "primary": primary,
            "types": types, "extents": extents,
            "properties": [{"type": kind.decode("ascii"), "hex": data[start:end].hex()}
                           for kind, start, end in properties],
            "configuration_nals": config, "coded_nals": coded,
            "primary_length_field": length_fields[primary], "mdat_size_field": dstart - 8}


def main():
    root, output = Path(sys.argv[1]), Path(sys.argv[2])
    source = (root / "input/device-like.heic").read_bytes()
    candidate = (root / "candidate/device-like.heic").read_bytes()
    before, baseline = inspect(source), inspect(candidate)
    require(before["configuration_nals"] == baseline["configuration_nals"]
            and before["properties"] == baseline["properties"]
            and before["coded_nals"] == baseline["coded_nals"], "original/candidate coded state")
    require(len(baseline["coded_nals"]) == 1 and baseline["coded_nals"][0]["type"] == 20,
            "single IDR_N_LP")
    start, length = baseline["extents"][baseline["primary"]]
    require(start + length == len(candidate), "primary final extent")
    changed = bytearray(candidate + PRIVATE_SUFFIX)
    for field in (baseline["primary_length_field"], baseline["mdat_size_field"], start):
        value = int.from_bytes(changed[field:field + 4], "big")
        struct.pack_into(">I", changed, field, value + len(PRIVATE_SUFFIX))
    after = inspect(changed)
    require(after["properties"] == baseline["properties"], "mutation property stability")
    require(changed[start + 4:start + length] == candidate[start + 4:start + length],
            "original VCL bytes unchanged")
    output.mkdir(parents=True, exist_ok=False)
    (output / "private-vcl-suffix.heic").write_bytes(changed)
    report = {"status": "diagnostic-only-no-admission", "source": before, "candidate": baseline,
              "private_vcl_suffix": after, "private_suffix_hex": PRIVATE_SUFFIX.hex(),
              "original_files_unchanged": (root / "input/device-like.heic").read_bytes() == source
              and (root / "candidate/device-like.heic").read_bytes() == candidate,
              "unproven": ["VPS/SPS/PPS RBSP syntax", "slice header syntax",
                           "CABAC termination/coded-data ownership", "privacy", "general conformance"]}
    (output / "framing.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"status": report["status"], "report": str(output / "framing.json"),
                      "control": str(output / "private-vcl-suffix.heic")}, sort_keys=True))


if __name__ == "__main__":
    main()
