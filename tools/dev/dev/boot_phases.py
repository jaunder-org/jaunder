"""`dev trace-analysis boot-phases`: median boot-phase decomposition per
`(source, project, cacheWarmth, experiment arm)` population (#818).

`analyze` reports maxima and means; this question is entirely medians, so it
is its own command rather than another section there.

Every decomposed segment is document-relative, so the decomposition target is
`mount_done.startTime` (`traces.decompose`). The Node-side `commitToMountMs`
and the skew between the two frames are reported and never decomposed: mixing
the frames charges event-delivery latency to the app's boot phases.
"""

import math
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from dev import report
from dev.traces import (
    CURRENT_TIMING_SCHEMA,
    BootOutcome,
    Decomposition,
    Filters,
    Span,
    as_i64,
    decompose,
    field_f64,
    member,
    parse_json_attr,
    plain_sum,
    read_spans,
)

# The shared bridge schema of frame-skew diagnostics recorded on navigations.
BRIDGE_SCHEMA = "bridge-v1"
# Remainder closure tolerances registered by #868; a population that misses
# either stays non-decisive.
BRIDGE_MEAN_TOLERANCE_MS = 1.0
BRIDGE_MAX_TOLERANCE_MS = 2.0
# A stylesheet sample needs every one of these; it keeps the last four.
STYLE_FIELDS = (
    "moduleBeforeInitMs",
    "jaunderCssResponseEndMs",
    "jaunderThemesCssResponseEndMs",
    "styleMaxResponseEndMs",
    "styleToModuleBeforeInitMs",
    "moduleBeforeInitToWasmFetchStartMs",
    "wasmFetchStartMs",
)
BRIDGE_FIELDS = (
    "commitToDocumentStartMs",
    "mountDoneToBindingMs",
    "frameSkewRemainderMs",
)
# The fields of a module shape signature, in signature order.
SHAPE_FIELDS = (
    "imports",
    "importedFunctions",
    "importedTables",
    "importedMemories",
    "exports",
    "exportedFunctions",
    "exportedTables",
    "exportedMemories",
    "customSections",
)
INIT_PATHS = ("streaming", "buffered")
PREAMBLE = (
    "Boot-phase decomposition (medians; even counts take the lower middle value).\n"
    "Segments are document-relative and close on mount_done.startTime exactly;\n"
    "commitToMountMs is Node-side wall clock and is reported, never decomposed.\n"
    "Frame-skew bridge diagnostics stay outside the app boot decomposition table.\n"
    "Per-segment medians are marginal, so they need not sum to the median total."
    "\n\n"
)
SEGMENT_HEADERS = ("segment", "median_ms", "share_of_boot_total_%", "n")


def _lower_middle(values: Sequence[float]) -> float:
    return sorted(values)[(len(values) - 1) // 2]


def median(values: Sequence[float]) -> float | None:
    """The median, taking the lower of the two middle values on an even count.

    No in-repo convention exists to inherit, so this one is pinned by a test.
    """
    return _lower_middle(values) if values else None


@dataclass(frozen=True)
class Estimate:
    """A diagnostic's mean and its standard error over `n` samples.

    The mean needs one sample and the error two; either is otherwise None.
    """

    n: int
    mean: float | None
    se: float | None

    @classmethod
    def of(cls, values: Sequence[float]) -> Estimate:
        n = len(values)
        if not n:
            return cls(0, None, None)
        mean = plain_sum(values) / n
        if n < 2:
            return cls(n, mean, None)
        squares = plain_sum([(value - mean) * (value - mean) for value in values])
        return cls(n, mean, math.sqrt(squares / (n - 1)) / math.sqrt(n))


@dataclass(frozen=True)
class SegmentMedian:
    """One segment's median across a population.

    `count` is kept because labels are discovered per navigation: a label on
    only some of them is a data defect a bare median would hide.
    """

    label: str
    median_ms: float
    count: int


@dataclass
class Population:
    """One `(source, project, cacheWarmth, arm)` population's navigations.

    `source` is the trace file, load-bearing because `projectName` is the
    browser and names no backend: without it SQLite pools with PostgreSQL.
    `navigations` counts every navigation seen and `decomposed` only those
    with a full mark set that closed; the two differing is the #818 signal.
    """

    source: str
    project: str
    cache_warmth: str
    experiment_arm: str
    navigations: int = 0
    current: int = 0
    mounted: int = 0
    # Per complete navigation, the last four of STYLE_FIELDS.
    style: list[list[float]] = field(default_factory=list)
    # Per complete navigation, BRIDGE_FIELDS.
    bridge: list[list[float]] = field(default_factory=list)
    direct_complete: int = 0
    streaming: int = 0
    buffered: int = 0
    legacy: int = 0
    wasm_api_ms: list[float] = field(default_factory=list)
    wasm_init_ms: list[float] = field(default_factory=list)
    shape_complete: int = 0
    shape_missing: int = 0
    shapes: set[str] = field(default_factory=set)
    # Navigations whose segments missed the boot total: excluded from every
    # median and counted here, never silently included or dropped.
    closure_violations: int = 0
    decomposed: list[Decomposition] = field(default_factory=list)

    def style_estimate(self, index: int) -> Estimate:
        return Estimate.of([sample[index] for sample in self.style])

    def bridge_estimate(self, index: int) -> Estimate:
        return Estimate.of([sample[index] for sample in self.bridge])

    @property
    def direct_missing(self) -> int:
        return self.current - self.direct_complete

    @property
    def style_ordering_pass(self) -> int:
        """Samples whose stylesheets finished before the module began."""
        return sum(1 for sample in self.style if sample[1] >= 0.0)

    @property
    def style_decisive(self) -> bool:
        """Every navigation carried a complete stylesheet sample."""
        return len(self.style) == self.navigations

    @property
    def remainder_max_abs_ms(self) -> float | None:
        return max((abs(sample[2]) for sample in self.bridge), default=None)

    @property
    def bridge_decisive(self) -> bool:
        mean = self.bridge_estimate(2).mean
        largest = self.remainder_max_abs_ms
        return (
            self.mounted > 0
            and len(self.bridge) == self.mounted
            and mean is not None
            and abs(mean) <= BRIDGE_MEAN_TOLERANCE_MS
            and largest is not None
            and largest <= BRIDGE_MAX_TOLERANCE_MS
        )

    @property
    def segments(self) -> list[SegmentMedian]:
        """Per-label medians, in first-seen segment order."""
        groups: dict[str, list[float]] = {}
        for decomposition in self.decomposed:
            for label, value in decomposition.segments:
                groups.setdefault(label, []).append(value)
        return [
            SegmentMedian(label, _lower_middle(values), len(values))
            for label, values in groups.items()
        ]

    @property
    def boot_total_ms(self) -> float | None:
        return median([d.boot_total_ms for d in self.decomposed])

    @property
    def commit_to_mount_ms(self) -> float | None:
        return median(
            [
                d.commit_to_mount_ms
                for d in self.decomposed
                if d.commit_to_mount_ms is not None
            ]
        )

    @property
    def frame_skew_ms(self) -> float | None:
        return median(
            [
                d.commit_to_mount_ms - d.boot_total_ms
                for d in self.decomposed
                if d.commit_to_mount_ms is not None
            ]
        )


def _finite_fields(navigation: Any, keys: Sequence[str]) -> list[float] | None:
    """Every one of `keys` as a finite number, or None if any is not.

    Historical traces omit diagnostic keys; that is non-decisive, not an error.
    """
    values = []
    for key in keys:
        value = field_f64(navigation, key)
        if value is None:
            return None
        values.append(value)
    return values


def _shape_signature(navigation: Any) -> str | None:
    """The wasm module's shape as text, when every count is a u64."""
    shape = member(navigation, "wasmModuleShape")
    parts = []
    for key in SHAPE_FIELDS:
        value = member(shape, key)
        if type(value) is not int or not 0 <= value < 2**64:
            return None
        parts.append(f"{key}={value}")
    return ",".join(parts)


def _marks_by_navigation(marks: Any) -> dict[int, list[Any]]:
    """Boot marks keyed by navigation id, from their own attribute: the boot
    total must come from somewhere other than the segments for the closure
    check to mean anything."""
    by_id: dict[int, list[Any]] = {}
    for entry in marks if isinstance(marks, list) else []:
        identifier, found = as_i64(member(entry, "id")), member(entry, "marks")
        if identifier is not None and isinstance(found, list):
            by_id[identifier] = found
    return by_id


def _text(navigation: Any, key: str, default: str) -> str:
    value = member(navigation, key)
    return value if isinstance(value, str) else default


def _add(population: Population, navigation: Any, marks: list[Any]) -> None:
    """Counts one navigation into its population."""
    population.navigations += 1
    if field_f64(navigation, "commitToMountMs") is not None:
        population.mounted += 1
    style = _finite_fields(navigation, STYLE_FIELDS)
    if style is not None:
        population.style.append(style[3:])
    if member(navigation, "frameSkewSchema") == BRIDGE_SCHEMA:
        bridge = _finite_fields(navigation, BRIDGE_FIELDS)
        if bridge is not None:
            population.bridge.append(bridge)
    if member(navigation, "wasmTimingSchema") != CURRENT_TIMING_SCHEMA:
        population.legacy += 1
        return
    population.current += 1
    api_ms = field_f64(navigation, "wasmApiMs")
    init_ms = field_f64(navigation, "wasmInitMs")
    path = member(navigation, "wasmInitPath")
    if (
        api_ms is not None
        and init_ms is not None
        and path in INIT_PATHS
        and 0.0 <= api_ms <= init_ms
    ):
        population.direct_complete += 1
        population.wasm_api_ms.append(api_ms)
        population.wasm_init_ms.append(init_ms)
        if path == "streaming":
            population.streaming += 1
        else:
            population.buffered += 1
        shape = _shape_signature(navigation)
        if shape is None:
            population.shape_missing += 1
        else:
            population.shape_complete += 1
            population.shapes.add(shape)
    outcome = decompose(navigation, marks)
    if isinstance(outcome, Decomposition):
        population.decomposed.append(outcome)
    elif outcome is BootOutcome.CLOSURE_VIOLATION:
        population.closure_violations += 1


def _warmth_rank(warmth: str) -> int:
    """Cold before warm; anything else last, so an unexpected label is
    visible rather than interleaved."""
    return {"cold": 0, "warm": 1}.get(warmth, 2)


def boot_phase_rows(spans: list[Span]) -> list[Population]:
    """Every `e2e.test` navigation in `spans`, grouped into populations.

    A malformed navigation or boot-mark attribute raises `MalformedJsonAttr`.
    """
    groups: dict[tuple[str, str, str, str], Population] = {}
    for span in spans:
        if span.name != "e2e.test":
            continue
        navigations = parse_json_attr(span.raw, "e2e.navigation_top_json", span.source)
        # Both optional attributes are read before either's absence can skip
        # the span, so a malformed sibling is never hidden.
        marks = parse_json_attr(span.raw, "e2e.boot_marks_json", span.source)
        if not isinstance(navigations, list):
            continue
        by_id = _marks_by_navigation(marks)
        for navigation in navigations:
            arm = _text(navigation, "wasmExperimentArm", "") or "-"
            key = (
                span.source,
                span.project or "-",
                _text(navigation, "cacheWarmth", "-"),
                arm,
            )
            population = groups.setdefault(key, Population(*key))
            identifier = as_i64(member(navigation, "id"))
            found = [] if identifier is None else by_id.get(identifier, [])
            _add(population, navigation, found)
    return sorted(
        groups.values(),
        key=lambda row: (
            row.source,
            row.project,
            _warmth_rank(row.cache_warmth),
            row.cache_warmth,
            row.experiment_arm,
        ),
    )


def boot_phases(inputs: Sequence[Path]) -> list[Population]:
    """Every input's spans, read unfiltered, as populations."""
    spans = []
    for path in inputs:
        spans.extend(read_spans(path, Filters()))
    return boot_phase_rows(spans)


def _ms(value: float | None) -> str:
    return "-" if value is None else f"{value:.3f}"


def _estimate(label: str, estimate: Estimate) -> str:
    return (
        f"mean {label} ± SE (n={estimate.n}): {_ms(estimate.mean)} ± {_ms(estimate.se)}"
    )


def _bridge_status(row: Population) -> str:
    if row.bridge_decisive:
        return "decisive"
    reasons = []
    if row.mounted == 0:
        reasons.append("no mounted navigations")
    if len(row.bridge) != row.mounted:
        reasons.append("coverage incomplete")
    else:
        mean = row.bridge_estimate(2).mean
        if mean is None or abs(mean) > BRIDGE_MEAN_TOLERANCE_MS:
            reasons.append("mean remainder out of bounds")
        largest = row.remainder_max_abs_ms
        if largest is None or largest > BRIDGE_MAX_TOLERANCE_MS:
            reasons.append("max remainder out of bounds")
    return f"non-decisive ({', '.join(reasons)})"


def _style_status(row: Population) -> str:
    # A population holds at least one navigation, so missing samples are the
    # only way to be non-decisive.
    return "decisive" if row.style_decisive else "non-decisive (coverage incomplete)"


def _summary(row: Population) -> list[str]:
    """A population's key/value lines, before its segment table; each line's
    fields are separated by two spaces."""
    style, bridge = len(row.style), len(row.bridge)
    rate = "-" if not style else f"{row.style_ordering_pass / style * 100.0:.1f}%"
    api, init = Estimate.of(row.wasm_api_ms), Estimate.of(row.wasm_init_ms)
    lines = [
        [
            (
                f"== {row.source} / {row.project} / {row.cache_warmth}"
                f" / arm:{row.experiment_arm} =="
            )
        ],
        [
            f"navigations: {row.navigations}",
            f"decomposed: {len(row.decomposed)}",
            f"closure violations: {row.closure_violations}",
        ],
        [
            f"current: {row.current}",
            f"mounted: {row.mounted}",
            f"bridge complete: {bridge}",
            f"bridge certification: {_bridge_status(row)}",
        ],
        [
            f"stylesheet complete: {style}",
            f"stylesheet certification: {_style_status(row)}",
        ],
        [
            (
                "stylesheet ordering pass (styleToModuleBeforeInitMs >= 0):"
                f" {row.style_ordering_pass}/{style} ({rate})"
            )
        ],
        [_estimate("styleToModuleBeforeInitMs", row.style_estimate(1))],
        [_estimate("moduleBeforeInitToWasmFetchStartMs", row.style_estimate(2))],
        [_estimate("styleMaxResponseEndMs", row.style_estimate(0))],
        [_estimate("wasmFetchStartMs", row.style_estimate(3))],
        [
            f"direct complete: {row.direct_complete}",
            f"direct missing: {row.direct_missing}",
            f"streaming: {row.streaming}",
            f"buffered: {row.buffered}",
            f"legacy: {row.legacy}",
        ],
        [
            f"shape complete: {row.shape_complete}",
            f"shape missing: {row.shape_missing}",
            f"unique shapes: {len(row.shapes)}",
        ],
        [
            f"median wasmApiMs (n={row.direct_complete}): {_ms(median(row.wasm_api_ms))}",
            f"median wasmInitMs (n={row.direct_complete}): {_ms(median(row.wasm_init_ms))}",
        ],
        [
            f"mean wasmApiMs ± SE: {_ms(api.mean)} ± {_ms(api.se)}",
            f"mean wasmInitMs ± SE: {_ms(init.mean)} ± {_ms(init.se)}",
        ],
        [_estimate("commitToDocumentStartMs", row.bridge_estimate(0))],
        [_estimate("mountDoneToBindingMs", row.bridge_estimate(1))],
        [
            _estimate("frameSkewRemainderMs", row.bridge_estimate(2)),
            f"max |remainder|: {_ms(row.remainder_max_abs_ms)}",
        ],
    ]
    return ["  ".join(fields) for fields in lines]


def render(rows: Sequence[Population]) -> str:
    """The populations as one block each, after the fixed preamble."""
    if not rows:
        return "No navigations found in the provided trace files.\n"
    out = PREAMBLE
    for row in rows:
        out += "".join(f"{line}\n" for line in _summary(row))
        # Loud, because an empty table and a dark instrument look alike, and
        # that is the #818 failure mode itself.
        if not row.decomposed:
            out += (
                f"no decomposed navigations (0 of {row.navigations})"
                " — nothing to decompose for this population\n\n"
            )
            continue
        total = row.boot_total_ms
        out += f"median bootTotalMs: {_ms(total)}\n"
        out += f"median commitToMountMs: {_ms(row.commit_to_mount_ms)}\n"
        out += (
            "median frame skew (commitToMountMs - bootTotalMs):"
            f" {_ms(row.frame_skew_ms)}\n"
        )
        cells = [
            [
                segment.label,
                f"{segment.median_ms:.3f}",
                f"{segment.median_ms / total * 100.0:.1f}"
                if total is not None and total > 0.0
                else "-",
                str(segment.count),
            ]
            for segment in row.segments
        ]
        out += report.table(SEGMENT_HEADERS, cells) + "\n\n"
    return out
