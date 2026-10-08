#!/usr/bin/env python3
"""Bit-at-a-time arithmetic syntax reader, H.265 V9 9.3.4.3.

No byte prefetch: at counts the normative bits inserted into ivlOffset.
After DecodeTerminate=1, the LAST inserted bit is rbsp_stop_one_bit
(9.3.4.3.5); only alignment zeroes and proven zero-word syntax may follow.
No pixel reconstruction or entropy repair. Context constants are separately
attributed libde265 derivatives; this arithmetic reader is authored code.
"""
from hevc_headers import require
from hevc_cabac_tables import INIT_I, LPS, NEXT_LPS, NEXT_MPS

MAX_BINS = 262144


class CABAC:
    def __init__(self, data, start, qp):
        self.data, self.at, self.bins = data, start * 8, 0
        self.range, self.offset, self.last = 510, 0, 0
        self.contexts = {}
        for name, values in INIT_I.items():
            result = []
            for value in values:
                state = max(1, min(126, (((value >> 4) * 5 - 45) * qp >> 4)
                                   + ((value & 15) * 8 - 16)))
                result.append([state - 64 if state >= 64 else 63 - state, int(state >= 64)])
            self.contexts[name] = result
        for _ in range(9):
            self.offset = (self.offset << 1) | self.read_bit()
        require(self.offset < 510, "CABAC initial offset")

    def read_bit(self):
        require(self.at < len(self.data) * 8, "CABAC physical truncation")
        self.last = (self.data[self.at >> 3] >> (7 - (self.at & 7))) & 1
        self.at += 1
        return self.last

    def event(self):
        self.bins += 1
        require(self.bins <= MAX_BINS, "CABAC bin budget")

    def renorm(self):
        while self.range < 256:
            self.range <<= 1
            self.offset = (self.offset << 1) | self.read_bit()
        require(self.offset < self.range, "CABAC arithmetic state")

    def bit(self, name, index=0):
        self.event()
        require(name in self.contexts and 0 <= index < len(self.contexts[name]), "CABAC context")
        model = self.contexts[name][index]
        state, mps = model
        lps = LPS[state][(self.range >> 6) & 3]
        self.range -= lps
        if self.offset < self.range:
            value = mps
            model[0] = NEXT_MPS[state]
        else:
            value = 1 - mps
            self.offset -= self.range
            self.range = lps
            model[0] = NEXT_LPS[state]
            if state == 0:
                model[1] = 1 - mps
        self.renorm()
        return value

    def bypass(self):
        self.event()
        self.offset = (self.offset << 1) | self.read_bit()
        if self.offset >= self.range:
            self.offset -= self.range
            return 1
        return 0

    def fixed(self, width):
        require(0 <= width <= 24, "CABAC bypass width")
        result = 0
        for _ in range(width):
            result = (result << 1) | self.bypass()
        return result

    def unary(self, maximum):
        for value in range(maximum):
            if not self.bypass():
                return value
        return maximum

    def eg(self, k=0):
        value = 0
        while self.bypass():
            require(k < 16, "CABAC Exp-Golomb budget")
            value += 1 << k
            k += 1
        return value + self.fixed(k)

    def term(self):
        self.event()
        self.range -= 2
        if self.offset >= self.range:
            require(self.last == 1, "CABAC terminal stop bit")
            return 1
        self.renorm()
        return 0

    def finish(self):
        require(self.last == 1, "CABAC terminal stop bit")
        while self.at & 7:
            require(self.read_bit() == 0, "CABAC trailing alignment")
        # Closed initial envelope excludes optional zero words, rather than
        # treating arbitrary unread bytes as coefficient/pixel data.
        require(self.at == len(self.data) * 8, "CABAC trailing/unowned bytes")
        return {"bins": self.bins, "consumed_bits": self.at}
