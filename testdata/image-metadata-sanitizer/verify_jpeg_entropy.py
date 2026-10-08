#!/usr/bin/env python3
"""Independent authored baseline Huffman/coefficient golden and rejection proof.

Codebooks contain explicit codewords, NOT codes obtained from the rewriter's
canonical-table function. Expected signed coefficients are input specifications,
not observations returned by the validator or encoder. This test-only observer
uses string prefix matching and full small-block models, unlike the prototype's
streaming numeric decoder. No raster reconstruction supplies the syntax oracle.
Owned constructed JPEG artwork is CC0; consumers may share libjpeg.
"""
import hashlib
import json
import struct
import subprocess
import sys
from pathlib import Path

from PIL import Image
from verify_jpeg import check, command, inspect, load, raw_oriented, verify

# Hand assigned, canonical fixed-width codewords, including unused code space.
DC = {symbol: format(symbol, "04b") for symbol in range(12)}
AC_ORDER = [0, 0xf0] + [16 * run + size for run in range(16) for size in range(1, 11)]
AC = {symbol: format(index, "08b") for index, symbol in enumerate(AC_ORDER)}
# Explicit mixed-width words include an actual empty length interval (11..14)
# and an all-ones reserved branch. No canonical-code builder authors these.
MIXED_DC = {0: "0", 1: "10", 2: "110", 3: "1110", 4: "11110", 5: "111110",
            6: "1111110", 7: "11111110", 8: "111111110", 9: "1111111110",
            10: "111111111100000", 11: "1111111111000010"}
MIXED_AC = {0: "0", 240: "10", 1: "110", 2: "1110", 3: "11110", 4: "111110",
            5: "1111110", 6: "11111110", 7: "111111110", 8: "1111111110",
            9: "11111111110", 10: "111111111110", 17: "1111111111110",
            33: "11111111111110", 49: "111111111111110", 65: "1111111111111110"}
EDGE_DIMENSIONS = [(1, 1), (7, 7), (8, 8), (9, 9), (15, 15), (16, 16),
                   (17, 17), (1, 17), (17, 1), (33, 25)]
# Independently enumerated padded MCU census, not the constructor's output size.
EDGE_MCUS = {(1, 1): (1, 1), (7, 7): (1, 1), (8, 8): (1, 1), (9, 9): (4, 1),
             (15, 15): (4, 1), (16, 16): (4, 1), (17, 17): (9, 4),
             (1, 17): (3, 2), (17, 1): (3, 2), (33, 25): (20, 6)}


def required_ids():
    # Census is independent of the constructor/observed report population.
    return ({f"dc-{size}-{sign}-{edge}" for size in range(1, 12)
             for sign in (-1, 1) for edge in ("low", "high")}
            | {f"ac-{run}-{size}-{sign}-{edge}" for run in range(16) for size in range(1, 11)
               for sign in (-1, 1) for edge in ("low", "high")}
            | {f"position-{position}-{sign}" for position in range(1, 64) for sign in (-1, 1)}
            | {f"width-{width}-s{sampling}" for width in range(1, 17) for sampling in (0, 2)}
            | {f"edge-{width}x{height}-s{sampling}" for width, height in EDGE_DIMENSIONS for sampling in (0, 2)}
            | {f"fill-{length}-s{sampling}" for length in range(1, 9) for sampling in (0, 2)}
            | {"dense-63", "zrl-at-47", "stuffed-final-fill", "predictors-420", "zero-category",
               "mixed-width-dc", "mixed-width-ac", "zrl-eob-at-63"}
            | {f"zrl-eob-{count}-s{sampling}" for count in range(1, 4) for sampling in (0, 2)})


def required_negative_ids():
    return ({"unallocated-prefix", "partial-dc-code", "partial-dc-amplitude", "partial-ac-code",
             "partial-ac-amplitude", "zrl-past-block", "unsupported-terminal-zrl-with-extra-block", "ac-run-past-63",
             "dc-positive-precision", "dc-negative-precision", "dc-predictor-precision", "dc-predictor-negative-precision",
             "illegal-dc-category", "illegal-ac-size",
             "illegal-ac-zero-size", "all-ones-dht", "hidden-extra-byte", "hidden-extra-stuffed-byte",
             "zero-final-fill", "missing-stuff-byte", "ff-restart-not-stuff", "ff-fill-not-stuff"}
            | {f"consumer-valid-unsupported-terminal-zrl-s{sampling}" for sampling in (0, 2)}
            | {f"padded-{state}-s{sampling}" for state in ("block-missing", "extra-block", "mcu-missing", "extra-mcu") for sampling in (0, 2)}
            | {f"fill-zero-length-{length}-position-{position}" for length in range(1, 9)
               for position in range((-3 * (1 + length)) % 8)}
            | {f"dc-amplitude-{size}-prefix-{present}" for size in range(1, 12) for present in range(size)}
            | {f"ac-amplitude-{size}-prefix-{present}" for size in range(1, 11) for present in range(size)})


def segment(marker, payload):
    return bytes((255, marker)) + struct.pack(">H", len(payload) + 2) + payload


def pack(bits, fill=True):
    if fill:
        bits += "1" * (-len(bits) % 8)
    check(len(bits) % 8 == 0, "test bitstring must be byte-aligned")
    data = bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8))
    return data.replace(b"\xff", b"\xff\0")


def dht(identifier, book):
    ordered = sorted(book, key=lambda symbol: (len(book[symbol]), int(book[symbol], 2)))
    counts = bytes(sum(len(book[symbol]) == width for symbol in book) for width in range(1, 17))
    # Serialize the explicitly authored schedule; do not derive canonical words.
    return bytes((identifier,)) + counts + bytes(ordered)


def canvas(width, height, sampling, bits, dc=DC, ac=AC, raw=None):
    frame = (bytes((8,)) + struct.pack(">HHB", height, width, 3)
             + bytes((1, 0x22 if sampling == 2 else 0x11, 0, 2, 0x11, 1, 3, 0x11, 1)))
    return (b"\xff\xd8" + segment(224, b"JFIF\0\1\1\0\0\1\0\1\0\0")
            + segment(219, b"\0" + b"\1" * 64 + b"\1" + b"\1" * 64)
            + segment(192, frame)
            + segment(196, dht(0, dc) + dht(1, dc) + dht(16, ac) + dht(17, ac))
            + segment(218, b"\3\1\0\2\x11\3\x11\0?\0")
            + (pack(bits) if raw is None else raw) + b"\xff\xd9")


def amplitude(value):
    size = abs(value).bit_length()
    number = value if value > 0 else value + (1 << size) - 1
    return size, format(number, f"0{size}b") if size else ""


def block_spec(dc_value=0, coefficients=()):
    return (dc_value, tuple(coefficients))


def schedule(width, height, sampling):
    side = 16 if sampling == 2 else 8
    cells = ((width + side - 1) // side) * ((height + side - 1) // side)
    return [component for _ in range(cells) for component in ((0, 0, 0, 0, 1, 2) if sampling == 2 else (0, 1, 2))]


def encode_models(components, models, dc, ac):
    previous, bits = [0, 0, 0], ""
    for component, (value, coefficients) in zip(components, models, strict=True):
        size, additional = amplitude(value - previous[component])
        previous[component] = value
        bits += dc[size] + additional
        next_position = 1
        for position, coefficient in coefficients:
            gap = position - next_position
            check(gap >= 0 and position <= 63, "authored coefficient positions")
            while gap >= 16:
                bits += ac[0xf0]
                gap -= 16
            size, additional = amplitude(coefficient)
            bits += ac[16 * gap + size] + additional
            next_position = position + 1
        if next_position <= 63:
            bits += ac[0]
    return bits


def independently_observe(data, width, height, sampling, dc, ac, expected):
    entries = inspect(data)
    tables = next(p for m, p in entries if m == 196)
    check(tables == dht(0, dc) + dht(1, dc) + dht(16, ac) + dht(17, ac), "authored DHT changed")
    entropy = next(p for m, p in entries if m == 0)
    byte_list, index, stuffed = [], 0, 0
    while index < len(entropy):
        value = entropy[index]
        byte_list.append(value)
        index += 1
        if value == 255:
            check(index < len(entropy) and entropy[index] == 0, "golden byte stuffing")
            index += 1
            stuffed += 1
    stream = "".join(format(value, "08b") for value in byte_list)
    position, previous = 0, {0: 0, 1: 0, 2: 0}
    widths, categories, symbols, digest = set(), set(), set(), hashlib.sha256()
    observed, eobs, zrls, nonzero = [], 0, 0, 0

    def token(book):
        nonlocal position
        matches = [(symbol, word) for symbol, word in book.items() if stream.startswith(word, position)]
        check(len(matches) == 1, "independent explicit prefix match")
        symbol, word = matches[0]
        position += len(word)
        widths.add(len(word))
        return symbol

    def signed(length):
        nonlocal position
        word = stream[position:position + length]
        check(len(word) == length, "independent amplitude extent")
        position += length
        # Independent sign interpretation via inverted negative magnitude bits.
        return int(word, 2) if word[0] == "1" else -int("".join("1" if bit == "0" else "0" for bit in word), 2)

    for component in schedule(width, height, sampling):
        category = token(dc)
        categories.add(category)
        difference = signed(category) if category else 0
        value = previous[component] + difference
        previous[component] = value
        coefficients, slot = [], 1
        digest.update(struct.pack(">Bh", component, value))
        while slot < 64:
            rs = token(ac)
            symbols.add(rs)
            if rs == 0:
                eobs += 1
                break
            if rs == 240:
                zrls += 1
                slot += 16
                check(slot < 64, "independent ZRL extent")
                continue
            slot += rs // 16
            check(slot < 64, "independent AC extent")
            value = signed(rs % 16)
            coefficients.append((slot, value))
            digest.update(struct.pack(">Bh", slot, value))
            nonzero += 1
            slot += 1
        digest.update(b"\xff")
        observed.append((previous[component], tuple(coefficients)))
    check(observed == expected, "independent coefficient/block golden mismatch")
    fill = stream[position:]
    check(len(fill) < 8 and fill == "1" * len(fill), "independent exact terminal fill")
    return {"mcus": len(observed) // (6 if sampling == 2 else 3), "blocks": len(observed),
            "bits": position, "fill": len(fill), "stuffed": stuffed,
            "widths": sorted(widths), "dc_categories": sorted(categories), "ac_symbols": sorted(symbols),
            "eobs": eobs, "zrls": zrls, "nonzero_ac": nonzero, "coefficient_sha256": digest.hexdigest()}


def cases():
    result = {}
    def add(name, width=8, height=8, sampling=0, changes=None, dc=DC, ac=AC):
        components = schedule(width, height, sampling)
        models = [block_spec() for _ in components]
        for index, model in (changes or {}).items():
            models[index] = model
        result[name] = (width, height, sampling, models, dc, ac)
    for size in range(1, 12):
        for sign in (-1, 1):
            for edge, magnitude in (("low", 1 << (size - 1)), ("high", (1 << size) - 1)):
                value = sign * magnitude
                if size == 11:
                    initial = -1024 if sign == 1 else 1023
                    changes = {0: block_spec(initial), 3: block_spec(initial + value)}
                    add(f"dc-{size}-{sign}-{edge}", width=16, changes=changes)
                else:
                    add(f"dc-{size}-{sign}-{edge}", changes={0: block_spec(value)})
    for run in range(16):
        for size in range(1, 11):
            for sign in (-1, 1):
                for edge, value in (("low", 1 << (size - 1)), ("high", (1 << size) - 1)):
                    add(f"ac-{run}-{size}-{sign}-{edge}", changes={0: block_spec(0, [(run + 1, sign * value)])})
    for position in range(1, 64):
        for sign in (-1, 1):
            add(f"position-{position}-{sign}", changes={0: block_spec(0, [(position, sign)])})
    for width in range(1, 17):
        for sampling in (0, 2):
            add(f"width-{width}-s{sampling}", sampling=sampling, dc={0: "0" * width}, ac={0: "0" * width})
    for width, height in EDGE_DIMENSIONS:
        for sampling in (0, 2):
            add(f"edge-{width}x{height}-s{sampling}", width, height, sampling)
    for length in range(1, 9):
        for sampling in (0, 2):
            # Different component schedules ensure all final bit residues exist.
            add(f"fill-{length}-s{sampling}", sampling=sampling, dc={0: "0"}, ac={0: "0" * length})
    add("dense-63", changes={0: block_spec(0, [(p, (-1 if p % 2 else 1) * ((1 << ((p - 1) % 10 + 1)) - 1))
                                             for p in range(1, 64)])})
    add("zrl-at-47", changes={0: block_spec(0, [(46, 1), (63, -1)])})
    add("stuffed-final-fill", dc={0: "0"}, ac={1: "00", 0: "01"})
    add("predictors-420", sampling=2, changes={0: block_spec(-1024), 1: block_spec(1023),
                                               2: block_spec(-1), 3: block_spec(1),
                                               4: block_spec(7), 5: block_spec(-8)})
    add("zero-category")
    # Authored DC coefficient series exercises categories 0..11, not values
    # reconstructed from a producer trace. All component predictors stay 11-bit.
    series = [0, 1, -1, 3, -5, 11, -21, 43, -85, 171, -341, 683]
    add("mixed-width-dc", 32, 24, changes={index * 3: block_spec(value) for index, value in enumerate(series)}, dc=MIXED_DC)
    add("mixed-width-ac", changes={0: block_spec(0, [(p, 1 << (p - 1)) for p in range(1, 11)]
                                                  + [(12, -1), (15, 1), (19, -1), (24, 1), (41, -1)])}, ac=MIXED_AC)
    add("zrl-eob-at-63", changes={0: block_spec(0, [(46, 1)])})
    for count in range(1, 4):
        for sampling in (0, 2):
            add(f"zrl-eob-{count}-s{sampling}", sampling=sampling)
    check(set(result) == required_ids(), "constructor versus independent required census")
    return result


def main():
    root = Path(sys.argv[1])
    rewrite, icc, webp, png, reader_path, lcms, magick = sys.argv[2:]
    root.mkdir(parents=True, exist_ok=True)
    work = root / "work"
    work.mkdir(exist_ok=True)
    rewriter, reader = load(rewrite, "jpeg_entropy_under_test"), load(reader_path, "jpeg_entropy_icc_reader")
    reports, negatives, mutations, unsupported = {}, {}, [], {}
    coverage = {"widths": set(), "dc_categories": set(), "ac_symbols": set(), "fills": set()}
    def invoke(source, output):
        return [sys.executable, "-B", rewrite, str(source), str(output), icc, webp, png]
    for name, (width, height, sampling, models, dc, ac) in cases().items():
        components = schedule(width, height, sampling)
        bits = encode_models(components, models, dc, ac)
        # Redundant trailing ZRL + EOB is valid F.2.2 decoding syntax even though
        # F.1's efficient encoder uses a single EOB for the trailing zero run.
        if name.startswith("zrl-eob-") and name != "zrl-eob-at-63":
            count = int(name.split("-")[2])
            bits = dc[0] + ac[240] * count + ac[0] + bits[12:]
        elif name == "zrl-eob-at-63":
            first = dc[0] + ac[240] * 2 + ac[0xd1] + "1" + ac[240] + ac[0]
            bits = first + encode_models(components[1:], models[1:], dc, ac)
        data = canvas(width, height, sampling, bits, dc, ac)
        observed = independently_observe(data, width, height, sampling, dc, ac, models)
        # Independent expected models/explicit codewords must agree with the
        # real prototype's bounded aggregate trace, not just a green CLI exit.
        tables = {}
        for identifier, book in ((0, dc), (1, dc), (16, ac), (17, ac)):
            definition = dht(identifier, book)
            compiled = rewriter.huffman_codes(definition[1:17], definition[17:])
            check(compiled == {(len(word), int(word, 2)): symbol for symbol, word in book.items()},
                  "canonical compiler versus explicit codeword golden: " + name)
            tables[identifier] = compiled
        scan = next(p for m, p in inspect(data) if m == 0)
        actual = rewriter.validate_entropy(scan, width, height, 0x22 if sampling == 2 else 0x11, tables)
        check(actual == observed, "validator versus authored golden: " + name)
        if name.startswith("edge-"):
            mcus = EDGE_MCUS[width, height][0 if sampling == 0 else 1]
            check(observed["mcus"] == mcus and observed["blocks"] == mcus * (3 if sampling == 0 else 6),
                  "independent padded MCU/block census: " + name)
        for key in ("widths", "dc_categories", "ac_symbols"):
            coverage[key].update(observed[key])
        coverage["fills"].add(observed["fill"])
        source, output = root / (name + ".jpg"), work / (name + ".output.jpg")
        source.write_bytes(data)
        command(invoke(source, output))
        check(source.read_bytes() == data == output.read_bytes(), "golden compressed bytes/input changed")
        # Both consumers must decode each valid golden without warnings.
        raw = raw_oriented(source)
        check(raw == raw_oriented(output) and raw[0][0] == (width, height), "Pillow golden consumer")
        pixels = command([magick, str(source), "-depth", "8", "RGB:-"])
        check(pixels == command([magick, str(output), "-depth", "8", "RGB:-"])
              and len(pixels) == width * height * 3, "ImageMagick golden consumer")
        if all(model == block_spec() for model in models):
            check(raw[0][1] == pixels == bytes((128,)) * (width * height * 3), "independent exact DC/EOB grey pixels")
        reports[name] = observed | {"dimensions": [width, height], "sampling": sampling,
                                    "consumer_sha256": hashlib.sha256(pixels).hexdigest()}
    check(set(reports) == required_ids(), "published golden census")
    check(coverage["widths"] == set(range(1, 17)) and coverage["dc_categories"] == set(range(12))
          and coverage["ac_symbols"] == set(AC_ORDER) and coverage["fills"] == set(range(8)), "required actual state coverage")
    check(reports["stuffed-final-fill"]["stuffed"] > 0 and reports["stuffed-final-fill"]["fill"] == 7
          and any(value["stuffed"] for name, value in reports.items() if name.startswith("ac-")), "actual stuffing coverage")

    def reject(name, data, reason):
        source, output = work / (name + ".bad.jpg"), work / (name + ".unexpected.jpg")
        source.write_bytes(data)
        result = subprocess.run(invoke(source, output), capture_output=True)
        check(result.returncode == 2 and not result.stdout and result.stderr.decode().strip() == "invalid JPEG " + reason
              and not output.exists() and source.read_bytes() == data,
              "targeted grammar rejection " + name + ": " + repr(result.stderr))
        negatives[name] = reason
    # Every source below retains valid JFIF/frame/scan ordering. Expected exact
    # diagnostics ensure header/order failures cannot mask entropy assertions.
    zero = encode_models(schedule(8, 8, 0), [block_spec()] * 3, DC, AC)
    reject("unallocated-prefix", canvas(8, 8, 0, "1" * 16 + zero), "entropy unallocated Huffman code")
    reject("partial-dc-code", canvas(8, 8, 0, "", {0: "0" * 16}, {0: "0"}, raw=b"\0"), "entropy truncated Huffman code")
    reject("partial-dc-amplitude", canvas(8, 8, 0, "", {11: "0" * 8}, {0: "0"}, raw=b"\0"), "entropy truncated DC amplitude")
    reject("partial-ac-code", canvas(8, 8, 0, "", {0: "0" * 8}, {0: "0" * 16}, raw=b"\0\0"), "entropy truncated Huffman code")
    reject("partial-ac-amplitude", canvas(8, 8, 0, "", {0: "0" * 8}, {10: "0" * 8}, raw=b"\0\0"), "entropy truncated AC amplitude")
    reject("zrl-past-block", canvas(8, 8, 0, DC[0] + AC[240] * 4 + zero), "entropy ZRL overflow")
    prefix = DC[0] + AC[240] * 2 + AC[0xe1] + "1"  # coefficient 47, next slot 48
    # Retain the earlier control's exact bytes under its corrected classification.
    reject("unsupported-terminal-zrl-with-extra-block", canvas(8, 8, 0, prefix + AC[240] + zero),
           "unsupported terminal ZRL")
    # Exact-terminal ZRL is conservatively unsupported, not proven malformed.
    # Observe both consumers before checking the explicit unsupported diagnostic.
    for sampling in (0, 2):
        components = schedule(8, 8, sampling)
        remaining = encode_models(components[1:], [block_spec()] * (len(components) - 1), DC, AC)
        baseline = canvas(8, 8, sampling, prefix + AC[0] + remaining)
        terminal = canvas(8, 8, sampling, prefix + AC[240] + remaining)
        before, after = work / "terminal-eob.jpg", work / "terminal-zrl.jpg"
        before.write_bytes(baseline)
        after.write_bytes(terminal)
        check(raw_oriented(before) == raw_oriented(after), "terminal ZRL Pillow consumer equivalence")
        pixels = command([magick, str(before), "-depth", "8", "RGB:-"])
        check(pixels == command([magick, str(after), "-depth", "8", "RGB:-"]),
              "terminal ZRL ImageMagick consumer equivalence")
        name = f"consumer-valid-unsupported-terminal-zrl-s{sampling}"
        reject(name, terminal, "unsupported terminal ZRL")
        unsupported[name] = {"Pillow_equivalent": True, "ImageMagick_equivalent_no_warning": True,
                             "terminal_zero_slots": [48, 63],
                             "consumer_sha256": hashlib.sha256(pixels).hexdigest(),
                             "classification": "conservatively unsupported; normative validity unresolved"}
    reject("ac-run-past-63", canvas(8, 8, 0, DC[0] + AC[240] * 3 + AC[0xf1] + "1" + zero), "entropy AC run overflow")
    reject("dc-positive-precision", canvas(8, 8, 0, DC[11] + "10000000000" + AC[0] + zero), "entropy DC coefficient precision")
    reject("dc-negative-precision", canvas(8, 8, 0, DC[11] + "01111111110" + AC[0] + zero), "entropy DC coefficient precision")
    overflow = encode_models(schedule(16, 8, 0), [block_spec(1023), block_spec(), block_spec(),
                                                block_spec(2046), block_spec(), block_spec()], DC, AC)
    reject("dc-predictor-precision", canvas(16, 8, 0, overflow), "entropy DC coefficient precision")
    overflow = encode_models(schedule(16, 8, 0), [block_spec(-1024), block_spec(), block_spec(),
                                                block_spec(-2048), block_spec(), block_spec()], DC, AC)
    reject("dc-predictor-negative-precision", canvas(16, 8, 0, overflow), "entropy DC coefficient precision")
    for name, dc, ac, reason in (("illegal-dc-category", {12: "0"}, {0: "0"}, "DHT symbols"),
                                 ("illegal-ac-size", {0: "0"}, {11: "0"}, "DHT symbols"),
                                 ("illegal-ac-zero-size", {0: "0"}, {16: "0"}, "DHT symbols"),
                                 ("all-ones-dht", {0: "0", 1: "1"}, {0: "0"}, "DHT oversubscribed/all-ones code")):
        reject(name, canvas(8, 8, 0, zero, dc, ac), reason)
    # Every proper amplitude-prefix length, including zero bits present, is
    # tested at physical byte EOF. Tables/order remain valid; exact diagnostics
    # prove the read reaches the intended amplitude, not a header failure.
    for size in range(1, 12):
        for present in range(size):
            length = (8 - present % 8) or 8
            prefix = "0" * length + "1" * present
            reject(f"dc-amplitude-{size}-prefix-{present}",
                   canvas(8, 8, 0, "", {size: "0" * length}, {0: "0"}, raw=pack(prefix, False)),
                   "entropy truncated DC amplitude")
    for size in range(1, 11):
        for present in range(size):
            length = (7 - present % 8) or 8
            prefix = "0" + "0" * length + "1" * present
            reject(f"ac-amplitude-{size}-prefix-{present}",
                   canvas(8, 8, 0, "", {0: "0"}, {size: "0" * length}, raw=pack(prefix, False)),
                   "entropy truncated AC amplitude")
    good_scan = pack(zero)
    for name, extra in (("hidden-extra-byte", b"\0"), ("hidden-extra-stuffed-byte", b"\xff\0")):
        reject(name, canvas(8, 8, 0, "", raw=good_scan + extra), "entropy extra bytes")
    # Fixed four/eight-bit codes use 36 significant bits; four final one-bits.
    check(len(zero) == 36, "authored fill boundary")
    reject("zero-final-fill", canvas(8, 8, 0, "", raw=good_scan[:-1] + bytes((good_scan[-1] & 0xfe,))), "entropy final fill")
    for length in range(1, 9):
        significant = 3 * (1 + length)
        fill = (-significant) % 8
        for position in range(fill):
            terminal = "1" * position + "0" + "1" * (fill - position - 1)
            reject(f"fill-zero-length-{length}-position-{position}",
                   canvas(8, 8, 0, "", {0: "0"}, {0: "0" * length},
                          raw=pack("0" * significant + terminal, False)), "entropy final fill")
    for sampling in (0, 2):
        required = schedule(17, 17, sampling)
        fewer = encode_models(required[:-1], [block_spec()] * (len(required) - 1), DC, AC)
        reject(f"padded-block-missing-s{sampling}", canvas(17, 17, sampling, fewer), "entropy truncated Huffman code")
        extra = encode_models(required + [0], [block_spec()] * (len(required) + 1), DC, AC)
        reject(f"padded-extra-block-s{sampling}", canvas(17, 17, sampling, extra), "entropy extra bytes")
        per_mcu = 6 if sampling == 2 else 3
        fewer = encode_models(required[:-per_mcu], [block_spec()] * (len(required) - per_mcu), DC, AC)
        reject(f"padded-mcu-missing-s{sampling}", canvas(17, 17, sampling, fewer), "entropy truncated Huffman code")
        extra = encode_models(required + required[:per_mcu], [block_spec()] * (len(required) + per_mcu), DC, AC)
        reject(f"padded-extra-mcu-s{sampling}", canvas(17, 17, sampling, extra), "entropy extra bytes")
    stuffed = pack("001001001")
    check(stuffed.endswith(b"\xff\0"), "authored stuffed fill byte")
    reject("missing-stuff-byte", canvas(8, 8, 0, "", {0: "0"}, {1: "00", 0: "01"}, raw=stuffed[:-1]), "unsupported scan marker/restart")
    reject("ff-restart-not-stuff", canvas(8, 8, 0, "", raw=b"\xff\xd0" + good_scan), "unsupported scan marker/restart")
    reject("ff-fill-not-stuff", canvas(8, 8, 0, "", raw=b"\xff\xff\0" + good_scan), "unsupported scan marker/restart")
    # Output-corruption sensitivity goes through the ACTUAL complete observer.
    source = root / "zero-category.jpg"
    data = source.read_bytes()
    scan_at = data.index(segment(218, b"\3\1\0\2\x11\3\x11\0?\0")) + 14
    for name, changed in (("entropy-codeword-drift", data[:scan_at] + bytes((data[scan_at] ^ 0x10,)) + data[scan_at + 1:]),
                          ("entropy-hidden-extra", data[:-2] + b"\0" + data[-2:]),
                          ("entropy-fill-drift", data[:-3] + bytes((data[-3] ^ 1,)) + data[-2:])):
        output = work / (name + ".corrupt.jpg")
        output.write_bytes(changed)
        try:
            verify(source, output, reader, lcms, magick, work)
        except AssertionError as error:
            check(str(error) == "compressed scan/control changed", "masked complete-observer mutation: " + name)
            mutations.append(name)
        else:
            raise AssertionError("complete observer accepted " + name)
    check(set(negatives) == required_negative_ids(), "independent rejection census")
    check(mutations == ["entropy-codeword-drift", "entropy-hidden-extra", "entropy-fill-drift"],
          "required complete-observer mutation census")
    print(json.dumps({"golden_count": len(reports), "required_ids": sorted(required_ids()), "goldens": reports,
                      "coverage": {key: sorted(values) for key, values in coverage.items()},
                      "negative_count": len(negatives), "rejections": negatives, "unsupported_consumer_controls": unsupported,
                      "complete_observer_mutations": mutations,
                      "limits": "small owned coefficient controls; no IDCT or production execution proof",
                      "consumer_caveat": "Pillow/ImageMagick may share libjpeg"}, sort_keys=True))


if __name__ == "__main__":
    main()
