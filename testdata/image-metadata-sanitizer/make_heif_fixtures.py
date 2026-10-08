#!/usr/bin/env python3
"""TEST construction only. Not upload repair, admission or an output oracle.

Owned compressed artwork/configuration NALs copied from existing CC0 witness.
Constructed hvcC uses bit7 completeness and bit6=0; current libheif writer uses
bit6. ISO edition-qualified conformance remains unresolved until source review.
Existing accepted witness is never modified or relabeled as conformant.
"""
import hashlib
import json
import struct
import sys
import zlib
from pathlib import Path
from verify_heif_coded_boundary import inspect


def box(kind, data):
    return struct.pack(">I4s", len(data) + 8, kind) + data


def construct(primary, descriptions, properties, links, coded, *, base=0, chain=False, names=True, padding=False, version=0):
    identities = [primary] + list(descriptions)
    types = {primary: b"hvc1", **{identity: kind for identity, (kind, _) in descriptions.items()}}
    name = b"Owned synthetic private item name" if names else b""
    handler = b"Owned synthetic private handler name" if names else b""
    hdlr = box(b"hdlr", bytes(8) + b"pict" + bytes(12) + handler + b"\0")
    pitm = box(b"pitm", bytes(4) + struct.pack(">H", primary))
    entries = []
    for identity in reversed(identities):
        kind = types[identity]
        entries.append(box(b"infe", bytes((2, 0, 0, int(identity != primary)))
                           + struct.pack(">HH4s", identity, 0, kind) + name + b"\0"
                           + (b"application/rdf+xml\0" if kind == b"mime" else b"")))
    iinf = box(b"iinf", bytes(4) + struct.pack(">H", len(entries)) + b"".join(entries))
    refs = []
    for identity in descriptions:
        target = next(iter(descriptions)) if chain and types[identity] == b"mime" else primary
        refs.append(box(b"cdsc", struct.pack(">HHH", identity, 1, target)))
    iref = box(b"iref", bytes(4) + b"".join(refs)) if refs else b""
    ipco = box(b"ipco", b"".join(box(kind, value) for kind, value in properties))
    ipma = box(b"ipma", bytes(4) + b"\0\0\0\1" + struct.pack(">HB", primary, len(links)) + bytes(links))
    iprp = box(b"iprp", ipco + ipma)
    payloads = {identity: payload for identity, (_, payload) in descriptions.items()}
    payloads[primary] = coded
    order = list(reversed(identities))
    def meta(start):
        locations, offset = [], start
        for identity in order:
            payload = payloads[identity]
            locations.append(struct.pack(">H", identity) + (bytes(2) if version else b"")
                             + struct.pack(">HIHII", 0, base, 1, offset - base, len(payload)))
            offset += len(payload)
        iloc = box(b"iloc", bytes((version, 0, 0, 0)) + b"\x44\x40" + struct.pack(">H", len(identities)) + b"".join(locations))
        return box(b"meta", bytes(4) + hdlr + pitm + iloc + iinf + iref + iprp)
    ftyp = box(b"ftyp", b"heic\0\0\0\0mif1heicmiaf")
    free = box(b"free", b"Owned synthetic private padding payload") if padding else b""
    provisional = meta(0 if not base else base)
    start = len(ftyp) + len(provisional) + len(free) + 8
    return ftyp + meta(start) + free + box(b"mdat", b"".join(payloads[i] for i in order))


def main():
    fixtures, output = map(Path, sys.argv[1:3])
    output.mkdir(parents=True, exist_ok=False)
    original = (fixtures / "input/device-like.heic").read_bytes()
    observed = inspect(original)
    properties = [(p["type"].encode(), bytes.fromhex(p["hex"])) for p in observed["properties"]]
    hvcc = bytearray(properties[0][1])
    at, changed = 23, []
    for kind in (32, 33, 34):
        if hvcc[at] != (kind | 64):
            raise AssertionError("owned source hvcC identity drift")
        changed.append({"offset_in_hvcc": at, "before": hvcc[at], "constructed": kind | 128})
        hvcc[at] = kind | 128
        at += 5 + int.from_bytes(hvcc[at + 3:at + 5], "big")
    if at != len(hvcc):
        raise AssertionError("owned configuration extent drift")
    properties[0] = (b"hvcC", bytes(hvcc))
    start, length = observed["extents"][observed["primary"]]
    coded = original[start:start + length]
    descriptions = {}
    for identity, kind in ((7, b"Exif"), (9, b"mime")):
        source_id = 2 if kind == b"Exif" else 3
        a, length = observed["extents"][source_id]
        descriptions[identity] = (kind, original[a:a + length])
    links = [129, 2, 3, 5, 132]
    records = {}
    def save(name, data):
        (output / (name + ".heic")).write_bytes(data)
        records[name] = {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    save("constructed", construct(5, descriptions, properties, links, coded))
    save("nonprimary-description", construct(5, descriptions, properties, links, coded, chain=True))
    save("v1-direct-extents", construct(5, descriptions, properties, links, coded, version=1))
    save("base-offset-and-padding", construct(5, descriptions, properties, links, coded, base=17, padding=True))
    permutation = [4, 1, 3, 0, 2]
    reordered = [properties[i] for i in permutation]
    remap = {old + 1: new + 1 for new, old in enumerate(permutation)}
    remapped = [remap[value & 127] | (value & 128) for value in links]
    save("permuted-property-ids", construct(12, descriptions, reordered, remapped, coded))
    for rotation in range(4):
        for mirror in (-1, 0, 1):
            props, assoc = list(properties), list(links)
            props.append((b"irot", bytes((rotation,))))
            assoc.append(128 | len(props))
            if mirror >= 0:
                props.append((b"imir", bytes((mirror,))))
                assoc.append(128 | len(props))
            save("transform-r%d-m%d" % (rotation, mirror), construct(5, descriptions, props, assoc, coded))
    # Original supplementary RGBA artwork, dedicated to CC0-1.0. Native HEIF
    # encoding is performed only test-side by the pinned full witness.
    def png_chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + bytes(value for x in range(32)
                    for value in ((x * 7) % 256, (y * 11) % 256, (x * 3 + y * 5) % 256,
                                  (x * 13 + y * 17) % 256)) for y in range(24))
    png = b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 32, 24, 8, 6, 0, 0, 0))
    png += png_chunk(b"IDAT", zlib.compress(rows)) + png_chunk(b"IEND", b"")
    (output / "alpha-source.png").write_bytes(png)
    (output / "construction.json").write_text(json.dumps({"source_sha256": hashlib.sha256(original).hexdigest(),
        "source_unchanged": (fixtures / "input/device-like.heic").read_bytes() == original,
        "hvcC_constructed_fields": changed, "records": records,
        "scope": "CC0 owned fixture construction, not sanitizer repair; ISO record conformance unresolved"}, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
