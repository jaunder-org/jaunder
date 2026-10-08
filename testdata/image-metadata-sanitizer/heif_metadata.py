#!/usr/bin/env python3
"""Declared Exif/RDF descriptions for the OWNED HEIF proof, not generic Exif.

Known optional technical defaults from the current ExifTool witness are frozen;
orientation other than absent/1, thumbnails, unknown IFD/XMP schemas reject.
The existing restrictive PNG TIFF/XMP parser is reused for its exact schema.
All admitted metadata is discarded; no descriptive bytes are retained.
"""
import struct
import xml.etree.ElementTree as ET
import rewrite_png


class MetadataDomainError(ValueError):
    pass


def require(condition, detail):
    if not condition:
        raise MetadataDomainError("invalid/unsupported HEIF metadata " + detail)


def exif(payload):
    require(12 <= len(payload) <= 4 * 1024 * 1024 and payload[:10] == b"\0\0\0\6Exif\0\0", "Exif header/budget")
    tiff = payload[10:]
    require(tiff[:2] in (b"II", b"MM"), "TIFF endian")
    endian = "<" if tiff[:2] == b"II" else ">"
    require(len(tiff) >= 8 and struct.unpack_from(endian + "H", tiff, 2)[0] == 42, "TIFF version")
    pending = [(struct.unpack_from(endian + "I", tiff, 4)[0], "root")]
    seen, external, structural, count, extended = set(), [], [(0, 8)], 0, False
    covered = bytearray(len(tiff))
    covered[:8] = b"\1" * 8
    while pending:
        offset, schema = pending.pop()
        require(8 <= offset <= len(tiff) - 2 and offset % 2 == 0 and offset not in seen and len(seen) < 3,
                "TIFF directory/cycle")
        seen.add(offset)
        number = struct.unpack_from(endian + "H", tiff, offset)[0]
        count += number
        end = offset + 2 + number * 12 + 4
        require(count <= 64 and end <= len(tiff) and struct.unpack_from(endian + "I", tiff, end - 4)[0] == 0,
                "TIFF records/thumbnail")
        structural.append((offset, end))
        covered[offset:end] = b"\1" * (end - offset)
        tags = set()
        for pos in range(offset + 2, end - 4, 12):
            tag, kind, length = struct.unpack_from(endian + "HHI", tiff, pos)
            widths = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1}
            require(tag not in tags and kind in widths and length > 0, "TIFF duplicate/type/count")
            tags.add(tag)
            size = widths[kind] * length
            start = pos + 8 if size <= 4 else struct.unpack_from(endian + "I", tiff, pos + 8)[0]
            require(size <= len(tiff) and start <= len(tiff) - size
                    and (size <= 4 or start >= 8 and start % 2 == 0), "TIFF field extent")
            value = tiff[start:start + size]
            if size > 4:
                external.append((start, start + size))
                covered[start:start + size] = b"\1" * size
            if schema == "gps":
                shapes = {0: (1, 4), 1: (2, 2), 2: (5, 3), 3: (2, 2), 4: (5, 3)}
                require(tag in shapes and (kind, length) == shapes[tag], "GPS field")
                require(kind != 5 or all(den for _, den in struct.iter_unpack(endian + "II", value)), "GPS denominator")
                require(kind != 2 or value[-1] == 0, "GPS string")
            elif schema == "exif":
                defaults = {0x9000: (7, 4, b"0232"), 0x9101: (7, 4, b"\1\2\3\0"),
                            0xa001: (3, 1, struct.pack(endian + "H", 0xffff))}
                if tag == 0x9003:
                    require(kind == 2 and length == 20 and value[-1] == 0, "Exif date")
                else:
                    require(tag in defaults and (kind, length, value) == defaults[tag], "Exif technical default")
            elif tag in (0x8769, 0x8825):
                require(kind == 4 and length == 1, "TIFF pointer")
                pending.append((struct.unpack(endian + "I", value)[0], "exif" if tag == 0x8769 else "gps"))
                extended |= tag == 0x8769
            elif tag in (0x0112, 0x0213):
                require(kind == 3 and length == 1 and struct.unpack(endian + "H", value)[0] == 1,
                        "Exif orientation/positioning")
                extended |= tag == 0x0213
            else:
                require(tag in {0x010e, 0x010f, 0x0110, 0x0132, 0x013b, 0x8298}
                        and kind == 2 and value[-1] == 0, "TIFF description")
    for i, (a, b) in enumerate(structural):
        require(all(b <= c or d <= a for c, d in structural[i + 1:]), "TIFF directory overlap")
    for i, (a, b) in enumerate(external):
        require(all(b <= c or d <= a for c, d in structural)
                and all(b <= c or d <= a for c, d in external[i + 1:]), "TIFF value overlap")
    require(all(byte == 0 for byte, used in zip(tiff, covered) if not used), "TIFF unowned bytes")
    if not extended:
        try:
            require(rewrite_png.orientation(tiff) is None, "Exif orientation")
        except ValueError as error:
            if not str(error).startswith("invalid PNG "):
                raise
            raise MetadataDomainError("invalid/unsupported HEIF metadata shared TIFF") from error


def xmp(payload):
    require(len(payload) <= 4 * 1024 * 1024 and b"\0" not in payload
            and b"<!DOCTYPE" not in payload.upper() and b"<!ENTITY" not in payload.upper(), "XMP budget/DTD")
    try:
        payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise MetadataDomainError("invalid/unsupported HEIF metadata XMP UTF-8") from error
    pull = ET.XMLPullParser(events=("start", "end", "pi", "comment"))
    depth = records = 0
    packets = []
    root = None
    try:
        for at in range(0, len(payload), 1024):
            pull.feed(payload[at:at + 1024])
            for event, node in pull.read_events():
                if event in ("pi", "comment"):
                    records += 1
                    require(records <= 128, "XMP depth/records")
                    if event == "pi":
                        text = node.text or ""
                        require(depth == 0 and text in (
                            "xpacket begin='\ufeff' id='W5M0MpCehiHzreSzNTczkc9d'",
                            "xpacket end='w'", "xpacket end='r'"), "XMP processing instruction")
                        packets.append(text)
                    continue
                if event == "start":
                    if root is None:
                        root = node
                    depth += 1
                    records += 1
                    require(depth <= 8 and records <= 128, "XMP depth/records")
                else:
                    depth -= 1
        pull.close()
    except ET.ParseError as error:
        raise MetadataDomainError("invalid/unsupported HEIF metadata XMP XML") from error
    require(root is not None and depth == 0, "XMP root")
    require(not packets or len(packets) == 2 and packets[0].startswith("xpacket begin=")
            and packets[1].startswith("xpacket end="), "XMP packet envelope")
    x, rdf, dc = "{adobe:ns:meta/}", "{http://www.w3.org/1999/02/22-rdf-syntax-ns#}", "{http://purl.org/dc/elements/1.1/}"
    ex = "{http://ns.adobe.com/exif/1.0/}"
    require(root.tag == x + "xmpmeta" and set(root.attrib) <= {x + "xmptk"}
            and len(root) == 1 and root[0].tag == rdf + "RDF" and not root[0].attrib, "XMP envelope")
    require(1 <= len(root[0]) <= 3, "XMP descriptions")
    for desc in root[0]:
        require(desc.tag == rdf + "Description", "XMP RDF description")
        if not len(desc) and set(desc.attrib) <= {dc + "description", dc + "creator", dc + "rights"}:
            # Exact existing shared schema, independently revalidated below.
            require(len(root[0]) == 1 and not root.attrib, "shared XMP envelope")
            try:
                rewrite_png.descriptive_text(b"XML:com.adobe.xmp", payload)
            except ValueError as error:
                if not str(error).startswith("invalid PNG "):
                    raise
                raise MetadataDomainError("invalid/unsupported HEIF metadata shared XMP") from error
            continue
        require(desc.attrib == {rdf + "about": ""}, "XMP about/attributes")
        for child in desc:
            require(not child.attrib, "XMP leaf attributes")
            if child.tag in (ex + "GPSLatitude", ex + "GPSLongitude"):
                require(not len(child), "XMP GPS leaf")
            else:
                require(child.tag in {dc + "description", dc + "creator", dc + "rights"}
                        and len(child) == 1 and child[0].tag == rdf + ("Seq" if child.tag == dc + "creator" else "Alt")
                        and not child[0].attrib and 1 <= len(child[0]) <= 8, "XMP descriptive collection")
                for leaf in child[0]:
                    require(leaf.tag == rdf + "li" and not len(leaf)
                            and (not leaf.attrib if child.tag == dc + "creator" else
                                 leaf.attrib.get("{http://www.w3.org/XML/1998/namespace}lang") in ("x-default", "en", "en-gb")
                                 and len(leaf.attrib) == 1), "XMP collection leaf")
    for node in root.iter():
        require(not (node.tail or "").strip() and (not len(node) or not (node.text or "").strip()), "XMP mixed content")
