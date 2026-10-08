#!/usr/bin/env python3
"""Real HEIF CLI/consumer proofs and SAME-complete-observer sensitivity matrix."""
import hashlib
import json
import struct
import subprocess
import sys
from pathlib import Path
from verify_heif import observe, verify, ObservationError
from make_heif_fixtures import construct, box
from verify_heif_coded_boundary import PRIVATE_SUFFIX
from hevc_normative_model import escape

REQUIRED_POSITIVE = {"constructed", "nonprimary-description", "base-offset-and-padding", "permuted-property-ids", "v1-direct-extents"} | {
    "transform-r%d-m%d" % (r, m) for r in range(4) for m in (-1, 0, 1)}
REQUIRED_NEGATIVE = {"unknown-brand", "box-extent", "duplicate-item-id", "iloc-offset", "iloc-base",
    "iloc-count", "iloc-extent-count", "iloc-external", "duplicate-iloc-id", "overlapping-extents",
    "reference-cycle", "unknown-reference", "dangling-reference", "property-owner", "unknown-property",
    "pixi-alpha", "coded-private-suffix", "different-coded-suffix", "hvcC-reserved-bit6",
    "hvcC-completeness-bit7", "unknown-XMP-rendering", "Exif-orientation", "unknown-MIME", "ICC",
    "file-budget", "box-record-budget", "pixel-envelope", "iloc-construction", "unknown-box",
    "meta-version", "unknown-derived-item", "unknown-primary-NAL", "private-prefix-SEI",
    "metadata-aggregate-budget", "owned-writer-layout", "aux-alpha", "unknown-XMP-processing", "XMP-DTD"}
REQUIRED_SENSITIVITY = {"physical-private-leftover", "private-names", "retained-coded-byte",
                        "retained-color-property", "retained-association", "wrong-offset", "wrong-orientation"}


def check(condition, detail):
    if not condition:
        raise AssertionError(detail)


def call(argv, output, label):
    completed = subprocess.run(argv, capture_output=True, timeout=120)
    (output / (label + ".out")).write_bytes(completed.stdout)
    (output / (label + ".err")).write_bytes(completed.stderr)
    return completed


def put(data, offset, value, width):
    changed = bytearray(data)
    changed[offset:offset + width] = value.to_bytes(width, "big")
    return bytes(changed)


def reference_model(base, rotation, mirror):
    width, height = 32, 24
    pixels = [[base[(y * width + x) * 4:(y * width + x + 1) * 4] for x in range(width)] for y in range(height)]
    for _ in range(rotation):
        pixels = [[pixels[y][x] for y in range(height)] for x in range(width - 1, -1, -1)]
        width, height = height, width
    if mirror == 0:
        pixels.reverse()
    elif mirror == 1:
        pixels = [row[::-1] for row in pixels]
    return b"".join(pixel for row in pixels for pixel in row), (width, height)


def main():
    generated, fixtures, output, rewriter, consumer = map(Path, sys.argv[1:6])
    alpha = Path(sys.argv[6])
    output.mkdir(parents=True, exist_ok=False)
    available = {p.stem for p in generated.glob("*.heic") if ".output" not in p.stem and p.stem != "aux-alpha"}
    check(available == REQUIRED_POSITIVE, "independent required positive census")
    positive, candidates, native_source = {}, {}, {}
    for name in ["constructed"] + sorted(REQUIRED_POSITIVE - {"constructed"}):
        source = generated / (name + ".heic")
        before = source.read_bytes()
        destination = output / (name + ".heic")
        def rewrite(target, source_path=source):
            result = call([sys.executable, "-B", str(rewriter), str(source_path), str(target)], output, target.stem + ".rewrite")
            check(result.returncode == 0 and not result.stdout and not result.stderr,
                  name + " real rewriter domain/infrastructure failure: " + result.stderr.decode())
        rewrite(destination)
        repeated, second = output / (name + ".repeated.heic"), output / (name + ".second.heic")
        rewrite(repeated)
        rewrite(second, destination)
        candidate = destination.read_bytes()
        check(candidate == repeated.read_bytes() == second.read_bytes(), "separate-process deterministic/idempotent output")
        check(source.read_bytes() == before, "original input changed")
        graph = verify(before, candidate)
        if name == "nonprimary-description":
            check(graph["source_typed_references"] == {7: 5, 9: 7}, "nonprimary descriptive graph privacy sensitivity")
        views = {}
        for view in ("raw", "display"):
            pixels, dimensions = [], []
            for label, image in (("source", source), ("candidate", destination)):
                target = output / (name + "." + label + "." + view + ".rgba")
                result = call([str(consumer), str(image), view, str(target)], output, target.stem + ".consume")
                check(result.returncode == 0 and not result.stderr, name + " real libheif decode/diagnostic failure: " + result.stderr.decode())
                metadata = json.loads(result.stdout)
                check(metadata["libheif"] == "1.23.1" and metadata["warnings"] == 0
                      and metadata["has_alpha"] == 0, "pinned consumer identity/warnings/opaque envelope")
                dimensions.append((metadata["width"], metadata["height"]))
                pixels.append(target.read_bytes())
                check(len(pixels[-1]) == metadata["width"] * metadata["height"] * 4, "decoded byte census")
            check(pixels[0] == pixels[1] and dimensions[0] == dimensions[1], "real raw/display color/alpha fidelity")
            check(view != "raw" or dimensions[0] == (64, 64), "coded/raw canvas")
            views[view] = {"dimensions": dimensions[0], "sha256": hashlib.sha256(pixels[0]).hexdigest(), "zero_diagnostics": True}
            native_source[(name, view)] = pixels[0]
        if name.startswith("transform-"):
            rotation, mirror = map(int, name.removeprefix("transform-r").split("-m"))
            expected, dimensions = reference_model(native_source[("constructed", "display")], rotation, mirror)
            check(native_source[(name, "display")] == expected and tuple(views["display"]["dimensions"]) == dimensions,
                  name + " independent geometric transformation behavior")
        else:
            check(tuple(views["display"]["dimensions"]) == (32, 24), "actual clap aperture")
        positive[name] = {"graph": graph, "views": views, "deterministic_idempotent": True, "input_unchanged": True}
        candidates[name] = candidate
    base = (generated / "constructed.heic").read_bytes()
    state = observe(base)
    props = state["properties"]
    links = [index | (128 if essential else 0) for index, essential in state["links"]]
    descriptions = {i: (item["type"], base[slice(*state["extents"][i])]) for i, item in state["items"].items() if i != state["primary"]}
    primary = state["primary"]
    def rebuilt(properties=props, metadata=descriptions):
        return construct(primary, metadata, properties, links, state["coded"])
    negatives = {}
    def add(name, data, detail, valid_consumer=False):
        negatives[name] = (data, detail, valid_consumer)
    add("unknown-brand", base[:8] + b"avif" + base[12:], "brands/minor/compatible identities")
    add("box-extent", put(base, 0, 1, 4), "box extent/32-bit shape")
    iinf = state["children"][b"iinf"][0]
    # Fixture descriptors reverse typed IDs9,7,5. First two both metadata.
    first = iinf + 6
    next_entry = first + int.from_bytes(base[first:first + 4], "big")
    add("duplicate-item-id", put(base, next_entry + 12, 9, 2), "infe ID/protection/duplicate")
    field = state["location_fields"][primary]
    add("iloc-offset", put(base, field["offset_at"], 0, 4), "iloc offset/base/mdat ownership")
    add("iloc-base", put(base, field["offset_at"] - 6, 0xffffffff, 4), "iloc offset/base/mdat ownership")
    iloc = state["children"][b"iloc"][0]
    add("iloc-count", put(base, iloc + 6, 2, 2), "iloc declared item count")
    add("iloc-extent-count", put(base, field["offset_at"] - 2, 2, 2), "iloc external reference/extent count")
    add("iloc-external", put(base, field["offset_at"] - 8, 1, 2), "iloc external reference/extent count")
    add("duplicate-iloc-id", put(base, field["offset_at"] - 10, 7, 2), "iloc typed ID/duplicate")
    metadata_field = state["location_fields"][9]
    length = int.from_bytes(base[metadata_field["length_at"]:metadata_field["length_at"] + 4], "big")
    add("overlapping-extents", put(base, metadata_field["length_at"], length + 1, 4), "iloc overlapping extents")
    iref = state["children"][b"iref"][0]
    first_ref = iref + 4
    second_ref = first_ref + int.from_bytes(base[first_ref:first_ref + 4], "big")
    cycle = put(put(base, first_ref + 12, 9, 2), second_ref + 12, 7, 2)
    add("reference-cycle", cycle, "reference cycle/unrooted graph")
    add("unknown-reference", base[:first_ref + 4] + b"thmb" + base[first_ref + 8:], "unknown/aux/thumbnail reference type/extent")
    add("dangling-reference", put(base, first_ref + 12, 65535, 2), "typed descriptive reference/duplicate/count")
    add("property-owner", put(base, state["association_start"] - 3, 7, 2), "property association owner/count")
    kind, a, b = state["property_entries"][1]
    add("unknown-property", base[:a - 4] + b"zzzz" + base[a:], "unknown/duplicate/rendering property set")
    kind, a, b = state["property_entries"][4]
    add("pixi-alpha", put(base, a + 4, 4, 1), "pixi channel/depth/alpha envelope")
    def suffix(payload):
        data = base + payload
        data = put(data, field["length_at"], len(state["coded"]) + len(payload), 4)
        data = put(data, state["extents"][primary][0], len(state["coded"]) - 4 + len(payload), 4)
        mdat = len(base) - sum(b - a for a, b in state["extents"].values()) - 8
        return put(data, mdat, len(data) - mdat, 4)
    add("coded-private-suffix", suffix(PRIVATE_SUFFIX), "coded ownership: invalid/unsupported HEVC CABAC trailing/unowned bytes", True)
    add("different-coded-suffix", suffix(b"another owned private description\x80"), "coded ownership: invalid/unsupported HEVC CABAC trailing/unowned bytes", True)
    kind, a, b = state["property_entries"][0]
    add("hvcC-reserved-bit6", put(base, a + 23, base[a + 23] | 64, 1), "hvcC typed NAL array/count", True)
    add("hvcC-completeness-bit7", put(base, a + 23, base[a + 23] & 127, 1), "hvcC typed NAL array/count", True)
    unknown_xmp = dict(descriptions)
    unknown_xmp[9] = (b"mime", b'<x:xmpmeta xmlns:x="adobe:ns:meta/" xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:e="http://ns.adobe.com/exif/1.0/"><r:RDF><r:Description r:about=""><e:Orientation>6</e:Orientation></r:Description></r:RDF></x:xmpmeta>')
    add("unknown-XMP-rendering", rebuilt(metadata=unknown_xmp), "metadata XMP descriptive collection", True)
    processing = dict(descriptions)
    processing[9] = (b"mime", b"<?xml-stylesheet type='text/xsl' href='Owned-private.xsl'?>" + descriptions[9][1])
    add("unknown-XMP-processing", rebuilt(metadata=processing), "metadata XMP processing instruction", True)
    dtd = dict(descriptions)
    dtd[9] = (b"mime", b'<!DOCTYPE x [<!ENTITY secret "Owned private">]>' + descriptions[9][1])
    add("XMP-DTD", rebuilt(metadata=dtd), "metadata XMP budget/DTD")
    exif = dict(descriptions)
    tiff = b"II" + struct.pack("<HIHHHIH", 42, 8, 1, 0x0112, 3, 1, 6) + bytes(6)
    exif[7] = (b"Exif", b"\0\0\0\6Exif\0\0" + tiff)
    add("Exif-orientation", rebuilt(metadata=exif), "metadata Exif orientation/positioning", True)
    marker = b"application/rdf+xml"
    add("unknown-MIME", base.replace(marker, b"application/zzz+xml"), "unknown MIME schema")
    iccprops = list(props)
    iccprops[1] = (b"colr", b"prof" + (fixtures / "icc/input-v4.icc").read_bytes())
    add("ICC", rebuilt(properties=iccprops), "nclx/ICC/HDR color envelope", True)
    add("file-budget", base + bytes(32 * 1024 * 1024 + 1 - len(base)), "file budget")
    add("box-record-budget", b"".join(box(b"free", b"") for _ in range(65537)), "box header/record budget")
    kind, a, b = state["property_entries"][2]
    add("pixel-envelope", put(base, a + 4, 65, 4), "ispe coded canvas/pixel budget")
    v1 = (generated / "v1-direct-extents.heic").read_bytes()
    v1_state = observe(v1)
    construction_at = v1_state["location_fields"][primary]["offset_at"] - 10
    add("iloc-construction", put(v1, construction_at, 1, 2), "iloc construction method/reserved")
    hdlr = state["children"][b"hdlr"][0]
    add("unknown-box", base[:hdlr - 4] + b"zzzz" + base[hdlr:], "unknown/duplicate child box")
    meta = state["children"][b"hdlr"][0] - 12
    add("meta-version", put(base, meta, 1, 1), "meta version/flags/budget")
    third_entry = next_entry + int.from_bytes(base[next_entry:next_entry + 4], "big")
    add("unknown-derived-item", base[:third_entry + 16] + b"grid" + base[third_entry + 20:], "unknown/aux/grid/derived/thumbnail item type")
    add("unknown-primary-NAL", put(base, state["extents"][primary][0] + 4, 78, 1),
        "coded ownership: invalid/unsupported HEVC NAL type/layer/temporal header")
    sei = b"\x4e\x01" + escape(bytes((5, 16 + len(b"Owned private SEI"))) + bytes(16) + b"Owned private SEI" + b"\x80")
    seiprops = list(props)
    config = bytearray(seiprops[0][1])
    config[22] = 4
    config.extend(bytes((128 | 39,)) + b"\0\1" + struct.pack(">H", len(sei)) + sei)
    seiprops[0] = (b"hvcC", bytes(config))
    add("private-prefix-SEI", rebuilt(properties=seiprops), "hvcC unknown/extra NAL arrays", True)
    large = {i: (kind, payload + (bytes(4 * 1024 * 1024 - len(payload)) if kind == b"Exif" else
             b" " * (4 * 1024 * 1024 - len(payload)))) for i, (kind, payload) in descriptions.items()}
    add("metadata-aggregate-budget", rebuilt(metadata=large), "metadata aggregate budget")
    add("owned-writer-layout", (fixtures / "input/device-like.heic").read_bytes(), "hvcC typed NAL array/count", True)
    add("aux-alpha", alpha.read_bytes(), "typed single primary image", True)
    check(set(negatives) == REQUIRED_NEGATIVE, "independent required negative census")
    negative_reports = {}
    for name, (data, detail, valid) in negatives.items():
        source, destination = output / (name + ".negative.heic"), output / (name + ".forbidden.heic")
        source.write_bytes(data)
        digest = hashlib.sha256(data).digest()
        result = call([sys.executable, "-B", str(rewriter), str(source), str(destination)], output, name + ".negative")
        prefix = "invalid/unsupported HEIF "
        expected = prefix + detail + "\n"
        check(result.returncode == 2 and result.stderr.decode() == expected and not result.stdout,
              name + " typed diagnostic/control-target/infrastructure mismatch: " + result.stderr.decode())
        check(not destination.exists() and hashlib.sha256(source.read_bytes()).digest() == digest, "domain rejection output/input mutation")
        record = {"exit": 2, "diagnostic": expected.strip(), "no_output": True, "input_unchanged": True}
        if valid:
            result = call([str(consumer), str(source), "display", str(output / (name + ".unsupported.rgba"))], output, name + ".unsupported-consumer")
            check(result.returncode == 0 and not result.stderr, name + " consumer-valid unsupported/control premise failed: " + result.stderr.decode())
            record["consumer_valid"] = json.loads(result.stdout)
            if name == "aux-alpha":
                rgba = (output / (name + ".unsupported.rgba")).read_bytes()
                check(record["consumer_valid"]["has_alpha"] == 1 and len(set(rgba[3::4])) > 1,
                      "actual native auxiliary-alpha/control premise")
        negative_reports[name] = record
    candidate = candidates["constructed"]
    after = observe(candidate)
    sensitivities = {}
    leftovers = box(b"free", b"Owned synthetic private leftover")
    mdat = after["extents"][primary][0] - 8
    leaked = candidate[:mdat] + leftovers + candidate[mdat:]
    leaked = put(leaked, after["location_fields"][primary]["offset_at"], after["extents"][primary][0] + len(leftovers), 4)
    sensitivities["physical-private-leftover"] = (base, leaked, "physical metadata boxes/references removed")
    sensitivities["private-names"] = (base, construct(primary, {}, props, links, state["coded"]), "canonical private names/item graph removed")
    at = after["extents"][primary][0] + 6
    sensitivities["retained-coded-byte"] = (base, put(candidate, at, candidate[at] ^ 1, 1), "all retained compressed picture bytes")
    _, a, b = after["property_entries"][1]
    sensitivities["retained-color-property"] = (base, put(candidate, a + 5, 2, 1), "all retained configuration/rendering property bytes")
    changed = bytearray(candidate)
    a = after["association_start"]
    changed[a], changed[a + 1] = changed[a + 1], changed[a]
    sensitivities["retained-association"] = (base, bytes(changed), "all retained property associations/essential semantics")
    sensitivities["wrong-offset"] = (base, put(candidate, after["location_fields"][primary]["offset_at"], 0, 4), "iloc typed extent ownership")
    rotated_source = (generated / "transform-r0-m-1.heic").read_bytes()
    rotated = candidates["transform-r0-m-1"]
    rotation_state = observe(rotated)
    _, a, b = next(entry for entry in rotation_state["property_entries"] if entry[0] == b"irot")
    sensitivities["wrong-orientation"] = (rotated_source, put(rotated, a, 1, 1), "all retained configuration/rendering property bytes")
    check(set(sensitivities) == REQUIRED_SENSITIVITY, "independent required sensitivity census")
    sensitivity_reports = {}
    for name, (before, mutation, detail) in sensitivities.items():
        (output / (name + ".mutation.heic")).write_bytes(mutation)
        try:
            verify(before, mutation)
        except ObservationError as error:
            check(str(error) == "HEIF observation " + detail, name + " SAME verifier diagnostic specificity")
            sensitivity_reports[name] = {"same_complete_verifier_rejected": True, "diagnostic": str(error)}
        else:
            raise AssertionError(name + " survived SAME complete verifier")
    report = {"positive": positive, "negative": negative_reports, "sensitivity": sensitivity_reports,
              "scope": "owned constructed closed-layout proof; ISO hvcC conformance unresolved; no native-device/Task1/runtime readiness",
              "consumer_caveat": "all raw/display consumers share pinned libheif1.23.1/libde2651.1.1 family"}
    (output / "heif-proof.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"positive": len(positive), "negative": len(negative_reports), "sensitivity": len(sensitivity_reports), "report": str(output / "heif-proof.json")}, sort_keys=True))


if __name__ == "__main__":
    main()
