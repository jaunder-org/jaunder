"""`dev trace-analysis`: manual diagnostics over the e2e lanes' OTel traces.

`analyze` reads exported `otel-traces.jsonl` files into one `Analysis` of
every report section and renders it as tables; `run` builds the e2e lanes and
analyzes their captures in one step; `boot-phases` (`boot_phases`) reports
medians, which `analyze` does not.
"""

import functools
import sys
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from dev import boot_phases, report, steps
from dev.evidence import CATALOG, EvidenceError, pair_lanes, read_catalog, report_specs
from dev.report import ms
from dev.traces import (
    CURRENT_TIMING_SCHEMA,
    LIFECYCLE_SPAN,
    MIN_BOOT_PHASES,
    PAGE_SPAN,
    TEST_SPAN,
    Filters,
    MalformedJsonAttr,
    Span,
    field_f64,
    get_attr,
    has_attr,
    interval_union_ms,
    member,
    parse_json_attr,
    parse_spans,
    parse_u64,
    read_spans,
    read_trace_member,
    span_interval_ms,
    to_url_path,
)

BACKENDS = ("sqlite", "postgres")
BROWSERS = ("chromium", "firefox")
SYSTEM = "x86_64-linux"


def _count(span: Span, key: str) -> int:
    """An integer-count attribute; 0 when absent or not a count."""
    return parse_u64(get_attr(span.raw, key)) or 0


def _label(value: str) -> str:
    """A grouping label, `-` when unset."""
    return value or "-"


def _sorted_desc[T](rows: Iterable[T], key: Callable[[T], float]) -> list[T]:
    """`rows`, largest `key` first; equal keys, NaN included, keep their order."""

    def compare(left: T, right: T) -> int:
        a, b = key(left), key(right)
        return -1 if a > b else 1 if a < b else 0

    return sorted(rows, key=functools.cmp_to_key(compare))


# Playwright's own per-attempt durations: the span-coverage denominator.


@dataclass(frozen=True)
class AttemptKey:
    """One test attempt, as the lifecycle span's `e2e.test`, `e2e.project` and
    `e2e.retry` name it. Retry is part of it: each attempt exports its spans
    and has its own wall clock."""

    test: str
    project: str
    retry: int


Attempts = dict[AttemptKey, float]


def _attempts(path: Path) -> Attempts:
    """Every attempt with a duration in one Playwright JSON report."""
    try:
        raw = path.read_text(encoding="utf-8")
    except (OSError, ValueError) as error:
        raise EvidenceError(
            f"failed to read playwright report {path}: {error}"
        ) from error
    specs = report_specs(raw, f"failed to parse playwright report {path}")
    attempts = {}
    for spec in specs:
        for test in spec.tests:
            for result in test.results:
                if result.duration is None:
                    continue
                key = AttemptKey(
                    spec.title or "", test.project_name or "", result.retry or 0
                )
                attempts[key] = result.duration
    return attempts


@dataclass(frozen=True)
class ReportedDurations:
    """Per-attempt wall clocks, scoped by the trace file they belong to.

    `AttemptKey` alone is ambiguous across backends: `projectName` is the
    browser, so SQLite and PostgreSQL runs of one browser share keys with
    different durations. `per_source` keys on `Span.source` so each trace joins
    its own report; `global_` serves reports given without that pairing.
    """

    per_source: dict[str, Attempts] = field(default_factory=dict)
    global_: Attempts = field(default_factory=dict)

    def get(self, source: str, key: AttemptKey) -> float | None:
        """An attempt's duration, preferring its own trace file's report."""
        scoped = self.per_source.get(source, {})
        return scoped[key] if key in scoped else self.global_.get(key)

    def __len__(self) -> int:
        return sum(map(len, self.per_source.values())) + len(self.global_)

    @classmethod
    def from_paths(cls, paths: Sequence[Path]) -> ReportedDurations:
        """Unpaired reports, as `--playwright-report` gives them.

        A key two reports give different durations is refused: they come from
        different runs with nothing to say which trace each belongs to.
        """
        merged: Attempts = {}
        for path in paths:
            for key, duration in _attempts(path).items():
                existing = merged.get(key)
                if (
                    existing is not None
                    and abs(existing - duration) >= sys.float_info.epsilon
                ):
                    raise EvidenceError(
                        f"playwright reports disagree on {key.test} [{key.project}]"
                        f" retry {key.retry}: {_number(existing)} ms vs"
                        f" {_number(duration)} ms. Reports from different backends"
                        " share test+project+retry keys, so they cannot be merged —"
                        " analyze one combo at a time, or use `dev trace-analysis"
                        " run`, which pairs each report with its own trace file."
                    )
                merged[key] = duration
        return cls(global_=merged)

    @classmethod
    def from_labeled(cls, pairs: Sequence[tuple[str, Path]]) -> ReportedDurations:
        """Reports paired with the `Span.source` of the trace they belong to."""
        return cls(per_source={source: _attempts(path) for source, path in pairs})


def _number(value: float) -> str:
    """A float as Rust's `Display` writes it: no `.0` on an integral value."""
    return str(int(value)) if value.is_integer() else repr(value)


# The analysis: every report section as already-sorted rows of cells.


@dataclass
class _Agg:
    """The count, total and maximum a hotspot section accumulates."""

    count: int = 0
    total_ms: float = 0.0
    max_ms: float = 0.0

    def add(self, value: float) -> None:
        self.count += 1
        self.total_ms += value
        self.max_ms = max(self.max_ms, value)

    @property
    def avg_ms(self) -> float:
        return self.total_ms / self.count


@dataclass(frozen=True)
class Section:
    """One report table: its title, headers and formatted rows.

    A ranked section is cut to `--top` rows when rendered; the others show
    every row.
    """

    title: str
    headers: tuple[str, ...]
    rows: list[list[str]]
    ranked: bool = True


@dataclass(frozen=True)
class Analysis:
    """Every report section, in report order, for `span_count` spans.

    `coverage_note` says why the per-test span coverage section is empty, so
    a missing report never reads as full coverage.
    """

    span_count: int
    project_filter: str | None
    coverage_note: str | None
    sections: list[Section]


def _hotspot_section(title: str, groups: dict[str, _Agg], name: str) -> Section:
    rows = _sorted_desc(groups.items(), lambda item: item[1].max_ms)
    return Section(
        title,
        ("max_ms", "avg_ms", "total_ms", "count", name),
        [
            [ms(a.max_ms), ms(a.avg_ms), ms(a.total_ms), str(a.count), key]
            for key, a in rows
        ],
    )


def _e2e_tests(spans: list[Span]) -> list[Span]:
    return [span for span in spans if span.name == TEST_SPAN]


def _navigation_spans(spans: list[Span]) -> list[Span]:
    """Spans whose navigation JSON describes a test-owned page's loads."""
    return [
        span
        for span in spans
        if span.name == TEST_SPAN
        or (span.name == PAGE_SPAN and has_attr(span.raw, "e2e.navigation_top_json"))
    ]


def _json_array(span: Span, key: str) -> list[Any] | None:
    value = parse_json_attr(span.raw, key, span.source)
    return value if isinstance(value, list) else None


def _span_coverage(spans: list[Span], reported: ReportedDurations) -> list[list[Any]]:
    """Per-test span coverage (#794): how much of each attempt's wall clock
    lies inside a named phase of its lifecycle tree.

    Covered time is the interval union of the envelope's children, never the
    envelope's own duration, which spans the whole lifecycle by construction.
    An attempt absent from the report is skipped rather than given a zero
    denominator. Returns (uncovered, cells) pairs, largest uncovered first.
    """
    children: dict[str, list[Span]] = {}
    for span in spans:
        if span.parent_span_id:
            children.setdefault(span.parent_span_id, []).append(span)
    rows = []
    for envelope in spans:
        if envelope.name != LIFECYCLE_SPAN:
            continue
        key = AttemptKey(
            get_attr(envelope.raw, "e2e.test"),
            get_attr(envelope.raw, "e2e.project"),
            parse_u64(get_attr(envelope.raw, "e2e.retry")) or 0,
        )
        # Scoped by the envelope's own trace file (see ReportedDurations).
        reported_ms = reported.get(envelope.source, key)
        if reported_ms is None:
            continue
        intervals = [
            interval
            for kid in children.get(envelope.span_id, [])
            if (interval := span_interval_ms(kid.raw)) is not None
        ]
        covered_ms = interval_union_ms(intervals)
        # Clamped: clock skew between the span stamps and Playwright's own
        # timing can put covered marginally above reported.
        uncovered_ms = max(reported_ms - covered_ms, 0.0)
        pct = uncovered_ms / reported_ms * 100.0 if reported_ms > 0.0 else 0.0
        cells = [
            _label(key.project),
            ms(reported_ms),
            ms(covered_ms),
            ms(uncovered_ms),
            f"{pct:.1f}",
            key.test,
        ]
        rows.append((uncovered_ms, cells))
    return [cells for _, cells in _sorted_desc(rows, lambda row: row[0])]


def _coverage_note(
    spans: list[Span], reported: ReportedDurations, rows: list[list[Any]]
) -> str | None:
    """Why the coverage section is empty, so "no report supplied" is never
    mistaken for "everything is attributed"."""
    if rows:
        return None
    if not len(reported):
        return (
            "no --playwright-report supplied; per-test coverage needs Playwright's"
            " own durations as the denominator"
        )
    if not any(span.name == LIFECYCLE_SPAN for span in spans):
        return f"no `{LIFECYCLE_SPAN}` spans in the capture (pre-#794 traces have none)"
    return (
        f"no lifecycle span matched a report entry ({len(reported)} attempt(s)"
        " in the report)"
    )


def _boot_coverage(spans: list[Span]) -> Section:
    """How much of the boot decomposition each `(source, project)` captured
    (#818), every row shown.

    Keyed on the trace file too, since the project names no backend. A pair
    with no navigations still gets a zeroed row, so a lane that captured
    nothing is visible. `mounted` is proxied by a present `commitToMountMs`,
    and `dropped` sums `e2e.navigation_top_dropped`, because the navigation
    JSON is the slowest 20 per page sink, a biased sample rather than a census.
    """
    counts: dict[tuple[str, str], list[int]] = {}
    for span in _navigation_spans(spans):
        # all, current, legacy, mounted, full marks, dropped
        row = counts.setdefault((span.source, _label(span.project)), [0] * 6)
        row[5] += _count(span, "e2e.navigation_top_dropped")
        for navigation in _json_array(span, "e2e.navigation_top_json") or []:
            row[0] += 1
            if member(navigation, "wasmTimingSchema") != CURRENT_TIMING_SCHEMA:
                row[2] += 1
                continue
            row[1] += 1
            if field_f64(navigation, "commitToMountMs") is not None:
                row[3] += 1
            phases = member(navigation, "bootPhases")
            if (
                isinstance(phases, dict)
                and len(phases) >= MIN_BOOT_PHASES
                and field_f64(navigation, "wasmInitStartMs") is not None
                and field_f64(navigation, "wasmInitStartToBootEntryMs") is not None
            ):
                row[4] += 1
    return Section(
        "Boot decomposition coverage (from e2e.test/e2e.page navigation JSON)",
        (
            "source",
            "project",
            "all navs",
            "current navs",
            "legacy navs",
            "mounted",
            "full marks",
            "dropped",
        ),
        [
            [source, project, *map(str, row)]
            for (source, project), row in counts.items()
        ],
        ranked=False,
    )


def _spans_sections(spans: list[Span], top: str) -> tuple[Section, Section]:
    """Every span, then every `e2e.test` span, slowest first."""
    slowest = _sorted_desc(spans, lambda span: span.duration_ms)
    tests = _sorted_desc(_e2e_tests(spans), lambda span: span.duration_ms)
    return (
        Section(
            f"Top {top} slowest spans",
            (
                "duration_ms",
                "trace_id",
                "span",
                "method",
                "uri",
                "busy_ns",
                "idle_ns",
                "source",
            ),
            [
                [
                    ms(span.duration_ms),
                    _label(span.trace_id),
                    _label(span.name),
                    _label(span.method),
                    _label(span.uri),
                    _label(span.busy_ns),
                    _label(span.idle_ns),
                    span.source,
                ]
                for span in slowest
            ],
        ),
        Section(
            f"Top {top} slowest e2e.test spans",
            ("duration_ms", "project", "actions", "requests", "trace_id", "test"),
            [
                [
                    ms(span.duration_ms),
                    _label(span.project),
                    str(_count(span, "e2e.action_count")),
                    str(_count(span, "e2e.request_count")),
                    _label(span.trace_id),
                    _label(get_attr(span.raw, "e2e.test")),
                ]
                for span in tests
            ],
        ),
    )


def _action_section(spans: list[Span], top: str) -> Section:
    """Action hotspots from `e2e.action_top_json`; unnamed actions skipped."""
    groups: dict[str, _Agg] = {}
    for span in _e2e_tests(spans):
        for action in _json_array(span, "e2e.action_top_json") or []:
            name = member(action, "name")
            duration = field_f64(action, "durationMs")
            if duration is not None and isinstance(name, str) and name:
                groups.setdefault(name, _Agg()).add(duration)
    return _hotspot_section(
        f"Top {top} e2e action hotspots (from e2e.action_top_json)", groups, "name"
    )


# The navigation phase fields aggregated as `navigation.<label>`.
NAVIGATION_PHASES = (
    ("navigation.total", "totalMs"),
    ("navigation.request", "requestMs"),
    ("navigation.commit_to_domcontentloaded", "commitToDomContentLoadedMs"),
    ("navigation.commit_to_mount", "commitToMountMs"),
    ("navigation.domcontentloaded_to_load", "domContentLoadedToLoadMs"),
)


def _navigation_sections(spans: list[Span], top: str) -> tuple[Section, Section]:
    """Navigation phase totals and slow navigation targets; negative values
    are dropped."""
    phases: dict[str, _Agg] = {}
    targets: dict[str, _Agg] = {}
    for span in _navigation_spans(spans):
        for navigation in _json_array(span, "e2e.navigation_top_json") or []:
            for label, key in NAVIGATION_PHASES:
                value = field_f64(navigation, key)
                if value is not None and value >= 0.0:
                    phases.setdefault(label, _Agg()).add(value)
            url = member(navigation, "url")
            path = to_url_path(url if isinstance(url, str) else "")
            total = field_f64(navigation, "totalMs")
            if path and total is not None and total >= 0.0:
                targets.setdefault(path, _Agg()).add(total)
    return (
        _hotspot_section(
            f"Top {top} navigation phase hotspots"
            " (from e2e.test/e2e.page navigation JSON)",
            phases,
            "name",
        ),
        _hotspot_section(f"Top {top} slow navigation targets", targets, "target"),
    )


def _long_task_sections(spans: list[Span], top: str) -> tuple[Section, Section]:
    """Long-task hotspots by task name, and every project's totals."""
    names: dict[str, _Agg] = {}
    # tests, tasks, total, max
    projects: dict[str, list[float]] = {}
    for span in _e2e_tests(spans):
        tasks = _json_array(span, "e2e.long_tasks_json")
        if tasks is None:
            continue
        project = projects.setdefault(_label(span.project), [0, 0, 0.0, 0.0])
        project[0] += 1
        for task in tasks:
            duration = field_f64(task, "duration")
            if duration is None or duration < 0.0:
                continue
            name = member(task, "name")
            names.setdefault(
                name if isinstance(name, str) and name else "longtask", _Agg()
            ).add(duration)
            project[1] += 1
            project[2] += duration
            project[3] = max(project[3], duration)
    by_project = _sorted_desc(projects.items(), lambda item: item[1][2] / item[1][0])
    return (
        _hotspot_section(
            f"Top {top} long-task hotspots (from e2e.long_tasks_json)", names, "name"
        ),
        Section(
            "Long-task totals by project",
            ("project", "tests", "tasks", "avg_ms_per_test", "max_task_ms"),
            [
                [name, str(int(p[0])), str(int(p[1])), ms(p[2] / p[0]), ms(p[3])]
                for name, p in by_project
            ],
            ranked=False,
        ),
    )


def _resource_sections(spans: list[Span], top: str) -> tuple[Section, Section]:
    """Resource initiator hotspots and slow assets from
    `e2e.resource_summary_json.topSlow`; negative durations are dropped."""
    initiators: dict[str, _Agg] = {}
    # Each asset keeps the initiator it was first seen with.
    assets: dict[str, tuple[str, _Agg]] = {}
    for span in _e2e_tests(spans):
        summary = parse_json_attr(span.raw, "e2e.resource_summary_json", span.source)
        items = member(summary, "topSlow")
        for item in items if isinstance(items, list) else []:
            duration = field_f64(item, "durationMs")
            if duration is None or duration < 0.0:
                continue
            initiator = member(item, "initiatorType")
            if not (isinstance(initiator, str) and initiator):
                initiator = "unknown"
            name = member(item, "name")
            asset = to_url_path(name) if isinstance(name, str) and name else "unknown"
            initiators.setdefault(initiator, _Agg()).add(duration)
            assets.setdefault(asset, (initiator, _Agg()))[1].add(duration)
    slow = _sorted_desc(assets.items(), lambda item: item[1][1].max_ms)
    return (
        _hotspot_section(
            f"Top {top} resource initiator hotspots (from e2e.resource_summary_json)",
            initiators,
            "name",
        ),
        Section(
            f"Top {top} slow resource assets",
            ("max_ms", "avg_ms", "total_ms", "count", "initiator", "asset"),
            [
                [
                    ms(a.max_ms),
                    ms(a.avg_ms),
                    ms(a.total_ms),
                    str(a.count),
                    initiator,
                    name,
                ]
                for name, (initiator, a) in slow
            ],
        ),
    )


def _totals_sections(spans: list[Span]) -> tuple[Section, Section]:
    """`e2e.test` durations by project, every row, then per-trace totals."""
    # tests, total, max, actions, requests
    projects: dict[str, list[float]] = {}
    for span in _e2e_tests(spans):
        p = projects.setdefault(_label(span.project), [0, 0.0, 0.0, 0, 0])
        p[0] += 1
        p[1] += span.duration_ms
        p[2] = max(p[2], span.duration_ms)
        p[3] += _count(span, "e2e.action_count")
        p[4] += _count(span, "e2e.request_count")
    # total, spans
    traces: dict[str, list[float]] = {}
    for span in spans:
        t = traces.setdefault(span.trace_id, [0.0, 0])
        t[0] += span.duration_ms
        t[1] += 1
    by_project = _sorted_desc(projects.items(), lambda item: item[1][1] / item[1][0])
    by_trace = _sorted_desc(traces.items(), lambda item: item[1][0])
    return (
        Section(
            "E2E test duration by project",
            ("project", "tests", "avg_ms", "max_ms", "avg_actions", "avg_requests"),
            [
                [
                    name,
                    str(int(p[0])),
                    ms(p[1] / p[0]),
                    ms(p[2]),
                    f"{p[3] / p[0]:.2f}",
                    f"{p[4] / p[0]:.2f}",
                ]
                for name, p in by_project
            ],
            ranked=False,
        ),
        Section(
            "Trace totals (sum of span durations)",
            ("total_ms", "spans", "trace_id"),
            [[ms(t[0]), str(int(t[1])), trace] for trace, t in by_trace],
        ),
    )


def analyze_spans(
    spans: list[Span],
    project_filter: str | None,
    reported: ReportedDurations,
    top: int,
) -> Analysis:
    """Every report section over already-parsed spans, in report order.

    `top` only titles the ranked sections. A malformed JSON attribute raises
    `MalformedJsonAttr`, the sections reading it in the order below.
    """
    shown = str(top)
    coverage = _span_coverage(spans, reported)
    note = _coverage_note(spans, reported, coverage)
    slowest, tests = _spans_sections(spans, shown)
    actions = _action_section(spans, shown)
    phases, targets = _navigation_sections(spans, shown)
    boot = _boot_coverage(spans)
    long_tasks, long_task_projects = _long_task_sections(spans, shown)
    initiators, assets = _resource_sections(spans, shown)
    by_project, traces = _totals_sections(spans)
    sections = [
        Section(
            f"Top {top} tests by unattributed time (reported vs lifecycle-tree union)",
            (
                "project",
                "reported_ms",
                "covered_ms",
                "uncovered_ms",
                "uncovered_%",
                "test",
            ),
            coverage,
        ),
        boot,
        slowest,
        tests,
        actions,
        phases,
        targets,
        long_tasks,
        long_task_projects,
        initiators,
        assets,
        by_project,
        traces,
    ]
    return Analysis(len(spans), project_filter, note, sections)


def render(analysis: Analysis, top: int) -> str:
    """The report text: ranked sections cut to `top` rows, empty ones left out."""
    if not analysis.span_count:
        return "No spans found in the provided trace files.\n"
    out = ""
    if analysis.project_filter is not None:
        out += f"Project filter: {analysis.project_filter}\n\n"
    for index, section in enumerate(analysis.sections):
        # First, because it is the question the others cannot answer: how much
        # of each test's wall clock a named phase accounts for at all (#794).
        if index == 0 and analysis.coverage_note is not None:
            out += (
                f"== Per-test span coverage ==\n(skipped: {analysis.coverage_note})\n\n"
            )
            continue
        rows = section.rows[:top] if section.ranked else section.rows
        if rows:
            out += f"{section.title}\n{report.table(section.headers, rows)}\n\n"
    return out


def analyze(
    inputs: Sequence[Path], filters: Filters, reported: ReportedDurations, top: int
) -> Analysis:
    """Every input's spans that pass `filters`, analyzed."""
    spans = []
    for path in inputs:
        spans.extend(read_spans(path, filters))
    return analyze_spans(spans, filters.project, reported, top)


# The commands. Each shows its report and one step; a malformed JSON
# attribute fails that step with status 1, and any other refusal (an
# unreadable or unparseable input, a conflicting report, a missing capture)
# with status 2.


def _status(error: EvidenceError) -> int:
    return 1 if isinstance(error, MalformedJsonAttr) else 2


def _show(text: str) -> None:
    sys.stdout.write(text)
    sys.stdout.flush()


def analyze_command(
    top: int,
    trace: str | None,
    project: str | None,
    playwright_reports: Sequence[Path],
    files: Sequence[Path],
) -> None:
    def decide() -> str:
        reported = ReportedDurations.from_paths(playwright_reports)
        analysis = analyze(files, Filters(trace, project), reported, top)
        _show(render(analysis, top))
        return f"{analysis.span_count} span(s)"

    steps.verdict("traces-analyze", decide, _status)


def boot_phases_command(files: Sequence[Path]) -> None:
    def decide() -> str:
        rows = boot_phases.boot_phases(files)
        _show(boot_phases.render(rows))
        return f"{len(rows)} population(s)"

    steps.verdict("traces-boot-phases", decide, _status)


@dataclass(frozen=True)
class Lane:
    """One e2e lane `run` builds: the identity its capture is named for, and
    the flake output (`kind` `checks` or `packages`) that produces it."""

    identity: str
    kind: str
    attribute: str

    @property
    def installable(self) -> str:
        return f".#{self.kind}.{SYSTEM}.{self.attribute}"


def lanes(catalog: Path, single_worker: bool, browser: str | None) -> list[Lane]:
    """The lanes to build: both backends, then the selected browsers.

    By default each enabled catalog lane's check; with `single_worker` each
    pair's single-worker package, whose captures keep the unsplit identity.
    """
    browsers = BROWSERS if browser is None else (browser,)
    if single_worker:
        return [
            Lane(
                f"{backend}-{name}-unsplit",
                "packages",
                f"e2e-{backend}-{name}-single-worker",
            )
            for backend in BACKENDS
            for name in browsers
        ]
    lanes = read_catalog(catalog)
    return [
        Lane(lane.identity, "checks", f"e2e-{lane.identity}")
        for backend in BACKENDS
        for name in browsers
        for lane in pair_lanes(lanes, backend, name)
    ]


def run_command(
    top: int, trace: str | None, single_worker: bool, browser: str | None
) -> None:
    """Builds the lanes, then analyzes their captures labelled by identity.

    A failed build stops with Nix's status. Each lane's Playwright report,
    when its output has one, is paired with its own trace (ReportedDurations).
    """
    selected = lanes(Path(CATALOG), single_worker, browser)
    outputs = [
        Path(steps.nix_build(lane.attribute, lane.installable, keep=False))
        for lane in selected
    ]

    def decide() -> str:
        spans: list[Span] = []
        pairs = []
        for lane, output in zip(selected, outputs, strict=True):
            capture = output / f"capture-{lane.identity}.tar.gz"
            if not capture.is_file():
                raise EvidenceError(f"capture tarball not found: {capture}")
            spans.extend(
                parse_spans(read_trace_member(capture), Filters(trace), lane.identity)
            )
            playwright = output / f"playwright-report-{lane.identity}.json"
            if playwright.is_file():
                pairs.append((lane.identity, playwright))
        reported = ReportedDurations.from_labeled(pairs)
        _show(render(analyze_spans(spans, None, reported, top), top))
        return f"{len(selected)} trace file(s)"

    steps.verdict("traces-run", decide, _status)
