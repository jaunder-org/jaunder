#!/usr/bin/env python3
"""Test-side byte routing only; no ingress, identity, decoder or isolation claim.

Recognition requests validation. Only the unchanged family rewrite callable can
produce a verified private output. Labels never provide a raster escape.
"""
import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import re
import sys

MAX_FILE = 32 * 1024 * 1024
MAX_PROLOG = 65536
MIME = {"jpeg": "image/jpeg", "png": "image/png", "gif": "image/gif",
        "webp": "image/webp", "heic": "image/heic"}
RASTER_SUFFIXES = frozenset("jpg jpeg jpe pjpeg png apng gif webp heic heif heics heifs hif bmp dib tif tiff avif avifs ico cur jp2 j2k jpf jpx jxl".split())
IMAGE_BRANDS = frozenset((b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs", b"mif1", b"msf1", b"avif", b"avis", b"miaf"))
# MP4RA registration evidence, not codec/layout validation. Exact four-byte
# case/space-sensitive brands; these only reject unsupported image containers.
IMAGE_BRANDS |= frozenset((
    b"avci", b"avcs", b"jpeg", b"jpgs", b"j2ki", b"j2is", b"j2ks", b"vvic", b"vvis",
    b"1pic", b"avio", b"evbi", b"evbs", b"evmi", b"evms", b"heoi", b"jpoi", b"jpsi",
    b"jp2 ", b"J2P0", b"J2P1", b"jpm ", b"jpx ", b"jpxb", b"jxl ", b"jxs ",
    b"jxsi", b"jxss", b"MA1A", b"MA1B", b"MiAB", b"MiAn", b"MiBu", b"MiHA",
    b"MiHB", b"MiHE", b"MiPr", b"mif2", b"pred", b"vvoi", b"jaii", b"tmap"))
HEIC_COMPATIBLES = frozenset((b"heic", b"mif1", b"miaf"))


class RasterRejected(ValueError):
    """Expected claimed/plausible raster or family-domain rejection."""


@dataclass(frozen=True)
class Identification:
    kind: str
    family: str | None
    reason: str


@dataclass(frozen=True)
class Routed:
    kind: str
    family: str | None
    content_type: str | None
    filename: str
    output: bytes


def heif_evidence(data):
    """Discover aligned top-level image evidence, not generic ISO validity.

    Complete unknown boxes advance by their declared extent; payloads are never
    searched. Every iteration consumes >=8 physical bytes. Brand scanning uses
    fixed booleans and four-byte slices, so image admission budgets cannot stop
    evidence discovery or turn generic media into a raster policy target.
    """
    at, records, length = 0, 0, len(data)
    while at + 8 <= length:
        records += 1
        size = int.from_bytes(data[at:at + 4], "big")
        kind = data[at + 4:at + 8]
        header = 8
        if size == 1:
            header = 16
            if at + header > length:
                return None
            size = int.from_bytes(data[at + 8:at + 16], "big")
        elif size == 0:
            size = length - at
        complete = header <= size <= length - at
        if kind == b"ftyp":
            start = at + header
            # A malformed ftyp can still carry a full aligned image major or
            # compatible brand. Partial brand bytes alone are not evidence.
            end = min(at + size, length) if size >= header else min(start + 4, length)
            major = data[start:start + 4] if start + 4 <= end else b""
            known = major in IMAGE_BRANDS
            allowed = major in HEIC_COMPATIBLES
            heic_compatible = False
            for offset in range(start + 8, end - 3, 4):
                brand = data[offset:offset + 4]
                known |= brand in IMAGE_BRANDS
                allowed &= brand in HEIC_COMPATIBLES
                heic_compatible |= brand == b"heic"
            if known:
                payload_size = end - start
                if not complete or payload_size < 8 or (payload_size - 8) % 4:
                    return Identification("reject", None, "malformed-image-brands")
                if records > 65536 or payload_size > MAX_PROLOG:
                    return Identification("reject", None, "image-evidence-admission-budget")
                if allowed and (major == b"heic" or major == b"mif1" and heic_compatible):
                    return Identification("identified", "heic", "heic-brands")
                return Identification("reject", None, "unsupported-image-brands")
        if not complete:
            # Without image evidence, invalid framing is opaque media, not
            # safety verification. No guessed boundary or interior resync.
            return None
        at += size
    return None


def svg_root(data):
    """Non-expanding UTF-8 lexical prolog/root inspection, not XML validation.

    DOCTYPE is skipped with quote/internal-subset tracking. No entity or external
    identifier is evaluated. Ambiguous namespace attributes do not confirm SVG.
    """
    try:
        text = data[:MAX_PROLOG].decode("utf-8-sig")
    except UnicodeDecodeError:
        return False
    at = 0
    while True:
        while at < len(text) and text[at] in " \t\r\n":
            at += 1
        if text.startswith("<!--", at):
            end = text.find("-->", at + 4)
            if end < 0:
                return False
            at = end + 3
        elif text.startswith("<?", at):
            end = text.find("?>", at + 2)
            if end < 0:
                return False
            at = end + 2
        elif text.startswith("<!DOCTYPE", at):
            quote, depth = None, 0
            at += 9
            while at < len(text):
                char = text[at]
                at += 1
                if quote:
                    if char == quote:
                        quote = None
                elif char in "\"'":
                    quote = char
                elif char == "[":
                    depth += 1
                elif char == "]":
                    depth -= 1
                    if depth < 0:
                        return False
                elif char == ">" and depth == 0:
                    break
            else:
                return False
        else:
            break
    match = re.match(r"<([A-Za-z_][\w.:-]*)(?=[\s/>])", text[at:])
    if not match:
        return False
    name = match[1]
    at += match.end()
    attributes = {}
    while True:
        tail = text[at:]
        if tail.startswith(">") or tail.startswith("/>"):
            break
        attribute = re.match(r"\s+([A-Za-z_][\w.:-]*)\s*=\s*([\"'])([^<]*?)\2", tail)
        if not attribute or attribute[1] in attributes:
            # Whitespace preceding the closing bracket is ordinary XML.
            if re.match(r"\s+/?>", tail):
                break
            return False
        attributes[attribute[1]] = attribute[3]
        at += attribute.end()
    if name == "svg":
        return attributes.get("xmlns", "") in ("", "http://www.w3.org/2000/svg")
    if name.count(":") == 1:
        prefix, local = name.split(":")
        return local == "svg" and attributes.get("xmlns:" + prefix) == "http://www.w3.org/2000/svg"
    return False


def identify(data, content_type=None, filename=""):
    if len(data) > MAX_FILE:
        return Identification("reject", None, "fixture-file-budget")
    if data.startswith(b"\xff\xd8"):
        return Identification("identified", "jpeg", "jpeg-soi")
    if data.startswith(b"\x89PNG"):
        return Identification("identified", "png", "png-prefix")
    if data.startswith(b"GIF8"):
        return Identification("identified", "gif", "gif-prefix")
    if data.startswith(b"RIFF") and data[8:12] == b"WEBP":
        return Identification("identified", "webp", "webp-form")
    if data.startswith(b"RIFF") and data[8:] == b"WEB":
        return Identification("reject", None, "truncated-webp-form")
    container = heif_evidence(data)
    if container is not None:
        return container
    mime = (content_type or "").split(";", 1)[0].strip().lower()
    suffix = filename.rsplit(".", 1)[-1].lower() if "." in filename else ""
    if mime.startswith("image/") and mime != "image/svg+xml" or suffix in RASTER_SUFFIXES:
        return Identification("reject", None, "raster-claim")
    if svg_root(data):
        return Identification("svg", None, "svg-root")
    return Identification("non-image", None, "no-raster-evidence")


def rewrite_family(family, source, destination):
    """Call actual accepted helpers; unexpected failures preserve their source."""
    import rewrite_rgb_icc as icc
    import rewrite_png as png
    import rewrite_gif as gif
    import rewrite_webp as webp
    import rewrite_jpeg as jpeg
    import rewrite_heif as heif
    webp.PNG_METADATA = png
    try:
        if family == "heic":
            heif.rewrite(source, destination)
        elif family == "jpeg":
            jpeg.rewrite(source, destination, icc, webp)
        elif family == "png":
            png.rewrite(source, destination, icc)
        elif family == "gif":
            gif.rewrite(source, destination, icc)
        elif family == "webp":
            webp.rewrite(source, destination, icc)
        else:
            raise RuntimeError("unmapped identified family")
    except (jpeg.JPEGDomainError, webp.WebPDomainError, heif.HEIFDomainError) as error:
        raise RasterRejected("family-domain:" + family) from error
    except ValueError as error:
        # These accepted prototypes expose domain errors as ValueError. Match
        # only their explicit domain prefixes; arbitrary programming errors escape.
        if str(error).startswith(("invalid PNG ", "invalid GIF ", "invalid ICC ")):
            raise RasterRejected("family-domain:" + family) from error
        raise


def route(source, destination, content_type=None, filename="", validator=rewrite_family):
    source, destination = Path(source), Path(destination)
    if source.resolve() == destination.resolve() or destination.exists():
        raise FileExistsError("private output must be a fresh distinct path")
    with source.open("rb") as stream:
        data = stream.read(MAX_FILE + 1)
    found = identify(data, content_type, filename)
    if found.kind == "reject":
        raise RasterRejected(found.reason)
    if found.kind == "identified":
        validator(found.family, source, destination)
        output = destination.read_bytes()
        if not output or len(output) > MAX_FILE:
            raise RuntimeError("invalid private output budget")
        return Routed("verified", found.family, MIME[found.family], filename, output)
    destination.write_bytes(data)
    return Routed(found.kind, None, content_type, filename, data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source")
    parser.add_argument("destination")
    parser.add_argument("--mime")
    parser.add_argument("--filename", default="")
    args = parser.parse_args()
    try:
        result = route(args.source, args.destination, args.mime, args.filename)
    except RasterRejected as error:
        print("RasterRejected: " + str(error), file=sys.stderr)
        cause = error.__cause__
        while cause is not None:
            print(type(cause).__name__ + ": " + str(cause), file=sys.stderr)
            cause = cause.__cause__
        print(json.dumps({"kind": "rejected", "reason": str(error)}))
        return 2
    print(json.dumps({"kind": result.kind, "family": result.family,
                      "content_type": result.content_type, "filename": result.filename,
                      "size": len(result.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
