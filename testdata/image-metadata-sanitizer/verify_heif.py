#!/usr/bin/env python3
"""Independent advancing graph/retention/privacy observer; no rewrite imports.

Observes typed IDs, every extent/property/association/reference and private
names, not just primaryExif/frame0. It is not an HEVC conformance oracle: real
syntax validation and independently authored/native entropy proofs are separate.
The same complete verify() is called for success and output-sensitivity controls.
"""
import hashlib
import json
import struct
import sys
from pathlib import Path


class ObservationError(ValueError):
    pass


def check(condition, detail):
    if not condition:
        raise ObservationError("HEIF observation " + detail)


class Cursor:
    def __init__(self, data, start, stop):
        self.data, self.at, self.stop = data, start, stop

    def take(self, count):
        check(0 <= count <= self.stop - self.at, "record extent")
        value = self.data[self.at:self.at + count]
        self.at += count
        return value

    def number(self, width):
        return int.from_bytes(self.take(width), "big")

    def text(self):
        start = self.at
        while self.at < self.stop and self.data[self.at]:
            self.at += 1
            check(self.at - start <= 4096, "text budget")
        check(self.at < self.stop, "text terminator")
        text = self.data[start:self.at]
        self.at += 1
        return text

    def done(self):
        check(self.at == self.stop, "unconsumed record")


def observe(data):
    check(0 < len(data) <= 32 * 1024 * 1024, "file guard")
    boxes_count = 0
    def descend(start, stop):
        nonlocal boxes_count
        cursor, result = Cursor(data, start, stop), []
        while cursor.at < stop:
            boxes_count += 1
            check(boxes_count <= 65536, "box count")
            size, kind = cursor.number(4), cursor.take(4)
            check(size >= 8 and size - 8 <= stop - cursor.at, "box bounded advance")
            body_start = cursor.at
            cursor.take(size - 8)
            result.append((kind, body_start, cursor.at))
        cursor.done()
        return result
    top = descend(0, len(data))
    check([k for k, _, _ in top if k not in (b"free", b"skip")] == [b"ftyp", b"meta", b"mdat"], "top shape")
    locations = {k: (a, b) for k, a, b in top if k not in (b"free", b"skip")}
    check(data[slice(*locations[b"ftyp"])] == b"heic\0\0\0\0mif1heicmiaf", "brand state")
    def submapping(start, stop, allowed, required):
        entries = descend(start, stop)
        kinds = [k for k, _, _ in entries]
        check(len(kinds) == len(set(kinds)) and set(kinds) <= allowed and required <= set(kinds), "child census")
        return {k: (a, b) for k, a, b in entries}
    a, b = locations[b"meta"]
    cursor = Cursor(data, a, b)
    check(cursor.take(4) == bytes(4), "meta version/flags")
    required = {b"hdlr", b"pitm", b"iinf", b"iloc", b"iprp"}
    children = submapping(cursor.at, b, required | {b"iref"}, required)
    def record(kind):
        return Cursor(data, *children[kind])
    c = record(b"hdlr")
    check(c.take(24) == bytes(8) + b"pict" + bytes(12), "handler technical fields")
    handler = c.text()
    c.done()
    c = record(b"pitm")
    check(c.take(4) == bytes(4), "pitm version/flags")
    primary = c.number(2)
    c.done()
    c = record(b"iinf")
    check(c.take(4) == bytes(4), "iinf version/flags")
    count = c.number(2)
    check(1 <= count <= 3, "typed item count")
    entries = descend(c.at, c.stop)
    check(len(entries) == count, "iinf count")
    items = {}
    for kind, a, b in entries:
        c = Cursor(data, a, b)
        check(kind == b"infe" and c.number(1) == 2, "infe version")
        flags, identity, protection, item_type = c.number(3), c.number(2), c.number(2), c.take(4)
        check(flags <= 1 and protection == 0 and identity > 0 and identity not in items
              and item_type in (b"hvc1", b"Exif", b"mime"), "typed descriptor ID/flags/type")
        name = c.text()
        content, encoding = b"", b""
        if item_type == b"mime":
            content = c.text()
            encoding = c.text() if c.at < c.stop else b""
            check(content == b"application/rdf+xml" and not encoding, "MIME schema")
        c.done()
        items[identity] = {"type": item_type, "name": name, "flags": flags}
    check(items.get(primary, {}).get("type") == b"hvc1"
          and sum(item["type"] == b"hvc1" for item in items.values()) == 1, "single typed primary")
    c = record(b"iloc")
    version = c.number(1)
    check(version in (0, 1) and c.number(3) == 0 and c.take(2) == b"\x44\x40", "iloc fields")
    check(c.number(2) == len(items), "iloc declared count")
    extents, fields = {}, {}
    mstart, mend = locations[b"mdat"]
    for _ in items:
        identity = c.number(2)
        check(not version or c.number(2) == 0, "iloc construction")
        reference, base = c.number(2), c.number(4)
        check(c.number(2) == 1, "iloc extent count")
        offset_at, offset, length_at, length = c.at, c.number(4), c.at, c.number(4)
        start = base + offset
        check(identity in items and identity not in extents and reference == 0 and length > 0
              and mstart <= start <= mend and length <= mend - start, "iloc typed extent ownership")
        extents[identity] = (start, start + length)
        fields[identity] = {"base": base, "offset": offset, "offset_at": offset_at, "length_at": length_at}
    c.done()
    sorted_extents = sorted(extents.values())
    check(sorted_extents[0][0] == mstart and sorted_extents[-1][1] == mend
          and all(b == a for (_, b), (a, _) in zip(sorted_extents, sorted_extents[1:])), "complete disjoint physical mdat")
    references = {}
    if b"iref" in children:
        c = record(b"iref")
        check(c.take(4) == bytes(4), "iref version/flags")
        for kind, a, b in descend(c.at, c.stop):
            c = Cursor(data, a, b)
            source, count, target = c.number(2), c.number(2), c.number(2)
            c.done()
            check(kind == b"cdsc" and source in items and target in items and source != primary
                  and count == 1 and source not in references, "typed reference census")
            references[source] = target
    check(set(references) == set(items) - {primary}, "descriptive graph coverage")
    for start in references:
        seen, at = set(), start
        while at != primary:
            check(at not in seen and at in references, "reference DAG/root")
            seen.add(at)
            at = references[at]
    iprp = submapping(*children[b"iprp"], {b"ipco", b"ipma"}, {b"ipco", b"ipma"})
    property_entries = descend(*iprp[b"ipco"])
    properties = [(kind, data[a:b]) for kind, a, b in property_entries]
    check(len(properties) <= 7, "property guard")
    c = Cursor(data, *iprp[b"ipma"])
    check(c.number(1) == 0, "ipma version")
    flags = c.number(3)
    check(flags in (0, 1) and c.number(4) == 1 and c.number(2) == primary, "ipma typed owner/count")
    count = c.number(1)
    check(count == len(properties), "ipma complete property coverage")
    links = []
    width, mask = (2, 32767) if flags else (1, 127)
    association_start = c.at
    for _ in range(count):
        value = c.number(width)
        index = value & mask
        check(1 <= index <= len(properties) and index not in [i for i, _ in links], "ipma unique typed index")
        links.append((index, bool(value & (mask + 1))))
    c.done()
    coded = data[slice(*extents[primary])]
    check(len(coded) >= 6 and int.from_bytes(coded[:4], "big") == len(coded) - 4, "primary single NAL framing")
    return {"primary": primary, "items": items, "extents": extents, "location_fields": fields,
            "coded": coded, "properties": properties, "links": links, "references": references,
            "handler": handler, "top": [k for k, _, _ in top], "children": children,
            "property_entries": property_entries, "association_start": association_start,
            "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}


def verify(original, candidate):
    before, after = observe(original), observe(candidate)
    check(after["top"] == [b"ftyp", b"meta", b"mdat"] and b"iref" not in after["children"], "physical metadata boxes/references removed")
    check(after["primary"] == before["primary"] and set(after["items"]) == {before["primary"]}
          and after["items"][after["primary"]] == {"type": b"hvc1", "name": b"", "flags": 0}
          and not after["handler"] and not after["references"], "canonical private names/item graph removed")
    check(after["coded"] == before["coded"], "all retained compressed picture bytes")
    check(after["properties"] == before["properties"], "all retained configuration/rendering property bytes")
    check(after["links"] == before["links"], "all retained property associations/essential semantics")
    check(after["location_fields"][after["primary"]]["base"] == 0, "canonical extent base")
    return {"source_sha256": before["sha256"], "output_sha256": after["sha256"], "output_size": after["size"],
            "primary": before["primary"], "coded_sha256": hashlib.sha256(after["coded"]).hexdigest(),
            "property_sha256": [hashlib.sha256(value).hexdigest() for _, value in after["properties"]],
            "source_typed_references": before["references"], "output_typed_references": {},
            "private_names_removed": True, "descriptive_payloads_physically_removed": True,
            "all_retained_compressed_properties_associations_identical": True}


if __name__ == "__main__":
    try:
        result = verify(Path(sys.argv[1]).read_bytes(), Path(sys.argv[2]).read_bytes())
        print(json.dumps(result, sort_keys=True))
    except ObservationError as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
