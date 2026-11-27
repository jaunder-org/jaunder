"""Fail-closed e2e verdicts over already-lifted lane evidence.

A lane's producer lifts its Playwright report, the lane manifest naming the
tests it selected, the duration-budget manifest, its capture and its VM phase
sidecar. `verify_lane` checks one lane as its producer finishes; `reconcile`
checks one backend/browser pair's lanes together against the lane catalog
and an independent census of the expected tests, and reports every failure
it finds, not just the first. Neither builds or runs anything.

Each input is decoded strictly (`evidence`): a missing field or a value of the
wrong kind is a failure, not a default.
"""

import json
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from dev.evidence import (
    BOOLEAN,
    COUNT,
    NUMBER,
    STRING,
    EvidenceError,
    ExpectedLane,
    ReportSpec,
    decode,
    objects,
    optional,
    pair_lanes,
    read,
    read_catalog,
    report_specs,
    required,
)
from dev.traces import (
    CURRENT_TIMING_SCHEMA,
    PAGE_SPAN,
    TEST_SPAN,
    BootOutcome,
    Filters,
    MalformedJsonAttr,
    Span,
    as_i64,
    boot_decomposition_outcome,
    field_f64,
    get_attr,
    has_attr,
    marks_by_navigation,
    navigation_marks,
    page_boot_decomposition_outcome,
    parse_json_attr,
    parse_spans,
    parse_u64,
    read_trace_member,
)

# The utilization at or above which an attempt is reported as duration
# pressure. It is a signal, never a verdict: only a `timedOut` attempt fails.
PRESSURE_THRESHOLD = 0.25
# Every status Playwright's JSON reporter gives an attempt.
ATTEMPT_STATUSES = ("passed", "failed", "timedOut", "skipped", "interrupted")
# The VM sidecar's phase that runs Playwright.
GATE_PHASE = "gate-execution"
# The VM sidecar's phases that must each have succeeded once (`nix/e2e_capture.py`).
REQUIRED_PHASES = (GATE_PHASE, "result-lift", "post-gate-checks")
PHASE_NAMES = (
    "nix-evaluation",
    "nix-substitution",
    "nix-local-build",
    "vm-startup-readiness",
    *REQUIRED_PHASES,
)
PHASE_OUTCOMES = ("success", "failed", "unavailable")


# Ownership: the pair's lanes, and that their reports together execute exactly
# the expected tests, each once.


# (project id, project name, test id)
Identity = tuple[str, str, str]


@dataclass(frozen=True)
class _Census:
    schema_version: int
    complete: bool
    topology: str
    tests: list[Identity]


@dataclass(frozen=True)
class _LaneManifest:
    schema_version: int
    complete: bool
    lane: str
    backend: str
    browser: str
    shard_index: int | None
    shard_count: int | None
    tests: list[Identity]


def _tests(document: dict[str, Any]) -> list[Identity]:
    return [
        (
            required(test, "project_id", STRING),
            required(test, "project_name", STRING),
            required(test, "test_id", STRING),
        )
        for test in objects(document, "tests")
    ]


def _census(document: dict[str, Any]) -> _Census:
    return _Census(
        required(document, "schema_version", COUNT),
        required(document, "complete", BOOLEAN),
        required(document, "topology", STRING),
        _tests(document),
    )


def _lane_manifest(document: dict[str, Any]) -> _LaneManifest:
    return _LaneManifest(
        required(document, "schema_version", COUNT),
        required(document, "complete", BOOLEAN),
        required(document, "lane", STRING),
        required(document, "backend", STRING),
        required(document, "browser", STRING),
        optional(document, "shard_index", COUNT),
        optional(document, "shard_count", COUNT),
        _tests(document),
    )


def _identities(tests: list[Identity], label: str) -> set[Identity]:
    if any(not part for test in tests for part in test):
        raise EvidenceError(f"malformed {label} identity")
    identities = set(tests)
    if len(identities) != len(tests):
        raise EvidenceError(f"duplicate {label} identity")
    return identities


def _foreign(identities: set[Identity], browser: str) -> bool:
    """Whether any of `identities` belongs to a project other than `browser`'s."""
    return any(project != browser for _, project, _ in identities)


def _report_identities(raw: str, lane: str) -> set[Identity]:
    specs = report_specs(raw, f"malformed report for `{lane}`")
    if not specs:
        raise EvidenceError(f"report for `{lane}` has no specs")
    identities = []
    for spec in specs:
        if not spec.id:
            raise EvidenceError(f"malformed report test id for `{lane}`")
        for test in spec.tests:
            if not test.project_id:
                raise EvidenceError(f"malformed report project id for `{lane}`")
            if not test.project_name:
                raise EvidenceError(f"malformed report project name for `{lane}`")
            identities.append((test.project_id, test.project_name, spec.id))
    unique = set(identities)
    if len(unique) != len(identities):
        raise EvidenceError(f"duplicate report identity for `{lane}`")
    return unique


def _selected(
    lane: ExpectedLane, manifest_raw: str, where: str, mismatched: str
) -> set[Identity]:
    """The tests `lane`'s lane manifest selected, once it is the complete,
    current manifest that lane's producer wrote; `mismatched` refuses any other.
    """
    manifest = decode(manifest_raw, _lane_manifest, f"malformed lane manifest{where}")
    if not (
        manifest.schema_version == 1
        and manifest.complete
        and manifest.lane == lane.identity
        and manifest.backend == lane.backend
        and manifest.browser == lane.browser
        and manifest.shard_index == lane.shard_index
        and manifest.shard_count == lane.shard_count
    ):
        raise EvidenceError(mismatched)
    return _identities(manifest.tests, "lane manifest")


def validate_population(lane: ExpectedLane, report_raw: str, manifest_raw: str) -> None:
    """The producer's manifest and report agree on the lane's population.

    That pair defines the runtime population (ADR-0165), which must all belong
    to the lane's one project, named for its browser.
    """
    selected = _selected(
        lane, manifest_raw, "", "mismatched or incomplete lane manifest"
    )
    reported = _report_identities(report_raw, lane.identity)
    if not selected or selected != reported:
        raise EvidenceError("lane manifest/report population mismatch")
    if _foreign(selected, lane.browser):
        raise EvidenceError(
            "lane project population differs from the lane's browser project"
        )


def reconcile_ownership(
    backend: str,
    browser: str,
    catalog: list[ExpectedLane],
    evidence: list[tuple[str, str, str, str]],
) -> str:
    """The pair's lanes execute exactly the expected census, each test once.

    Evidence is `(lane identity, expected census, report, lane manifest)`.
    """
    lanes = {lane.identity: lane for lane in pair_lanes(catalog, backend, browser)}
    supplied = {identity for identity, *_ in evidence}
    # The catalog's shape is checked by `scripts/e2e-matrix.jq`; here the
    # evidence must name exactly the pair's enabled lanes.
    if not lanes or set(lanes) != supplied or len(evidence) != len(supplied):
        raise EvidenceError("shard lane evidence does not exactly match the catalog")
    expected_all: set[Identity] | None = None
    observed: set[Identity] = set()
    for identity, census_raw, report_raw, manifest_raw in evidence:
        census = decode(
            census_raw, _census, f"malformed expected census for `{identity}`"
        )
        if (
            census.schema_version != 1
            or not census.complete
            or census.topology != f"{backend}-{browser}-split"
        ):
            raise EvidenceError(
                f"incomplete or mismatched expected census for `{identity}`"
            )
        expected = _identities(census.tests, "expected census")
        if not expected:
            raise EvidenceError("empty expected census")
        if _foreign(expected, browser):
            raise EvidenceError("expected census has unowned project")
        if expected_all is None:
            expected_all = expected
        elif expected_all != expected:
            raise EvidenceError("lifted expected censuses are not identical")
        selected = _selected(
            lanes[identity],
            manifest_raw,
            f" for `{identity}`",
            f"mismatched lane manifest for `{identity}`",
        )
        if not selected or _foreign(selected, browser):
            raise EvidenceError(f"lane manifest has unowned project for `{identity}`")
        reported = _report_identities(report_raw, identity)
        if selected != reported:
            raise EvidenceError(
                f"lane manifest/report population mismatch for `{identity}`"
            )
        if observed & reported:
            raise EvidenceError(
                f"duplicate report identity across lanes for `{identity}`"
            )
        observed |= reported
    if expected_all != observed:
        raise EvidenceError("global expected census/report union mismatch")
    return (
        f"{backend}/{browser}: {len(lanes)} candidate lanes, "
        f"{len(observed)} expected tests"
    )


# The duration budget: the duration manifest owns the selected population and
# each attempt's effective timeout, the report owns measured durations and
# statuses. A lane passes only when their identities and retry streams agree
# exactly and no attempt timed out. Attempts at or above the pressure threshold
# are reported, not refused: a slow pass is still a pass.


@dataclass(frozen=True, order=True)
class _TestIdentity:
    test_id: str
    project_id: str
    project_name: str
    title: str
    file: str
    line: int

    def describe(self) -> str:
        return (
            f"test_id={self.test_id} project_id={self.project_id} "
            f"project_name={self.project_name} {self.file}:{self.line} "
            f"title={self.title}"
        )

    def validate(self, source: str) -> None:
        for name in ("test_id", "project_id", "project_name", "title", "file"):
            if not getattr(self, name):
                raise EvidenceError(f"{source} has an empty {name}")
        if self.line == 0:
            raise EvidenceError(f"{source} has line 0")


# Each selected test's attempts: retry → effective timeout, ms.
_Timeouts = dict[_TestIdentity, dict[int, float]]
# Each reported test's attempts: retry → (duration in ms, status).
_Results = dict[_TestIdentity, dict[int, tuple[float, str]]]
# A duration manifest's tests: each identity with its (retry, timeout) attempts.
_ManifestTests = list[tuple[_TestIdentity, list[tuple[int, float]]]]


def _duration_manifest(document: dict[str, Any]) -> tuple[int, bool, _ManifestTests]:
    return (
        required(document, "schema_version", COUNT),
        required(document, "complete", BOOLEAN),
        [
            (
                _TestIdentity(
                    required(test, "test_id", STRING),
                    required(test, "project_id", STRING),
                    required(test, "project_name", STRING),
                    required(test, "title", STRING),
                    required(test, "file", STRING),
                    required(test, "line", COUNT),
                ),
                [
                    (
                        required(attempt, "retry", COUNT),
                        required(attempt, "effective_timeout_ms", NUMBER),
                    )
                    for attempt in objects(test, "attempts")
                ],
            )
            for test in objects(document, "tests")
        ],
    )


def _contiguous[T](retries: dict[int, T], source: str, identity: _TestIdentity) -> None:
    for expected, actual in enumerate(sorted(retries)):
        if actual != expected:
            raise EvidenceError(
                f"{source} retries must start at zero and be contiguous; expected "
                f"{expected}, found {actual}: {identity.describe()}"
            )


def _manifest_attempts(tests: _ManifestTests) -> _Timeouts:
    if not tests:
        raise EvidenceError("manifest selects no tests")
    expected: _Timeouts = {}
    for identity, attempts in tests:
        identity.validate("manifest test")
        if not attempts:
            raise EvidenceError(f"manifest test has no attempts: {identity.describe()}")
        timeouts: dict[int, float] = {}
        for retry, timeout in attempts:
            if timeout <= 0:
                raise EvidenceError(
                    f"manifest attempt retry {retry} has a non-positive or "
                    f"non-finite effective timeout: {identity.describe()}"
                )
            if retry in timeouts:
                raise EvidenceError(
                    f"manifest has duplicate retry {retry}: {identity.describe()}"
                )
            timeouts[retry] = timeout
        _contiguous(timeouts, "manifest", identity)
        if identity in expected:
            raise EvidenceError(
                f"manifest has a duplicate selected test: {identity.describe()}"
            )
        expected[identity] = timeouts
    return expected


def _report_attempts(specs: list[ReportSpec]) -> _Results:
    if not specs:
        raise EvidenceError("report contains no selected tests")
    observed: _Results = {}
    for spec in specs:
        if not spec.tests:
            raise EvidenceError(
                f"report spec {spec.id or '<missing>'} has no project results"
            )
        for test in spec.tests:
            identity = _TestIdentity(
                spec.id or "",
                test.project_id or "",
                test.project_name or "",
                spec.title or "",
                spec.file or "",
                spec.line or 0,
            )
            identity.validate("report test")
            if not test.results:
                raise EvidenceError(
                    f"report test has no results: {identity.describe()}"
                )
            results: dict[int, tuple[float, str]] = {}
            for attempt in test.results:
                retry, duration, status = (
                    attempt.retry,
                    attempt.duration,
                    attempt.status,
                )
                if retry is None:
                    raise EvidenceError(
                        f"report attempt has no retry: {identity.describe()}"
                    )
                if duration is None:
                    raise EvidenceError(
                        f"report retry {retry} has no duration: {identity.describe()}"
                    )
                if duration < 0:
                    raise EvidenceError(
                        f"report retry {retry} has a non-finite or negative "
                        f"duration: {identity.describe()}"
                    )
                if status not in ATTEMPT_STATUSES:
                    raise EvidenceError(
                        f"report retry {retry} has a missing or unknown status "
                        f"{json.dumps(status)}: {identity.describe()}"
                    )
                if retry in results:
                    raise EvidenceError(
                        f"report has duplicate retry {retry}: {identity.describe()}"
                    )
                results[retry] = (duration, status)
            _contiguous(results, "report", identity)
            if identity in observed:
                raise EvidenceError(
                    f"report has a duplicate selected test: {identity.describe()}"
                )
            observed[identity] = results
    return observed


@dataclass(frozen=True)
class Pressure:
    """One attempt's measured duration against its effective timeout."""

    identity: _TestIdentity
    retry: int
    duration: float
    timeout: float

    @property
    def utilization(self) -> float:
        return self.duration / self.timeout

    def describe(self) -> str:
        return (
            f"{self.utilization * 100:.2f}%: {self.identity.describe()}; "
            f"retry {self.retry}; duration {self.duration:.3f} ms / effective "
            f"timeout {self.timeout:.3f} ms"
        )


@dataclass(frozen=True)
class DurationBudget:
    """A lane's passing duration budget: its greatest pressure, and every
    attempt at or above the threshold, worst first."""

    maximum: Pressure
    pressured: list[Pressure]

    def headline(self) -> str:
        return (
            f"{len(self.pressured)} attempt(s) at or above "
            f"{PRESSURE_THRESHOLD * 100:.0f}% of their timeout"
        )

    def detail(self) -> str:
        if not self.pressured:
            return f"maximum duration pressure {self.maximum.describe()}"
        return "\n".join(
            [
                f"duration pressure: {self.headline()}",
                *(f"  {pressure.describe()}" for pressure in self.pressured),
            ]
        )


def _pressure(expected: _Timeouts, observed: _Results) -> DurationBudget:
    if expected.keys() != observed.keys():
        missing = sorted(expected.keys() - observed.keys())
        unexpected = sorted(observed.keys() - expected.keys())
        if missing and unexpected:
            raise EvidenceError(
                f"selected-test identity mismatch; missing {missing[0].describe()} "
                f"and unexpected {unexpected[0].describe()}"
            )
        if missing:
            raise EvidenceError(
                f"report is missing selected test: {missing[0].describe()}"
            )
        raise EvidenceError(
            f"report has unexpected selected test: {unexpected[0].describe()}"
        )
    for identity in sorted(expected):
        if expected[identity].keys() != observed[identity].keys():
            raise EvidenceError(f"retry reconciliation mismatch: {identity.describe()}")
    attempts = []
    for identity in sorted(expected):
        for retry, timeout in sorted(expected[identity].items()):
            duration, status = observed[identity][retry]
            attempt = Pressure(identity, retry, duration, timeout)
            if status == "timedOut":
                raise EvidenceError(f"attempt timed out at {attempt.describe()}")
            attempts.append(attempt)
    # Worst first; both keep identity and retry order on a tie.
    by_pressure = sorted(attempts, key=lambda attempt: -attempt.utilization)
    return DurationBudget(
        by_pressure[0],
        [
            attempt
            for attempt in by_pressure
            if attempt.utilization >= PRESSURE_THRESHOLD
        ],
    )


def validate_duration_json(report_json: str, manifest_json: str) -> DurationBudget:
    """The duration budget over in-memory report and manifest JSON."""
    specs = report_specs(report_json, "parsing Playwright report")
    version, complete, tests = decode(
        manifest_json, _duration_manifest, "parsing duration-budget manifest"
    )
    if version != 1:
        raise EvidenceError(
            f"unsupported manifest schema_version {version}; expected 1"
        )
    if not complete:
        raise EvidenceError("manifest is not complete")
    return _pressure(_manifest_attempts(tests), _report_attempts(specs))


def validate_duration_files(report: Path, manifest: Path) -> DurationBudget:
    """The duration budget over a lane's lifted report and duration manifest."""
    return validate_duration_json(read(report, "report"), read(manifest, "manifest"))


# Boot decomposition coverage: every project the report executed has mounted
# navigations in the trace, and every mounted navigation is current, complete
# and closes within 1 ms; no navigation was dropped (ADR-0165).


def _report_projects(specs: list[ReportSpec]) -> set[str]:
    projects = {test.project_name for spec in specs for test in spec.tests}
    if None in projects or "" in projects:
        raise EvidenceError("Playwright report test has missing or empty projectName")
    if not projects:
        raise EvidenceError("Playwright report has no complete project population")
    return {project for project in projects if project}


def _dropped(span: Span, project: str) -> None:
    if not has_attr(span.raw, "e2e.navigation_top_dropped"):
        raise EvidenceError(f"project {project}: missing dropped-navigation count")
    text = get_attr(span.raw, "e2e.navigation_top_dropped")
    count = parse_u64(text)
    if count is None:
        raise EvidenceError(
            f"project {project}: malformed dropped-navigation count {json.dumps(text)}"
        )
    if count:
        raise EvidenceError(f"project {project}: {count} dropped navigation record(s)")


def _boot_marks(marks: Any, project: str) -> dict[int, list[Any]]:
    """The boot marks by navigation id, once every entry is well formed and
    names its navigation once."""
    if marks is not None and not isinstance(marks, list):
        raise EvidenceError(f"project {project}: boot-mark evidence is not an array")
    seen: set[int] = set()
    for entry in marks or []:
        if not isinstance(entry, dict):
            raise EvidenceError(f"project {project}: boot-mark entry is not an object")
        identity = as_i64(entry.get("id"))
        if identity is None:
            raise EvidenceError(f"project {project}: boot-mark entry has no integer id")
        records = entry.get("marks")
        if not isinstance(records, list):
            raise EvidenceError(
                f"project {project}: boot-mark entry has no marks array"
            )
        for mark in records:
            if not isinstance(mark, dict):
                raise EvidenceError(
                    f"project {project}: boot-mark record is not an object"
                )
            if not isinstance(mark.get("name"), str):
                raise EvidenceError(
                    f"project {project}: boot-mark record has no string name"
                )
            if field_f64(mark, "startTime") is None:
                raise EvidenceError(
                    f"project {project}: boot-mark startTime is not a finite number"
                )
        if identity in seen:
            raise EvidenceError(
                f"project {project}: duplicate boot-mark navigation id {identity}"
            )
        seen.add(identity)
    return marks_by_navigation(marks)


def _navigation(
    project: str, navigation: Any, marks: dict[int, list[Any]], page: bool
) -> None:
    if navigation.get("wasmTimingSchema") != CURRENT_TIMING_SCHEMA:
        raise EvidenceError(f"project {project}: legacy or missing timing schema")
    # The shared decomposition owns both the boot-interval floor and closure,
    # so this gate cannot count unrelated properties.
    classify = page_boot_decomposition_outcome if page else boot_decomposition_outcome
    outcome = classify(navigation, navigation_marks(navigation, marks))
    if outcome is BootOutcome.INCOMPLETE:
        raise EvidenceError(f"project {project}: incomplete direct-init decomposition")
    if outcome is BootOutcome.CLOSURE_VIOLATION:
        raise EvidenceError(
            f"project {project}: document-frame decomposition does not close "
            "within 1 ms"
        )


def _attr_json(span: Span, key: str, project: str, what: str) -> Any:
    try:
        return parse_json_attr(span.raw, key, span.source)
    except MalformedJsonAttr as error:
        raise EvidenceError(f"project {project}: malformed {what}: {error}") from error


def _trace_projects(spans: list[Span]) -> set[str]:
    projects: set[str] = set()
    mounted: dict[str, int] = {}
    for span in spans:
        if span.name not in (TEST_SPAN, PAGE_SPAN):
            continue
        project = span.project
        if not project:
            raise EvidenceError(f"{span.name} span has no e2e.project")
        projects.add(project)
        _dropped(span, project)
        navigations = _attr_json(
            span, "e2e.navigation_top_json", project, "navigation evidence"
        )
        if navigations is None:
            raise EvidenceError(f"project {project}: missing navigation evidence")
        if not isinstance(navigations, list):
            raise EvidenceError(
                f"project {project}: navigation evidence is not an array"
            )
        marks = _boot_marks(
            _attr_json(span, "e2e.boot_marks_json", project, "boot-mark evidence"),
            project,
        )
        for navigation in navigations:
            if not isinstance(navigation, dict):
                raise EvidenceError(
                    f"project {project}: navigation entry is not an object"
                )
            if "commitToMountMs" not in navigation:
                raise EvidenceError(
                    f"project {project}: navigation evidence has no commitToMountMs"
                )
            # The non-null mount proxy defines mounted membership only.
            if navigation["commitToMountMs"] is None:
                continue
            if field_f64(navigation, "commitToMountMs") is None:
                raise EvidenceError(
                    f"project {project}: navigation commitToMountMs is not null or "
                    "a finite number"
                )
            mounted[project] = mounted.get(project, 0) + 1
            _navigation(project, navigation, marks, span.name == PAGE_SPAN)
    for project in sorted(projects):
        if not mounted.get(project):
            raise EvidenceError(f"project {project}: no mounted navigation evidence")
    if not projects:
        raise EvidenceError(
            "trace capture has no navigation-bearing e2e.test or e2e.page spans"
        )
    return projects


def validate_boot_json(report_json: str, trace_jsonl: str) -> str:
    """Boot decomposition coverage over an in-memory report and trace."""
    expected = _report_projects(report_specs(report_json, "parsing Playwright report"))
    try:
        spans = parse_spans(trace_jsonl, Filters(), "trace capture")
    except EvidenceError as error:
        raise EvidenceError(f"parsing trace capture: {error}") from error
    observed = _trace_projects(spans)
    if expected != observed:
        raise EvidenceError(
            "project-set mismatch: missing trace projects "
            f"[{', '.join(sorted(expected - observed))}]; unexpected trace projects "
            f"[{', '.join(sorted(observed - expected))}]"
        )
    return f"boot-decomposition evidence complete for {len(expected)} project(s)"


def validate_boot_files(report: Path, capture: Path) -> str:
    """Boot decomposition coverage over a lane's lifted report and capture."""
    report_json = read(report, "Playwright report")
    if not report_json.strip():
        raise EvidenceError(f"Playwright report {report} is empty")
    try:
        trace = read_trace_member(capture)
    except EvidenceError as error:
        raise EvidenceError(f"reading capture {capture}: {error}") from error
    return validate_boot_json(report_json, trace)


# The VM phase sidecar: the lane's identity, and success of every phase that
# must have run.


def _phase_sidecar(document: dict[str, Any]) -> tuple[int, str, str, str, list[Any]]:
    phases = []
    for phase in objects(document, "phases"):
        name = required(phase, "name", STRING)
        outcome = required(phase, "outcome", STRING)
        if name not in PHASE_NAMES:
            raise EvidenceError(f"unknown phase `{name}`")
        if outcome not in PHASE_OUTCOMES:
            raise EvidenceError(f"unknown phase outcome `{outcome}`")
        duration = optional(phase, "duration_ms", COUNT)
        phases.append((name, duration, outcome, required(phase, "detail", STRING)))
    return (
        required(document, "schema_version", COUNT),
        required(document, "backend", STRING),
        required(document, "browser", STRING),
        required(document, "lane", STRING),
        phases,
    )


def _phase_sidecar_ok(path: Path, backend: str, browser: str, lane: str) -> None:
    version, sidecar_backend, sidecar_browser, sidecar_lane, phases = decode(
        read(path, "phase sidecar"), _phase_sidecar, f"parsing phase sidecar {path}"
    )
    if (version, sidecar_backend, sidecar_browser, sidecar_lane) != (
        1,
        backend,
        browser,
        lane,
    ):
        raise EvidenceError(f"{lane} phase sidecar: identity mismatch")
    for wanted in REQUIRED_PHASES:
        matches = [phase for phase in phases if phase[0] == wanted]
        if len(matches) != 1:
            raise EvidenceError(
                f"{lane} phase sidecar: requires exactly one {wanted} phase"
            )
        _, duration, outcome, detail = matches[0]
        if outcome != "success" or duration is None or not detail:
            raise EvidenceError(
                f"{lane} phase sidecar: {wanted} phase is incomplete or failed"
            )
    if any(phase[2] == "failed" for phase in phases):
        raise EvidenceError(f"{lane} phase sidecar: records a failed phase")


# Lane evidence, as producers lift it.


@dataclass(frozen=True)
class LaneEvidence:
    identity: str
    census: Path
    report: Path
    lane_manifest: Path
    duration_manifest: Path
    capture: Path
    phase: Path


def lane_evidence(identity: str, root: Path) -> LaneEvidence:
    """The lane's lifted files under `root`, named as every consumer names them."""
    return LaneEvidence(
        identity,
        root / f"e2e-census-{identity}.json",
        root / f"playwright-report-{identity}.json",
        root / f"e2e-lane-manifest-{identity}.json",
        root / f"duration-budget-manifest-{identity}.json",
        root / f"capture-{identity}.tar.gz",
        root / f"e2e-phase-{identity}.json",
    )


def verify_lane(identity: str, catalog: Path, root: Path) -> tuple[str, DurationBudget]:
    """One enabled lane's lifted evidence under `root`: its population, duration
    budget and boot decomposition coverage. The verdict, and the duration
    budget its producer reports."""
    lane = next(
        (
            lane
            for lane in read_catalog(catalog)
            if lane.enabled and lane.identity == identity
        ),
        None,
    )
    if lane is None:
        raise EvidenceError("unknown or disabled lane")
    evidence = lane_evidence(identity, root)
    validate_population(
        lane,
        read(evidence.report, "report"),
        read(evidence.lane_manifest, "manifest"),
    )
    duration = validate_duration_files(evidence.report, evidence.duration_manifest)
    validate_boot_files(evidence.report, evidence.capture)
    verdict = f"{identity}: population, retry-duration and boot evidence verified"
    if duration.pressured:
        verdict += f"\n{duration.detail()}"
    return verdict, duration


def _ownership(
    backend: str, browser: str, lanes: list[ExpectedLane], evidence: list[LaneEvidence]
) -> None:
    ownership = [
        (
            item.identity,
            read(item.census, "expected census"),
            read(item.report, "Playwright report"),
            read(item.lane_manifest, "lane manifest"),
        )
        for item in evidence
    ]
    try:
        reconcile_ownership(backend, browser, lanes, ownership)
    except EvidenceError as error:
        raise EvidenceError(f"ownership census: {error}") from error


def _collect[T](
    errors: list[str], prefix: str, check: Callable[..., T], *args: Any
) -> T | None:
    """Runs `check`, keeping its failure in `errors` rather than stopping."""
    try:
        return check(*args)
    except EvidenceError as error:
        errors.append(f"{prefix}{error}")
        return None


def reconcile(
    backend: str, browser: str, lanes: list[ExpectedLane], evidence: list[LaneEvidence]
) -> str:
    """One pair's lanes reconciled: ownership first, then each lane's own
    evidence. Every failure is collected, in that order, before refusing.
    Duration pressure is named, but each producer already reported it."""
    errors = []
    pressured = 0
    supplied = {item.identity for item in evidence}
    if len(supplied) != len(evidence):
        errors.append("duplicate lane evidence")
    _collect(errors, "", _ownership, backend, browser, lanes, evidence)
    # Not redundant with the ownership census: that one is skipped when a lane's
    # census, report or manifest cannot be read, and this mismatch must still
    # be reported.
    if supplied != {lane.identity for lane in pair_lanes(lanes, backend, browser)}:
        errors.append("lane evidence does not exactly match the catalog")
    by_identity = {lane.identity: lane for lane in lanes}
    for item in evidence:
        lane = by_identity.get(item.identity)
        if lane is None:
            continue
        if lane.backend != backend or lane.browser != browser or not lane.enabled:
            errors.append(f"unexpected lane `{item.identity}`")
            continue
        duration = _collect(
            errors,
            f"{item.identity} duration evidence: ",
            validate_duration_files,
            item.report,
            item.duration_manifest,
        )
        if duration is not None:
            pressured += len(duration.pressured)
        _collect(
            errors,
            f"{item.identity} trace evidence: ",
            validate_boot_files,
            item.report,
            item.capture,
        )
        _collect(
            errors, "", _phase_sidecar_ok, item.phase, backend, browser, item.identity
        )
    if errors:
        raise EvidenceError("\n".join(errors))
    verdict = (
        f"{backend}/{browser}: reconciled {len(evidence)} lane-qualified evidence sets"
    )
    if pressured:
        verdict += (
            f"; {pressured} attempt(s) under duration pressure, reported by "
            "their lane producers"
        )
    return verdict


def reconciled_lanes(
    backend: str, browser: str, catalog: Path, root: Path
) -> tuple[str, list[LaneEvidence]]:
    """One pair's lanes lifted under `root/e2e-<lane>/`, reconciled: the
    verdict and their evidence, only when every lane verified."""
    lanes = read_catalog(catalog)
    evidence = [
        lane_evidence(lane.identity, root / f"e2e-{lane.identity}")
        for lane in pair_lanes(lanes, backend, browser)
    ]
    return reconcile(backend, browser, lanes, evidence), evidence
