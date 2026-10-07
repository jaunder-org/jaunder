#!/usr/bin/env python3
"""Adversarial controls for both the ICC canonicalizer and independent inspector."""
import importlib.util
import struct
import subprocess
import sys
from pathlib import Path


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def load(path):
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def table(data):
    return {
        signature: (132 + 12 * index, offset, size)
        for index in range(int.from_bytes(data[128:132], "big"))
        for signature, offset, size in [struct.unpack_from(">4sII", data, 132 + 12 * index)]
    }


def changed(data, offset, value):
    result = bytearray(data)
    result[offset:offset + len(value)] = value
    return result


def mutations(data):
    entries = table(data)
    yield "version", changed(data, 9, b"\xff")
    yield "intent", changed(data, 64, struct.pack(">I", 4))
    yield "illuminant", changed(data, 68, b"\0" * 12)
    yield "header-reserved", changed(data, 127, b"\1")
    yield "flags", changed(data, 44, struct.pack(">I", 0x80000000))
    yield "attributes", changed(data, 56, b"\x80" + b"\0" * 7)
    yield "date", changed(data, 24, bytes.fromhex("07ea0002001f000000000000"))
    if data[8] == 2:
        yield "v2-reserved-id", changed(data, 84, b"X")
    for signature, (entry, offset, size) in entries.items():
        label = signature.decode()
        yield label + "-reserved", changed(data, offset + 4, b"\1")
        yield label + "-truncated", changed(data, entry + 8, struct.pack(">I", 8))
        if size % 4:
            # Enlarge all identical aliases together so overlap rejection cannot
            # hide a body parser accepting external padding inside the extent.
            enlarged = bytearray(data)
            for alias_entry, alias_offset, alias_size in entries.values():
                if (alias_offset, alias_size) == (offset, size):
                    struct.pack_into(">I", enlarged, alias_entry + 8, (size + 3) & ~3)
            yield label + "-absorbed-padding", enlarged
    yield "missing-chad", changed(data, entries[b"chad"][0], b"zzzz")
    yield "missing-cprt", changed(data, entries[b"cprt"][0], b"zzzz")
    yield "duplicate-tag", changed(data, entries[b"cprt"][0], b"desc")
    xyz_entry, xyz_offset, _ = entries[b"wtpt"]
    yield "tag-in-table", changed(data, xyz_entry + 4, struct.pack(">I", 132))
    yield "tag-past-eof", changed(data, xyz_entry + 4, struct.pack(">I", len(data) + 4))
    yield "overlap", changed(data, entries[b"rXYZ"][0] + 4, struct.pack(">I", xyz_offset + 4))
    for signature in (b"desc", b"cprt", b"rTRC", b"chrm"):
        _, offset, _ = entries[signature]
        if data[offset:offset + 4] in (b"desc", b"curv", b"mluc"):
            yield signature.decode() + "-count-overflow", changed(data, offset + 8, b"\xff" * 4)
    for signature in (b"desc", b"cprt"):
        _, offset, size = entries[signature]
        typ = data[offset:offset + 4]
        if typ == b"mluc":
            yield signature.decode() + "-record-size", changed(data, offset + 12, struct.pack(">I", 16))
            yield signature.decode() + "-string-offset", changed(data, offset + 24, struct.pack(">I", 4))
            yield signature.decode() + "-string-odd", changed(data, offset + 20, struct.pack(">I", 3))
            yield signature.decode() + "-locale", changed(data, offset + 16, b"0000")
            string_at = int.from_bytes(data[offset + 24:offset + 28], "big")
            yield signature.decode() + "-unicode", changed(data, offset + string_at, b"\xd8\0")
        elif typ == b"desc":
            ascii_count = int.from_bytes(data[offset + 8:offset + 12], "big")
            unicode_at = offset + 12 + ascii_count
            yield "desc-ascii-termination", changed(data, unicode_at - 1, b"X")
            yield "desc-unicode-count", changed(data, unicode_at + 4, b"\xff" * 4)
            unicode_count = int.from_bytes(data[unicode_at + 4:unicode_at + 8], "big")
            script_at = unicode_at + 8 + 2 * unicode_count
            yield "desc-script-count", changed(data, script_at + 2, b"\xff")
            if unicode_count:
                yield "desc-unicode-termination", changed(data, script_at - 2, b"XX")
                yield "desc-unicode-encoding", changed(data, unicode_at + 8, b"\xd8\0")
        elif typ == b"text":
            yield "text-termination", changed(data, offset + size - 1, b"X")
    yield "singular-adaptation", changed(data, entries[b"chad"][1] + 8, b"\0" * 36)
    singular = bytearray(data)
    for signature in (b"rXYZ", b"gXYZ", b"bXYZ"):
        offset = entries[signature][1]
        singular[offset + 8:offset + 20] = b"\0" * 12
    yield "singular-colorants", singular
    yield "zero-white", changed(data, entries[b"wtpt"][1] + 12, b"\0" * 4)
    _, offset, size = entries[b"rTRC"]
    if data[offset:offset + 4] == b"para":
        yield "para-function", changed(data, offset + 8, b"\0\xff")
        yield "para-reserved", changed(data, offset + 10, b"\1")
        yield "para-gamma", changed(data, offset + 12, b"\0" * 4)
        function = int.from_bytes(data[offset + 8:offset + 10], "big")
        if function != 0:
            yield "para-scale", changed(data, offset + 16, b"\0" * 4)
        if function in (3, 4):
            yield "para-slope", changed(data, offset + 24, struct.pack(">i", -1))
            yield "para-threshold", changed(data, offset + 28, struct.pack(">i", 65537))
    else:
        count = int.from_bytes(data[offset + 8:offset + 12], "big")
        if count == 1:
            yield "curve-zero-gamma", changed(data, offset + 12, b"\0\0")
        elif count > 1:
            yield "curve-nonmonotonic", changed(data, offset + 12, b"\xff\xff")
    ranges = sorted(set((offset, offset + size) for _, offset, size in entries.values()))
    cursor = 132 + 12 * len(entries)
    for offset, end in ranges:
        if offset > cursor:
            yield "gap-padding", changed(data, cursor, b"X")
            break
        cursor = end
    if len(data) > cursor:
        yield "final-padding", changed(data, cursor, b"X")


def main():
    root, rewriter, inspector_path, work = map(Path, sys.argv[1:5])
    inspector = load(inspector_path)
    canonicalizer = load(rewriter)
    work.mkdir(parents=True, exist_ok=True)
    count = 0
    for space in ("srgb", "display-p3"):
        for version in (2, 4):
            stem = f"{space}-v{version}"
            source = root / "ordinary" / f"{stem}.icc"
            output = work / f"{stem}.clean.icc"
            subprocess.run([sys.executable, "-B", rewriter, source, output], check=True)
            inspector.profile(source)
            inspector.sanitized_profile(output)
            source_data = source.read_bytes()
            # Test the bodies directly too: table/gap rejection must not mask
            # an admitted-type parser which accepts truncation or hidden suffixes.
            for signature, (_, offset, size) in table(source_data).items():
                payload = source_data[offset:offset + size]
                canonicalizer.body(payload, version, signature)
                inspector.inspect_body(payload, version, signature)
                for damaged in (payload[:8], payload + b"LEAK"):
                    try:
                        canonicalizer.body(damaged, version, signature)
                    except ValueError as error:
                        require(str(error).startswith("invalid ICC "), "wrong body rejection")
                    else:
                        raise AssertionError(f"{stem}/{signature!r}: malformed body accepted")
                    try:
                        inspector.inspect_body(damaged, version, signature)
                    except AssertionError as error:
                        require(str(error).startswith("invalid inspected ICC "), "wrong body inspection failure")
                    else:
                        raise AssertionError(f"{stem}/{signature!r}: malformed body passed inspection")
                    count += 2
            for label, data in mutations(source_data):
                path = work / f"{stem}-{label}.icc"
                destination = path.with_suffix(".output.icc")
                destination.unlink(missing_ok=True)
                path.write_bytes(data)
                result = subprocess.run([sys.executable, "-B", rewriter, path, destination], capture_output=True, text=True)
                final = result.stderr.splitlines()[-1:] or [""]
                require(result.returncode == 1 and final[0].startswith("ValueError: invalid ICC "), f"{stem}/{label}: wrong rejection: {result.stderr}")
                require(not destination.exists(), f"{stem}/{label}: rejected input left output")
                count += 1
            for label, data in mutations(output.read_bytes()):
                path = work / f"{stem}-output-{label}.icc"
                path.write_bytes(data)
                try:
                    inspector.sanitized_profile(path)
                except AssertionError as error:
                    require(str(error).startswith("invalid inspected ICC "), f"{stem}/{label}: wrong inspection failure: {error}")
                else:
                    raise AssertionError(f"{stem}/{label}: corrupt output passed independent inspection")
                count += 1
            for offset in (4, 40, 48, 52, 80, 84):
                identifiers = changed(output.read_bytes(), offset, b"LEAK")
                path = work / f"{stem}-output-identifier-{offset}.icc"
                path.write_bytes(identifiers)
                try:
                    inspector.sanitized_profile(path)
                except AssertionError as error:
                    require(str(error).startswith("invalid inspected ICC "), "wrong identifier inspection failure")
                else:
                    raise AssertionError("unscrubbed output identifier passed independent inspection")
                count += 1
    print(f"ICC structure controls passed: {count} malformed-input/output checks")


if __name__ == "__main__":
    main()
