#!/usr/bin/env python3
"""Typed no-output rejections and complete-observer corrupt-output controls."""
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

from verify_jpeg import check, command, inspect, load, raw_oriented, verify


def segment(marker, payload):
    return bytes((255, marker)) + struct.pack(">H", len(payload) + 2) + payload


def encode(entries):
    return b"\xff\xd8" + b"".join(p if m == 0 else segment(m, p) for m, p in entries) + b"\xff\xd9"


def main():
    root = Path(sys.argv[1])
    rewrite, icc, webp, png, reader_path, lcms, magick, exiftool = sys.argv[2:]
    reader = load(reader_path, "jpeg_control_icc_reader")
    original_path = root / "input/srgb-v4-o6-s0.jpg"
    cleaned_path = root / "output/srgb-v4-o6-s0.jpg"
    original, cleaned = inspect(original_path.read_bytes()), inspect(cleaned_path.read_bytes())
    negatives, corruptions, edges = [], [], []
    with tempfile.TemporaryDirectory() as directory:
        work = Path(directory)
        def reject(name, data):
            source, output = work / "negative.jpg", work / "unexpected.jpg"
            check(not output.exists(), "stale control output")
            source.write_bytes(data)
            result = subprocess.run([sys.executable, "-B", rewrite, str(source), str(output), icc, webp, png], capture_output=True)
            check(result.returncode == 2 and result.stderr.startswith(b"invalid JPEG ")
                  and not result.stdout and not output.exists(), "non-domain failure/accepted: " + name + repr(result.stderr))
            check(source.read_bytes() == data, "rejected input changed")
            negatives.append(name)
        def corrupt(name, entries):
            output = work / "corrupt.jpg"
            output.write_bytes(encode(entries))
            try:
                verify(original_path, output, reader, lcms, magick, work)
            except AssertionError:
                corruptions.append(name)
            else:
                raise AssertionError("complete verifier accepted corruption " + name)
        def replace(entries, code, transform):
            done, result = False, []
            for m, p in entries:
                if m == code and not done:
                    p, done = transform(p), True
                result.append((m, p))
            check(done, "missing mutation marker")
            return result
        for name, data in (("truncated-marker", b"\xff\xd8\xff"),
                           ("trailing-output", original_path.read_bytes() + b"private"),
                           ("missing-eoi", original_path.read_bytes()[:-2]),
                           ("file-budget-plus-one", b"\xff\xd8" + b"\0" * (32 * 1024 * 1024 - 1))):
            reject(name, data)
        for code in (0xc1, 0xc2, 0xc3, 0xc9, 0xca, 0xcb):
            reject(f"unsupported-sof-{code:x}", encode([(code if m == 0xc0 else m, p) for m, p in original]))
        for code in (0xe3, 0xed, 0xef, 0xdd):
            reject(f"unknown-app-or-dri-{code:x}", encode(original[:1] + [(code, b"private")] + original[1:]))
        for name, code, transform in (
                ("jfif-thumbnail", 0xe0, lambda p: p[:-2] + b"\1\1" + b"\0" * 3),
                ("adobe-transform", 0xee, lambda p: p[:-1] + b"\2"),
                ("zero-dqt", 0xdb, lambda p: p[:1] + b"\0" + p[2:]),
                ("dht-count", 0xc4, lambda p: p[:1] + b"\xff" + p[2:]),
                ("scan-components", 0xda, lambda p: b"\2" + p[1:]),
                ("pixel-budget", 0xc0, lambda p: p[:1] + b"\xff" * 4 + p[5:]),
                ("unsupported-cmyk", 0xc0, lambda p: p[:5] + b"\4" + p[6:]),
                ("icc-sequence-zero", 0xe2, lambda p: p[:12] + b"\0" + p[13:]),
                ("icc-count-mismatch", 0xe2, lambda p: p[:13] + b"\2" + p[14:]),
                ("icc-malformed", 0xe2, lambda p: p[:14] + b"\xff" + p[15:]),
                ("exif-offset", 0xe1, lambda p: p[:10] + b"\xff" * 4 + p[14:]),
                ("exif-type", 0xe1, lambda p: p[:18] + b"\0\0" + p[20:]),
                ("exif-count", 0xe1, lambda p: p[:20] + b"\0" * 4 + p[24:]),
                ("exif-orientation", 0xe1, lambda p: p[:60] + b"\11\0" + p[62:]),
                ("exif-cycle", 0xe1, lambda p: p[:112] + struct.pack("<I", 8) + p[116:]),
                ("exif-value-overlap", 0xe1, lambda p: p[:24] + struct.pack("<I", 8) + p[28:])):
            reject(name, encode(replace(original, code, transform)))
        jfif = next(item for item in original if item[0] == 224)
        without_jfif = [item for item in original if item[0] != 224]
        reject("jfif-not-first", encode(without_jfif[:1] + [jfif] + without_jfif[1:]))
        icc_part = next(p for m, p in original if m == 226)
        reject("duplicate-icc", encode(original[:1] + [(226, icc_part)] + original[1:]))
        reject("duplicate-sof", encode(original[:-2] + [next(item for item in original if item[0] == 192)] + original[-2:]))
        reject("restart-unproved", encode(replace(original, 0, lambda p: p[:2] + b"\xff\xd0" + p[2:])))
        xmp_at = next(i for i, (m, p) in enumerate(original) if m == 225 and p.startswith(b"http"))
        unknown_xmp = list(original)
        unknown_xmp[xmp_at] = (225, unknown_xmp[xmp_at][1].replace(b"dc:description", b"dc:orientation"))
        reject("unknown-xmp-rendering", encode(unknown_xmp))
        # Independently consumer-valid preview-bearing EXIF, built by pinned tool.
        thumbnail = work / "thumbnail.jpg"
        thumbnail.write_bytes(original_path.read_bytes())
        command([exiftool, "-overwrite_original", "-ThumbnailImage<=" + str(root / "input/plain-s0.jpg"), str(thumbnail)])
        raw_oriented(thumbnail)
        command([magick, str(thumbnail), "-depth", "8", "RGB:-"])
        reject("consumer-valid-exif-thumbnail", thumbnail.read_bytes())
        # Actual metadata/record boundaries use COMs, which have no rendering role.
        plain = inspect((root / "input/plain-s0.jpg").read_bytes())
        base_meta = sum(len(p) for m, p in plain if m >= 224)
        remaining = 8 * 1024 * 1024 - base_meta
        comments = [(254, b"x" * 65533) for _ in range(remaining // 65533)]
        if remaining % 65533:
            comments.append((254, b"x" * (remaining % 65533)))
        boundary = encode(plain[:1] + comments + plain[1:])
        source, output = work / "edge.jpg", work / "edge-output.jpg"
        source.write_bytes(boundary)
        command([sys.executable, "-B", rewrite, str(source), str(output), icc, webp, png])
        verify(source, output, reader, lcms, magick, work)
        edges.append("metadata=8388608 admitted with consumer proof")
        reject("metadata-plus-one", encode(plain[:1] + [(254, b"x")] + comments + plain[1:]))
        # Parser counts segment records and the scan extent; EOI also requires
        # headroom. State that precise implementation boundary, not T.81 limits.
        comments = [(254, b"")] * (65535 - len(plain))
        source.write_bytes(encode(plain[:1] + comments + plain[1:]))
        command([sys.executable, "-B", rewrite, str(source), str(output), icc, webp, png])
        verify(source, output, reader, lcms, magick, work)
        edges.append("65535 segment/scan records admitted with consumer proof")
        reject("record-plus-one", encode(plain[:1] + [(254, b"")] + comments + plain[1:]))
        for name, addition in (("leaked-exif", next(item for item in original if item[0] == 225)),
                               ("leaked-xmp", original[xmp_at]), ("leaked-com", (254, b"private"))):
            after_icc = 1 + sum(m == 226 for m, _ in cleaned)
            corrupt(name, cleaned[:after_icc] + [addition] + cleaned[after_icc:])
        first_icc = next(item for item in cleaned if item[0] == 226)
        corrupt("icc-before-jfif", [first_icc] + [item for item in cleaned if item is not first_icc])
        corrupt("icc-after-exif", [item for item in cleaned if item is not first_icc][:2]
                + [first_icc] + [item for item in cleaned if item is not first_icc][2:])
        corrupt("wrong-orientation", replace(cleaned, 225, lambda p: p[:24] + b"\3\0" + p[26:]))
        for name, code in (("entropy-change", 0), ("DQT-change", 219), ("DHT-change", 196),
                           ("SOF-change", 192), ("SOS-change", 218), ("JFIF-change", 224), ("Adobe-change", 238)):
            corrupt(name, replace(cleaned, code, lambda p: p[:-1] + bytes((p[-1] ^ 1,))))
        corrupt("dirty-icc-header", replace(cleaned, 226, lambda p: p[:62] + b"JAUD" + p[66:]))
        payload = next(p for m, p in cleaned if m == 226)
        profile = payload[14:]
        desc = next(struct.unpack_from(">III", profile, i)[1] for i in range(132, 264, 12)
                    if profile[i:i + 4] == b"desc")
        at = 14 + desc + 28
        corrupt("dirty-icc-description", replace(cleaned, 226, lambda p: p[:at] + b"\0X" + p[at + 2:]))
        # Exercise exact file/pixel guard comparisons on consumer-valid small
        # inputs with lowered test-only limits, not giant decoder allocations.
        rewriter = load(rewrite, "jpeg_under_test")
        icc_module = load(icc, "jpeg_control_icc")
        metadata = load(webp, "jpeg_control_metadata")
        metadata.PNG_METADATA = load(png, "jpeg_control_png")
        for attribute, boundary in (("MAX_FILE", len(original_path.read_bytes())), ("MAX_PIXELS", 768)):
            saved = getattr(rewriter, attribute)
            positive, negative = work / (attribute + ".jpg"), work / (attribute + "-negative.jpg")
            try:
                setattr(rewriter, attribute, boundary)
                rewriter.rewrite(original_path, positive, icc_module, metadata)
                verify(original_path, positive, reader, lcms, magick, work)
                setattr(rewriter, attribute, boundary - 1)
                try:
                    rewriter.rewrite(original_path, negative, icc_module, metadata)
                except rewriter.JPEGDomainError:
                    pass
                else:
                    raise AssertionError("budget guard accepted " + attribute)
                check(not negative.exists() and original_path.read_bytes() == encode(original), "budget cleanup/input")
                edges.append(attribute + " exact/plus-one comparison proved at lowered test-only limit")
            finally:
                setattr(rewriter, attribute, saved)
        class BrokenICC:
            @staticmethod
            def rewrite(source, destination):
                raise RuntimeError("injected programming failure")
        try:
            rewriter.rewrite(original_path, work / "programming.jpg", BrokenICC, metadata)
        except RuntimeError as error:
            check(str(error) == "injected programming failure", "wrong programming failure")
        else:
            raise AssertionError("programming failure swallowed")
        check(not (work / "programming.jpg").exists(), "programming partial output")
        class BrokenValueICC:
            @staticmethod
            def rewrite(source, destination):
                raise ValueError("injected programming value failure")
        try:
            rewriter.rewrite(original_path, work / "programming-value.jpg", BrokenValueICC, metadata)
        except rewriter.JPEGDomainError as error:
            raise AssertionError("programming ValueError disguised as profile rejection") from error
        except ValueError as error:
            check(str(error) == "injected programming value failure", "wrong programming value failure")
        else:
            raise AssertionError("programming ValueError swallowed")
        check(not (work / "programming-value.jpg").exists(), "programming value partial output")
        # Infrastructure failures are not domain rejections.
        result = subprocess.run([sys.executable, "-B", rewrite, str(work / "missing.jpg"), str(work / "absent.jpg"),
                                 icc, webp, png], capture_output=True)
        check(result.returncode != 2 and b"FileNotFoundError" in result.stderr
              and not (work / "absent.jpg").exists(), "I/O disguised as domain rejection")
    print(json.dumps({"negative_count": len(negatives), "negatives": negatives,
                      "corrupt_output_count": len(corruptions), "corrupt_outputs": corruptions,
                      "budget_edges": edges, "infrastructure": "missing input propagates"}, sort_keys=True))


if __name__ == "__main__":
    main()
