#!/usr/bin/env python3
"""Independent static routing goldens plus actual callable/CLI dispatch proof.

These expectations do not import detector tables or derive outcomes from magic
checks. Pixel/privacy proofs remain the unchanged family witnesses in probe.nix.
"""
import hashlib
import json
from pathlib import Path
import struct
import shutil
import subprocess
import sys
from dataclasses import replace
import route_image as routing


def check(value, detail):
    if not value:
        raise AssertionError(detail)


def ftyp(major, compatible=()):
    body = major + bytes(4) + b"".join(compatible)
    return struct.pack(">I4s", len(body) + 8, b"ftyp") + body


# Each row declares its own expected kind/family; prefixes are not valid images.
GOLDENS = [
    ("jpeg-minimum", b"\xff\xd8", None, "x.bin", "identified", "jpeg"),
    ("jpeg-bad-following", b"\xff\xd8bad", "image/svg+xml", "x.svg", "identified", "jpeg"),
    ("jpeg-one-byte-ambiguous", b"\xff", None, "x.dat", "non-image", None),
    ("png-minimum", b"\x89PNG", None, "x", "identified", "png"),
    ("png-damaged-tail", b"\x89PNGbroken", "text/plain", "x.txt", "identified", "png"),
    ("png-tail-alone", b"\r\n\x1a\n", None, "x", "non-image", None),
    ("png-short-ambiguous", b"\x89PN", None, "x", "non-image", None),
    ("gif-minimum", b"GIF8", None, "x", "identified", "gif"),
    ("gif-damaged-version", b"GIF8xx", "image/svg+xml", "x.svg", "identified", "gif"),
    ("gif-short-ambiguous", b"GIF", None, "x", "non-image", None),
    ("webp-minimum", b"RIFF\x04\0\0\0WEBP", None, "x", "identified", "webp"),
    ("webp-truncated-form", b"RIFF\x04\0\0\0WEB", "image/svg+xml", "x.svg", "reject", None),
    ("webp-short-ambiguous", b"RIFF\x04\0\0\0WE", None, "x", "non-image", None),
    ("wave", b"RIFF\x04\0\0\0WAVE", "audio/wav", "x.wav", "non-image", None),
    ("avi", b"RIFF\x04\0\0\0AVI ", "video/x-msvideo", "x.avi", "non-image", None),
    ("riff-alone", b"RIFF", None, "x", "non-image", None),
    ("heic", ftyp(b"heic", (b"mif1", b"heic", b"miaf")), None, "x", "identified", "heic"),
    ("mif1-with-heic", ftyp(b"mif1", (b"heic",)), "video/mp4", "x.mp4", "identified", "heic"),
    ("mif1-alone", ftyp(b"mif1"), None, "x", "reject", None),
    ("heif-sequence", ftyp(b"hevc", (b"msf1",)), None, "x", "reject", None),
    ("avif", ftyp(b"avif", (b"mif1",)), None, "x", "reject", None),
    ("video-brand-conflict", ftyp(b"mp42", (b"heic",)), "video/mp4", "x.mp4", "reject", None),
    ("heic-brand-conflict", ftyp(b"heic", (b"avif",)), "image/svg+xml", "x.svg", "reject", None),
    ("heic-unknown-compatible", ftyp(b"heic", (b"zzzz",)), None, "x", "reject", None),
    ("video", ftyp(b"isom", (b"mp42",)), "video/mp4", "x.mp4", "non-image", None),
    ("unknown-ftyp", ftyp(b"zzzz"), None, "x.whatever", "non-image", None),
    ("truncated-ftyp", b"\0\0\0\x18ftypheic", None, "x", "reject", None),
    ("bad-ftyp-length", b"\0\0\0\x07ftyp", None, "x", "non-image", None),
    ("unaligned-brand-list", struct.pack(">I4s", 17, b"ftyp") + b"heic\0\0\0\0x", None, "x", "reject", None),
    ("padded-heic", struct.pack(">I4s", 8, b"free") + ftyp(b"heic", (b"mif1",)), None, "x", "identified", "heic"),
    ("padded-video", struct.pack(">I4s", 8, b"skip") + ftyp(b"isom"), None, "x", "non-image", None),
    ("bad-leading-padding", b"\0\0\0\x40free" + ftyp(b"heic"), None, "x", "non-image", None),
    ("svg-plain", b"<svg/>", "image/svg+xml", "x.svg", "svg", None),
    ("svg-namespaced", b'<svg xmlns="http://www.w3.org/2000/svg"/>', None, "x", "svg", None),
    ("svg-prefixed", b'<s:svg xmlns:s="http://www.w3.org/2000/svg"/>', "text/plain", "x.txt", "svg", None),
    ("svg-prolog", b'\xef\xbb\xbf<?xml version="1.0"?><!--original--><!DOCTYPE svg><svg />', None, "x", "svg", None),
    ("svg-external", b'<!DOCTYPE svg SYSTEM "file:///never-open"><svg/>', None, "x", "svg", None),
    ("svg-entities", b'<!DOCTYPE svg [<!ENTITY secret SYSTEM "https://never-fetch"><!ENTITY x "expansion">]><svg a="&x;">&secret;</svg>', None, "x", "svg", None),
    ("svg-foreign", b'<svg xmlns="urn:foreign"/>', "image/svg+xml", "x.svg", "non-image", None),
    ("svg-nested-html", b"<html><svg/></html>", "image/svg+xml", "x.svg", "non-image", None),
    ("svg-claim-only", b"ordinary text", "image/svg+xml", "x.svg", "non-image", None),
    ("svg-namespace-entity-ambiguous", b'<svg xmlns="&namespace;"/>', None, "x", "non-image", None),
    ("svg-utf16-ambiguous", "<svg/>".encode("utf-16"), "image/svg+xml", "x.svg", "non-image", None),
    ("svg-raster-claim", b"<svg/>", "image/tiff", "x.svg", "reject", None),
    ("svg-raster-extension", b"<svg/>", "image/svg+xml", "x.HEIF", "reject", None),
    ("png-svg-polyglot", b"\x89PNG<svg/>", "image/svg+xml", "x.svg", "identified", "png"),
    ("jpeg-png-conflict", b"\xff\xd8\x89PNG", None, "x", "identified", "jpeg"),
    ("pdf", b"%PDF-1.7\n", "application/pdf", "PII untouched.PDF", "non-image", None),
    ("text", b"plain text", None, "x.unfamiliar", "non-image", None),
    ("unknown-image-mime", b"plain text", "IMAGE/AVIF; parameter=x", "x", "reject", None),
]
CLAIM_SUFFIXES = "jpg jpeg jpe pjpeg png apng gif webp heic heif heics heifs hif bmp dib tif tiff avif avifs ico cur jp2 j2k jpf jpx jxl".split()
for suffix in CLAIM_SUFFIXES:
    GOLDENS.append(("claim-suffix-" + suffix, b"not an image", "text/plain", "original." + suffix.upper(), "reject", None))
for mime in ("image/jpeg", "image/pjpeg", "image/x-png", "image/gif", "image/webp", "image/heic", "image/heif", "image/heic-sequence", "image/heif-sequence", "image/bmp", "image/tiff", "image/unknown"):
    GOLDENS.append(("claim-mime-" + mime, b"not an image", mime, "x.dat", "reject", None))
REQUIRED_GOLDENS = frozenset("""jpeg-minimum jpeg-bad-following jpeg-one-byte-ambiguous
png-minimum png-damaged-tail png-tail-alone png-short-ambiguous gif-minimum gif-damaged-version
 gif-short-ambiguous webp-minimum webp-truncated-form webp-short-ambiguous wave avi riff-alone
heic mif1-with-heic mif1-alone heif-sequence avif video-brand-conflict heic-brand-conflict
heic-unknown-compatible video unknown-ftyp truncated-ftyp bad-ftyp-length unaligned-brand-list
padded-heic padded-video bad-leading-padding svg-plain svg-namespaced svg-prefixed svg-prolog
svg-external svg-entities svg-foreign svg-nested-html svg-claim-only svg-namespace-entity-ambiguous
svg-utf16-ambiguous svg-raster-claim svg-raster-extension png-svg-polyglot jpeg-png-conflict pdf
text unknown-image-mime claim-suffix-jpg claim-suffix-jpeg claim-suffix-jpe claim-suffix-pjpeg
claim-suffix-png claim-suffix-apng claim-suffix-gif claim-suffix-webp claim-suffix-heic claim-suffix-heif
claim-suffix-heics claim-suffix-heifs claim-suffix-hif claim-suffix-bmp claim-suffix-dib claim-suffix-tif
claim-suffix-tiff claim-suffix-avif claim-suffix-avifs claim-suffix-ico claim-suffix-cur claim-suffix-jp2
claim-suffix-j2k claim-suffix-jpf claim-suffix-jpx claim-suffix-jxl claim-mime-image/jpeg
claim-mime-image/pjpeg claim-mime-image/x-png claim-mime-image/gif claim-mime-image/webp
claim-mime-image/heic claim-mime-image/heif claim-mime-image/heic-sequence claim-mime-image/heif-sequence
claim-mime-image/bmp claim-mime-image/tiff claim-mime-image/unknown""".split())


# Static registration-derived expected rejections, separate from detector data.
REGISTERED_IMAGE_CASES = (
    b"avci", b"avcs", b"jpeg", b"jpgs", b"j2ki", b"j2is", b"j2ks", b"vvic", b"vvis",
    b"1pic", b"avio", b"evbi", b"evbs", b"evmi", b"evms", b"heoi", b"jpoi", b"jpsi",
    b"jp2 ", b"J2P0", b"J2P1", b"jpm ", b"jpx ", b"jpxb", b"jxl ", b"jxs ",
    b"jxsi", b"jxss", b"MA1A", b"MA1B", b"MiAB", b"MiAn", b"MiBu", b"MiHA",
    b"MiHB", b"MiHE", b"MiPr", b"mif2", b"pred", b"vvoi", b"jaii", b"tmap")
REQUIRED_REGISTERED_BRANDS = frozenset("avci avcs jpeg jpgs j2ki j2is j2ks vvic vvis 1pic avio evbi evbs evmi evms heoi jpoi jpsi J2P0 J2P1 jpxb jxsi jxss MA1A MA1B MiAB MiAn MiBu MiHA MiHB MiHE MiPr mif2 pred vvoi jaii tmap".split()) | {"jp2 ", "jpm ", "jpx ", "jxl ", "jxs "}
NEIGHBOR_NON_IMAGE = (b"avc1", b"av01", b"hevd", b"hevi", b"hvci", b"hvce", b"hvcx", b"vvci", b"mj2s", b"mjp2", b"MiAC", b"MiCm", b"jxsc", b"ma1a", b"miha", b"jp2x", b"jxlx")
check({brand.decode("ascii") for brand in REGISTERED_IMAGE_CASES} == REQUIRED_REGISTERED_BRANDS, "required registration brand census")
for brand in REGISTERED_IMAGE_CASES:
    for placement in ("major", "compatible"):
        body = ftyp(brand) if placement == "major" else ftyp(b"mp42", (brand,))
        for label, mime, filename in (("omitted", None, "x"), ("video", "video/mp4", "x.mp4"), ("svg", "image/svg+xml", "x.svg")):
            GOLDENS.append(("registered-" + brand.hex() + "-" + placement + "-" + label, body, mime, filename, "reject", None))
for brand in NEIGHBOR_NON_IMAGE:
    for placement in ("major", "compatible"):
        body = ftyp(brand) if placement == "major" else ftyp(b"isom", (brand,))
        GOLDENS.append(("neighbor-" + brand.hex() + "-" + placement, body, "video/mp4", "x.mp4", "non-image", None))
REQUIRED_GOLDENS |= frozenset("registered-" + brand.encode("ascii").hex() + "-" + placement + "-" + label
    for brand in REQUIRED_REGISTERED_BRANDS for placement in ("major", "compatible") for label in ("omitted", "video", "svg"))
REQUIRED_GOLDENS |= frozenset("neighbor-" + brand.hex() + "-" + placement for brand in
    (b"avc1", b"av01", b"hevd", b"hevi", b"hvci", b"hvce", b"hvcx", b"vvci", b"mj2s", b"mjp2", b"MiAC", b"MiCm", b"jxsc", b"ma1a", b"miha", b"jp2x", b"jxlx") for placement in ("major", "compatible"))


def framed(kind, payload=b"", width="32"):
    if width == "64":
        return struct.pack(">I4sQ", 1, kind, len(payload) + 16) + payload
    if width == "eof":
        return struct.pack(">I4s", 0, kind) + payload
    return struct.pack(">I4s", len(payload) + 8, kind) + payload


# Parent's five immutable counterexample vectors; only output directories vary.
PARENT_NONIMAGE = {
    "extended-video-ftyp": struct.pack(">I4sQ", 1, b"ftyp", 24) + b"isom" + bytes(4),
    "to-eof-video-ftyp": struct.pack(">I4s", 0, b"ftyp") + b"isom" + bytes(4),
    "truncated-video-ftyp": struct.pack(">I4s", 64, b"ftyp") + b"isom" + bytes(4),
    "opaque-padding-only": struct.pack(">I4s", 8, b"free"),
    "ordinary-text-free": b"1234free",
}
REQUIRED_PARENT = frozenset("extended-video-ftyp to-eof-video-ftyp truncated-video-ftyp opaque-padding-only ordinary-text-free".split())
check(set(PARENT_NONIMAGE) == REQUIRED_PARENT, "parent vector census")
for name, data in PARENT_NONIMAGE.items():
    for label, mime, filename, expected in (("neutral", "application/octet-stream", "Original.unknown", "non-image"),
                                           ("claim", "image/heif", "Original.unknown", "reject"),
                                           ("svg", "image/svg+xml", "Original.svg", "non-image")):
        GOLDENS.append(("parent-" + name + "-" + label, data, mime, filename, expected, None))
REQUIRED_GOLDENS |= frozenset("parent-" + name + "-" + label for name in REQUIRED_PARENT for label in ("neutral", "claim", "svg"))

# Independently specified framing/evidence boundaries, not admission fixtures.
FRAMING_CASES = [
    ("truncated-known-major", struct.pack(">I4s", 64, b"ftyp") + b"heic", "reject", None),
    ("truncated-known-compatible", struct.pack(">I4s", 64, b"ftyp") + b"isom" + bytes(4) + b"avif", "reject", None),
    ("short-unknown-major", struct.pack(">I4s", 64, b"ftyp") + b"iso", "non-image", None),
    ("partial-known-major-ambiguous", struct.pack(">I4s", 64, b"ftyp") + b"hei", "non-image", None),
    ("partial-compatible-ambiguous", struct.pack(">I4s", 64, b"ftyp") + b"isom" + bytes(4) + b"hei", "non-image", None),
    ("undersized-known-ftyp", struct.pack(">I4s", 7, b"ftyp") + b"heic", "reject", None),
    ("extended-undersized-known", struct.pack(">I4sQ", 1, b"ftyp", 15) + b"heic", "reject", None),
    ("extended-undersized-video", struct.pack(">I4sQ", 1, b"ftyp", 15) + b"isom", "non-image", None),
    ("extended-truncated-size", struct.pack(">I4sI", 1, b"ftyp", 0), "non-image", None),
    ("extended-max-unknown", struct.pack(">I4sQ", 1, b"ftyp", (1 << 64) - 1) + b"isom" + bytes(4), "non-image", None),
    ("extended-max-known", struct.pack(">I4sQ", 1, b"ftyp", (1 << 64) - 1) + b"heic" + bytes(4), "reject", None),
    ("ordinary-max-known", struct.pack(">I4s", (1 << 32) - 1, b"ftyp") + b"heic" + bytes(4), "reject", None),
    ("empty-eof-ftyp", framed(b"ftyp", width="eof"), "non-image", None),
    ("unaligned-video", framed(b"ftyp", b"isom" + bytes(4) + b"x"), "non-image", None),
    ("unaligned-known-compatible", framed(b"ftyp", b"isom" + bytes(4) + b"heic" + b"x"), "reject", None),
    ("malformed-leading-no-resync", struct.pack(">I4s", 2, b"uuid") + ftyp(b"heic"), "non-image", None),
    ("unknown-payload-no-interior-scan", framed(b"uuid", ftyp(b"heic")), "non-image", None),
    ("eof-payload-no-interior-scan", framed(b"uuid", ftyp(b"heic"), "eof"), "non-image", None),
    ("duplicate-generic-then-image", ftyp(b"isom") + ftyp(b"heic"), "identified", "heic"),
    ("many-records-image-boundary", framed(b"free") * 65535 + ftyp(b"heic"), "identified", "heic"),
    ("many-records-image-excess", framed(b"free") * 65536 + ftyp(b"heic"), "reject", None),
    ("many-records-video-excess", framed(b"uuid") * 65537 + ftyp(b"isom"), "non-image", None),
    ("many-records-unsupported-excess", framed(b"uuid") * 65537 + ftyp(b"avif"), "reject", None),
    ("brand-budget-known-boundary", framed(b"ftyp", b"heic" + bytes(4) + b"mif1" * 16382), "identified", "heic"),
    ("brand-budget-known-excess", framed(b"ftyp", b"heic" + bytes(4) + b"mif1" * 16383), "reject", None),
    ("brand-budget-video-excess", framed(b"ftyp", b"isom" + bytes(4) + b"mp42" * 16383), "non-image", None),
    ("brand-budget-image-at-end", framed(b"ftyp", b"isom" + bytes(4) + b"mp42" * 16383 + b"heic"), "reject", None),
    ("brand-budget-video-then-later-image", framed(b"ftyp", b"isom" + bytes(4) + b"mp42" * 16383) + ftyp(b"heic"), "identified", "heic"),
]
REQUIRED_FRAMING = frozenset("""truncated-known-major truncated-known-compatible short-unknown-major partial-known-major-ambiguous
partial-compatible-ambiguous undersized-known-ftyp extended-undersized-known extended-undersized-video extended-truncated-size
extended-max-unknown extended-max-known ordinary-max-known empty-eof-ftyp unaligned-video unaligned-known-compatible
malformed-leading-no-resync unknown-payload-no-interior-scan eof-payload-no-interior-scan duplicate-generic-then-image
many-records-image-boundary many-records-image-excess many-records-video-excess many-records-unsupported-excess
brand-budget-known-boundary brand-budget-known-excess brand-budget-video-excess brand-budget-image-at-end brand-budget-video-then-later-image""".split())
check({row[0] for row in FRAMING_CASES} == REQUIRED_FRAMING, "required framing census")
for width in ("32", "64", "eof"):
    for major, compatibles, expected, family in ((b"heic", (b"mif1", b"heic", b"miaf"), "identified", "heic"),
                                               (b"mif1", (b"heic",), "identified", "heic"),
                                               (b"avif", (), "reject", None), (b"isom", (b"mp42",), "non-image", None)):
        body = framed(b"ftyp", major + bytes(4) + b"".join(compatibles), width)
        for leading in ("direct", "unknown"):
            data = body if leading == "direct" else framed(b"uuid", b"opaque", "64") + body
            name = "width-" + width + "-" + major.decode() + "-" + leading
            FRAMING_CASES.append((name, data, expected, family))
REQUIRED_FRAMING |= frozenset("width-" + width + "-" + major + "-" + leading
    for width in ("32", "64", "eof") for major in ("heic", "mif1", "avif", "isom") for leading in ("direct", "unknown"))
for name, data, expected, family in FRAMING_CASES:
    for label, mime, filename in (("neutral", None, "Original.unknown"), ("video", "video/mp4", "Original.mp4"), ("svg", "image/svg+xml", "Original.svg"), ("claim", "image/heif", "Original.unknown")):
        kind = "reject" if label == "claim" and expected == "non-image" else expected
        GOLDENS.append(("framing-" + name + "-" + label, data, mime, filename, kind, family))
REQUIRED_GOLDENS |= frozenset("framing-" + name + "-" + label for name in REQUIRED_FRAMING for label in ("neutral", "video", "svg", "claim"))
# Every registered unsupported brand remains evidence behind a complete unknown
# leading box, both as major and compatible (neutral and SVG labels).
for brand in REGISTERED_IMAGE_CASES:
    for placement in ("major", "compatible"):
        body = ftyp(brand) if placement == "major" else ftyp(b"mp42", (brand,))
        for label, mime in (("neutral", None), ("svg", "image/svg+xml")):
            GOLDENS.append(("later-registered-" + brand.hex() + "-" + placement + "-" + label,
                            framed(b"uuid") + body, mime, "Original.unknown", "reject", None))
REQUIRED_GOLDENS |= frozenset("later-registered-" + brand.encode().hex() + "-" + placement + "-" + label
    for brand in REQUIRED_REGISTERED_BRANDS for placement in ("major", "compatible") for label in ("neutral", "svg"))


def assert_goldens(rows, detector=routing.identify):
    check(len(rows) == len(REQUIRED_GOLDENS) and {r[0] for r in rows} == REQUIRED_GOLDENS, "required golden census")
    for name, data, mime, filename, kind, family in rows:
        actual = detector(data, mime, filename)
        check((actual.kind, actual.family) == (kind, family), name + " static golden")


def inventory():
    result = [("jpeg/input/" + f"{space}-v{v}-o{o}-s{s}.jpg", "jpeg", "image/jpeg")
              for space in ("srgb", "display-p3") for v in (2, 4) for o in range(1, 9) for s in (0, 2)]
    result += [("jpeg/input/plain-s%d.jpg" % s, "jpeg", "image/jpeg") for s in (0, 2)]
    result += [("png/input/%s-v%d.png" % (mode, v), "png", "image/png") for mode in ("rgb", "rgba", "apng-first", "apng-default") for v in (2, 4)]
    result += [("gif/input/bare87.gif", "gif", "image/gif")]
    result += [("gif/input/%s-v%d.gif" % (mode, v), "gif", "image/gif") for mode in ("static", "finite", "infinite") for v in (2, 4)]
    result += [("webp/input/%s-%s-alpha-v%d.webp" % (mode, codec, v), "webp", "image/webp") for mode in ("static", "animated") for codec in ("lossy", "lossless") for v in (2, 4)]
    result += [("webp/input/plain-%s-rgb.webp" % codec, "webp", "image/webp") for codec in ("lossy", "lossless")]
    names = ["constructed", "nonprimary-description", "base-offset-and-padding", "permuted-property-ids", "v1-direct-extents"] + ["transform-r%d-m%d" % (r, m) for r in range(4) for m in (-1, 0, 1)]
    result += [("heif-owned/" + name + ".heic", "heic", "image/heic") for name in names]
    return result


def assert_verified(actual, family, mime, filename, expected):
    check((actual.kind, actual.family, actual.content_type, actual.filename, actual.output) ==
          ("verified", family, mime, filename, expected), "complete verified result")


def execute(argv, work, label):
    done = subprocess.run(argv, capture_output=True, timeout=120)
    (work / (label + ".out")).write_bytes(done.stdout)
    (work / (label + ".err")).write_bytes(done.stderr)
    (work / (label + ".command.json")).write_text(json.dumps({"argv": list(map(str, argv)), "exit": done.returncode}))
    return done


def source_mutations(fixtures, work, cli):
    """Each actual parser mutation must fail the SAME whole executable verifier."""
    original = cli.read_text()
    mutations = [
        ("generic-invalid-reject", '            return None\n        at += size',
         '            return Identification("reject", None, "mutated generic validity")\n        at += size'),
        ("unknown-leading-stop", '        kind = data[at + 4:at + 8]\n',
         '        kind = data[at + 4:at + 8]\n        if kind not in (b"free", b"skip", b"ftyp"):\n            return None\n'),
        ("extended-framing-stop", '        if size == 1:\n            header = 16',
         '        if size == 1:\n            return None\n            header = 16'),
        ("eof-framing-stop", '        elif size == 0:\n            size = length - at',
         '        elif size == 0:\n            return None\n            size = length - at'),
        ("record-budget-fallback", '        records += 1\n',
         '        records += 1\n        if records > 65536:\n            return None\n'),
        ("brand-budget-fallback", '            for offset in range(start + 8, end - 3, 4):',
         '            if end - start > MAX_PROLOG:\n                return None\n            for offset in range(start + 8, end - 3, 4):'),
        ("malformed-known-fallback", '            if known:\n                payload_size',
         '            if known:\n                if not complete:\n                    return None\n                payload_size'),
    ]
    reports = []
    for name, before, after in mutations:
        check(original.count(before) == 1, name + " unique actual parser seam")
        source = work / ("mutation-" + name) / "source"
        shutil.copytree(cli.parent, source)
        changed = original.replace(before, after)
        compile(changed, str(source / cli.name), "exec")
        # Nix sources are read-only; mutation owns only this private copy.
        (source / cli.name).chmod(0o600)
        (source / cli.name).write_text(changed)
        completed = execute([sys.executable, "-B", source / "verify_image_routing.py", fixtures,
                             work / ("mutation-" + name) / "proof", source / cli.name, "--mutation-child"],
                            work, "mutation-" + name)
        check(completed.returncode == 1 and b"AssertionError:" in completed.stderr
              and b"static golden" in completed.stderr, name + " SAME whole verifier must detect actual source mutation")
        reports.append({"name": name, "exit": completed.returncode,
                        "source_sha256": hashlib.sha256(changed.encode()).hexdigest()})
    return reports


def main():
    fixtures, work, cli = map(Path, sys.argv[1:4])
    work.mkdir(parents=True, exist_ok=False)
    assert_goldens(GOLDENS)
    sensitivity = []
    for row in GOLDENS:
        try:
            assert_goldens([r for r in GOLDENS if r[0] != row[0]])
        except AssertionError:
            sensitivity.append("omitted:" + row[0])
        else:
            raise AssertionError("omission survived")
    for field, value in (("kind", "non-image"), ("family", "gif")):
        def damaged(data, mime, filename):
            found = routing.identify(data, mime, filename)
            return replace(found, **{field: value})
        try:
            assert_goldens(GOLDENS, damaged)
        except AssertionError:
            sensitivity.append("wrong-" + field)
        else:
            raise AssertionError("detector damage survived")
    for brand in REGISTERED_IMAGE_CASES:
        def omitted_brand(data, mime, filename):
            if data == ftyp(brand):
                return routing.Identification("non-image", None, "injected missing brand")
            return routing.identify(data, mime, filename)
        try:
            assert_goldens(GOLDENS, omitted_brand)
        except AssertionError:
            sensitivity.append("unrecognized-brand:" + brand.hex())
        else:
            raise AssertionError("unrecognized registered image survived")
    specimen = routing.Routed("verified", "png", "image/png", "Original.\u00e9.Name", b"verified bytes")
    assert_verified(specimen, "png", "image/png", "Original.\u00e9.Name", b"verified bytes")
    for field, value in (("kind", "identified"), ("family", "gif"), ("content_type", "image/svg+xml"),
                         ("filename", "changed.png"), ("output", b"original fallback")):
        try:
            assert_verified(replace(specimen, **{field: value}), "png", "image/png", "Original.\u00e9.Name", b"verified bytes")
        except AssertionError:
            sensitivity.append("verified-" + field)
        else:
            raise AssertionError("verified result damage survived")
    check(routing.identify(b"x" * (32 * 1024 * 1024 + 1)).kind == "reject", "fixture file budget")
    golden_runs = []
    for index, row in enumerate(GOLDENS):
        name, data, mime, filename, kind, family = row
        source = work / ("golden-%d.input" % index)
        source.write_bytes(data)
        target = work / ("golden-%d.output" % index)
        domain = kind in ("identified", "reject")
        try:
            actual = routing.route(source, target, mime, filename)
        except routing.RasterRejected:
            check(domain, name + " unexpected rejection")
            check(not target.exists(), name + " rejected output")
        else:
            check(not domain and actual.kind == kind and actual.output == data and actual.filename == filename and actual.content_type == mime, name + " passthrough")
        argv = [sys.executable, "-B", cli, source, work / ("golden-%d.cli-output" % index), "--filename", filename]
        if mime is not None:
            argv += ["--mime", mime]
        done = execute(argv, work, "golden-%d.cli" % index)
        check(done.returncode == (2 if domain else 0) and
              (done.stderr.startswith(b"RasterRejected: ") if domain else done.stderr == b""), name + " CLI domain result")
        report = json.loads(done.stdout)
        check(report["kind"] == ("rejected" if domain else kind), name + " CLI kind")
        if domain:
            check(not Path(argv[4]).exists(), name + " CLI rejection output")
        else:
            check(report == {"kind": kind, "family": None, "content_type": mime, "filename": filename, "size": len(data)}
                  and Path(argv[4]).read_bytes() == data, name + " complete CLI passthrough")
        golden_runs.append(name)
    positives = []
    items = inventory()
    # Independently authored population formulas, not a detector-derived glob.
    check(len(items) == 108, "real fixture census")
    for index, (relative, family, mime) in enumerate(items):
        source = fixtures / relative
        before = source.read_bytes()
        reference = work / ("real-%d.reference" % index)
        routing.rewrite_family(family, source, reference)
        expected = reference.read_bytes()
        for label, claim, filename in (("omitted", None, "Original.unfamiliar"), ("wrong", "application/pdf", "Original.PDF"), ("svg", "image/svg+xml", "Original.svg")):
            found = routing.identify(before, claim, filename)
            check((found.kind, found.family) == ("identified", family), relative + " recognition")
            target = work / ("real-%d.%s.output" % (index, label))
            actual = routing.route(source, target, claim, filename)
            assert_verified(actual, family, mime, filename, expected)
            argv = [sys.executable, "-B", cli, source, work / ("real-%d.%s.cli-output" % (index, label)), "--filename", filename]
            if claim is not None:
                argv += ["--mime", claim]
            done = execute(argv, work, "real-%d.%s.cli" % (index, label))
            report = json.loads(done.stdout)
            check(done.returncode == 0 and done.stderr == b"" and report == {"kind": "verified", "family": family, "content_type": mime, "filename": filename, "size": len(expected)}, relative + " CLI dispatch")
            check(Path(argv[4]).read_bytes() == expected, relative + " CLI bytes")
        second = routing.route(reference, work / ("real-%d.second" % index), None, "Unchanged.Name")
        check(second.output == expected and source.read_bytes() == before, relative + " idempotent/input invariance")
        positives.append({"path": relative, "family": family, "mime": mime, "input_sha256": hashlib.sha256(before).hexdigest(), "output_sha256": hashlib.sha256(expected).hexdigest()})
    # Real supported coded bytes behind unsupported framing must enter actual
    # closed family validation and reject, not pass the original through.
    owned = (fixtures / "heif-owned/constructed.heic").read_bytes()
    first_size = int.from_bytes(owned[:4], "big")
    for name, data in (("unknown-leading", framed(b"uuid", b"opaque") + owned),
                       ("extended-ftyp", framed(b"ftyp", owned[8:first_size], "64") + owned[first_size:]),
                       ("eof-ftyp", framed(b"ftyp", owned[8:first_size] + owned[first_size:], "eof"))):
        source = work / (name + ".real-input")
        source.write_bytes(data)
        found = routing.identify(data, "image/svg+xml", "Original.svg")
        check(found.kind in ("identified", "reject"), name + " real image evidence")
        target = work / (name + ".real-output")
        try:
            routing.route(source, target, "image/svg+xml", "Original.svg")
        except routing.RasterRejected:
            check(not target.exists(), name + " real unsupported framing output")
        else:
            raise AssertionError(name + " real unsupported framing escaped")
        done = execute([sys.executable, "-B", cli, source, work / (name + ".real-cli-output"),
                        "--mime", "image/svg+xml", "--filename", "Original.svg"], work, name + ".real-cli")
        check(done.returncode == 2 and done.stderr.startswith(b"RasterRejected:")
              and json.loads(done.stdout)["kind"] == "rejected" and not (work / (name + ".real-cli-output")).exists(),
              name + " real CLI unsupported framing")
    errors = []
    probe = work / "error.input"
    probe.write_bytes(b"\xff\xd8bad")
    for error in (OSError("injected read/write"), RuntimeError("invariant"), ValueError("unexpected programming value"), routing.RasterRejected("injected domain")):
        def failed(family, source, target):
            Path(target).write_bytes(b"private partial")
            raise error
        try:
            routing.route(probe, work / ("partial-%d" % len(errors)), "image/svg+xml", "unchanged.svg", failed)
        except Exception as caught:
            check(caught is error, "error source erased")
            errors.append(type(error).__name__)
        else:
            raise AssertionError("failure fell back")
    for original, target, expected_error in ((probe, probe, FileExistsError),
                                             (probe, work / "partial-0", FileExistsError),
                                             (work / "absent-source", work / "absent-output", FileNotFoundError)):
        try:
            routing.route(original, target)
        except expected_error:
            errors.append(expected_error.__name__)
        else:
            raise AssertionError("file guard fell back")
    def missing_output(family, source, target):
        return None
    try:
        routing.route(probe, work / "validator-missing-output", validator=missing_output)
    except FileNotFoundError:
        errors.append("validator-missing-output")
    else:
        raise AssertionError("missing output fell back")
    missing = execute([sys.executable, "-B", cli, work / "missing", work / "missing-output"], work, "missing-source")
    check(missing.returncode == 1 and b"FileNotFoundError" in missing.stderr and not (work / "missing-output").exists(), "CLI infrastructure classification")
    for relative in ("input/device-like.heic", "jpeg/input/unsupported-progressive.jpg", "heif-owned/aux-alpha.heic"):
        target = work / ("unsupported-%d" % len(errors))
        try:
            routing.route(fixtures / relative, target, "image/svg+xml", "Original.svg")
        except routing.RasterRejected:
            check(not target.exists(), "unsupported original fallback")
            errors.append(relative)
        else:
            raise AssertionError(relative + " unsupported admitted")
        done = execute([sys.executable, "-B", cli, fixtures / relative, work / ("unsupported-%d.cli-output" % len(errors)),
                        "--mime", "image/svg+xml", "--filename", "Original.svg"], work, "unsupported-%d.cli" % len(errors))
        check(done.returncode == 2 and done.stderr.startswith(b"RasterRejected: family-domain:")
              and b"DomainError:" in done.stderr and json.loads(done.stdout)["kind"] == "rejected", "unsupported CLI classification")
    parser_mutations = [] if "--mutation-child" in sys.argv else source_mutations(fixtures, work, cli)
    sensitivity += ["actual-source:" + entry["name"] for entry in parser_mutations]
    result = {"goldens": golden_runs, "real_positive": positives, "label_variants": 3,
              "parser_mutations": parser_mutations, "unsupported_real_framing": ["unknown-leading", "extended-ftyp", "eof-ftyp"],
              "sensitivity": sensitivity, "errors": errors,
              "scope": "test-side routing only; existing independent family witnesses own privacy/fidelity"}
    (work / "routing-proof.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"goldens": len(golden_runs), "real_positive": len(positives), "sensitivity": len(sensitivity), "errors": len(errors), "report": str(work / "routing-proof.json")}))


if __name__ == "__main__":
    main()
