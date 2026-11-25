"""Fail-closed e2e verdicts over already-lifted lane evidence.

A lane's producer lifts its Playwright report, the lane manifest naming the
tests it selected, the duration-budget manifest, its capture and its VM phase
sidecar. `verify_lane` checks one lane as its producer finishes; `reconcile`
checks one backend/browser pair's lanes together against the lane catalog
and an independent census of the expected tests, and reports every failure
it finds, not just the first. Neither builds or runs anything.

Each input is decoded strictly, as its Rust writer types it: a missing field
or a value of the wrong type is a failure, not a default.
"""

import json
import math
import re
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from dev.traces import (
    BootOutcome,
    EvidenceError,
    Filters,
    MalformedJsonAttr,
    Span,
    as_i64,
    boot_decomposition_outcome,
    field_f64,
    get_attr,
    has_attr,
    page_boot_decomposition_outcome,
    parse_json_attr,
    parse_spans,
    read_trace_member,
)

# The utilization at or above which an attempt's duration fails the budget.
PRESSURE_THRESHOLD = 0.80
# The VM sidecar's phases that must each have succeeded once (`nix/e2e_capture.py`).
REQUIRED_PHASES = ("gate-execution", "result-lift", "post-gate-checks")
PHASE_NAMES = (
    "nix-evaluation",
    "nix-substitution",
    "nix-local-build",
    "vm-startup-readiness",
    *REQUIRED_PHASES,
)
PHASE_OUTCOMES = ("success", "failed", "unavailable")


# Strict decoding: each reader returns the field as its writer types it, or
# raises EvidenceError naming the field. An optional field may be absent or
# null; a required one may be neither.


def _object(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError("invalid type: expected an object")
    return value


def _present(value: dict[str, Any], key: str) -> Any:
    found = value.get(key)
    if found is None:
        raise EvidenceError(f"missing field `{key}`")
    return found


def _as_string(found: Any, key: str) -> str:
    if not isinstance(found, str):
        raise EvidenceError(f"invalid type for `{key}`: expected a string")
    return found


def _string(value: dict[str, Any], key: str) -> str:
    return _as_string(_present(value, key), key)


def _optional_string(value: dict[str, Any], key: str) -> str | None:
    found = value.get(key)
    return None if found is None else _as_string(found, key)


def _boolean(value: dict[str, Any], key: str) -> bool:
    found = _present(value, key)
    if not isinstance(found, bool):
        raise EvidenceError(f"invalid type for `{key}`: expected a boolean")
    return found


def _as_unsigned(found: Any, key: str, bits: int) -> int:
    if not (type(found) is int and 0 <= found < 2**bits):
        raise EvidenceError(f"invalid value for `{key}`: expected a u{bits}")
    return found


def _unsigned(value: dict[str, Any], key: str, bits: int) -> int:
    return _as_unsigned(_present(value, key), key, bits)


def _optional_unsigned(value: dict[str, Any], key: str, bits: int) -> int | None:
    found = value.get(key)
    return None if found is None else _as_unsigned(found, key, bits)


def _as_number(found: Any, key: str) -> float:
    if isinstance(found, bool) or not isinstance(found, int | float):
        raise EvidenceError(f"invalid type for `{key}`: expected a number")
    return float(found)


def _number(value: dict[str, Any], key: str) -> float:
    return _as_number(_present(value, key), key)


def _optional_number(value: dict[str, Any], key: str) -> float | None:
    found = value.get(key)
    return None if found is None else _as_number(found, key)


def _array(value: dict[str, Any], key: str, default: bool = False) -> list[Any]:
    """A required array, or with `default` one that may be absent (not null)."""
    if default and key not in value:
        return []
    found = _present(value, key)
    if not isinstance(found, list):
        raise EvidenceError(f"invalid type for `{key}`: expected an array")
    return found


def _decode[T](raw: str, decode: Callable[[Any], T]) -> T:
    """`raw` JSON decoded by `decode`, any failure an EvidenceError."""
    try:
        document = json.loads(raw)
    except ValueError as error:
        raise EvidenceError(str(error)) from error
    return decode(document)


def _read(path: Path, kind: str) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, ValueError) as error:
        raise EvidenceError(f"reading {kind} {path}: {error}") from error


# The Playwright JSON report, the authority for which tests and projects
# executed (ADR-0165).


@dataclass(frozen=True)
class ReportAttempt:
    retry: int | None
    duration: float | None


@dataclass(frozen=True)
class ReportTest:
    project_id: str | None
    project_name: str | None
    results: list[ReportAttempt]


@dataclass(frozen=True)
class ReportSpec:
    id: str | None
    title: str | None
    file: str | None
    line: int | None
    tests: list[ReportTest]


def _report_specs(document: Any) -> list[ReportSpec]:
    """Every spec of a report, nested suites included, in reporter order."""
    specs: list[ReportSpec] = []

    def suite(value: Any) -> None:
        value = _object(value)
        children = _array(value, "suites", default=True)
        for spec in _array(value, "specs"):
            spec = _object(spec)
            tests = [
                ReportTest(
                    _optional_string(test, "projectId"),
                    _optional_string(test, "projectName"),
                    [
                        ReportAttempt(
                            _optional_unsigned(attempt, "retry", 32),
                            _optional_number(attempt, "duration"),
                        )
                        for attempt in map(
                            _object, _array(test, "results", default=True)
                        )
                    ],
                )
                for test in map(_object, _array(spec, "tests"))
            ]
            specs.append(
                ReportSpec(
                    _optional_string(spec, "id"),
                    _optional_string(spec, "title"),
                    _optional_string(spec, "file"),
                    _optional_unsigned(spec, "line", 64),
                    tests,
                )
            )
        for child in children:
            suite(child)

    for child in _array(_object(document), "suites", default=True):
        suite(child)
    return specs


def report_specs(raw: str) -> list[ReportSpec]:
    """A Playwright JSON report's specs, decoded strictly."""
    return _decode(raw, _report_specs)


# The lane catalog and ownership: which lanes must exist, and that their
# reports together execute exactly the expected tests, each once.


@dataclass(frozen=True)
class ExpectedLane:
    """A lane's runtime expectations, not its build or scheduling."""

    backend: str
    browser: str
    shard_index: int | None
    shard_count: int | None
    identity: str
    enabled: bool


def _catalog(document: Any) -> tuple[int, list[ExpectedLane]]:
    document = _object(document)
    version = _unsigned(document, "schemaVersion", 8)
    lanes = [
        ExpectedLane(
            _string(lane, "backend"),
            _string(lane, "browser"),
            _optional_unsigned(lane, "shardIndex", 8),
            _optional_unsigned(lane, "shardCount", 8),
            _string(lane, "identity"),
            _boolean(lane, "enabled"),
        )
        for lane in map(_object, _array(document, "lanes"))
    ]
    return version, lanes


def read_catalog(path: Path) -> list[ExpectedLane]:
    """The caller-selected lane catalog."""
    try:
        raw = path.read_text(encoding="utf-8")
    except (OSError, ValueError) as error:
        raise EvidenceError(f"reading {path}: {error}") from error
    try:
        version, lanes = _decode(raw, _catalog)
    except EvidenceError as error:
        raise EvidenceError(f"parsing {path}: {error}") from error
    if version != 1 or not lanes:
        raise EvidenceError("unsupported or empty ownership catalog")
    if len({lane.identity for lane in lanes}) != len(lanes):
        raise EvidenceError("duplicate ownership catalog identity")
    return lanes


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
            _string(test, "project_id"),
            _string(test, "project_name"),
            _string(test, "test_id"),
        )
        for test in map(_object, _array(document, "tests"))
    ]


def _census(document: Any) -> _Census:
    document = _object(document)
    return _Census(
        _unsigned(document, "schema_version", 8),
        _boolean(document, "complete"),
        _string(document, "topology"),
        _tests(document),
    )


def _lane_manifest(document: Any) -> _LaneManifest:
    document = _object(document)
    return _LaneManifest(
        _unsigned(document, "schema_version", 8),
        _boolean(document, "complete"),
        _string(document, "lane"),
        _string(document, "backend"),
        _string(document, "browser"),
        _optional_unsigned(document, "shard_index", 8),
        _optional_unsigned(document, "shard_count", 8),
        _tests(document),
    )


def _decoded[T](raw: str, decode: Callable[[Any], T], what: str) -> T:
    try:
        return _decode(raw, decode)
    except EvidenceError as error:
        raise EvidenceError(f"malformed {what}: {error}") from error


def _identities(tests: list[Identity], label: str) -> set[Identity]:
    if any(not part for test in tests for part in test):
        raise EvidenceError(f"malformed {label} identity")
    identities = set(tests)
    if len(identities) != len(tests):
        raise EvidenceError(f"duplicate {label} identity")
    return identities


def _report_identities(raw: str, lane: str) -> set[Identity]:
    specs = _decoded(raw, _report_specs, f"report for `{lane}`")
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


def _names_lane(
    manifest: _LaneManifest, lane: ExpectedLane, backend: str, browser: str
) -> bool:
    """Whether a complete current manifest is the one `lane`'s producer wrote."""
    return (
        manifest.schema_version == 1
        and manifest.complete
        and manifest.lane == lane.identity
        and manifest.backend == backend
        and manifest.browser == browser
        and manifest.shard_index == lane.shard_index
        and manifest.shard_count == lane.shard_count
    )


def validate_population(lane: ExpectedLane, report_raw: str, manifest_raw: str) -> None:
    """The producer's manifest and report agree on the lane's population.

    That pair defines the runtime population (ADR-0165), which must all belong
    to the lane's one project, named for its browser.
    """
    manifest = _decoded(manifest_raw, _lane_manifest, "lane manifest")
    if not _names_lane(manifest, lane, lane.backend, lane.browser):
        raise EvidenceError("mismatched or incomplete lane manifest")
    selected = _identities(manifest.tests, "lane manifest")
    reported = _report_identities(report_raw, lane.identity)
    if not selected or selected != reported:
        raise EvidenceError("lane manifest/report population mismatch")
    if any(project != lane.browser for _, project, _ in selected):
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
    lanes = [
        lane
        for lane in catalog
        if lane.backend == backend and lane.browser == browser and lane.enabled
    ]
    supplied = {identity for identity, *_ in evidence}
    # The catalog's shape is checked by `scripts/e2e-matrix.jq`; here the
    # evidence must name exactly the pair's enabled lanes.
    if (
        not lanes
        or {lane.identity for lane in lanes} != supplied
        or len(evidence) != len(supplied)
    ):
        raise EvidenceError("shard lane evidence does not exactly match the catalog")
    expected_all: set[Identity] | None = None
    observed: set[Identity] = set()
    for identity, census_raw, report_raw, manifest_raw in evidence:
        lane = next(lane for lane in lanes if lane.identity == identity)
        census = _decoded(census_raw, _census, f"expected census for `{identity}`")
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
        if any(project != browser for _, project, _ in expected):
            raise EvidenceError("expected census has unowned project")
        if expected_all is None:
            expected_all = expected
        elif expected_all != expected:
            raise EvidenceError("lifted expected censuses are not identical")
        manifest = _decoded(
            manifest_raw, _lane_manifest, f"lane manifest for `{identity}`"
        )
        if not _names_lane(manifest, lane, backend, browser):
            raise EvidenceError(f"mismatched lane manifest for `{identity}`")
        selected = _identities(manifest.tests, "lane manifest")
        if not selected or any(project != browser for _, project, _ in selected):
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
# each attempt's effective timeout, the report owns measured durations. A lane
# passes only when their identities and retry streams agree exactly and no
# attempt reaches the pressure threshold.


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


# Each selected test's attempts: retry → effective timeout or duration, ms.
_Attempts = dict[_TestIdentity, dict[int, float]]
# A duration manifest's tests: each identity with its (retry, timeout) attempts.
_ManifestTests = list[tuple[_TestIdentity, list[tuple[int, float]]]]


def _duration_manifest(document: Any) -> tuple[int, bool, _ManifestTests]:
    document = _object(document)
    return (
        _unsigned(document, "schema_version", 32),
        _boolean(document, "complete"),
        [
            (
                _TestIdentity(
                    _string(test, "test_id"),
                    _string(test, "project_id"),
                    _string(test, "project_name"),
                    _string(test, "title"),
                    _string(test, "file"),
                    _unsigned(test, "line", 64),
                ),
                [
                    (
                        _unsigned(attempt, "retry", 32),
                        _number(attempt, "effective_timeout_ms"),
                    )
                    for attempt in map(_object, _array(test, "attempts"))
                ],
            )
            for test in map(_object, _array(document, "tests"))
        ],
    )


def _contiguous(
    retries: dict[int, float], source: str, identity: _TestIdentity
) -> None:
    for expected, actual in enumerate(sorted(retries)):
        if actual != expected:
            raise EvidenceError(
                f"{source} retries must start at zero and be contiguous; expected "
                f"{expected}, found {actual}: {identity.describe()}"
            )


def _manifest_attempts(tests: _ManifestTests) -> _Attempts:
    if not tests:
        raise EvidenceError("manifest selects no tests")
    expected: _Attempts = {}
    for identity, attempts in tests:
        identity.validate("manifest test")
        if not attempts:
            raise EvidenceError(f"manifest test has no attempts: {identity.describe()}")
        timeouts: dict[int, float] = {}
        for retry, timeout in attempts:
            if not math.isfinite(timeout) or timeout <= 0:
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


def _report_attempts(specs: list[ReportSpec]) -> _Attempts:
    if not specs:
        raise EvidenceError("report contains no selected tests")
    observed: _Attempts = {}
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
            durations: dict[int, float] = {}
            for attempt in test.results:
                retry, duration = attempt.retry, attempt.duration
                if retry is None:
                    raise EvidenceError(
                        f"report attempt has no retry: {identity.describe()}"
                    )
                if duration is None:
                    raise EvidenceError(
                        f"report retry {retry} has no duration: {identity.describe()}"
                    )
                if not math.isfinite(duration) or duration < 0:
                    raise EvidenceError(
                        f"report retry {retry} has a non-finite or negative "
                        f"duration: {identity.describe()}"
                    )
                if retry in durations:
                    raise EvidenceError(
                        f"report has duplicate retry {retry}: {identity.describe()}"
                    )
                durations[retry] = duration
            _contiguous(durations, "report", identity)
            if identity in observed:
                raise EvidenceError(
                    f"report has a duplicate selected test: {identity.describe()}"
                )
            observed[identity] = durations
    return observed


def _pressure(expected: _Attempts, observed: _Attempts) -> str:
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
    # The greatest utilization; `max` keeps the first in identity and retry
    # order on a tie.
    utilization, identity, retry, duration, timeout = max(
        (
            (
                observed[identity][retry] / timeout,
                identity,
                retry,
                observed[identity][retry],
                timeout,
            )
            for identity in sorted(expected)
            for retry, timeout in sorted(expected[identity].items())
        ),
        key=lambda attempt: attempt[0],
    )
    if utilization >= PRESSURE_THRESHOLD:
        raise EvidenceError(
            f"duration pressure {utilization * 100:.2f}% (threshold 80.00%): "
            f"{identity.describe()}; retry {retry}; duration {duration:.3f} ms; "
            f"effective timeout {timeout:.3f} ms"
        )
    return (
        f"maximum duration pressure {utilization * 100:.2f}%: {identity.describe()}; "
        f"retry {retry}; duration {duration:.3f} ms / effective timeout "
        f"{timeout:.3f} ms"
    )


def validate_duration_json(report_json: str, manifest_json: str) -> str:
    """The duration budget over in-memory report and manifest JSON."""
    try:
        specs = report_specs(report_json)
    except EvidenceError as error:
        raise EvidenceError(f"parsing Playwright report: {error}") from error
    try:
        version, complete, tests = _decode(manifest_json, _duration_manifest)
    except EvidenceError as error:
        raise EvidenceError(f"parsing duration-budget manifest: {error}") from error
    if version != 1:
        raise EvidenceError(
            f"unsupported manifest schema_version {version}; expected 1"
        )
    if not complete:
        raise EvidenceError("manifest is not complete")
    return _pressure(_manifest_attempts(tests), _report_attempts(specs))


def validate_duration_files(report: Path, manifest: Path) -> str:
    """The duration budget over a lane's lifted report and duration manifest."""
    return validate_duration_json(_read(report, "report"), _read(manifest, "manifest"))


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
    value = get_attr(span.raw, "e2e.navigation_top_dropped")
    if not re.fullmatch(r"\+?[0-9]+", value) or int(value) >= 2**64:
        raise EvidenceError(
            f"project {project}: malformed dropped-navigation count {json.dumps(value)}"
        )
    if int(value):
        raise EvidenceError(
            f"project {project}: {int(value)} dropped navigation record(s)"
        )


def _marks_by_navigation(marks: Any, project: str) -> dict[int, list[Any]]:
    if marks is None:
        return {}
    if not isinstance(marks, list):
        raise EvidenceError(f"project {project}: boot-mark evidence is not an array")
    by_navigation: dict[int, list[Any]] = {}
    for entry in marks:
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
        if identity in by_navigation:
            raise EvidenceError(
                f"project {project}: duplicate boot-mark navigation id {identity}"
            )
        by_navigation[identity] = records
    return by_navigation


def _navigation(
    project: str, navigation: Any, marks: dict[int, list[Any]], page: bool
) -> None:
    if navigation.get("wasmTimingSchema") != "direct-init-v1":
        raise EvidenceError(f"project {project}: legacy or missing timing schema")
    # The shared decomposition owns both the boot-interval floor and closure,
    # so this gate cannot count unrelated properties.
    identity = as_i64(navigation.get("id"))
    own = [] if identity is None else marks.get(identity, [])
    classify = page_boot_decomposition_outcome if page else boot_decomposition_outcome
    outcome = classify(navigation, own)
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
        if span.name not in ("e2e.test", "e2e.page"):
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
        marks = _marks_by_navigation(
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
            _navigation(project, navigation, marks, span.name == "e2e.page")
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
    try:
        specs = report_specs(report_json)
    except EvidenceError as error:
        raise EvidenceError(f"parsing Playwright report: {error}") from error
    expected = _report_projects(specs)
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
    report_json = _read(report, "Playwright report")
    if not report_json.strip():
        raise EvidenceError(f"Playwright report {report} is empty")
    try:
        trace = read_trace_member(capture)
    except EvidenceError as error:
        raise EvidenceError(f"reading capture {capture}: {error}") from error
    return validate_boot_json(report_json, trace)


# The VM phase sidecar: the lane's identity, and success of every phase that
# must have run.


def _phase_sidecar(document: Any) -> tuple[int, str, str, str, list[Any]]:
    document = _object(document)
    phases = []
    for phase in map(_object, _array(document, "phases")):
        name, outcome = _string(phase, "name"), _string(phase, "outcome")
        if name not in PHASE_NAMES:
            raise EvidenceError(f"unknown phase `{name}`")
        if outcome not in PHASE_OUTCOMES:
            raise EvidenceError(f"unknown phase outcome `{outcome}`")
        duration = _optional_unsigned(phase, "duration_ms", 128)
        phases.append((name, duration, outcome, _string(phase, "detail")))
    return (
        _unsigned(document, "schema_version", 8),
        _string(document, "backend"),
        _string(document, "browser"),
        _string(document, "lane"),
        phases,
    )


def _phase_sidecar_ok(path: Path, backend: str, browser: str, lane: str) -> None:
    raw = _read(path, "phase sidecar")
    try:
        version, sidecar_backend, sidecar_browser, sidecar_lane, phases = _decode(
            raw, _phase_sidecar
        )
    except EvidenceError as error:
        raise EvidenceError(f"parsing phase sidecar {path}: {error}") from error
    if (version, sidecar_backend, sidecar_browser, sidecar_lane) != (
        1,
        backend,
        browser,
        lane,
    ):
        raise EvidenceError(f"{lane} phase sidecar: identity mismatch")
    for required in REQUIRED_PHASES:
        matches = [phase for phase in phases if phase[0] == required]
        if len(matches) != 1:
            raise EvidenceError(
                f"{lane} phase sidecar: requires exactly one {required} phase"
            )
        _, duration, outcome, detail = matches[0]
        if outcome != "success" or duration is None or not detail:
            raise EvidenceError(
                f"{lane} phase sidecar: {required} phase is incomplete or failed"
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


def verify_lane(identity: str, catalog: Path, root: Path) -> str:
    """One enabled lane's lifted evidence under `root`: its population, duration
    budget and boot decomposition coverage."""
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
        _read(evidence.report, "report"),
        _read(evidence.lane_manifest, "manifest"),
    )
    validate_duration_files(evidence.report, evidence.duration_manifest)
    validate_boot_files(evidence.report, evidence.capture)
    return f"{identity}: population, retry-duration and boot evidence verified"


def _ownership(
    backend: str, browser: str, lanes: list[ExpectedLane], evidence: list[LaneEvidence]
) -> None:
    ownership = [
        (
            item.identity,
            _read(item.census, "expected census"),
            _read(item.report, "Playwright report"),
            _read(item.lane_manifest, "lane manifest"),
        )
        for item in evidence
    ]
    try:
        reconcile_ownership(backend, browser, lanes, ownership)
    except EvidenceError as error:
        raise EvidenceError(f"ownership census: {error}") from error


def _collect(
    errors: list[str], prefix: str, check: Callable[..., object], *args: Any
) -> None:
    """Runs `check`, keeping its failure in `errors` rather than stopping."""
    try:
        check(*args)
    except EvidenceError as error:
        errors.append(f"{prefix}{error}")


def reconcile(
    backend: str, browser: str, lanes: list[ExpectedLane], evidence: list[LaneEvidence]
) -> str:
    """One pair's lanes reconciled: ownership first, then each lane's own
    evidence. Every failure is collected, in that order, before refusing."""
    errors = []
    supplied = {item.identity for item in evidence}
    if len(supplied) != len(evidence):
        errors.append("duplicate lane evidence")
    _collect(errors, "", _ownership, backend, browser, lanes, evidence)
    expected = {
        lane.identity
        for lane in lanes
        if lane.backend == backend and lane.browser == browser and lane.enabled
    }
    if supplied != expected:
        errors.append("lane evidence does not exactly match the catalog")
    by_identity = {lane.identity: lane for lane in lanes}
    for item in evidence:
        lane = by_identity.get(item.identity)
        if lane is None:
            continue
        if lane.backend != backend or lane.browser != browser or not lane.enabled:
            errors.append(f"unexpected lane `{item.identity}`")
            continue
        _collect(
            errors,
            f"{item.identity} duration evidence: ",
            validate_duration_files,
            item.report,
            item.duration_manifest,
        )
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
    return (
        f"{backend}/{browser}: reconciled {len(evidence)} lane-qualified evidence sets"
    )


def reconciled_lanes(
    backend: str, browser: str, catalog: Path, root: Path
) -> tuple[str, list[LaneEvidence]]:
    """One pair's lanes lifted under `root/e2e-<lane>/`, reconciled: the
    verdict and their evidence, only when every lane verified."""
    lanes = read_catalog(catalog)
    evidence = [
        lane_evidence(lane.identity, root / f"e2e-{lane.identity}")
        for lane in lanes
        if lane.enabled and lane.backend == backend and lane.browser == browser
    ]
    return reconcile(backend, browser, lanes, evidence), evidence
