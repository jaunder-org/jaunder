#!/usr/bin/env python3
"""Typed no-output and envelope controls for the bounded WebP rewriter."""
import importlib.util
import shutil
import struct
import sys
from pathlib import Path

from PIL import Image, ImageOps


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def records(data):
    at, result = 12, []
    while at < len(data):
        size = struct.unpack_from("<I", data, at + 4)[0]
        end = at + 8 + size
        result.append((data[at:at + 4], data[at + 8:end]))
        at = end + (size & 1)
    return result


def assemble(items):
    body = b"".join(kind + struct.pack("<I", len(value)) + value + (b"\0" if len(value) & 1 else b"") for kind, value in items)
    return b"RIFF" + struct.pack("<I", len(body) + 4) + b"WEBP" + body


def invoke(rewriter, icc, source, output):
    try:
        rewriter.rewrite(source, output, icc)
    except rewriter.WebPDomainError as error:
        return str(error)
    return None


def frame_hashes(path):
    with Image.open(path) as image:
        raw, shown = [], []
        for index in range(image.n_frames):
            image.seek(index)
            frame = image.copy()
            raw.append(frame.convert("RGBA").tobytes())
            shown.append(ImageOps.exif_transpose(frame).convert("RGBA").tobytes())
        return raw, shown


def main():
    root, rewrite_path, icc_path, png_path, observer_path, verifier_path, icc_inspector_path, lcms = map(Path, sys.argv[1:])
    lcms = lcms.resolve()
    rewriter, icc, observer = load("rewrite_webp_under_test", rewrite_path), load("ordinary_icc", icc_path), load("webp_observer", observer_path)
    verifier, icc_inspector = load("independent_webp_verifier", verifier_path), load("independent_icc_inspector", icc_inspector_path)
    rewriter.PNG_METADATA = load("proven_png_metadata", png_path)
    work = root / "rewrite-controls"; work.mkdir(exist_ok=True)
    source = (root / "input" / "static-lossy-alpha-v4.webp").read_bytes()
    values = records(source)
    by_kind = {kind: index for index, (kind, _) in enumerate(values)}
    cases = {}
    malformed = list(values); malformed[by_kind[b"ICCP"]] = (b"ICCP", b"not an ordinary ICC")
    cases["unsupported-icc-profile"] = (assemble(malformed), "invalid ICC size")
    malformed = list(values); malformed[by_kind[b"EXIF"]] = (b"EXIF", b"Exif\0\0not-raw-tiff")
    cases["unsupported-exif-prefix"] = (assemble(malformed), "invalid PNG TIFF header")
    cases["unknown-chunk"] = (assemble(values + [(b"PRIV", b"private")]), "unknown/empty chunk")
    malformed = list(values); vp8x = bytearray(malformed[by_kind[b"VP8X"]][1]); vp8x[1] = 1; malformed[by_kind[b"VP8X"]] = (b"VP8X", bytes(vp8x))
    cases["vp8x-reserved"] = (assemble(malformed), "VP8X reserved")
    animated = (root / "input" / "animated-lossy-alpha-v4.webp").read_bytes()
    animation_values = records(animated)
    anim_index = next(i for i, (kind, _) in enumerate(animation_values) if kind == b"ANIM")
    short_anim = list(animation_values); short_anim[anim_index] = (b"ANIM", b"short")
    cases["anim-size"] = (assemble(short_anim), "ANIM size")
    anmf_index = next(i for i, (kind, _) in enumerate(animation_values) if kind == b"ANMF")
    bad_frame = list(animation_values); frame = bytearray(bad_frame[anmf_index][1]); frame[15] = 0; bad_frame[anmf_index] = (b"ANMF", bytes(frame))
    cases["partial-blend-frame"] = (assemble(bad_frame), "ANMF closed frame control")
    exif_position = next(i for i, (kind, _) in enumerate(animation_values) if kind == b"EXIF")
    four = (list(animation_values[:exif_position])
            + [(b"ANMF", next(value for kind, value in animation_values if kind == b"ANMF"))]
            + list(animation_values[exif_position:]))
    cases["four-frames"] = (assemble(four), "animated frame count")
    no_image = [(kind, value) for kind, value in values if kind not in {b"ALPH", b"VP8 "}]
    cases["exif-without-image"] = (assemble(no_image), "missing rendering image")
    without_iccp = [(kind, value) for kind, value in values if kind != b"ICCP"]
    moved_iccp = (without_iccp[:next(i for i, (kind, _) in enumerate(without_iccp) if kind == b"EXIF")]
                  + [(b"ICCP", values[by_kind[b"ICCP"]][1])]
                  + without_iccp[next(i for i, (kind, _) in enumerate(without_iccp) if kind == b"EXIF"):])
    cases["late-iccp"] = (assemble(moved_iccp), "ICCP order")
    private_trailer = list(values); private = bytearray(private_trailer[by_kind[b"EXIF"]][1]) + b"x"; private_trailer[by_kind[b"EXIF"]] = (b"EXIF", bytes(private))
    cases["exif-private-trailer"] = (assemble(private_trailer), "EXIF trailing data")
    bad_xmp = list(values); bad_xmp[by_kind[b"XMP "]] = (b"XMP ", b"not XML")
    cases["xmp-syntax"] = (assemble(bad_xmp), "invalid PNG XMP syntax")
    # Distinct directory offsets can overlap structurally: the empty GPS IFD
    # starts inside the zero next-directory pointer of the outer IFD.
    overlap = b"II" + struct.pack("<HIH", 42, 8, 1) + struct.pack("<HHII", 0x8825, 4, 1, 24) + b"\0" * 8
    malformed = list(values)
    malformed[by_kind[b"EXIF"]] = (b"EXIF", overlap)
    cases["ifd-structural-overlap"] = (assemble(malformed), "EXIF IFD structural overlap")
    # Individually plausible chunks are not permission for unwitnessed layouts.
    moved = list(animation_values)
    iccp_at = next(i for i, (kind, _) in enumerate(moved) if kind == b"ICCP")
    item = moved.pop(iccp_at)
    moved.insert(next(i for i, (kind, _) in enumerate(moved) if kind == b"ANIM") + 1, item)
    cases["iccp-after-anim"] = (assemble(moved), "ICCP order")
    moved = list(values)
    item = moved.pop(by_kind[b"ICCP"])
    moved.insert(next(i for i, (kind, _) in enumerate(moved) if kind == b"ALPH") + 1, item)
    cases["iccp-after-alph"] = (assemble(moved), "ICCP order")
    bare_animation = [(kind, bytes([payload[0] & ~0x2c]) + payload[1:] if kind == b"VP8X" else payload)
                      for kind, payload in animation_values if kind not in {b"ICCP", b"EXIF", b"XMP "}]
    cases["unproved-metadata-free-animation"] = (assemble(bare_animation), "unsupported chunk layout")
    partial = [(kind, bytes([payload[0] & ~4]) + payload[1:] if kind == b"VP8X" else payload)
               for kind, payload in values if kind != b"XMP "]
    cases["descriptive-partial-metadata"] = (assemble(partial), "unsupported partial metadata layout")
    # Exercise TIFF schema boundaries through the WebP entry point, not merely
    # by assuming the independently tested PNG helper remains connected.
    tiff = values[by_kind[b"EXIF"]][1]
    root_ifd = struct.unpack_from("<I", tiff, 4)[0]
    count = struct.unpack_from("<H", tiff, root_ifd)[0]
    entries = {struct.unpack_from("<H", tiff, root_ifd + 2 + index * 12)[0]: root_ifd + 2 + index * 12
               for index in range(count)}
    gps_ifd = struct.unpack_from("<I", tiff, entries[0x8825] + 8)[0]
    changes = (
        ("tiff-offset", 4, "I", len(tiff) + 2, "TIFF directory"),
        ("tiff-type", entries[274] + 2, "H", 0, "TIFF field type/count"),
        ("tiff-count", entries[274] + 4, "I", 0, "TIFF field type/count"),
        ("orientation-type", entries[274] + 4, "I", 2, "orientation type"),
        ("orientation-value", entries[274] + 8, "H", 0, "orientation value"),
        ("gps-offset", entries[0x8825] + 8, "I", len(tiff) + 2, "TIFF directory"),
        ("gps-type", entries[0x8825] + 2, "H", 3, "GPS pointer"),
        ("gps-field", gps_ifd + 2, "H", 9, "unsupported GPS field"),
        ("next-ifd", root_ifd + 2 + count * 12, "I", 8, "unsupported TIFF next directory"),
        ("duplicate-tiff", root_ifd + 2 + (count - 1) * 12, "H",
         struct.unpack_from("<H", tiff, root_ifd + 2)[0], "duplicate TIFF tag"),
        ("tiff-value-offset", entries[270] + 8, "I", len(tiff) + 2, "TIFF field bounds"),
    )
    for label, at, fmt, value, reason in changes:
        changed = bytearray(tiff)
        struct.pack_into("<" + fmt, changed, at, value)
        items = list(values)
        items[by_kind[b"EXIF"]] = (b"EXIF", bytes(changed))
        cases[label] = (assemble(items), "invalid PNG " + reason)
    shared = (b"II" + struct.pack("<HIH", 42, 8, 2)
              + struct.pack("<HHII", 270, 2, 6, 38)
              + struct.pack("<HHII", 271, 2, 6, 38) + b"\0" * 4 + b"owner\0")
    embedded = b"II" + struct.pack("<HIH", 42, 8, 1) + struct.pack("<HHII", 270, 2, 6, 8) + b"\0" * 4
    for label, blob, reason in (("external-value-overlap", shared, "EXIF external value overlap"),
                                ("external-ifd-overlap", embedded, "EXIF external/IFD overlap")):
        items = list(values)
        items[by_kind[b"EXIF"]] = (b"EXIF", blob)
        cases[label] = (assemble(items), reason)
    for label, payload, reason in (
        ("xmp-unsupported", b'<other/>', "unsupported XMP structure"),
        ("xmp-orientation-conflict", values[by_kind[b"XMP "]][1].replace(
            b'<rdf:Description', b'<rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:Orientation="1"'),
         "unsupported XMP rendering/attribute"),
        ("xmp-entity", b'<!DOCTYPE x>' + values[by_kind[b"XMP "]][1], "fixture XMP size/declarations"),
    ):
        items = list(values)
        items[by_kind[b"XMP "]] = (b"XMP ", payload)
        cases[label] = (assemble(items), "invalid PNG " + reason)
    bad_profile = bytearray(values[by_kind[b"ICCP"]][1])
    struct.pack_into(">I", bad_profile, 136, 0)
    items = list(values)
    items[by_kind[b"ICCP"]] = (b"ICCP", bytes(bad_profile))
    cases["icc-tag-offset"] = (assemble(items), "invalid ICC tag bounds")
    for label, control in (("dispose-frame", 3), ("blend-dispose-frame", 1)):
        items = list(animation_values)
        payload = bytearray(items[anmf_index][1])
        payload[15] = control
        items[anmf_index] = (b"ANMF", bytes(payload))
        cases[label] = (assemble(items), "ANMF closed frame control")
    # A wider canvas containing the original full compressed frames is a valid
    # partial-rectangle animation, but outside this prototype's proved layout.
    items = list(animation_values)
    canvas_bytes = bytearray(items[0][1])
    canvas_bytes[4:7] = (33).to_bytes(3, "little")
    items[0] = (b"VP8X", bytes(canvas_bytes))
    cases["partial-rectangle"] = (assemble(items), "ANMF closed frame control")
    # Restore every malformed input made by the committed observer controls.
    # Their valid odd-padding and valid late-frame positives remain positives.
    legacy_reasons = {
        "declared-size": "RIFF extent", "trailer": "RIFF extent",
        "chunk-length": "chunk extent/padding", "padding": "chunk extent/padding",
        "vp8x-reserved": "VP8X reserved", "vp8x-feature": "VP8X feature flags 0x3a/0x3e",
        "anim-order": "animation pairing/order", "duplicate-iccp": "duplicate singleton",
        "duplicate-exif": "duplicate singleton", "duplicate-xmp": "duplicate singleton",
        "duplicate-anim": "duplicate singleton", "duplicate-vp8x": "duplicate singleton",
        "anmf-overflow": "ANMF closed frame control", "anmf-nested-boundary": "chunk extent/padding",
        "vp8-header": "VP8 frame tag", "static-alph-order": "static image",
        "alph-with-vp8l": "static image", "alph-header": "ALPH header",
        "vp8l-header": "VP8L reserved bits", "static-canvas": "static canvas",
    }
    for label, reason in legacy_reasons.items():
        path = root / "controls" / (label + ".webp")
        assert path.exists(), label
        cases["legacy-" + label] = (path.read_bytes(), reason)
    rejected = 0
    for label, (data, reason) in cases.items():
        before, after = work / (label + ".webp"), work / (label + ".output.webp")
        assert not after.exists()
        before.write_bytes(data)
        if label in {"partial-blend-frame", "dispose-frame", "blend-dispose-frame", "partial-rectangle",
                     "four-frames", "unproved-metadata-free-animation", "descriptive-partial-metadata"}:
            # These must actually be valid renderable unsupported variants.
            observer.inspect(before)
            assert frame_hashes(before)[0], label
        actual = invoke(rewriter, icc, before, after)
        assert actual is not None and actual == "invalid WebP " + reason, (label, actual)
        assert not after.exists() and before.read_bytes() == data
        rejected += 1
    # Small owned inputs exercise exact prototype-budget edges without creating
    # large allocations; no inference about production enforcement is made.
    metadata_bytes = sum(len(payload) for kind, payload in values if kind in {b"ICCP", b"EXIF", b"XMP "})
    nested_total = len(animation_values) + sum(len(records(b"\0" * 12 + payload[16:]))
                                              for kind, payload in animation_values if kind == b"ANMF")
    budgets = (("MAX_FILE", len(source), "RIFF/WebP", source),
               ("MAX_META", metadata_bytes, "metadata aggregate", source),
               ("MAX_RECORDS", len(values), "chunk header/count", source),
               ("MAX_RECORDS", nested_total, "total record count", animated),
               ("MAX_PIXELS", 32 * 24, "canvas bounds", source))
    for index, (name, boundary, reason, data) in enumerate(budgets):
        saved = getattr(rewriter, name)
        before = work / f"budget-{index}.webp"
        before.write_bytes(data)
        try:
            setattr(rewriter, name, boundary)
            assert invoke(rewriter, icc, before, work / f"budget-{index}.positive.webp") is None
            setattr(rewriter, name, boundary - 1)
            after = work / f"budget-{index}.negative.webp"
            assert invoke(rewriter, icc, before, after) == "invalid WebP " + reason
            assert not after.exists() and before.read_bytes() == data
            rejected += 1
        finally:
            setattr(rewriter, name, saved)
    # A programming/infrastructure exception is deliberately not a domain
    # rejection. The typed oracle must let it escape rather than call it green.
    class BrokenIcc:
        def rewrite(self, source, destination):
            raise RuntimeError("simulated infrastructure failure")
    before, after = work / "infrastructure.webp", work / "infrastructure.output.webp"
    before.write_bytes(source)
    try:
        rewriter.rewrite(before, after, BrokenIcc())
    except RuntimeError:
        assert not after.exists() and before.read_bytes() == source
    else:
        raise AssertionError("infrastructure failure was classified as domain rejection")
    # Metadata-free simple VP8/VP8L controls are admitted byte-identically.
    for name in ("plain-lossy-rgb.webp", "plain-lossless-rgb.webp"):
        before, after = root / "input" / name, work / (name + ".output.webp")
        rewriter.rewrite(before, after, icc)
        assert before.read_bytes() == after.read_bytes()
    # All eight TIFF orientation values: value 1 removes EXIF/its flag, values
    # 2--8 retain canonical TIFF and preserve both raw and displayed pixels.
    exif_index = by_kind[b"EXIF"]
    for value in range(1, 9):
        varied = list(values); blob = bytearray(varied[exif_index][1])
        at = struct.unpack_from("<I", blob, 4)[0]; count = struct.unpack_from("<H", blob, at)[0]
        orient = next(at + 2 + index * 12 for index in range(count) if struct.unpack_from("<H", blob, at + 2 + index * 12)[0] == 0x0112)
        struct.pack_into("<H", blob, orient + 8, value); varied[exif_index] = (b"EXIF", bytes(blob))
        before, after = work / (f"orientation-{value}.webp"), work / (f"orientation-{value}.output.webp")
        before.write_bytes(assemble(varied)); rewriter.rewrite(before, after, icc)
        old_raw, old_shown = frame_hashes(before); new_raw, new_shown = frame_hashes(after)
        assert old_raw == new_raw and old_shown == new_shown
        view = observer.inspect(after)
        exifs = [payload for kind, payload in records(after.read_bytes()) if kind == b"EXIF"]
        if value == 1:
            assert not exifs and not view["flags"] & 8
        else:
            assert exifs == [b"II" + struct.pack("<HIHHHIH", 42, 8, 1, 0x0112, 3, 1, value) + b"\0" * 6]
            assert view["flags"] & 8
    # Repeat every orientation through the three-frame animated envelope.
    animated_exif = next(i for i, (kind, _) in enumerate(animation_values) if kind == b"EXIF")
    for value in range(1, 9):
        varied = list(animation_values); blob = bytearray(varied[animated_exif][1])
        at = struct.unpack_from("<I", blob, 4)[0]; count = struct.unpack_from("<H", blob, at)[0]
        orient = next(at + 2 + index * 12 for index in range(count) if struct.unpack_from("<H", blob, at + 2 + index * 12)[0] == 0x0112)
        struct.pack_into("<H", blob, orient + 8, value); varied[animated_exif] = (b"EXIF", bytes(blob))
        before, after = work / (f"animated-orientation-{value}.webp"), work / (f"animated-orientation-{value}.output.webp")
        before.write_bytes(assemble(varied)); rewriter.rewrite(before, after, icc)
        assert frame_hashes(before) == frame_hashes(after)
        exifs = [payload for kind, payload in records(after.read_bytes()) if kind == b"EXIF"]
        assert (not exifs) if value == 1 else exifs == [b"II" + struct.pack("<HIHHHIH", 42, 8, 1, 0x0112, 3, 1, value) + b"\0" * 6]
    # The independent orientation parser is sensitive to every structural
    # field. A plausible orientation value cannot hide a wrong tag/type/count,
    # TIFF offset, padding, or next-IFD pointer.
    canonical = b"II" + struct.pack("<HIHHHIH", 42, 8, 1, 0x0112, 3, 1, 6) + b"\0" * 6
    for offset, value in ((4, 9), (10, 0x13), (12, 4), (14, 2), (20, 1), (22, 1)):
        mutated = bytearray(canonical)
        if offset in (10, 12, 20, 22): mutated[offset] = value
        else: struct.pack_into("<H", mutated, offset, value)
        try: verifier.minimal_orientation(mutated)
        except AssertionError: pass
        else: raise AssertionError("independent orientation parser accepted a structural mutation")
    # Independent output/privacy oracle sensitivity: top-level extraction is
    # record-walked, so an EXIF byte sequence inside ICCP cannot manufacture a
    # top-level EXIF observation. Leaked XMP/descriptive EXIF and a dirty ICC
    # header each make the independent checks fail.
    collision = work / "embedded-exif-in-iccp.webp"
    plain = records((root / "input" / "plain-lossy-rgb.webp").read_bytes())
    collision.write_bytes(assemble([(b"ICCP", b"EXIF embedded bytes")] + plain))
    assert not [value for kind, value in verifier.outer(collision) if kind == b"EXIF"]
    clean = work / "orientation-6.output.webp"
    clean_records = records(clean.read_bytes())
    def private_free(items):
        assert [kind for kind, _ in items].count(b"XMP ") == 0
        exifs = [value for kind, value in items if kind == b"EXIF"]
        assert len(exifs) == 1 and verifier.minimal_orientation(exifs[0]) == 6
    private_free(clean_records)
    for changed in (clean_records + [(b"XMP ", b"leak")],
                    [(kind, source if kind == b"EXIF" else value) for kind, value in clean_records]):
        try: private_free(changed)
        except AssertionError: pass
        else: raise AssertionError("privacy output mutation passed independent oracle")
    profile = next(value for kind, value in clean_records if kind == b"ICCP")
    profile_path, dirty_path = work / "clean.icc", work / "dirty.icc"
    profile_path.write_bytes(profile); icc_inspector.sanitized_profile(profile_path)
    dirty = bytearray(profile); dirty[4] = 1; dirty_path.write_bytes(dirty)
    try: icc_inspector.sanitized_profile(dirty_path)
    except AssertionError: pass
    else: raise AssertionError("dirty ICC header passed independent inspector")
    # Run the same complete verifier used for published fixture evidence on
    # deliberately corrupted outputs. A separate, weaker privacy predicate
    # cannot stand in for sensitivity of that actual proof boundary.
    static_source = root / "input" / "static-lossy-alpha-v4.webp"
    animated_source = root / "input" / "animated-lossy-alpha-v4.webp"
    animated_clean = work / "animated-orientation-6.output.webp"
    verifier.verify_pair(static_source, clean, observer, icc_inspector, lcms, work)
    verifier.verify_pair(animated_source, animated_clean, observer, icc_inspector, lcms, work)
    static_entries, animated_entries = records(clean.read_bytes()), records(animated_clean.read_bytes())

    def replace(entries, kind, transform):
        return [(name, transform(payload) if name == kind else payload) for name, payload in entries]

    wrong_orientation = bytearray(next(payload for kind, payload in static_entries if kind == b"EXIF"))
    struct.pack_into("<H", wrong_orientation, 18, 5)
    _, tags = icc_inspector.profile(profile_path)
    dirty_description = bytearray(profile)
    dirty_description[tags[b"desc"][0] + 29] ^= 1
    dirty_header = bytearray(profile)
    dirty_header[4] = 1
    leaked_exif = values[by_kind[b"EXIF"]][1]
    mutations = [
        ("orientation", static_source, replace(static_entries, b"EXIF", lambda payload: bytes(wrong_orientation))),
        ("description-leak", static_source, replace(static_entries, b"EXIF", lambda payload: leaked_exif)),
        ("xmp-leak", static_source, static_entries + [(b"XMP ", values[by_kind[b"XMP "]][1])]),
        ("icc-description", static_source, replace(static_entries, b"ICCP", lambda payload: bytes(dirty_description))),
        ("icc-header", static_source, replace(static_entries, b"ICCP", lambda payload: bytes(dirty_header))),
        ("static-payload", static_source, replace(static_entries, b"VP8 ", lambda payload: payload[:-1] + bytes([payload[-1] ^ 1]))),
    ]
    frame_at = next(i for i, (kind, _) in enumerate(animated_entries) if kind == b"ANMF")
    for label, offset in (("frame-payload", -1), ("frame-timing", 12), ("frame-blend", 15)):
        items = list(animated_entries)
        changed = bytearray(items[frame_at][1])
        changed[offset] ^= 1
        items[frame_at] = (b"ANMF", bytes(changed))
        mutations.append((label, animated_source, items))
    for label, offset in (("background", 0), ("loop", 4)):
        items = replace(animated_entries, b"ANIM", lambda payload: payload[:offset] + bytes([payload[offset] ^ 1]) + payload[offset + 1:])
        mutations.append((label, animated_source, items))
    for label, original_path, entries in mutations:
        mutant = work / ("output-oracle-" + label + ".webp")
        mutant.write_bytes(assemble(entries))
        try:
            verifier.verify_pair(original_path, mutant, observer, icc_inspector, lcms, work)
        except (AssertionError, observer.WebPContainerError):
            continue
        raise AssertionError("complete verifier accepted output drift: " + label)
    # Valid final-frame mutation is independently accepted by the committed
    # observer, changes only the final consumer canvas, and survives rewriting.
    candidate = root / "controls" / "valid-last-frame-mutant.webp"
    observer.inspect(candidate)
    source_frames = frame_hashes(root / "input" / "animated-lossy-alpha-v4.webp")[0]
    changed_frames = frame_hashes(candidate)[0]
    assert source_frames[:-1] == changed_frames[:-1] and source_frames[-1] != changed_frames[-1]
    late = work / "late-frame.output.webp"; rewriter.rewrite(candidate, late, icc)
    late_source, late_output = observer.inspect(candidate), observer.inspect(late)
    assert observer.compressed(late_source) == observer.compressed(late_output)
    assert late_source["canvas"] == late_output["canvas"]
    assert late_source["animation"] == late_output["animation"]
    assert late_source["frames"] == late_output["frames"]
    assert frame_hashes(candidate) == frame_hashes(late)
    print(f"{rejected} typed domain rejections/no-output checks; {len(budgets)} exact budget edges; {len(mutations)} complete-verifier output mutations; infrastructure escape, simple RGB, static/animated eight orientations, and actual-rewriter late-frame sensitivity passed")


if __name__ == "__main__":
    main()
