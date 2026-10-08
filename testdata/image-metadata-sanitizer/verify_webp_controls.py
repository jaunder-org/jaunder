#!/usr/bin/env python3
"""Deliberate domain-error and late-frame controls for the WebP observer."""
import importlib.util
import struct
import sys
from pathlib import Path


def chunk(kind, payload):
    return kind + struct.pack("<I", len(payload)) + payload + (b"\0" if len(payload) & 1 else b"")


def load(path):
    spec = importlib.util.spec_from_file_location("webp_observer", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def outer_records(data):
    at, records = 12, []
    while at < len(data):
        length = struct.unpack_from("<I", data, at + 4)[0]
        end = at + 8 + length + (length & 1)
        records.append((data[at:at + 4], data[at + 8:at + 8 + length], at, end))
        at = end
    return records


def assemble(records):
    body = b"".join(chunk(kind, payload) for kind, payload in records)
    return b"RIFF" + struct.pack("<I", len(body) + 4) + b"WEBP" + body


def rejected(observe, path, reason):
    try:
        observe.inspect(path)
    except observe.WebPContainerError as error:
        assert reason in str(error), (reason, error)
        return
    raise AssertionError(f"{path.name} unexpectedly accepted")


if __name__ == "__main__":
    root, observer_path = map(Path, sys.argv[1:])
    observe = load(observer_path)
    work = root / "controls"
    work.mkdir(exist_ok=True)
    source_path = root / "input" / "animated-lossy-alpha-v4.webp"
    source = source_path.read_bytes()
    records = outer_records(source)
    for name, expected in (("plain-lossy-rgb.webp", "VP8 "), ("plain-lossless-rgb.webp", "VP8L")):
        plain = observe.inspect(root / "input" / name)
        assert plain["kinds"] == [expected] and plain["flags"] == 0
    vp8x = next(index for index, (kind, *_rest) in enumerate(records) if kind == b"VP8X")
    anim = next(index for index, (kind, *_rest) in enumerate(records) if kind == b"ANIM")
    anmf = [index for index, (kind, *_rest) in enumerate(records) if kind == b"ANMF"]
    iccp = next(index for index, (kind, *_rest) in enumerate(records) if kind == b"ICCP")
    assert len(anmf) == 3
    cases = {
        "declared-size": (source[:4] + struct.pack("<I", len(source)) + source[8:], "RIFF declared extent/trailer"),
        "trailer": (source + b"x", "RIFF declared extent/trailer"),
        # The enclosing RIFF length remains untouched: this is specifically a
        # chunk-extent failure rather than an outer-size failure.
        "chunk-length": (source[:records[vp8x][2] + 4] + struct.pack("<I", 0xffffffff) + source[records[vp8x][2] + 8:], "chunk extent/padding"),
    }
    xmp = next(index for index, (kind, *_rest) in enumerate(records) if kind == b"XMP ")
    # Preserve or create an admitted odd-sized XMP payload, then corrupt only
    # its padding byte. XMP is recognized, so no unknown-chunk rejection masks it.
    extra = b"" if len(records[xmp][1]) & 1 else b"x"
    odd_records = [(kind, payload + extra if index == xmp else payload) for index, (kind, payload, *_rest) in enumerate(records)]
    valid_odd = bytearray(assemble(odd_records))
    admitted_odd = work / "admitted-odd-padding.webp"
    admitted_odd.write_bytes(valid_odd)
    observe.inspect(admitted_odd)
    odd_xmp = next(record for record in outer_records(valid_odd) if record[0] == b"XMP ")
    assert len(odd_xmp[1]) & 1
    valid_odd[odd_xmp[3] - 1] = 1
    cases["padding"] = (bytes(valid_odd), "nonzero RIFF padding")
    data = bytearray(source)
    data[records[vp8x][2] + 8 + 1] = 1
    cases["vp8x-reserved"] = (bytes(data), "VP8X reserved bytes")
    data = bytearray(source)
    data[records[vp8x][2] + 8] &= ~0x04
    cases["vp8x-feature"] = (bytes(data), "VP8X feature flags")
    swapped = list(records)
    swapped[anim], swapped[anmf[0]] = swapped[anmf[0]], swapped[anim]
    cases["anim-order"] = (assemble((kind, payload) for kind, payload, *_ in swapped), "ANIM ordering")
    cases["duplicate-iccp"] = (assemble([(kind, payload) for kind, payload, *_ in records[:iccp + 1]] + [(b"ICCP", records[iccp][1])] + [(kind, payload) for kind, payload, *_ in records[iccp + 1:]]), "duplicate ICCP")
    for label, kind in (("duplicate-exif", b"EXIF"), ("duplicate-xmp", b"XMP "),
                        ("duplicate-anim", b"ANIM"), ("duplicate-vp8x", b"VP8X")):
        index = next(i for i, (name, *_rest) in enumerate(records) if name == kind)
        duplicated = [(name, payload) for name, payload, *_ in records[:index + 1]] + [(kind, records[index][1])] + [(name, payload) for name, payload, *_ in records[index + 1:]]
        cases[label] = (assemble(duplicated), f"duplicate {kind.decode('ascii')}")
    start = records[anmf[-1]][2] + 8
    data = bytearray(source)
    data[start + 6:start + 9] = (0xffffff).to_bytes(3, "little")
    cases["anmf-overflow"] = (bytes(data), "ANMF flags/rectangle")
    data = bytearray(source)
    data[start + 16 + 4:start + 16 + 8] = (0xffffffff).to_bytes(4, "little")
    cases["anmf-nested-boundary"] = (bytes(data), "chunk extent/padding")
    # First frame uses ALPH+VP8; change the VP8 frame-tag to an inter-frame.
    data = bytearray(source)
    first_payload = records[anmf[0]][2] + 8
    alpha_payload = first_payload + 16 + 8
    alpha_length = struct.unpack_from("<I", data, first_payload + 16 + 4)[0]
    vp8_payload = alpha_payload + alpha_length + (alpha_length & 1) + 8
    data[vp8_payload] |= 1
    cases["vp8-header"] = (bytes(data), "VP8 non-key frame")
    static_lossy = outer_records((root / "input" / "static-lossy-alpha-v4.webp").read_bytes())
    static_lossless = outer_records((root / "input" / "static-lossless-alpha-v4.webp").read_bytes())
    alph = next(i for i, (kind, *_rest) in enumerate(static_lossy) if kind == b"ALPH")
    vp8 = next(i for i, (kind, *_rest) in enumerate(static_lossy) if kind == b"VP8 ")
    swapped_static = list(static_lossy)
    swapped_static[alph], swapped_static[vp8] = swapped_static[vp8], swapped_static[alph]
    cases["static-alph-order"] = (assemble((kind, payload) for kind, payload, *_ in swapped_static), "static ALPH/VP8 ordering")
    cases["alph-with-vp8l"] = (assemble([(kind, payload) for kind, payload, *_ in static_lossless[:2]] + [(b"ALPH", static_lossy[alph][1])] + [(kind, payload) for kind, payload, *_ in static_lossless[2:]]), "ALPH with VP8L")
    data = bytearray(assemble((kind, payload) for kind, payload, *_ in static_lossy))
    static_alph = next(record for record in outer_records(data) if record[0] == b"ALPH")
    data[static_alph[2] + 8] |= 0x40
    cases["alph-header"] = (bytes(data), "ALPH reserved bits")
    data = bytearray(assemble((kind, payload) for kind, payload, *_ in static_lossless))
    static_vp8l = next(record for record in outer_records(data) if record[0] == b"VP8L")
    data[static_vp8l[2] + 8 + 4] |= 0x20
    cases["vp8l-header"] = (bytes(data), "VP8L version/reserved bits")
    data = bytearray(assemble((kind, payload) for kind, payload, *_ in static_lossy))
    static_vp8x = next(record for record in outer_records(data) if record[0] == b"VP8X")
    data[static_vp8x[2] + 8 + 4] ^= 1
    cases["static-canvas"] = (bytes(data), "VP8X/static canvas mismatch")
    for name, (data, reason) in cases.items():
        path = work / f"{name}.webp"
        path.write_bytes(data)
        rejected(observe, path, reason)
    alternate = outer_records((root / "input" / "animated-lossy-alpha-v4-alternate.webpm").read_bytes())
    replacement = [payload for kind, payload, *_ in alternate if kind == b"ANMF"][-1]
    mutant = work / "valid-last-frame-mutant.webp"
    mutant.write_bytes(assemble((kind, replacement if index == anmf[-1] else payload) for index, (kind, payload, *_rest) in enumerate(records)))
    before, changed = observe.decode(source_path), observe.decode(mutant)
    assert before["frames"][:-1] == changed["frames"][:-1] and before["frames"][-1] != changed["frames"][-1]
    for key in ("timings", "loop", "orientation", "icc", "xmp", "exif_tags"):
        assert before[key] == changed[key], key
    assert observe.compressed(observe.inspect(source_path)) != observe.compressed(observe.inspect(mutant))
    print(f"{len(cases)} domain-specific malformed container controls rejected; valid final-frame replacement preserved controls and changed only final canvas")
