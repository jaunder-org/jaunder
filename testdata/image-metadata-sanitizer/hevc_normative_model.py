#!/usr/bin/env python3
"""Independent TEST model, authored from H.265 V9 9.3.5 (informative).

No imports from the validator or its adapted tables. Numeric facts transcribed
from normative Tables9-52/53; selected I initial values from Tables9-5..9-31.
This writes arithmetic codewords and authored syntax cases, never pixels.
It is not a production encoder or universal syntax/conformance oracle.
"""

LPS_ROWS = """
128 176 208 240
128 167 197 227
128 158 187 216
123 150 178 205
116 142 169 195
111 135 160 185
105 128 152 175
100 122 144 166
95 116 137 158
90 110 130 150
85 104 123 142
81 99 117 135
77 94 111 128
73 89 105 122
69 85 100 116
66 80 95 110
62 76 90 104
59 72 86 99
56 69 81 94
53 65 77 89
51 62 73 85
48 59 69 80
46 56 66 76
43 53 63 72
41 50 59 69
39 48 56 65
37 45 54 62
35 43 51 59
33 41 48 56
32 39 46 53
30 37 43 50
29 35 41 48
27 33 39 45
26 31 37 43
24 30 35 41
23 28 33 39
22 27 32 37
21 26 30 35
20 24 29 33
19 23 27 31
18 22 26 30
17 21 25 28
16 20 23 27
15 19 22 25
14 18 21 24
14 17 20 23
13 16 19 22
12 15 18 21
12 14 17 20
11 14 16 19
11 13 15 18
10 12 15 17
10 12 14 16
9 11 13 15
9 11 12 14
8 10 12 14
8 9 11 13
7 9 11 12
7 9 10 12
7 8 10 11
6 8 9 11
6 7 9 10
6 7 8 9
2 2 2 2
"""
LPS = [tuple(map(int, line.split())) for line in LPS_ROWS.strip().splitlines()]
TRANS_LPS = tuple(map(int, "0 0 1 2 2 4 4 5 6 7 8 9 9 11 11 12 13 13 15 15 16 16 18 18 19 19 21 21 22 22 23 24 24 25 26 26 27 27 28 29 29 30 30 30 31 32 32 33 33 33 34 34 35 35 35 36 36 36 37 37 37 38 38 63".split()))
INIT = {"split_cu": (139, 141, 157), "prev_intra": (184,), "chroma_mode": (63,),
        "cbf_y": (111, 141), "cbf_c": (94, 138, 182, 154), "split_tb": (153, 138, 138),
        "sao_merge": (153,), "sao_type": (200,), "last_x": (110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63),
        "last_y": (110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63),
        "gt1": (140, 92, 137, 138, 140, 152, 138, 139, 153, 74, 149, 92, 139, 107, 122, 152, 140, 179, 166, 182, 140, 227, 122, 197),
        "gt2": (138, 153, 136, 167, 152, 152), "qp_delta": (154, 154),
        "sig": (111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125)}


class Encoder:
    def __init__(self, qp=22):
        self.low, self.range, self.outstanding, self.first = 0, 510, 0, True
        self.output, self.events = [], []
        self.contexts = {}
        for name, values in INIT.items():
            states = []
            for value in values:
                m = 5 * (value // 16) - 45
                n = 8 * (value % 16) - 16
                pre = max(1, min(126, (m * qp // 16) + n))
                states.append([63 - pre if pre <= 63 else pre - 64, int(pre > 63)])
            self.contexts[name] = states

    def put(self, value):
        if self.first:
            self.first = False
        else:
            self.output.append(value)
        self.output.extend([1 - value] * self.outstanding)
        self.outstanding = 0

    def normalize(self):
        while self.range < 256:
            if self.low < 256:
                self.put(0)
            elif self.low >= 512:
                self.low -= 512
                self.put(1)
            else:
                self.low -= 256
                self.outstanding += 1
            self.low *= 2
            self.range *= 2

    def bit(self, name, index, value):
        self.events.append(("bit", name, index, value))
        state, mps = self.contexts[name][index]
        lps = LPS[state][(self.range >> 6) % 4]
        self.range -= lps
        if value != mps:
            self.low += self.range
            self.range = lps
            self.contexts[name][index] = [TRANS_LPS[state], mps ^ int(state == 0)]
        else:
            self.contexts[name][index][0] = min(62, state + 1) if state != 63 else 63
        self.normalize()

    def bypass(self, value):
        self.events.append(("bypass", value))
        self.low = 2 * self.low + value * self.range
        if self.low >= 1024:
            self.low -= 1024
            self.put(1)
        elif self.low < 512:
            self.put(0)
        else:
            self.low -= 512
            self.outstanding += 1

    def term(self, value):
        self.events.append(("term", value))
        self.range -= 2
        if value:
            self.low += self.range
            self.range = 2
        self.normalize()
        if value:
            self.put((self.low >> 9) & 1)
            # Figure9-15 writes TWO bits ((ivlLow >> 7) & 3) | 1.
            self.output.extend([(self.low >> 8) & 1, 1])

    def finish(self):
        bits = self.output + [0] * ((-len(self.output)) % 8)
        return bytes(sum(bit << (7 - j) for j, bit in enumerate(bits[i:i + 8]))
                     for i in range(0, len(bits), 8))


def escape(data):
    result, zeros = bytearray(), 0
    for byte in data:
        if zeros == 2 and byte < 4:
            result.append(3)
            zeros = 0
        result.append(byte)
        zeros = zeros + 1 if byte == 0 else 0
    return bytes(result)


def flat_i_slice(dc=False, level=1, negative=False, hide=False, qp_delta=0, sao=0,
                 residual_maps=None, coverage=None):
    """Authored sixteen-CTU16 I-picture: no CU/TU splits; optional luma DC1.

    SAO flags are present in the actual slice header but all CTU SAO types
    disabled. All intra modes use the first MPM (DC for this construction).
    Optional single DC coefficient per CTU independently exercises residual
    syntax without importing the grammar reader's traversal/scan logic.
    """
    encoder = Encoder()
    for address in range(16):
        x, y = address % 4, address // 4
        if x:
            encoder.bit("sao_merge", 0, 0)
        if y:
            encoder.bit("sao_merge", 0, 0)
        for component in range(3):
            if component < 2:
                encoder.bit("sao_type", 0, int(sao > 0))
                if sao:
                    encoder.bypass(sao - 1)
            if sao:
                # Boundary absolute offset7 (truncated unary); BO signs1 and
                # band31, or EO class3 shared for chroma. Normative7.3.8.3.
                for _ in range(4 * 7):
                    encoder.bypass(1)
                if sao == 1:
                    for _ in range(4):
                        encoder.bypass(1)
                    for _ in range(5):
                        encoder.bypass(1)
                elif component < 2:
                    encoder.bypass(1)
                    encoder.bypass(1)
        encoder.bit("split_cu", 0, 0)
        encoder.bit("prev_intra", 0, 1)
        encoder.bypass(0)
        encoder.bit("chroma_mode", 0, 0)
        encoder.bit("split_tb", 1, 0)
        if residual_maps is not None:
            from hevc_residual_vectors import write_residual
            chosen = residual_maps if address == 0 else {}
            encoder.bit("cbf_c", 0, int(bool(chosen.get(1))))
            encoder.bit("cbf_c", 0, int(bool(chosen.get(2))))
            encoder.bit("cbf_y", 1, int(bool(chosen.get(0))))
            if any(chosen.values()):
                encoder.bit("qp_delta", 0, 0)
            for component in range(3):
                if chosen.get(component):
                    write_residual(encoder, component, chosen[component], coverage)
            encoder.term(int(address == 15))
            continue
        encoder.bit("cbf_c", 0, 0)
        encoder.bit("cbf_c", 0, 0)
        encoder.bit("cbf_y", 1, int(dc))
        if dc:
            absolute = abs(qp_delta)
            encoder.bit("qp_delta", 0, int(absolute > 0))
            if absolute:
                for _ in range(min(absolute, 5) - 1):
                    encoder.bit("qp_delta", 1, 1)
                if absolute < 5:
                    encoder.bit("qp_delta", 1, 0)
                else:
                    binary = format(absolute - 5 + 1, "b")
                    for _ in range(len(binary) - 1):
                        encoder.bypass(1)
                    encoder.bypass(0)
                    for bit in binary[1:]:
                        encoder.bypass(int(bit))
                encoder.bypass(int(qp_delta < 0))
            for name in ("last_x", "last_y"):
                for prefix in range(3 if hide else 0):
                    encoder.bit(name, 6 + prefix // 2, 1)
                encoder.bit(name, 7 if hide else 6, 0)
            if hide:
                # Last=(3,3), fifteen preceding 4x4 diagonal positions;
                # only DC also significant, gap15 activates sign hiding.
                diagonal = [(x, total - x) for total in range(7)
                            for x in range(4) if 0 <= total - x < 4]
                for x, y in reversed(diagonal[1:15]):
                    encoder.bit("sig", 21 + int(x + y < 3), 0)
                encoder.bit("sig", 0, 1)
            encoder.bit("gt1", 1, int(level > 1))
            if hide:
                assert level == 1
                encoder.bit("gt1", 2, 0)
            elif level > 1:
                encoder.bit("gt2", 0, int(level > 2))
            encoder.bypass(int(negative))
            if level > 2:
                remaining = level - 3
                prefix = remaining if remaining <= 3 else (remaining - 2).bit_length() + 2
                for _ in range(prefix):
                    encoder.bypass(1)
                encoder.bypass(0)
                if prefix > 3:
                    width = prefix - 3
                    suffix = remaining - (1 << width) - 2
                    for bit in range(width - 1, -1, -1):
                        encoder.bypass((suffix >> bit) & 1)
        encoder.term(int(address == 15))
    # Independently authored header bits: first1/prior0/PPS0/I2/SAO1,1/
    # signedQPdelta-4/loopAcross1/alignment1. No observed-payload/hash oracle.
    bits = "1" + "0" + "1" + "011" + "1" + "1" + "0001001" + "1" + "1"
    bits += "0" * (-len(bits) % 8)
    header = bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8))
    return b"\x28\x01" + escape(header + encoder.finish()), encoder
