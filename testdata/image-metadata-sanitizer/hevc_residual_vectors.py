"""Independent TEST codewords from H.265 V9, not libde265/validator code.

Semantic inputs are explicit signed coordinate maps. Projection to syntax follows
7.3.8.11; contexts follow 9.3.4.2.3..7; remainder intervals follow 9.3.3.11.
Only gated luma16/chroma8 diagonal geometry is modeled. No pixels or transforms.
Numeric initialization facts: Tables9-28/29 (I initialization, ctxIdx0..41).
"""
from hevc_normative_model import INIT, flat_i_slice

# Independently stated diagonal order (6.5.3); not a production scan import.
DIAGONAL4 = ((0, 0), (0, 1), (1, 0), (0, 2), (1, 1), (2, 0), (0, 3), (1, 2),
             (2, 1), (3, 0), (1, 3), (2, 2), (3, 1), (2, 3), (3, 2), (3, 3))
DIAGONAL2 = ((0, 0), (0, 1), (1, 0), (1, 1))
LAST = ((0, ""), (1, ""), (2, ""), (3, ""), (4, "0"), (4, "1"),
        (5, "0"), (5, "1"), (6, "00"), (6, "01"), (6, "10"), (6, "11"),
        (7, "00"), (7, "01"), (7, "10"), (7, "11"))
# Tables9-28/29, separately transcribed from the primary PDF.
INIT["coded_sb"] = (91, 171, 134, 141)
INIT["sig"] = (111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141,
               179, 153, 125, 107, 125, 141, 179, 153, 125, 107, 125, 141,
               179, 153, 125, 140, 139, 182, 182, 152, 136, 152, 136, 153,
               136, 139, 111, 136, 139, 111)


def rank(component, point):
    groups = DIAGONAL4 if component == 0 else DIAGONAL2
    x, y = point
    return 16 * groups.index((x // 4, y // 4)) + DIAGONAL4.index((x % 4, y % 4))


def signed(component, magnitudes):
    """Declare parity-consistent maps; signs remain explicit semantic inputs."""
    result = dict(magnitudes)
    groups = DIAGONAL4 if component == 0 else DIAGONAL2
    for gx, gy in groups:
        points = sorted((p for p in result if (p[0] // 4, p[1] // 4) == (gx, gy)),
                        key=lambda p: rank(component, p))
        if points and rank(component, points[-1]) - rank(component, points[0]) > 3:
            first = points[0]
            result[first] = abs(result[first]) * (-1 if sum(abs(result[p]) for p in points) % 2 else 1)
    return result


def expected_coefficients(maps):
    return [(c, 0, 0, 4 if c == 0 else 3, x, y, values[x, y])
            for c, values in sorted(maps.items())
            for x, y in sorted(values, key=lambda p: rank(c, p), reverse=True)]


def write_residual(e, component, values, coverage):
    """Semantic map -> syntax tokens, equations from primary clauses above.

    Group flags are projected from declared map, not recovered from actual/native
    traversal. Significance contexts use already-known right/below map flags.
    Base levels are algebraic functions of independently specified magnitudes.
    """
    seen = coverage if coverage is not None else set()
    def mark(name):
        seen.add("c%d:%s" % (component, name))
    groups = DIAGONAL4 if component == 0 else DIAGONAL2
    ordered = sorted(values, key=lambda p: rank(component, p), reverse=True)
    last = ordered[0]
    for axis, name in enumerate(("last_x", "last_y")):
        prefix, _ = LAST[last[axis]]
        cap, offset = (7, 6) if component == 0 else (5, 15)
        for b in range(prefix + int(prefix < cap)):
            e.bit(name, offset + b // 2, int(b < prefix))
        mark("last:%s:p%d" % (name, prefix))
    for axis in range(2):
        _, suffix = LAST[last[axis]]
        for b in suffix:
            e.bypass(int(b))
        mark("suffix:w%d" % len(suffix))
    final_group = rank(component, last) // 16
    flags = {g: int(any((x // 4, y // 4) == g for x, y in values)) for g in groups[:final_group + 1]}
    flags[groups[0]] = 1  # 7.4.9.11: inferred coded, even when DC group empty.
    previous_greater = False
    for group_number in range(final_group, -1, -1):
        gx, gy = groups[group_number]
        group = (gx, gy)
        mask = flags.get((gx + 1, gy), 0) + 2 * flags.get((gx, gy + 1), 0)
        if 0 < group_number < final_group:
            ctx = (2 if component else 0) + int(mask != 0)
            e.bit("coded_sb", ctx, flags[group])
            mark("coded:ctx%d:v%d" % (ctx, flags[group]))
        else:
            mark("group-inferred:%s" % ("last" if group_number == final_group else "origin"))
        if not flags[group]:
            mark("uncoded-group")
            continue
        members = [p for p in ordered if (p[0] // 4, p[1] // 4) == group]
        ceiling = rank(component, last) % 16 - 1 if group_number == final_group else 15
        for n in range(ceiling, -1, -1):
            px, py = DIAGONAL4[n]
            point = (4 * gx + px, 4 * gy + py)
            inferred = n == 0 and 0 < group_number < final_group and members == [point]
            if inferred:
                mark("DC-inferred")
                continue
            if point == (0, 0):
                ctx = 27 if component else 0
                mark("sig-origin")
            else:
                # Equations9-45..48 stated as four local-coordinate predicates.
                bucket = ((2 if px + py == 0 else int(px + py < 3)),
                          (2 if py == 0 else int(py == 1)),
                          (2 if px == 0 else int(px == 1)), 2)[mask]
                ctx = bucket + (21 + 3 * int(gx + gy > 0) if component == 0 else 36)
                mark("sig-mask%d-bucket%d" % (mask, bucket))
            e.bit("sig", ctx, int(point in values))
            mark("sig:ctx%d:v%d" % (ctx, int(point in values)))
            if n == 0:
                mark("DC-explicit%d" % int(point in values))
        if not members:
            mark("empty-origin")
            continue
        count = len(members)
        mark("count%d" % count)
        context_set = (2 if component == 0 and group_number else 0) + int(previous_greater)
        absolute = [abs(values[p]) for p in members]
        first_greater = next((n for n in range(min(8, count)) if absolute[n] > 1), None)
        for n in range(min(8, count)):
            # Eq9-59: previous greater flag resets to0; otherwise increment,
            # saturated only when selecting context. No native state imported.
            c = 0 if any(a > 1 for a in absolute[:n]) else min(3, n + 1)
            ctx = 4 * context_set + c + (16 if component else 0)
            e.bit("gt1", ctx, int(absolute[n] > 1))
            mark("gt1:ctx%d:v%d" % (ctx, int(absolute[n] > 1)))
        previous_greater = any(a > 1 for a in absolute[:8])
        if first_greater is not None:
            ctx = context_set + (4 if component else 0)
            e.bit("gt2", ctx, int(absolute[first_greater] > 2))
            mark("gt2:ctx%d:v%d" % (ctx, int(absolute[first_greater] > 2)))
        gap = rank(component, members[0]) - rank(component, members[-1])
        hidden = gap > 3
        mark("sign:%s" % ("hidden%d" % (sum(absolute) % 2) if hidden else "explicit"))
        for n, p in enumerate(members):
            if not hidden or n != count - 1:
                e.bypass(int(values[p] < 0))
        if hidden:
            assert (values[members[-1]] < 0) == bool(sum(absolute) % 2), "bad independently declared hidden sign"
        rice = 0
        for n, magnitude in enumerate(absolute):
            first = n == first_greater
            base = 1 if n >= 8 else (1 if magnitude == 1 else (min(3, magnitude) if first else 2))
            threshold = 1 if n >= 8 else (3 if first else 2)
            if base != threshold:
                continue
            remaining = magnitude - base
            if remaining < (4 << rice):
                prefix, width = remaining >> rice, rice
                low = prefix << rice
            else:
                exponent = ((remaining >> rice) - 2).bit_length() - 1
                prefix, width = 3 + exponent, rice + exponent
                low = (2 + (1 << exponent)) << rice
            mark("rice%d:p%d:w%d" % (rice, prefix, width))
            for _ in range(prefix):
                e.bypass(1)
            e.bypass(0)
            suffix = remaining - low
            for b in range(width - 1, -1, -1):
                e.bypass((suffix >> b) & 1)
            if magnitude > (3 << rice):
                mark("rice-increment%d" % rice)
                rice = min(4, rice + 1)
            else:
                mark("rice-retain%d" % rice)


def vectors():
    """Finite named semantic matrix. Neither actual parser nor native imported."""
    cases = {}
    def add(name, maps):
        assert name not in cases
        cases[name] = {c: signed(c, m) for c, m in maps.items()}
    for c in range(3):
        side = 16 if c == 0 else 8
        groups = DIAGONAL4 if c == 0 else DIAGONAL2
        for x in range(side):
            for y in range(side):
                add("last-c%d-x%d-y%d" % (c, x, y), {c: {(x, y): (-1 if (x + y) % 2 else 1)}})
        for g in groups:
            for mask in range(4):
                occupied = {g}
                if mask & 1 and g[0] + 1 < side // 4:
                    occupied.add((g[0] + 1, g[1]))
                if mask & 2 and g[1] + 1 < side // 4:
                    occupied.add((g[0], g[1] + 1))
                later = [p for p in groups[groups.index(g) + 1:]
                         if p not in ((g[0] + 1, g[1]), (g[0], g[1] + 1))]
                if later:
                    occupied.add(later[-1])
                m = {(4 * gx, 4 * gy): 1 for gx, gy in occupied}
                m.update({(4 * g[0] + x, 4 * g[1] + y): 1 for x, y in DIAGONAL4})
                add("neighbours-c%d-g%d-mask%d" % (c, groups.index(g), mask), {c: m})
        for count in range(1, 17):
            for magnitude in (1, 2, 3):
                m = {(4 * gx + x, 4 * gy + y): magnitude
                     for gx, gy in groups for x, y in DIAGONAL4[:count]}
                add("count-c%d-n%d-level%d" % (c, count, magnitude), {c: m})
        for first in range(8):
            for level in (2, 3):
                m = {(4 * gx + x, 4 * gy + y): (level if 15 - n == first else 1)
                     for gx, gy in groups for n, (x, y) in enumerate(DIAGONAL4)}
                add("greater-c%d-first%d-level%d" % (c, first, level), {c: m})
        for first in (1, 2):
            m = {(4 * gx + x, 4 * gy + y): 1 for gx, gy in groups for x, y in DIAGONAL4}
            m[DIAGONAL4[15 - first]] = 2
            add("origin-late-greater-c%d-first%d" % (c, first), {c: m})
        for k in range(5):
            training = [4, 7, 13, 25][:k]
            base = 3 if k == 0 else 2
            for prefix in range(18 - k):
                width = k if prefix <= 3 else k + prefix - 3
                lower = prefix << k if prefix <= 3 else (2 + (1 << (prefix - 3))) << k
                for edge, suffix in (("low", 0), ("high", min((1 << width) - 1, 32767 - base - lower))):
                    assert suffix >= 0
                    levels = training + [base + lower + suffix]
                    points = list(reversed(DIAGONAL4[:len(levels)]))
                    add("rice-c%d-k%d-p%d-%s" % (c, k, prefix, edge),
                        {c: dict(zip(points, levels))})
        add("rice-c%d-saturate-and-count9" % c, {c: dict(zip(reversed(DIAGONAL4),
            (4, 7, 13, 25, 49, 98, 200, 32767, 100, 50, 49, 48, 25, 13, 7, 4)))})
        for level in (32767, -32768):
            add("precision-c%d-%d" % (c, level), {c: {(0, 0): level}})
        # Signed explicit gap3 versus hidden gap4, both parities.
        for span in (3, 4, 15):
            for level in (1, 2):
                add("sign-c%d-gap%d-parity%d" % (c, span, (1 + level) % 2),
                    {c: {DIAGONAL4[0]: -1, DIAGONAL4[span]: -level}})
    add("component-order-Y-Cb-Cr", {0: {(15, 15): 3, (0, 0): 1},
                                   1: {(7, 7): 2, (0, 0): 1},
                                   2: {(7, 7): -17, (0, 0): 1}})
    return cases


def required_coverage():
    """Reachable branch obligations, stated independently of case generation.

    No width1/4x4, luma8, non-diagonal, transform-skip, or extension branch is
    reachable through frozen header + no-split guards. For Rice k, precision15
    bounds unary prefix to17-k and bypass suffix to14 (9.3.3.11/7.4.9.11).
    """
    required = set()
    for c in range(3):
        def need(s):
            required.add("c%d:%s" % (c, s))
        for axis in ("last_x", "last_y"):
            for p in range(8 if c == 0 else 6):
                need("last:%s:p%d" % (axis, p))
        for w in range(3 if c == 0 else 2):
            need("suffix:w%d" % w)
        for ctx in ((0, 1) if c == 0 else (2, 3)):
            for bit in range(2):
                need("coded:ctx%d:v%d" % (ctx, bit))
        for mask in range(4):
            for bucket in ((2,) if mask == 3 else (0, 1, 2)):
                need("sig-mask%d-bucket%d" % (mask, bucket))
        for ctx in ((0, 21, 22, 23, 24, 25, 26) if c == 0 else (27, 36, 37, 38)):
            for bit in range(2):
                need("sig:ctx%d:v%d" % (ctx, bit))
        for ctx in (range(16) if c == 0 else range(16, 24)):
            for bit in range(2):
                need("gt1:ctx%d:v%d" % (ctx, bit))
        for ctx in (range(4) if c == 0 else (4, 5)):
            for bit in range(2):
                need("gt2:ctx%d:v%d" % (ctx, bit))
        for n in (1, 8, 9, 16):
            need("count%d" % n)
        for s in ("group-inferred:last", "group-inferred:origin", "DC-inferred",
                  "DC-explicit0", "DC-explicit1", "uncoded-group", "empty-origin",
                  "sign:explicit", "sign:hidden0", "sign:hidden1"):
            need(s)
        for k in range(5):
            need("rice-increment%d" % k)
            need("rice-retain%d" % k)
            for p in range(18 - k):
                need("rice%d:p%d:w%d" % (k, p, k if p <= 3 else k + p - 3))
    return required


def required_names():
    """Independent finite census, not the producer's dictionary keys."""
    names = {"residual-component-order-Y-Cb-Cr"}
    for c in range(3):
        side, groups = (16, 16) if c == 0 else (8, 4)
        names |= {"residual-last-c%d-x%d-y%d" % (c, x, y) for x in range(side) for y in range(side)}
        names |= {"residual-neighbours-c%d-g%d-mask%d" % (c, g, m) for g in range(groups) for m in range(4)}
        names |= {"residual-count-c%d-n%d-level%d" % (c, n, a) for n in range(1, 17) for a in (1, 2, 3)}
        names |= {"residual-greater-c%d-first%d-level%d" % (c, n, a) for n in range(8) for a in (2, 3)}
        names |= {"residual-origin-late-greater-c%d-first%d" % (c, n) for n in (1, 2)}
        names |= {"residual-rice-c%d-k%d-p%d-%s" % (c, k, p, edge)
                  for k in range(5) for p in range(18 - k) for edge in ("low", "high")}
        names |= {"residual-precision-c%d-%d" % (c, a) for a in (32767, -32768)}
        names |= {"residual-sign-c%d-gap%d-parity%d" % (c, gap, parity) for gap in (3, 4, 15) for parity in (0, 1)}
        names.add("residual-rice-c%d-saturate-and-count9" % c)
    assert len(names) == 1186
    return names


def assert_coverage(cases):
    assert set(cases) == required_names(), "independent required residual vector census"
    union = set().union(*(set(case["coverage"]) for case in cases.values()))
    missing = required_coverage() - union
    assert not missing, "missing independent residual obligations: " + repr(sorted(missing))
    # Coordinate semantics themselves, not branch labels alone, prove all last
    # positions and all three components. Required names are independent census.
    for c in range(3):
        side = 16 if c == 0 else 8
        for x in range(side):
            for y in range(side):
                name = "residual-last-c%d-x%d-y%d" % (c, x, y)
                assert name in cases and len(cases[name]["expected"]) == 1
                assert cases[name]["expected"][0][4:6] == (x, y)
    return sorted(union)


def build_vectors():
    result = {}
    for name, maps in vectors().items():
        coverage = set()
        nal, encoder = flat_i_slice(residual_maps=maps, coverage=coverage)
        result["residual-" + name] = {"nal": nal, "maps": maps,
            "expected": expected_coefficients(maps), "events": encoder.events,
            "coverage": sorted(coverage)}
    return result
