#!/usr/bin/env python3
"""Independent complete output observer + real consumers for owned JPEGs.

Does not import rewrite_jpeg. Framing observations do NOT certify entropy.
Pillow and ImageMagick may share libjpeg/LittleCMS; not independent codecs.
"""
import hashlib
import importlib.util
import io
import json
import struct
import subprocess
import sys
import tempfile
import warnings
from pathlib import Path
from PIL import Image, ImageOps, features

warnings.simplefilter("error")


def check(condition, detail):
    if not condition:
        raise AssertionError(detail)


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def inspect(data):
    check(data[:2] == b"\xff\xd8", "SOI")
    cursor, entries = 2, []
    while cursor < len(data):
        check(data[cursor:cursor + 1] == b"\xff" and cursor + 2 <= len(data), "marker framing")
        code = data[cursor + 1]
        cursor += 2
        if code == 217:
            check(cursor == len(data), "EOI/trailer")
            return entries
        check(code not in (0, 255) and cursor + 2 <= len(data), "marker header")
        size = int.from_bytes(data[cursor:cursor + 2], "big")
        check(size >= 2 and cursor + size <= len(data), "marker length")
        entries.append((code, data[cursor + 2:cursor + size]))
        cursor += size
        if code == 218:
            start = cursor
            while cursor < len(data):
                if data[cursor] != 255:
                    cursor += 1
                elif data[cursor:cursor + 2] == b"\xff\0":
                    cursor += 2
                else:
                    break
            check(cursor > start, "scan extent")
            entries.append((0, data[start:cursor]))
    check(False, "missing EOI")


def embedded(entries):
    parts = [p for m, p in entries if m == 226]
    if not parts:
        return None
    check(all(p[:12] == b"ICC_PROFILE\0" and len(p) > 14 for p in parts), "APP2 privacy")
    check([p[12] for p in parts] == list(range(1, len(parts) + 1))
          and all(p[13] == len(parts) for p in parts), "ICC chunks")
    return b"".join(p[14:] for p in parts)


def raw_oriented(path):
    with Image.open(path) as image:
        image.load()
        check(image.mode == "RGB", "consumer mode")
        raw = image.copy()
        display = ImageOps.exif_transpose(image)
        return (raw.size, raw.tobytes()), (display.size, display.tobytes())


def command(args):
    result = subprocess.run(args, capture_output=True)
    if result.returncode:
        sys.stderr.buffer.write(result.stderr)
        result.check_returncode()
    check(not result.stderr, "consumer/tool warning: " + result.stderr.decode(errors="replace"))
    return result.stdout


def verify(source, output, icc_reader, lcms, magick, work):
    original, cleaned = inspect(source.read_bytes()), inspect(output.read_bytes())
    for entries in (original, cleaned):
        check(entries and entries[0][0] == 224, "JFIF immediately after SOI")
    icc_positions = [i for i, (m, _) in enumerate(cleaned) if m == 226]
    check(icc_positions == list(range(1, 1 + len(icc_positions))), "canonical ICC after JFIF")
    control = lambda entries: [(m, p) for m, p in entries if m < 224]
    check(control(original) == control(cleaned), "compressed scan/control changed")
    for code in (224, 238):
        check([p for m, p in original if m == code] == [p for m, p in cleaned if m == code],
              "rendering APP changed")
    check(all(m in {0, 192, 196, 218, 219, 224, 225, 226, 238} for m, _ in cleaned), "privacy marker")
    with Image.open(source) as image:
        orientation = image.getexif().get(274, 1)
        source_size = image.size
    check(orientation in range(1, 9), "source orientation")
    transforms = {2: Image.Transpose.FLIP_LEFT_RIGHT, 3: Image.Transpose.ROTATE_180,
                  4: Image.Transpose.FLIP_TOP_BOTTOM, 5: Image.Transpose.TRANSPOSE,
                  6: Image.Transpose.ROTATE_270, 7: Image.Transpose.TRANSVERSE,
                  8: Image.Transpose.ROTATE_90}
    exif = [p for m, p in cleaned if m == 225]
    if orientation == 1:
        check(not exif, "unneeded/leaked EXIF")
    else:
        # Independent literal TIFF serializer: one SHORT, no next IFD/values.
        expected = (b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0"
                    + bytes((orientation, 0)) + b"\0" * 6)
        check(exif == [expected], "orientation/privacy TIFF")
    before, after = embedded(original), embedded(cleaned)
    check((before is None) == (after is None), "ICC presence")
    if before is not None:
        old, new = work / "source.icc", work / "output.icc"
        old.write_bytes(before)
        new.write_bytes(after)
        a, tags_a = icc_reader.profile(old)
        b, tags_b = icc_reader.sanitized_profile(new)
        for tag in icc_reader.COLOR_TAGS:
            check(tags_a[tag][2] == tags_b[tag][2], "ICC transform bytes")
        expected_header = bytearray(a[:128])
        expected_header[:4] = b[:4]
        expected_header[24:36] = icc_reader.CLEAN_DATE
        for start, end in ((4, 8), (40, 44), (48, 56), (80, 128)):
            expected_header[start:end] = b"\0" * (end - start)
        check(b[:128] == expected_header, "ICC header semantics")
        check(command([lcms, str(old), str(new)]) ==
              b"LittleCMS ordinary RGB equivalence passed for 4 intents x 4,096 samples\n",
              "LittleCMS proof output")
    source_rasters = raw_oriented(source)
    check(source_rasters == raw_oriented(output), "Pillow raw/display mismatch")
    expected = Image.frombytes("RGB", source_size, source_rasters[0][1])
    if orientation != 1:
        expected = expected.transpose(transforms[orientation])
    check(source_rasters[1] == (expected.size, expected.tobytes()), "Pillow orientation semantics")
    im_raw = command([magick, str(source), "-depth", "8", "RGB:-"])
    im_expected = Image.frombytes("RGB", source_size, im_raw)
    if orientation != 1:
        im_expected = im_expected.transpose(transforms[orientation])
    check(command([magick, str(source), "-auto-orient", "-depth", "8", "RGB:-"]) == im_expected.tobytes(),
          "ImageMagick orientation semantics")
    for orient in (False, True):
        option = ["-auto-orient"] if orient else []
        a = command([magick, str(source), *option, "-depth", "8", "RGB:-"])
        b = command([magick, str(output), *option, "-depth", "8", "RGB:-"])
        check(a == b, "ImageMagick raw/display mismatch")
        shape_a = command([magick, str(source), *option, "-format", "%wx%h", "info:"])
        shape_b = command([magick, str(output), *option, "-format", "%wx%h", "info:"])
        check(shape_a == shape_b, "ImageMagick dimensions")
    return {"scan_sha256": hashlib.sha256(next(p for m, p in cleaned if m == 0)).hexdigest(),
            "orientation": orientation, "icc": before is not None,
            "pillow_raw": {"dimensions": source_rasters[0][0], "sha256": hashlib.sha256(source_rasters[0][1]).hexdigest()},
            "pillow_oriented": {"dimensions": source_rasters[1][0], "sha256": hashlib.sha256(source_rasters[1][1]).hexdigest()},
            "imagemagick_raw_sha256": hashlib.sha256(im_raw).hexdigest(),
            "imagemagick_oriented_sha256": hashlib.sha256(im_expected.tobytes()).hexdigest(),
            "lcms": "4 intents x 4096 samples identical" if before is not None else "no embedded profile"}


def main():
    root = Path(sys.argv[1])
    rewrite, icc, webp, png, reader_path, lcms, magick, fixture_path = sys.argv[2:]
    fixture = load(fixture_path, "jpeg_fixture")
    reader = load(reader_path, "jpeg_reader")
    inputs, outputs = root / "input", root / "output"
    inputs.mkdir(parents=True, exist_ok=True)
    outputs.mkdir(parents=True, exist_ok=True)
    art = Image.new("RGB", (32, 24))
    art.putdata([((x * 7) % 256, (y * 11) % 256, ((x + y) * 13) % 256)
                 for y in range(24) for x in range(32)])
    reports = {}
    def run(source, destination):
        command([sys.executable, "-B", rewrite, str(source), str(destination), icc, webp, png])
    with tempfile.TemporaryDirectory() as directory:
        work = Path(directory)
        for space in ("srgb", "display-p3"):
            for version in (2, 4):
                profile = (root.parent / "ordinary" / f"{space}-v{version}.icc").read_bytes()
                for orientation in range(1, 9):
                    for sampling in (0, 2):
                        name = f"{space}-v{version}-o{orientation}-s{sampling}"
                        source, output = inputs / (name + ".jpg"), outputs / (name + ".jpg")
                        buffer = io.BytesIO()
                        art.save(buffer, format="JPEG", quality=92, subsampling=sampling, icc_profile=profile)
                        tiff = bytearray(fixture.exif())
                        # IFD0 fourth record is orientation in the owned constructor.
                        struct.pack_into("<H", tiff, 8 + 2 + 3 * 12 + 8, orientation)
                        xmp = (b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF '
                               b'xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
                               b'<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/" '
                               b'dc:description="synthetic JPEG private description"/>'
                               b'</rdf:RDF></x:xmpmeta>')
                        def seg(m, p):
                            return bytes((255, m)) + struct.pack(">H", len(p) + 2) + p
                        extra = (seg(225, b"Exif\0\0" + tiff)
                                 + seg(225, b"http://ns.adobe.com/xap/1.0/\0" + xmp)
                                 + seg(254, b"Synthetic JPEG private comment")
                                 + seg(238, b"Adobe\0d\0\0\0\0\1"))
                        constructed = inspect(buffer.getvalue())
                        # Exercise actual embedded multipart ICC extraction, not
                        # merely reinsertion of a fixed replacement profile.
                        chunks = [profile[i:i + 128] for i in range(0, len(profile), 128)]
                        check(constructed[0][0] == 224, "constructor JFIF placement")
                        rebuilt = b"\xff\xd8" + seg(*constructed[0]) + extra
                        for marker, payload in constructed[1:]:
                            if marker == 226:
                                rebuilt += b"".join(seg(226, b"ICC_PROFILE\0" + bytes((i + 1, len(chunks))) + p)
                                                    for i, p in enumerate(chunks))
                            else:
                                rebuilt += payload if marker == 0 else seg(marker, payload)
                        source.write_bytes(rebuilt + b"\xff\xd9")
                        unchanged = source.read_bytes()
                        with Image.open(source) as consumer:
                            fields = consumer.getexif()
                            check(fields.get(274) == orientation and fields.get(271) == "Synthetic camera company"
                                  and fields.get(272) == "Fixture camera model"
                                  and fields.get(306) == "2026:10:07 12:34:56"
                                  and fields.get(315) == "Fixture PNG author"
                                  and fields.get(33432) == "CC0 fixture description"
                                  and set(fields.get_ifd(34853)) == {0, 1, 2, 3, 4}
                                  and consumer.info.get("icc_profile") == profile,
                                  "source metadata/ICC not independently consumed")
                        run(source, output)
                        reports[name] = verify(source, output, reader, lcms, magick, work)
                        repeat, second = work / "repeat.jpg", work / "second.jpg"
                        run(source, repeat)
                        run(output, second)
                        check(output.read_bytes() == repeat.read_bytes() == second.read_bytes(), "process determinism/idempotence")
                        check(source.read_bytes() == unchanged, "input mutated")
        for sampling in (0, 2):
            source = inputs / f"plain-s{sampling}.jpg"
            output = outputs / source.name
            art.save(source, quality=92, subsampling=sampling)
            run(source, output)
            reports[source.stem] = verify(source, output, reader, lcms, magick, work)
            check(source.read_bytes() == output.read_bytes(), "plain positive changed")
        # Progressive source is valid for both consumers, but not admitted.
        progressive = inputs / "unsupported-progressive.jpg"
        art.save(progressive, quality=92, progressive=True)
        raw_oriented(progressive)
        command([magick, str(progressive), "-depth", "8", "RGB:-"])
        result = subprocess.run([sys.executable, "-B", rewrite, str(progressive), str(work / "rejected.jpg"),
                                 icc, webp, png], capture_output=True)
        check(result.returncode == 2 and result.stderr.startswith(b"invalid JPEG ")
              and not (work / "rejected.jpg").exists(), "progressive rejection")
    print(json.dumps({"positive_count": len(reports), "outputs": reports,
                      "progressive": "consumer-valid, explicitly rejected; unproved mode",
                      "consumers": {"pillow": Image.__version__, "libjpeg": features.version_codec("jpg"),
                                    "imagemagick_version": command([magick, "-version"]).decode(),
                                    "imagemagick_path": magick,
                                    "caveat": "Pillow/ImageMagick may share libjpeg and LittleCMS"}}, sort_keys=True))


if __name__ == "__main__":
    main()
