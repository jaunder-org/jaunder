#!/usr/bin/env python3
"""Owned HEIF byte-copy prototype: bounded hvc1 still-IDR, not ingress.

No decoder/encoder/reconstruction/native calls. Only syntax neighbour/scalar
state is allocated. Unknown schemas reject; no original fallback. ICC, alpha,
grid/derived/thumbnail/aux/multi-image and HDR are outside this proof lane.
"""
import struct
import sys
from pathlib import Path
from hevc_headers import HEVCDomainError
from validate_hevc import validate_nals
import heif_metadata

MAX_FILE, MAX_META, MAX_RECORDS = 32 * 1024 * 1024, 8 * 1024 * 1024, 65536
BRANDS = b"heic\0\0\0\0mif1heicmiaf"


class HEIFDomainError(ValueError):
    pass


def require(condition, detail):
    if not condition:
        raise HEIFDomainError("invalid/unsupported HEIF " + detail)


def box(kind, data):
    require(len(data) <= MAX_FILE - 8, "box output budget")
    return struct.pack(">I4s", len(data) + 8, kind) + data


class Reader:
    def __init__(self, data):
        self.data, self.records = data, 0

    def boxes(self, start, end):
        result = []
        while start < end:
            self.records += 1
            require(self.records <= MAX_RECORDS and start + 8 <= end, "box header/record budget")
            size, kind = struct.unpack_from(">I4s", self.data, start)
            require(8 <= size <= end - start, "box extent/32-bit shape")
            result.append((kind, start + 8, start + size))
            start += size
        require(start == end, "box advancing boundary")
        return result

    def children(self, start, end, allowed, required):
        result = {}
        for kind, a, b in self.boxes(start, end):
            require(kind in allowed and kind not in result, "unknown/duplicate child box")
            result[kind] = (a, b)
        require(required <= result.keys(), "required child boxes")
        return result


def full(data, versions=(0,), flags=(0,)):
    require(len(data) >= 4 and data[0] in versions and int.from_bytes(data[1:4], "big") in flags,
            "fullbox version/flags")
    return data[0], int.from_bytes(data[1:4], "big"), data[4:]


def string(data, at):
    end = data.find(b"\0", at)
    require(end >= at and end - at <= 4096, "item/handler name termination/budget")
    try:
        data[at:end].decode("utf-8")
    except UnicodeDecodeError as error:
        raise HEIFDomainError("invalid/unsupported HEIF item/handler name encoding") from error
    return data[at:end], end + 1


def configuration(payload):
    require(len(payload) >= 23, "hvcC header")
    require(payload[0] == 1 and payload[1] == 3 and payload[2:6] == b"\x70\0\0\0"
            and payload[6:12] == bytes(6) and payload[12] == 30,
            "hvcC Main Still Picture profile/level")
    require(payload[13:22] == b"\xf0\0\xfc\xfd\xf8\xf8\0\0\x0f",
            "hvcC reserved/format/temporal/length fields")
    require(payload[22] == 3, "hvcC unknown/extra NAL arrays")
    at, nals = 23, []
    for expected in (32, 33, 34):
        require(at + 5 <= len(payload), "hvcC array extent")
        kind, count, length = struct.unpack_from(">BHH", payload, at)
        at += 5
        require(kind == (expected | 128) and count == 1 and 2 <= length <= len(payload) - at,
                "hvcC typed NAL array/count")
        nals.append(payload[at:at + length])
        at += length
    require(at == len(payload), "hvcC unowned/unknown NAL bytes")
    return nals


def parse(data):
    require(16 <= len(data) <= MAX_FILE, "file budget")
    reader = Reader(data)
    top = reader.boxes(0, len(data))
    require([kind for kind, _, _ in top if kind not in (b"free", b"skip")] == [b"ftyp", b"meta", b"mdat"],
            "top-level box shape")
    mapping = {kind: (a, b) for kind, a, b in top if kind not in (b"free", b"skip")}
    require(data[slice(*mapping[b"ftyp"])] == BRANDS, "brands/minor/compatible identities")
    start, end = mapping[b"meta"]
    meta_length = end - start
    require(end - start <= MAX_META and data[start:start + 4] == bytes(4), "meta version/flags/budget")
    required = {b"hdlr", b"iloc", b"iinf", b"pitm", b"iprp"}
    children = reader.children(start + 4, end, required | {b"iref"}, required)
    def payload(kind):
        return data[slice(*children[kind])]
    _, _, handler = full(payload(b"hdlr"))
    require(len(handler) >= 21 and handler[:20] == bytes(4) + b"pict" + bytes(12), "handler type/reserved")
    _, at = string(handler, 20)
    require(at == len(handler), "handler trailing bytes")
    _, _, primary_data = full(payload(b"pitm"))
    require(len(primary_data) == 2, "pitm extent")
    primary = int.from_bytes(primary_data, "big")
    start, end = children[b"iinf"]
    require(end - start >= 6, "iinf header")
    full(data[start:start + 4])
    count = int.from_bytes(data[start + 4:start + 6], "big")
    require(1 <= count <= 3, "item count/budget")
    entries = reader.boxes(start + 6, end)
    require(len(entries) == count, "iinf declared count")
    items = {}
    for kind, a, b in entries:
        require(kind == b"infe", "iinf entry type")
        _, flags, record = full(data[a:b], (2,), (0, 1))
        require(len(record) >= 9, "infe header")
        identity, protection, item_type = struct.unpack_from(">HH4s", record)
        require(identity > 0 and identity not in items and protection == 0, "infe ID/protection/duplicate")
        _, at = string(record, 8)
        require(item_type in (b"hvc1", b"Exif", b"mime"), "unknown/aux/grid/derived/thumbnail item type")
        if item_type == b"mime":
            mime, at = string(record, at)
            require(mime == b"application/rdf+xml", "unknown MIME schema")
            if at < len(record):
                encoding, at = string(record, at)
                require(not encoding, "MIME content encoding")
        require(at == len(record) and (identity != primary or flags == 0), "infe trailing/primary hidden flag")
        items[identity] = item_type
    require(items.get(primary) == b"hvc1" and list(items.values()).count(b"hvc1") == 1
            and list(items.values()).count(b"Exif") <= 1 and list(items.values()).count(b"mime") <= 1,
            "typed single primary image")
    location = payload(b"iloc")
    version, _, location = full(location, (0, 1))
    require(len(location) >= 4 and location[:2] == b"\x44\x40", "iloc offset/length/base/index sizes")
    count = int.from_bytes(location[2:4], "big")
    require(count == len(items), "iloc declared item count")
    at, extents = 4, {}
    mstart, mend = mapping[b"mdat"]
    for _ in range(count):
        require(at + 2 <= len(location), "iloc item bounds")
        identity = int.from_bytes(location[at:at + 2], "big")
        at += 2
        if version:
            require(at + 2 <= len(location), "iloc construction bounds")
            require(location[at:at + 2] == bytes(2), "iloc construction method/reserved")
            at += 2
        require(at + 16 <= len(location), "iloc record bounds")
        reference, base, extent_count, offset, length = struct.unpack_from(">HIHII", location, at)
        at += 16
        require(identity in items and identity not in extents, "iloc typed ID/duplicate")
        require(reference == 0 and extent_count == 1, "iloc external reference/extent count")
        absolute = base + offset
        require(length > 0 and mstart <= absolute <= mend and length <= mend - absolute, "iloc offset/base/mdat ownership")
        extents[identity] = (absolute, length)
    require(at == len(location) and extents.keys() == items.keys(), "iloc trailing/ID coverage")
    ranges = sorted((a, a + length) for a, length in extents.values())
    require(all(b <= c for (_, b), (c, _) in zip(ranges, ranges[1:])), "iloc overlapping extents")
    require(ranges[0][0] == mstart and ranges[-1][1] == mend
            and all(b == c for (_, b), (c, _) in zip(ranges, ranges[1:])), "unowned mdat bytes")
    require(meta_length + sum(length for identity, (_, length) in extents.items() if identity != primary) <= MAX_META,
            "metadata aggregate budget")
    references = {}
    if b"iref" in children:
        start, end = children[b"iref"]
        require(end - start >= 4, "iref header")
        full(data[start:start + 4])
        for kind, a, b in reader.boxes(start + 4, end):
            require(kind == b"cdsc" and b - a == 6, "unknown/aux/thumbnail reference type/extent")
            source, count, target = struct.unpack_from(">HHH", data, a)
            require(source in items and target in items and source != primary and count == 1
                    and source not in references, "typed descriptive reference/duplicate/count")
            references[source] = target
    require(set(references) == set(items) - {primary}, "descriptive reference ownership")
    for identity in references:
        visited, current = set(), identity
        while current != primary:
            require(current not in visited and current in references, "reference cycle/unrooted graph")
            visited.add(current)
            current = references[current]
    start, end = children[b"iprp"]
    iprp = reader.children(start, end, {b"ipco", b"ipma"}, {b"ipco", b"ipma"})
    properties = [(kind, data[a:b]) for kind, a, b in reader.boxes(*iprp[b"ipco"])]
    kinds = [kind for kind, _ in properties]
    require(5 <= len(properties) <= 7 and len(kinds) == len(set(kinds))
            and set(kinds) <= {b"hvcC", b"colr", b"ispe", b"clap", b"pixi", b"irot", b"imir"}
            and {b"hvcC", b"colr", b"ispe", b"clap", b"pixi"} <= set(kinds), "unknown/duplicate/rendering property set")
    _, flags, associations = full(data[slice(*iprp[b"ipma"])], (0,), (0, 1))
    require(len(associations) >= 7 and associations[:4] == b"\0\0\0\1"
            and int.from_bytes(associations[4:6], "big") == primary, "property association owner/count")
    count = associations[6]
    width = 2 if flags else 1
    require(count == len(properties) and len(associations) == 7 + count * width, "property association extent/count")
    links = []
    for at in range(7, len(associations), width):
        value = int.from_bytes(associations[at:at + width], "big")
        essential, index = bool(value & (1 << (width * 8 - 1))), value & ((1 << (width * 8 - 1)) - 1)
        require(1 <= index <= len(properties) and index not in [i for i, _ in links], "property typed index/duplicate")
        links.append((index, essential))
    transforms = [properties[index - 1][0] for index, _ in links if properties[index - 1][0] in (b"clap", b"irot", b"imir")]
    require(transforms == [kind for kind in (b"clap", b"irot", b"imir") if kind in kinds], "transform association order")
    for kind, value in properties:
        if kind == b"colr":
            require(value == b"nclx\0\1\0\x0d\0\6\x80", "nclx/ICC/HDR color envelope")
        elif kind == b"ispe":
            require(value == bytes(4) + struct.pack(">II", 64, 64), "ispe coded canvas/pixel budget")
        elif kind == b"pixi":
            require(value == bytes(4) + b"\3\10\10\10", "pixi channel/depth/alpha envelope")
        elif kind == b"clap":
            require(value == struct.pack(">IIIIiIiI", 32, 1, 24, 1, -32, 2, -40, 2), "clap aperture/crop envelope")
        elif kind == b"irot":
            require(len(value) == 1 and value[0] <= 3, "irot angle/reserved")
        elif kind == b"imir":
            require(len(value) == 1 and value[0] <= 1, "imir axis/reserved")
    coded_start, coded_length = extents[primary]
    coded = data[coded_start:coded_start + coded_length]
    require(len(coded) >= 6 and int.from_bytes(coded[:4], "big") == len(coded) - 4,
            "single primary NAL length/extent")
    params = configuration(properties[kinds.index(b"hvcC")][1])
    try:
        ownership = validate_nals(*params, coded[4:])
    except HEVCDomainError as error:
        raise HEIFDomainError("invalid/unsupported HEIF coded ownership: " + str(error)) from error
    try:
        for identity, kind in items.items():
            if identity == primary:
                continue
            a, length = extents[identity]
            (heif_metadata.exif if kind == b"Exif" else heif_metadata.xmp)(data[a:a + length])
    except heif_metadata.MetadataDomainError as error:
        raise HEIFDomainError(str(error)) from error
    return {"primary": primary, "coded": coded, "properties": properties, "links": links,
            "ownership": ownership, "references": references, "items": items, "extents": extents}


def serialize(state):
    identity = state["primary"]
    hdlr = box(b"hdlr", bytes(8) + b"pict" + bytes(12) + b"\0")
    pitm = box(b"pitm", bytes(4) + struct.pack(">H", identity))
    infe = box(b"infe", b"\2\0\0\0" + struct.pack(">HH4s", identity, 0, b"hvc1") + b"\0")
    iinf = box(b"iinf", bytes(4) + b"\0\1" + infe)
    ipco = box(b"ipco", b"".join(box(kind, value) for kind, value in state["properties"]))
    links = bytes(index | (128 if essential else 0) for index, essential in state["links"])
    ipma = box(b"ipma", bytes(4) + b"\0\0\0\1" + struct.pack(">HB", identity, len(links)) + links)
    iprp = box(b"iprp", ipco + ipma)
    ftyp = box(b"ftyp", BRANDS)
    def meta(offset):
        iloc = box(b"iloc", bytes(4) + b"\x44\x40\0\1"
                   + struct.pack(">HHIHII", identity, 0, 0, 1, offset, len(state["coded"])))
        return box(b"meta", bytes(4) + hdlr + pitm + iloc + iinf + iprp)
    provisional = meta(0)
    result = ftyp + meta(len(ftyp) + len(provisional) + 8) + box(b"mdat", state["coded"])
    require(len(result) <= MAX_FILE, "output file budget")
    # Independent verifier is test-side. This self-check is NOT its substitute.
    verified = parse(result)
    require(verified["coded"] == state["coded"] and verified["properties"] == state["properties"]
            and verified["links"] == state["links"], "output retention invariant")
    return result


def rewrite(source, destination):
    source, destination = Path(source), Path(destination)
    require(source.resolve() != destination.resolve(), "input/output alias")
    require(not destination.exists(), "output already exists")
    with source.open("rb") as stream:
        data = stream.read(MAX_FILE + 1)
    result = serialize(parse(data))
    destination.write_bytes(result)


if __name__ == "__main__":
    try:
        rewrite(sys.argv[1], sys.argv[2])
    except HEIFDomainError as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
