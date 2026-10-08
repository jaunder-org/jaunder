#!/usr/bin/env python3
"""Executable syntax/ownership controls with independent normative/native axes.

The native decoder is TEST-SIDE ONLY. Its source/prefetch counters are not the
normative oracle: independently authored arithmetic encoder cases also run.
CLI rejection must be typed, leave no output and preserve the adapter input.
"""
import hashlib
import json
import re
import struct
import subprocess
import sys
from pathlib import Path
import hevc_headers as headers
from hevc_cabac import CABAC
from hevc_normative_model import Encoder, LPS, TRANS_LPS, flat_i_slice, escape
from validate_hevc import validate_nals
from hevc_intra_syntax import IntraSyntax
from hevc_residual_vectors import build_vectors, assert_coverage, required_coverage
from verify_heif_coded_boundary import inspect, PRIVATE_SUFFIX

REQUIRED_POSITIVE = {"owned-original", "normative-zero-residual", "normative-dc-residual",
    "normative-level2", "normative-level3", "normative-level7", "normative-level8", "normative-level17",
    "normative-negative-level17", "normative-hidden-sign", "normative-SAO-BO", "normative-SAO-EO"} | {
    "normative-QP%d" % delta for delta in range(-25, 26)}
REQUIRED_NEGATIVE = {"accepted-private-suffix", "different-private-suffix", "VPS-extension",
                     "SPS-extension", "PPS-extension", "slice-header-extension", "unknown-SEI",
                     "invalid-emulation-prevention", "unowned-parameter-suffix",
    "CTB-size", "minCB-size", "minTB-size", "maxTB-size", "inter-depth", "intra-depth", "SAO-enable",
    "output-flag", "sign-hiding", "PPS-QP", "constrained-intra", "transform-skip", "CU-delta",
    "CU-delta-depth", "chroma-offset-header", "deblocking-header", "slice-SAO-Y", "slice-SAO-C", "slice-QP",
    "missing-terminal-stop", "nonzero-alignment", "CU-split", "TU-split",
    "CU-QP-minus26-unsupported", "CU-QP-minus27-malformed", "CU-QP-plus26-malformed",
    "VPS-level", "SPS-level", "VPS-compatibility", "SPS-compatibility",
    "coefficient-positive-overflow", "coefficient-negative-overflow", "coefficient-prefix-overflow"}


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def invoke(command, directory, label):
    result = subprocess.run(command, capture_output=True, timeout=60)
    (directory / (label + ".out")).write_bytes(result.stdout)
    (directory / (label + ".err")).write_bytes(result.stderr)
    return result


def adapter(nals):
    return dict(zip(("vps", "sps", "pps", "coded"), (n.hex() for n in nals)))


def replace_bit(nal, kind, bit, value):
    data = bytearray(headers.rbsp(nal, kind))
    mask = 1 << (7 - bit % 8)
    data[bit // 8] = (data[bit // 8] | mask) if value else (data[bit // 8] & ~mask)
    return nal[:2] + escape(data)


def replace_field(nal, kind, record, field, value, encoding):
    raw = headers.rbsp(nal, kind)
    bits = "".join(format(byte, "08b") for byte in raw)
    location = record["fields"][field]
    if encoding == "bit":
        replacement = str(value)
    elif encoding == "u":
        replacement = format(value, "0%db" % location["width"])
    else:
        code = value if encoding == "ue" else (2 * value - 1 if value > 0 else -2 * value)
        binary = format(code + 1, "b")
        replacement = "0" * (len(binary) - 1) + binary
    alignment = "byte_alignment.one" if kind == 20 else "rbsp_trailing_bits.one"
    stop = record["fields"][alignment]["bit"]
    after = location["bit"] + location["width"]
    prefix = bits[:location["bit"]] + replacement + bits[after:stop] + "1"
    prefix += "0" * (-len(prefix) % 8)
    new = bytes(int(prefix[i:i + 8], 2) for i in range(0, len(prefix), 8))
    if kind == 20:
        new += raw[record["cabac_start"]:]
    return nal[:2] + escape(new)


def arithmetic_models():
    """Cross engine branches at all states, qRange classes, MPS polarities.

    The encoder model has separately transcribed normative tables; initial
    states are independently specified, not copied from validator contexts.
    Encoding/decoding operate in opposite directions/different representations.
    """
    checks = 0
    quantized_ranges = set()
    for case in range(63 * 2 * 2 * 4):
                state, remainder = divmod(case, 16)
                mps, remainder = divmod(remainder, 8)
                value, prefix_length = divmod(remainder, 4)
                e = Encoder()
                for _ in range(prefix_length):
                    e.bit("gt1", 1, 0)
                quantized_ranges.add((e.range >> 6) % 4)
                e.contexts["split_cu"][0] = [state, mps]
                e.bit("split_cu", 0, value)
                for bit in (0, 1, 1, 0, 1, 0):
                    e.bypass(bit)
                e.term(0)
                e.term(1)
                data = e.finish()
                c = CABAC(data, 0, 22)
                for _ in range(prefix_length):
                    require(c.bit("gt1", 1) == 0, "normative quantized-range prefix")
                c.contexts["split_cu"][0] = [state, mps]
                require(c.bit("split_cu") == value, "normative arithmetic decision")
                for bit in (0, 1, 1, 0, 1, 0):
                    require(c.bypass() == bit, "normative bypass decision")
                require(c.term() == 0 and c.term() == 1, "normative termination")
                c.finish()
                checks += 1
    require(quantized_ranges == {0, 1, 2, 3}, "normative quantized-range coverage")
    # Numeric table facts are independent from the constants under test.
    from hevc_cabac_tables import LPS as actual_lps, NEXT_LPS, NEXT_MPS
    require(tuple(LPS) == actual_lps and TRANS_LPS == NEXT_LPS, "normative numeric tables")
    require(NEXT_MPS == tuple(min(62, n + 1) if n != 63 else 63 for n in range(64)),
            "normative MPS transitions")
    return checks


def wire_matches(actual, report):
    return json.loads(json.dumps(actual)) == report


def wire_regression():
    actual = {"shapes": [(0, 0, 4)], "nested": {"coordinates": ((1, 2),)}, "count": 1}
    wire = {"shapes": [[0, 0, 4]], "nested": {"coordinates": [[1, 2]]}, "count": 1}
    require(wire_matches(actual, wire), "tuple/list wire equivalence")
    for changed in ({**wire, "shapes": [[0, 0, 3]]}, {**wire, "count": 2},
                    {**wire, "count": "1"}, {**wire, "extra": 0}):
        require(not wire_matches(actual, changed), "wire normalization hid semantic mismatch")


def check_residual_map(report, native, expected):
    expected_hash = hashlib.sha256(b"".join(struct.pack(">7i", *row) for row in expected)).hexdigest()
    require(native == expected, "independent residual coordinate/value/sign map")
    require(report["coefficient_sha256"] == expected_hash and report["coefficients"] == len(expected),
            "actual validator independent residual map")
    require(report["transforms"] == len({row[0] for row in expected}), "independent component census")


def actual_syntax_tokens(nals):
    """Observe the REAL callable, without using it to generate expected tokens."""
    methods = {name: getattr(CABAC, name) for name in ("bit", "bypass", "term")}
    tokens = []
    def bit(self, name, index=0):
        value = methods["bit"](self, name, index)
        tokens.append(("bit", name, int(index), value))
        return value
    def bypass(self):
        value = methods["bypass"](self)
        tokens.append(("bypass", value))
        return value
    def term(self):
        value = methods["term"](self)
        tokens.append(("term", value))
        return value
    CABAC.bit, CABAC.bypass, CABAC.term = bit, bypass, term
    try:
        return validate_nals(*nals), tokens
    finally:
        for name, method in methods.items():
            setattr(CABAC, name, method)


def residual_sensitivity(params, cases):
    """Real validator mutations and SAME independent semantic/token assertions."""
    detected = {}
    case = cases["residual-count-c0-n16-level3"]
    nals = params + [case["nal"]]
    def wrong_last(old):
        return lambda self, size, component, name: (old(self, size, component, name) + 1) % (2 * size)
    def wrong_context(old):
        return lambda self, *args: (old(self, *args) + 1) % 42
    def wrong_remaining(old):
        return lambda self, rice: old(self, rice) + 1
    def omitted_residual(old):
        return lambda self, *args: None
    for name, method, mutation in (("last-position", "last_prefix", wrong_last),
            ("significance-context", "sig_context", wrong_context),
            ("Rice-level", "remaining", wrong_remaining),
            ("omitted-residual", "residual", omitted_residual)):
        original = getattr(IntraSyntax, method)
        setattr(IntraSyntax, method, mutation(original))
        try:
            try:
                report, tokens = actual_syntax_tokens(nals)
                check_residual_map(report, case["expected"], case["expected"])
                require(tokens == case["events"], "altered real residual consumption")
            except (headers.HEVCDomainError, AssertionError) as error:
                detected[name] = str(error)
            require(name in detected, "residual sensitivity remained green: " + name)
        finally:
            setattr(IntraSyntax, method, original)
    original_bit = CABAC.bit
    def wrong_greater_state(self, name, index=0):
        if name == "gt1":
            self.contexts[name][index][1] ^= 1
        return original_bit(self, name, index)
    CABAC.bit = wrong_greater_state
    try:
        try:
            report, tokens = actual_syntax_tokens(nals)
            check_residual_map(report, case["expected"], case["expected"])
            require(tokens == case["events"], "altered greater1 state consumption")
        except (headers.HEVCDomainError, AssertionError) as error:
            detected["greater1-state"] = str(error)
        require("greater1-state" in detected, "greater1 state mutation remained green")
    finally:
        CABAC.bit = original_bit
    report = validate_nals(*nals)
    for name, expected in (("bad-independent-oracle", [(*case["expected"][0][:-1], 17)] + case["expected"][1:]),):
        try:
            check_residual_map(report, case["expected"], expected)
        except AssertionError as error:
            detected[name] = str(error)
        require(name in detected, "bad independent oracle remained green")
    for suffix in ("last-c1-x7-y7", "rice-c2-k4-p13-low"):
        omitted = dict(cases)
        del omitted["residual-" + suffix]
        try:
            assert_coverage(omitted)
        except AssertionError:
            detected["omitted-required-vector-" + suffix] = "independent required name/map census"
        require("omitted-required-vector-" + suffix in detected, "missing vector remained green")
    # Guard controls call actual entry points BEFORE any scalar/bin consumption.
    video, sequence = headers.vps(params[0]), None
    sequence = headers.sps(params[1], video)
    picture = headers.pps(params[2], sequence)
    header = headers.slice_header(case["nal"], sequence, picture)
    for name, callback in (("horizontal-unreachable", lambda p: p.scan(4, 1)),
            ("vertical-unreachable", lambda p: p.scan(4, 2)),
            ("luma8-unreachable", lambda p: p.residual(0, 0, 3, 0)),
            ("chroma4-unreachable", lambda p: p.residual(0, 0, 2, 1)),
            ("other-component-unreachable", lambda p: p.residual(0, 0, 3, 3)),
            ("Rice5-unreachable", lambda p: p.remaining(5))):
        parser = IntraSyntax(sequence, picture, header)
        before = parser.c.at, parser.c.bins, parser.transforms
        try:
            callback(parser)
        except headers.HEVCDomainError as error:
            detected[name] = str(error)
        require(name in detected and before == (parser.c.at, parser.c.bins, parser.transforms),
                "unreachable branch not guarded before consumption")
    return detected


def main():
    wire_regression()
    fixtures, output, native, validator = map(Path, sys.argv[1:5])
    output.mkdir(parents=True, exist_ok=False)
    data = (fixtures / "input/device-like.heic").read_bytes()
    observed = inspect(data)
    params = [bytes.fromhex(n["hex"]) for n in observed["configuration_nals"]]
    start, length = observed["extents"][observed["primary"]]
    coded = data[start + 4:start + length]
    positive = {"owned-original": params + [coded],
                "normative-zero-residual": params + [flat_i_slice(False)[0]],
                "normative-dc-residual": params + [flat_i_slice(True)[0]]}
    for level in (2, 3, 7, 8, 17):
        positive["normative-level%d" % level] = params + [flat_i_slice(True, level)[0]]
    positive["normative-negative-level17"] = params + [flat_i_slice(True, 17, True)[0]]
    positive["normative-hidden-sign"] = params + [flat_i_slice(True, hide=True)[0]]
    for delta in range(-25, 26):
        positive["normative-QP%d" % delta] = params + [flat_i_slice(True, qp_delta=delta)[0]]
    positive["normative-SAO-BO"] = params + [flat_i_slice(True, sao=1)[0]]
    positive["normative-SAO-EO"] = params + [flat_i_slice(True, sao=2)[0]]
    require(set(positive) == REQUIRED_POSITIVE, "required positive census")
    residual_cases = build_vectors()
    residual_coverage = assert_coverage(residual_cases)
    for name, case in residual_cases.items():
        positive[name] = params + [case["nal"]]
    (output / "independent-residual-matrix.json").write_text(json.dumps({
        "primary": "H.265 V9 7.3.8.11;7.4.9.11;9.3.3.11;9.3.4.2.3-7;Tables9-28/29",
        "required_coverage": sorted(required_coverage()), "coverage": residual_coverage,
        "cases": {n: {"expected": c["expected"], "coverage": c["coverage"],
                      "syntax_tokens": c["events"]} for n, c in residual_cases.items()}}, indent=2) + "\n")
    reports = {}
    for name, nals in positive.items():
        source, destination = output / (name + ".json"), output / (name + ".validated.json")
        source.write_text(json.dumps(adapter(nals)))
        before = source.read_bytes()
        result = invoke([sys.executable, "-B", str(validator), str(source), str(destination)], output, name + ".validator")
        require(result.returncode == 0 and not result.stdout and not result.stderr,
                name + " validator infrastructure/domain failure: " + result.stderr.decode())
        require(source.read_bytes() == before, "validator changed input")
        report = json.loads(destination.read_text())
        stream = output / (name + ".h265")
        stream.write_bytes(b"".join(b"\0\0\0\1" + nal for nal in nals))
        native_argv = [str(native), "-q", str(stream)]
        require(not any(arg in ("-t", "--threads") for arg in native_argv), "default serial invocation regression")
        result = invoke(native_argv, output, name + ".native")
        require(result.returncode == 0, name + " native infrastructure/decode failure")
        text, diagnostics = result.stdout.decode(), result.stderr.decode()
        require(not re.search(r"warning|error|invalid", text + diagnostics, re.I),
                name + " native diagnostics: " + diagnostics)
        require("nFrames decoded: 1 (64x64" in diagnostics, name + " native displayed census")
        native_ctus = [tuple(map(int, m)) for m in re.findall(r"HEVC_CTU (\d+) (\d+) (\d+) (\d+)", text)]
        native_units = [tuple(map(int, m)) for m in re.findall(r"HEVC_CU (\d+) (\d+) (\d+)", text)]
        native_residuals = [tuple(map(int, m)) for m in re.findall(r"HEVC_RESIDUAL (\d+) (\d+) (\d+) (\d+)", text)]
        native_coefficients = [tuple(map(int, m)) for m in re.findall(r"HEVC_COEFF (\d+) (\d+) (\d+) (\d+) (\d+) (\d+) (-?\d+)", text)]
        native_qp = [tuple(map(int, m)) for m in re.findall(r"HEVC_QP (\d+) (\d+) (\d+) (\d+) (-?\d+)", text)]
        require([entry[-1] for entry in native_qp] == report["cu_qp_deltas"], "native signed CU-QP census")
        coefficient_hash = hashlib.sha256(b"".join(struct.pack(">7i", *entry) for entry in native_coefficients)).hexdigest()
        require(coefficient_hash == report["coefficient_sha256"] and len(native_coefficients) == report["coefficients"],
                name + " native coefficient scalar/position census")
        require([a for a, _, _, _ in native_ctus] == list(range(16)), "native CTU address coverage")
        require([v for _, v, _, _ in native_ctus] == [0] * 15 + [1], "native termination census")
        require([bits for _, _, bits, _ in native_ctus] == report["ctu_end_bits"],
                name + " semantic bit boundary mismatch")
        require(native_units == list(map(tuple, report["coding_unit_shapes"]))
                and native_residuals == list(map(tuple, report["residual_shapes"])),
                name + " native syntax shape mismatch")
        require(native_ctus[-1][2] + report["cabac_start"] * 8 <= report["consumed_bits"],
                "prefetch/semantic ownership accounting")
        if name in residual_cases:
            case = residual_cases[name]
            check_residual_map(report, native_coefficients, case["expected"])
            actual, tokens = actual_syntax_tokens(nals)
            require(wire_matches(actual, report) and tokens == case["events"], name + " independent syntax-token/context/state consumption")
        if name == "normative-zero-residual":
            require(report["coefficients"] == 0 and report["transforms"] == 0, "authored zero residual")
        if name.startswith("normative-") and name != "normative-zero-residual":
            hidden = name == "normative-hidden-sign"
            level = -17 if name == "normative-negative-level17" else (
                int(name.removeprefix("normative-level")) if name.startswith("normative-level") else 1)
            expected = []
            for address in range(16):
                x, y = (address % 4) * 16, (address // 4) * 16
                if hidden:
                    expected.append((0, x, y, 4, 3, 3, 1))
                expected.append((0, x, y, 4, 0, 0, level))
            require(native_coefficients == expected and report["transforms"] == 16,
                    "independently authored coefficient value/position/sign cases")
            delta = int(name.removeprefix("normative-QP")) if name.startswith("normative-QP") else 0
            require(report["cu_qp_deltas"] == [delta] * 16, "independently authored signed CU-QP boundary cases")
        reports[name] = {**report, "native_ctus": native_ctus, "native_exit": result.returncode,
                         "native_diagnostics": diagnostics, "native_argv": native_argv, "coded_sha256": hashlib.sha256(nals[-1]).hexdigest()}
    video = headers.vps(params[0])
    sequence = headers.sps(params[1], video)
    picture = headers.pps(params[2], sequence)
    negatives = {}
    for name, index, kind, record, field in (
            ("VPS-extension", 0, 32, video, "vps_extension_flag"),
            ("SPS-extension", 1, 33, sequence, "sps_extension_present_flag"),
            ("PPS-extension", 2, 34, picture, "pps_extension_present_flag"),
            ("slice-header-extension", 2, 34, picture, "slice_segment_header_extension_present_flag")):
        nals = params + [coded]
        nals[index] = replace_bit(nals[index], kind, record["fields"][field]["bit"], 1)
        negatives[name] = (nals, field)
    negatives["accepted-private-suffix"] = (params + [coded + PRIVATE_SUFFIX], "CABAC trailing/unowned bytes")
    negatives["different-private-suffix"] = (params + [coded + b"different unowned description\x80"], "CABAC trailing/unowned bytes")
    negatives["unknown-SEI"] = (params + [b"\x4e\x01" + coded[2:]], "NAL type/layer/temporal header")
    negatives["invalid-emulation-prevention"] = ([params[0] + b"\0\0\3\x7f"] + params[1:] + [coded], "emulation prevention suffix")
    negatives["unowned-parameter-suffix"] = ([params[0] + b"\x80"] + params[1:] + [coded], "parameter trailing bytes")
    slice_record = headers.slice_header(coded, sequence, picture)
    controls = (
        ("CTB-size", 1, 33, sequence, "log2_diff_max_min_luma_coding_block_size", 2, "ue", "CTB16/minCB8/minTB4/maxTB16"),
        ("minCB-size", 1, 33, sequence, "log2_min_luma_coding_block_size_minus3", 1, "ue", "CTB16/minCB8/minTB4/maxTB16"),
        ("minTB-size", 1, 33, sequence, "log2_min_luma_transform_block_size_minus2", 1, "ue", "CTB16/minCB8/minTB4/maxTB16"),
        ("maxTB-size", 1, 33, sequence, "log2_diff_max_min_luma_transform_block_size", 1, "ue", "CTB16/minCB8/minTB4/maxTB16"),
        ("inter-depth", 1, 33, sequence, "max_transform_hierarchy_depth_inter", 1, "ue", "inter transform depth0"),
        ("intra-depth", 1, 33, sequence, "max_transform_hierarchy_depth_intra", 0, "ue", "intra transform depth1"),
        ("SAO-enable", 1, 33, sequence, "sample_adaptive_offset_enabled_flag", 0, "bit", "sample_adaptive_offset_enabled_flag"),
        ("output-flag", 2, 34, picture, "output_flag_present_flag", 1, "bit", "output_flag_present_flag"),
        ("sign-hiding", 2, 34, picture, "sign_data_hiding_enabled_flag", 0, "bit", "sign_data_hiding_enabled_flag"),
        ("PPS-QP", 2, 34, picture, "init_qp_minus26", -1, "se", "PPS init QP26"),
        ("constrained-intra", 2, 34, picture, "constrained_intra_pred_flag", 1, "bit", "constrained_intra_pred_flag"),
        ("transform-skip", 2, 34, picture, "transform_skip_enabled_flag", 1, "bit", "transform_skip_enabled_flag"),
        ("CU-delta", 2, 34, picture, "cu_qp_delta_enabled_flag", 0, "bit", "cu_qp_delta_enabled_flag"),
        ("CU-delta-depth", 2, 34, picture, "diff_cu_qp_delta_depth", 1, "ue", "diff_cu_qp_delta_depth"),
        ("chroma-offset-header", 2, 34, picture, "pps_slice_chroma_qp_offsets_present_flag", 1, "bit", "pps_slice_chroma_qp_offsets_present_flag"),
        ("deblocking-header", 2, 34, picture, "deblocking_filter_control_present_flag", 1, "bit", "deblocking_filter_control_present_flag"),
        ("slice-SAO-Y", 3, 20, slice_record, "slice_sao_luma_flag", 0, "bit", "slice_sao_luma_flag"),
        ("slice-SAO-C", 3, 20, slice_record, "slice_sao_chroma_flag", 0, "bit", "slice_sao_chroma_flag"),
        ("slice-QP", 3, 20, slice_record, "slice_qp_delta", -3, "se", "SliceQpY22"))
    for name, index, kind, record, field, value, encoding, diagnostic in controls:
        nals = params + [coded]
        nals[index] = replace_field(nals[index], kind, record, field, value, encoding)
        negatives[name] = (nals, diagnostic)
    zero, encoder = flat_i_slice(False)
    encoder.output[-1] = 0
    negatives["missing-terminal-stop"] = (params + [zero[:2] + escape(headers.rbsp(zero, 20)[:3] + encoder.finish())], "CABAC terminal stop bit")
    for level in (1, 2, 3, 7, 8, 17):
        nal, encoder = flat_i_slice(True, level)
        if len(encoder.output) % 8:
            raw = bytearray(encoder.finish())
            bit = len(encoder.output)
            raw[bit // 8] |= 1 << (7 - bit % 8)
            negatives["nonzero-alignment"] = (params + [nal[:2] + escape(headers.rbsp(nal, 20)[:3] + raw)], "CABAC trailing alignment")
            break
    for name, index, kind, record, field, value, diagnostic in (
            ("VPS-level", 0, 32, video, "level_idc", 60, "level_idc"),
            ("SPS-level", 1, 33, sequence, "level_idc", 60, "level_idc"),
            ("VPS-compatibility", 0, 32, video, "compatibility", 0x30000000, "profile compatibility"),
            ("SPS-compatibility", 1, 33, sequence, "compatibility", 0x30000000, "profile compatibility")):
        nals = params + [coded]
        nals[index] = replace_field(nals[index], kind, record, field, value, "u")
        negatives[name] = (nals, diagnostic)
    for name, delta, diagnostic in (
            ("CU-QP-minus26-unsupported", -26, "CU QP delta outside native-proved envelope"),
            ("CU-QP-minus27-malformed", -27, "CU QP delta"),
            ("CU-QP-plus26-malformed", 26, "CU QP delta")):
        negatives[name] = (params + [flat_i_slice(True, qp_delta=delta)[0]], diagnostic)
    for name, syntax in (("CU-split", "split_cu"), ("TU-split", "split_tb")):
        nal, original_encoder = flat_i_slice(False)
        encoder, changed = Encoder(), False
        for event in original_encoder.events:
            if event[0] == "bit":
                _, context, index, value = event
                if context == syntax and not changed:
                    value, changed = 1, True
                encoder.bit(context, index, value)
            elif event[0] == "bypass":
                encoder.bypass(event[1])
            else:
                encoder.term(event[1])
        require(changed, "actual split-gate mutation")
        negatives[name] = (params + [nal[:2] + escape(headers.rbsp(nal, 20)[:3] + encoder.finish())],
                           ("CU" if syntax == "split_cu" else "TU") + " split outside proved shape")
    for name, level, detail in (("coefficient-positive-overflow", 32768, "coefficient precision"),
            ("coefficient-negative-overflow", -32769, "coefficient precision"),
            ("coefficient-prefix-overflow", 65538, "coefficient remainder prefix")):
        nal, _ = flat_i_slice(residual_maps={0: {(0, 0): level}})
        negatives[name] = (params + [nal], detail)
    require(set(negatives) == REQUIRED_NEGATIVE, "required negative census")
    rejected = {}
    for name, (nals, detail) in negatives.items():
        source, destination = output / (name + ".json"), output / (name + ".rejected.json")
        source.write_text(json.dumps(adapter(nals)))
        before = source.read_bytes()
        result = invoke([sys.executable, "-B", str(validator), str(source), str(destination)], output, name + ".validator")
        expected = "HEVC_DOMAIN: invalid/unsupported HEVC " + detail + "\n"
        require(result.returncode == 2 and result.stderr.decode() == expected and not result.stdout,
                name + " rejection specificity/infrastructure mismatch: " + result.stderr.decode())
        require(not destination.exists() and source.read_bytes() == before, "rejection output/input mutation")
        rejected[name] = {"exit": 2, "diagnostic": expected.strip(), "no_output": True, "input_unchanged": True}
    arithmetic = arithmetic_models()
    sensitivity = residual_sensitivity(params, residual_cases)
    report = {"positive": reports, "negative": rejected, "normative_arithmetic_checks": arithmetic,
              "independent_residual_cases": len(residual_cases), "residual_branch_coverage": residual_coverage,
              "residual_sensitivity": sensitivity,
              "original_unchanged": (fixtures / "input/device-like.heic").read_bytes() == data,
              "scope": "64x64 Main Still Picture syntax-only; not HEIF rewrite/general conformance",
              "native_caveat": "pinned libde265 test-side reconstruction and shared adapted syntax family; independent normative model also required"}
    (output / "ownership.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"positive": len(reports), "negative": len(rejected), "arithmetic": arithmetic,
                      "report": str(output / "ownership.json")}, sort_keys=True))


if __name__ == "__main__":
    main()
