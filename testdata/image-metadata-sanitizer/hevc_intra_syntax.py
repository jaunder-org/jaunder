# SPDX-License-Identifier: LGPL-3.0-or-later
# H.265 video codec.
# Copyright (c) 2013-2014 struktur AG, Dirk Farin <farin@struktur.de>
# Authors: struktur AG, Dirk Farin <farin@struktur.de>
#          Min Chen <chenm003@163.com>
# Modified/adapted for this proof: Python syntax-only I-slice traversal,
# fixed neighbour syntax state, checked failures, no image/transform API.
# Adapted from libde2651.1.1 slice.cc:1653-1985,2281-2524,2735-3450,
# 3870-4028,4315-4740. Exact source NAR:
# sha256-ZHfPC86oylqt2bwWMJRWVjdMEEmX6UOKR7XkR0HPyok=.
# This is free software: redistribute/modify under GNU LGPL version3
# or (at your option) any later version. WITHOUT ANY WARRANTY; without
# even implied MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See HEVC-LGPL-3.0.txt and repository LICENSE for LGPL/GPL text.
"""Bounded residual/coding syntax only. No samples, dequantization or IDCT.

Main Still Picture I-slice, 64x64 canvas, 4:2:0, no extensions/PCM/tiles.
Memory: 256 prediction-mode cells + 64 coding-depth cells and at most
16 coefficient scalars/64 significance flags per transform; never pixels.
Normative processes: H.265 V9 7.3.8 and 9.3; intra-mode syntax derivation
8.4.2 supplies scan choice only, not intra sample prediction.
"""
import hashlib
import struct
from hevc_headers import require
from hevc_cabac import CABAC


class IntraSyntax:
    def __init__(self, sequence, picture, header):
        self.s, self.p, self.h = sequence, picture, header
        self.c = CABAC(header["data"], header["cabac_start"], header["qp"])
        self.depth = [-1] * 64
        self.modes = [1] * 256
        self.chroma = [1] * 256
        self.delta_coded = False
        self.units = self.transforms = self.coefficients = self.records = 0
        self.ctu_ends, self.unit_shapes, self.residual_shapes = [], [], []
        self.qp_deltas = []
        self.event_hash = hashlib.sha256()
        self.coefficient_hash = hashlib.sha256()

    def record(self, *values):
        self.records += 1
        require(self.records <= 65536, "syntax record budget")
        self.event_hash.update(struct.pack(">" + "i" * len(values), *values))

    def mode_at(self, x, y, chroma=False):
        require(0 <= x < 64 and 0 <= y < 64, "prediction-mode syntax address")
        return (self.chroma if chroma else self.modes)[(y // 4) * 16 + x // 4]

    def set_mode(self, x, y, side, mode, chroma=False):
        require(0 <= mode <= 34 and x + side <= 64 and y + side <= 64, "prediction-mode syntax range")
        target = self.chroma if chroma else self.modes
        for row in range(y // 4, (y + side) // 4):
            for col in range(x // 4, (x + side) // 4):
                target[row * 16 + col] = mode

    def sao(self, x, y):
        if not (self.h["sao_y"] or self.h["sao_c"]):
            return
        merged = self.c.bit("sao_merge") if x else 0
        if y and not merged:
            merged = self.c.bit("sao_merge")
        if merged:
            return
        kind = 0
        for component in range(3):
            if not (self.h["sao_y"] if component == 0 else self.h["sao_c"]):
                continue
            if component < 2:
                kind = (1 + self.c.bypass()) if self.c.bit("sao_type") else 0
            if kind:
                offsets = [self.c.unary(7) for _ in range(4)]
                if kind == 1:
                    for value in offsets:
                        if value:
                            self.c.bypass()
                    self.c.fixed(5)
                elif component < 2:
                    self.c.fixed(2)

    def cu(self, x, y, logsize, depth):
        side = 1 << logsize
        context = 0
        if x:
            context += self.depth[(y // 8) * 8 + (x - 1) // 8] > depth
        if y:
            context += self.depth[((y - 1) // 8) * 8 + x // 8] > depth
        split = self.c.bit("split_cu", context) if logsize > self.s["min_cb"] else 0
        require(split == 0, "CU split outside proved shape")
        if self.p["delta"] and logsize >= self.s["ctb"] - self.p["delta_depth"]:
            self.delta_coded = False
        self.record(1, x, y, logsize, split)
        if split:
            step = side // 2
            for dx, dy in ((0, 0), (step, 0), (0, step), (step, step)):
                self.cu(x + dx, y + dy, logsize - 1, depth + 1)
            return
        self.units += 1
        self.unit_shapes.append((x, y, logsize))
        for row in range(y // 8, (y + side) // 8):
            for col in range(x // 8, (x + side) // 8):
                self.depth[row * 8 + col] = depth
        intra_split = not self.c.bit("part_mode") if logsize == self.s["min_cb"] else False
        block = side // 2 if intra_split else side
        positions = [(x + dx, y + dy) for dy in range(0, side, block) for dx in range(0, side, block)]
        prev = [self.c.bit("prev_intra") for _ in positions]
        for (px, py), flag in zip(positions, prev):
            a = self.mode_at(px - 1, py) if px else 1
            # H.2658.4.2: above-CTU modes unavailable for MPM derivation.
            b = self.mode_at(px, py - 1) if py % (1 << self.s["ctb"]) else 1
            if a == b:
                candidates = [0, 1, 26] if a < 2 else [a, 2 + ((a + 29) % 32), 2 + ((a - 1) % 32)]
            else:
                candidates = [a, b, 0 if a and b else (1 if a != 1 and b != 1 else 26)]
            if flag:
                mode = candidates[self.c.unary(2)]
            else:
                mode = self.c.fixed(5)
                for candidate in sorted(candidates):
                    if mode >= candidate:
                        mode += 1
            self.set_mode(px, py, block, mode)
            self.record(2, px, py, mode)
        if self.c.bit("chroma_mode"):
            index = self.c.fixed(2)
            mode = (0, 26, 10, 1)[index]
            if mode == self.mode_at(x, y):
                mode = 34
        else:
            mode = self.mode_at(x, y)
        self.set_mode(x, y, side, mode, True)
        self.transform_tree(x, y, logsize, 0, 0, self.s["intra_depth"] + int(intra_split),
                intra_split, 1, 1, x, y)

    def qp_delta(self):
        if not self.p["delta"] or self.delta_coded:
            return
        value = 0
        if self.c.bit("qp_delta"):
            value = 1
            while value < 5 and self.c.bit("qp_delta", 1):
                value += 1
            if value == 5:
                value += self.c.eg()
        sign = self.c.bypass() if value else 0
        delta = -value if sign else value
        require(-26 <= delta <= 25, "CU QP delta")
        # H.265 V9 permits -26, but pinned libde2651.1.1 rejects abs26
        # regardless of sign. Its failed consumer proof is not admission.
        require(delta >= -25, "CU QP delta outside native-proved envelope")
        self.delta_coded = True
        self.qp_deltas.append(delta)
        self.record(3, delta)

    def transform_tree(self, x, y, logsize, depth, blockindex, maxdepth, intra_split, cb, cr, bx, by):
        require(2 <= logsize <= 6 and 0 <= depth <= 5, "transform recursion budget")
        if logsize <= self.s["max_tb"] and logsize > 2 and depth < maxdepth and not (intra_split and depth == 0):
            split = self.c.bit("split_tb", 5 - logsize)
        else:
            split = int(logsize > self.s["max_tb"] or (intra_split and depth == 0))
        require(split == 0, "TU split outside proved shape")
        if logsize > 2:
            cb = self.c.bit("cbf_c", depth) if cb else 0
            cr = self.c.bit("cbf_c", depth) if cr else 0
        self.record(4, x, y, logsize, split, cb, cr)
        if split:
            step = 1 << (logsize - 1)
            for index, (dx, dy) in enumerate(((0, 0), (step, 0), (0, step), (step, step))):
                self.transform_tree(x + dx, y + dy, logsize - 1, depth + 1, index, maxdepth,
                        intra_split, cb, cr, x, y)
            return
        cy = self.c.bit("cbf_y", int(depth == 0))
        if cy or cb or cr:
            self.qp_delta()
        if cy:
            self.residual(x, y, logsize, 0)
        if logsize > 2:
            if cb:
                self.residual(x, y, logsize - 1, 1)
            if cr:
                self.residual(x, y, logsize - 1, 2)
        elif blockindex == 3:
            if cb:
                self.residual(bx, by, logsize, 1)
            if cr:
                self.residual(bx, by, logsize, 2)

    def last_prefix(self, logsize, component, name):
        offset = 3 * (logsize - 2) + ((logsize - 1) >> 2) if component == 0 else 15
        shift = (logsize + 1) >> 2 if component == 0 else logsize - 2
        maximum = 2 * logsize - 1
        for value in range(maximum):
            if not self.c.bit(name, offset + (value >> shift)):
                return value
        return maximum

    @staticmethod
    def scan(side, kind):
        require(kind == 0 and side in (2, 4), "diagonal scan outside frozen residual shape")
        return [(x, total - x) for total in range(2 * side - 1)
                for x in range(side) if 0 <= total - x < side]

    def sig_context(self, x, y, logsize, component, kind, neighbours):
        require(component in (0, 1, 2) and logsize == (4 if component == 0 else 3)
                and kind == 0, "significance outside frozen residual shape")
        width = 1 << (logsize - 2)
        if x + y == 0:
            context = 0
        else:
            sx, sy, px, py = x >> 2, y >> 2, x & 3, y & 3
            previous = neighbours[sy * width + sx]
            if previous == 0:
                context = 0 if px + py >= 3 else (1 if px + py else 2)
            elif previous == 1:
                context = 2 if py == 0 else (1 if py == 1 else 0)
            elif previous == 2:
                context = 2 if px == 0 else (1 if px == 1 else 0)
            else:
                context = 2
            if component == 0:
                if sx + sy:
                    context += 3
                context += 21
            else:
                context += 9
        return context + (27 if component else 0)

    def remaining(self, rice):
        require(0 <= rice <= 4, "Rice outside frozen residual precision")
        prefix = 0
        while self.c.bypass():
            prefix += 1
            require(prefix <= 17 - rice, "coefficient remainder prefix")
        if prefix <= 3:
            return (prefix << rice) + self.c.fixed(rice)
        return (((1 << (prefix - 3)) + 2) << rice) + self.c.fixed(prefix - 3 + rice)

    def residual(self, x, y, logsize, component):
        require(component in (0, 1, 2) and logsize == (4 if component == 0 else 3),
                "residual outside frozen component/transform shape")
        self.transforms += 1
        self.residual_shapes.append((x, y, logsize, component))
        xp = self.last_prefix(logsize, component, "last_x")
        yp = self.last_prefix(logsize, component, "last_y")
        def suffix(prefix):
            if prefix <= 3:
                return prefix
            width = (prefix >> 1) - 1
            return ((2 + (prefix & 1)) << width) + self.c.fixed(width)
        lx, ly = suffix(xp), suffix(yp)
        # H.265 V9 7.4.9.11: non-diagonal intra scans require luma TB<=8
        # or chroma TB<=4. No CU/TU splits admit only luma16/chroma8 here.
        kind = 0
        width = 1 << (logsize - 2)
        sub = self.scan(width, kind)
        positions = self.scan(4, kind)
        require(lx < 1 << logsize and ly < 1 << logsize, "last coefficient position")
        last_sub = sub.index((lx >> 2, ly >> 2))
        last_pos = positions.index((lx & 3, ly & 3))
        neighbours, previous_c1 = [0] * (width * width), 1
        for index in range(last_sub, -1, -1):
            sx, sy = sub[index]
            ni = sy * width + sx
            coded = 1
            infer_dc = 0
            if 0 < index < last_sub:
                coded = self.c.bit("coded_sb", int(neighbours[ni] != 0) + (2 if component else 0))
                infer_dc = 1
            if coded:
                if sx:
                    neighbours[ni - 1] |= 1
                if sy:
                    neighbours[ni - width] |= 2
            significant = [last_pos] if index == last_sub else []
            end = last_pos - 1 if index == last_sub else 15
            if coded:
                for pos in range(end, 0, -1):
                    px, py = positions[pos]
                    if self.c.bit("sig", self.sig_context(sx * 4 + px, sy * 4 + py,
                                                          logsize, component, kind, neighbours)):
                        significant.append(pos)
                        infer_dc = 0
                if end >= 0 and (infer_dc or self.c.bit("sig", self.sig_context(sx * 4, sy * 4,
                                                                          logsize, component, kind, neighbours))):
                    significant.append(0)
            if not significant:
                continue
            count = len(significant)
            require(count <= 16, "coefficient group size")
            levels, maximums = [1] * count, [1] * count
            context_set = (2 if index and component == 0 else 0) + int(previous_c1 == 0)
            c1, first_gt1 = 1, -1
            for n in range(min(8, count)):
                flag = self.c.bit("gt1", context_set * 4 + c1 + (16 if component else 0))
                if flag:
                    levels[n] = 2
                    c1 = 0
                    if first_gt1 < 0:
                        first_gt1 = n
                else:
                    maximums[n] = 0
                    if c1:
                        c1 = min(3, c1 + 1)
            previous_c1 = c1
            if first_gt1 >= 0:
                flag = self.c.bit("gt2", context_set + (4 if component else 0))
                levels[first_gt1] += flag
                maximums[first_gt1] = flag
            hidden = self.p["hiding"] and significant[0] - significant[-1] > 3
            signs = [self.c.bypass() for _ in range(count - int(hidden))]
            rice = 0
            for n in range(count):
                if maximums[n]:
                    levels[n] += self.remaining(rice)
                    if levels[n] > 3 * (1 << rice):
                        rice = min(4, rice + 1)
            if hidden:
                signs.append(sum(levels) & 1)
            for pos, level, sign in zip(significant, levels, signs):
                require(level <= (32768 if sign else 32767), "coefficient precision")
                signed = -level if sign else level
                self.record(5, component, logsize, sx, sy, pos, signed)
                px, py = positions[pos]
                self.coefficient_hash.update(struct.pack(">7i", component, x, y, logsize, sx * 4 + px, sy * 4 + py, signed))
                self.coefficients += 1

    def validate(self):
        side = 1 << self.s["ctb"]
        count = (64 // side) ** 2
        for address in range(count):
            cx, cy = address % (64 // side), address // (64 // side)
            self.sao(cx, cy)
            self.cu(cx * side, cy * side, self.s["ctb"], 0)
            end = self.c.term()
            self.ctu_ends.append(self.c.at - self.h["cabac_start"] * 8)
            require(end == int(address == count - 1), "slice CTU termination/coverage")
        report = self.c.finish()
        return {**report, "ctus": count, "coding_units": self.units, "transforms": self.transforms,
                "coefficients": self.coefficients, "syntax_sha256": self.event_hash.hexdigest(),
                "coefficient_sha256": self.coefficient_hash.hexdigest(),
                "ctu_end_bits": self.ctu_ends, "coding_unit_shapes": self.unit_shapes,
                "residual_shapes": self.residual_shapes, "cu_qp_deltas": self.qp_deltas}
