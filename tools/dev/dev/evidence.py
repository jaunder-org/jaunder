"""Lifted e2e evidence, decoded strictly: the refusal every e2e verdict and
trace diagnostic raises, the Playwright JSON report and the lane catalog.

The evidence is written by the e2e harness (`end2end/e2eLaneManifest.ts`,
`end2end/durationBudgetManifest.ts`), Playwright's JSON reporter and the VM
driver (`nix/e2e_capture.py`). A required field that is missing, or any field
holding the wrong kind of value, is a failure, never a default.
"""

import json
import math
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# The lane catalog, under the checkout root.
CATALOG = "end2end/e2e-lanes.json"


class EvidenceError(Exception):
    """Evidence that cannot support a verdict: unreadable, malformed or refused.

    The message names what was wrong; callers add where it was found.
    """


def _nonstandard(token: str) -> None:
    raise ValueError(f"`{token}` is not standard JSON")


def loads(raw: str | bytes) -> Any:
    """`raw` as standard JSON: a `NaN` or `Infinity` token is malformed, as it
    was to the Rust readers these verdicts replace."""
    return json.loads(raw, parse_constant=_nonstandard)


def finite(value: Any) -> float | None:
    """A JSON number as a finite float, else None (bool is not a number here)."""
    if isinstance(value, bool) or not isinstance(value, int | float):
        return None
    try:
        number = float(value)
    except OverflowError:
        return None
    return number if math.isfinite(number) else None


def read(path: Path, kind: str) -> str:
    """The text of the `kind` file at `path`."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, ValueError) as error:
        raise EvidenceError(f"reading {kind} {path}: {error}") from error


# Strict decoding: a field is read as one `Kind`, or the read names the field.


@dataclass(frozen=True)
class Kind[T]:
    """What a field must hold: `read` gives its value, or None if it holds
    anything else."""

    expected: str
    read: Callable[[Any], T | None]


STRING = Kind[str]("a string", lambda value: value if isinstance(value, str) else None)
BOOLEAN = Kind[bool](
    "a boolean", lambda value: value if isinstance(value, bool) else None
)
# A count, index or version; bool is an int to Python, never to the writers.
COUNT = Kind[int](
    "a non-negative integer",
    lambda value: value if type(value) is int and value >= 0 else None,
)
NUMBER = Kind[float]("a finite number", finite)
ARRAY = Kind[list[Any]](
    "an array", lambda value: value if isinstance(value, list) else None
)


def required[T](value: dict[str, Any], key: str, kind: Kind[T]) -> T:
    """`value[key]`, which must be present, not null, and of `kind`."""
    found = optional(value, key, kind)
    if found is None:
        raise EvidenceError(f"missing field `{key}`")
    return found


def optional[T](value: dict[str, Any], key: str, kind: Kind[T]) -> T | None:
    """`value[key]`, None when absent or null, otherwise of `kind`."""
    found = value.get(key)
    if found is None:
        return None
    read = kind.read(found)
    if read is None:
        raise EvidenceError(f"invalid type for `{key}`: expected {kind.expected}")
    return read


def document(value: Any) -> dict[str, Any]:
    """`value`, which must be a JSON object."""
    if not isinstance(value, dict):
        raise EvidenceError("invalid type: expected an object")
    return value


def objects(
    value: dict[str, Any], key: str, absent: bool = False
) -> list[dict[str, Any]]:
    """The required array of objects `value[key]`; with `absent`, one that may
    be missing (but not null)."""
    if absent and key not in value:
        return []
    return [document(item) for item in required(value, key, ARRAY)]


def decode[T](raw: str, decoder: Callable[[dict[str, Any]], T], context: str) -> T:
    """The JSON object `raw`, read by `decoder`; a failure is `context: why`."""
    try:
        return decoder(document(loads(raw)))
    except (ValueError, EvidenceError) as error:
        raise EvidenceError(f"{context}: {error}") from error


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


def _report_specs(report: dict[str, Any]) -> list[ReportSpec]:
    """Every spec of a report, nested suites included, in reporter order."""
    specs: list[ReportSpec] = []

    def suite(value: dict[str, Any]) -> None:
        children = objects(value, "suites", absent=True)
        for spec in objects(value, "specs"):
            tests = [
                ReportTest(
                    optional(test, "projectId", STRING),
                    optional(test, "projectName", STRING),
                    [
                        ReportAttempt(
                            optional(attempt, "retry", COUNT),
                            optional(attempt, "duration", NUMBER),
                        )
                        for attempt in objects(test, "results", absent=True)
                    ],
                )
                for test in objects(spec, "tests")
            ]
            specs.append(
                ReportSpec(
                    optional(spec, "id", STRING),
                    optional(spec, "title", STRING),
                    optional(spec, "file", STRING),
                    optional(spec, "line", COUNT),
                    tests,
                )
            )
        for child in children:
            suite(child)

    for child in objects(report, "suites", absent=True):
        suite(child)
    return specs


def report_specs(raw: str, context: str) -> list[ReportSpec]:
    """A Playwright JSON report's specs; a failure is `context: why`."""
    return decode(raw, _report_specs, context)


# The lane catalog: which lanes exist and what each must run.


@dataclass(frozen=True)
class ExpectedLane:
    """A lane's runtime expectations, not its build or scheduling."""

    backend: str
    browser: str
    shard_index: int | None
    shard_count: int | None
    identity: str
    enabled: bool


def _catalog(catalog: dict[str, Any]) -> tuple[int, list[ExpectedLane]]:
    version = required(catalog, "schemaVersion", COUNT)
    lanes = [
        ExpectedLane(
            required(lane, "backend", STRING),
            required(lane, "browser", STRING),
            optional(lane, "shardIndex", COUNT),
            optional(lane, "shardCount", COUNT),
            required(lane, "identity", STRING),
            required(lane, "enabled", BOOLEAN),
        )
        for lane in objects(catalog, "lanes")
    ]
    return version, lanes


def read_catalog(path: Path) -> list[ExpectedLane]:
    """The caller-selected lane catalog."""
    version, lanes = decode(read(path, "lane catalog"), _catalog, f"parsing {path}")
    if version != 1 or not lanes:
        raise EvidenceError("unsupported or empty ownership catalog")
    if len({lane.identity for lane in lanes}) != len(lanes):
        raise EvidenceError("duplicate ownership catalog identity")
    return lanes


def pair_lanes(
    catalog: list[ExpectedLane], backend: str, browser: str
) -> list[ExpectedLane]:
    """The catalog's enabled lanes of one backend/browser pair, in catalog order."""
    return [
        lane
        for lane in catalog
        if lane.enabled and (lane.backend, lane.browser) == (backend, browser)
    ]
