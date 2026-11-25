"""The trace core: the one reader of e2e captures and their OTLP spans.

A capture is the `capture-*.tar.gz` bundle an e2e lane lifts: the collector's
`otel-traces.jsonl` beside one `diag.log` per server instance. This module
reads it fail-closed, flattens the OTLP JSONL into `Span`s, reads span
attributes, and classifies a navigation's boot decomposition (ADR-0165), so
every e2e verdict and trace diagnostic applies the same parsing and schema.

A parsed span keeps its start and end Unix nanoseconds and its span and parent
ids, which is what a timeline view (such as a Perfetto track) needs.
"""

import json
import math
import re
import tarfile
import urllib.parse
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import IO, Any, cast

# The capture layout `host::capture` writes. Each spelling is paired with its
# Rust counterpart in the cross-language literal table (ADR-0109), so the two
# cannot drift apart silently.
INSTANCE_PREFIX = "instance-"
DIAG_LOG = "diag.log"
TRACE_FILE = "otel-traces.jsonl"
# The bundle's root directory: it is made by `tar … -C /var/lib/jaunder capture`.
CAPTURE_ROOT = "capture"
TRACE_MEMBER = f"{CAPTURE_ROOT}/{TRACE_FILE}"

# A timestamp written as text is read within the 128-bit range; one written as
# a JSON number must fit int64.
_I128 = 2**127


class EvidenceError(Exception):
    """Evidence that cannot support a verdict: unreadable, malformed or refused.

    The message names what was wrong; callers add where it was found.
    """


class MalformedJsonAttr(EvidenceError):
    """A present `e2e.*_json` attribute whose string is not valid JSON.

    Its own type, because this correctness failure is the one a trace
    diagnostic reports as a failed result rather than a refusal.
    """

    def __init__(self, key: str, span_source: str, cause: ValueError) -> None:
        super().__init__(f"parsing JSON trace attribute {key} from {span_source}")
        self.key = key
        self.span_source = span_source
        self.cause = cause

    def __str__(self) -> str:
        return f"{self.args[0]}: {self.cause}"


def instance_directory(index: int) -> str:
    """The capture subdirectory server instance `index` writes its streams to."""
    return f"{INSTANCE_PREFIX}{index}"


def instance_index(name: str) -> int | None:
    """The instance a capture subdirectory name denotes, or None."""
    digits = name.removeprefix(INSTANCE_PREFIX)
    if digits == name or not digits or not (digits.isascii() and digits.isdigit()):
        return None
    return int(digits)


def instance_diag_member(index: int) -> str:
    """Instance `index`'s WARN-and-above diagnostic log within a capture.

    Each Playwright worker's server writes its own (#2076). It may be empty: a
    clean run warns about nothing.
    """
    return f"{CAPTURE_ROOT}/{instance_directory(index)}/{DIAG_LOG}"


def _instance_diag_index(member: str) -> int | None:
    """The instance whose diagnostic log `member` is, the inverse of
    `instance_diag_member`."""
    parts = member.split("/", 2)
    if len(parts) != 3 or parts[0] != CAPTURE_ROOT or parts[2] != DIAG_LOG:
        return None
    return instance_index(parts[1])


def read_capture_members(
    capture: Path, wanted: Callable[[str], bool]
) -> dict[str, str]:
    """Every regular-file member whose path `wanted(path)` accepts, as text.

    Every capture consumer reads through here, so malformed evidence fails the
    same way everywhere: a selected member that is not a regular file, that
    appears twice, or that is not UTF-8 is an error.
    """
    try:
        file = capture.open("rb")
    except OSError as error:
        raise EvidenceError(f"opening capture {capture}: {error}") from error
    found: dict[str, str] = {}
    with file:
        try:
            with tarfile.open(fileobj=file, mode="r|gz") as archive:
                for entry in archive:
                    if not wanted(entry.name):
                        continue
                    if not entry.isfile():
                        raise EvidenceError(
                            f"capture member `{entry.name}` is not a regular file"
                        )
                    if entry.name in found:
                        raise EvidenceError(
                            f"capture has ambiguous member `{entry.name}`"
                        )
                    # A regular file always has content to extract.
                    content = cast(IO[bytes], archive.extractfile(entry))
                    try:
                        found[entry.name] = content.read().decode()
                    except UnicodeDecodeError as error:
                        raise EvidenceError(
                            f"reading capture member `{entry.name}`: {error}"
                        ) from error
        except (tarfile.TarError, OSError, EOFError) as error:
            raise EvidenceError(f"malformed capture archive: {error}") from error
    return found


def read_capture_member(capture: Path, member: str) -> str:
    """The single required `member` of a capture."""
    found = read_capture_members(capture, lambda path: path == member)
    if member not in found:
        raise EvidenceError(f"capture member `{member}` is missing from {capture}")
    return found[member]


def read_trace_member(capture: Path) -> str:
    """The capture's trace, which unlike its other members must not be empty."""
    trace = read_capture_member(capture, TRACE_MEMBER)
    if not trace.strip():
        raise EvidenceError(f"capture member `{TRACE_MEMBER}` is empty")
    return trace


def read_instance_diag_logs(capture: Path) -> list[str]:
    """Every server instance's `diag.log`, in instance order.

    Instances must be numbered contiguously from 0, so a lost instance's log
    cannot pass unnoticed, and at least one must exist.
    """
    logs = read_capture_members(
        capture, lambda path: _instance_diag_index(path) is not None
    )
    if not logs:
        raise EvidenceError(
            f"capture has no `{instance_diag_member(0)}` member: {capture}"
        )
    ordered = []
    for index in range(len(logs)):
        member = instance_diag_member(index)
        if member not in logs:
            raise EvidenceError(f"capture member `{member}` is missing from {capture}")
        ordered.append(logs[member])
    return ordered


def member(value: Any, key: str) -> Any:
    """`value[key]` when `value` is a JSON object holding it, else None."""
    return value.get(key) if isinstance(value, dict) else None


def as_i64(value: Any) -> int | None:
    """A JSON integer within int64, else None (bool is not an integer here)."""
    if type(value) is int and -(2**63) <= value < 2**63:
        return value
    return None


def as_f64(value: Any) -> float | None:
    """A JSON number as a float, else None (bool is not a number here)."""
    if isinstance(value, bool) or not isinstance(value, int | float):
        return None
    return float(value)


def field_f64(value: Any, key: str) -> float | None:
    """A finite number field of a JSON object, else None."""
    number = as_f64(member(value, key))
    return number if number is not None and math.isfinite(number) else None


def get_attr(span: Any, key: str) -> str:
    """A string attribute of a span's `attributes[]`: its `stringValue`, else
    its `intValue` as text (OTLP encodes int64 as a number or a string), else
    the empty string."""
    attributes = member(span, "attributes")
    for attribute in attributes if isinstance(attributes, list) else []:
        if member(attribute, "key") != key:
            continue
        value = member(attribute, "value")
        text = member(value, "stringValue")
        if isinstance(text, str):
            return text
        number = member(value, "intValue")
        if as_i64(number) is not None:
            return str(number)
        return number if isinstance(number, str) else ""
    return ""


def has_attr(span: Any, key: str) -> bool:
    """Whether a span's `attributes[]` declares `key` at all."""
    attributes = member(span, "attributes")
    return isinstance(attributes, list) and any(
        member(attribute, "key") == key for attribute in attributes
    )


def parse_json_attr(span: Any, key: str, span_source: str) -> Any | None:
    """A JSON-string attribute (the `e2e.*_json` blobs), parsed.

    An absent attribute is None; a present one that is not JSON raises
    `MalformedJsonAttr`.
    """
    if not has_attr(span, key):
        return None
    try:
        return json.loads(get_attr(span, key))
    except ValueError as error:
        raise MalformedJsonAttr(key, span_source, error) from error


def _nanos(value: Any) -> int | None:
    """An OTLP nanosecond timestamp, encoded as a JSON string or integer."""
    if not isinstance(value, str):
        return as_i64(value)
    if not re.fullmatch(r"[+-]?[0-9]+", value):
        return None
    number = int(value)
    return number if -_I128 <= number < _I128 else None


def parse_duration_ms(span: Any) -> float:
    """`endTimeUnixNano - startTimeUnixNano` in milliseconds; a missing or
    unparseable bound counts as 0, so a malformed span degrades rather than
    aborting the read. The difference saturates at the 128-bit range."""
    start = _nanos(member(span, "startTimeUnixNano")) or 0
    end = _nanos(member(span, "endTimeUnixNano")) or 0
    return max(-_I128, min(end - start, _I128 - 1)) / 1_000_000


def span_interval_ms(raw: Any) -> tuple[float, float] | None:
    """A span's `[start, end)` in milliseconds, or None when either bound is
    absent, unparseable, not finite, or the interval is inverted.

    Excluding a malformed span keeps it from contributing a zero-length or
    wildly wrong interval to a union.
    """

    def millis(key: str) -> float | None:
        value = member(raw, key)
        if isinstance(value, str):
            try:
                return float(value) / 1_000_000
            except ValueError:
                return None
        number = as_f64(value)
        return None if number is None else number / 1_000_000

    start, end = millis("startTimeUnixNano"), millis("endTimeUnixNano")
    if start is None or end is None:
        return None
    if not (math.isfinite(start) and math.isfinite(end)) or end < start:
        return None
    return start, end


def interval_union_ms(intervals: list[tuple[float, float]]) -> float:
    """The total length `intervals` cover, overlaps counted once."""
    total = 0.0
    current: tuple[float, float] | None = None
    for start, end in sorted(intervals, key=lambda interval: interval[0]):
        if current is None:
            current = (start, end)
        elif start > current[1]:
            total += current[1] - current[0]
            current = (start, end)
        elif end > current[1]:
            current = (current[0], end)
    return total if current is None else total + (current[1] - current[0])


def to_url_path(value: str) -> str:
    """A URL as `host[:port]/path`, the port only when it is not the scheme's
    default and the path at least `/` for a web URL; a value with no scheme is
    returned unchanged."""
    if not value:
        return ""
    try:
        parts = urllib.parse.urlsplit(value)
        port = parts.port
    except ValueError:
        return value
    if not parts.scheme:
        return value
    default = {"http": 80, "https": 443, "ws": 80, "wss": 443, "ftp": 21}
    web = parts.scheme in {*default, "file"}
    shown = "" if port is None or port == default.get(parts.scheme) else f":{port}"
    path = parts.path or ("/" if web else "")
    return f"{parts.hostname or ''}{shown}{path}"


@dataclass(frozen=True)
class Filters:
    """The span filters a trace analysis accepts."""

    trace: str | None = None
    project: str | None = None


@dataclass
class Span:
    """One flattened OTLP span: the scalar fields the e2e verdicts and trace
    reports read, the trace file it came from, and the raw span for on-demand
    attribute reads. Absent ids and attributes are the empty string; absent or
    unparseable timestamps are None."""

    trace_id: str
    span_id: str
    parent_span_id: str
    name: str
    method: str
    uri: str
    project: str
    busy_ns: str
    idle_ns: str
    start_unix_nano: int | None
    end_unix_nano: int | None
    duration_ms: float
    source: str
    raw: dict[str, Any]


def _passes(span: dict[str, Any], name: str, project: str, filters: Filters) -> bool:
    """The trace filter drops a span of another trace; the project filter drops
    only an `e2e.` span of another project, never an HTTP or server span."""
    if filters.trace is not None and _string(span, "traceId") != filters.trace:
        return False
    return not (
        filters.project is not None
        and name.startswith("e2e.")
        and project != filters.project
    )


def _string(span: dict[str, Any], key: str) -> str:
    value = span.get(key)
    return value if isinstance(value, str) else ""


def _array(value: Any, field: str, where: str) -> list[Any]:
    found = member(value, field)
    if not isinstance(found, list):
        raise EvidenceError(f"{where} has no {field} array")
    return found


def parse_spans(content: str, filters: Filters, source: str) -> list[Span]:
    """The spans of OTLP JSONL `content` that pass `filters`.

    `source` names the content in errors and labels each span. Blank lines
    are skipped; a malformed line, structure or span is an error.
    """
    spans = []
    for number, line in enumerate(content.split("\n"), 1):
        line = line.removesuffix("\r")
        if not line.strip():
            continue
        where = f"JSON line {number} in {source}"
        try:
            record = json.loads(line)
        except ValueError as error:
            raise EvidenceError(f"failed to parse {where}: {error}") from error
        for resource in _array(record, "resourceSpans", where):
            for scope in _array(resource, "scopeSpans", where):
                for span in _array(scope, "spans", where):
                    if not isinstance(span, dict):
                        raise EvidenceError(
                            f"{where} has a span entry that is not an object"
                        )
                    name = span.get("name")
                    if not isinstance(name, str) or not name:
                        raise EvidenceError(
                            f"{where} has a span entry without a nonempty string name"
                        )
                    project = get_attr(span, "e2e.project")
                    if not _passes(span, name, project, filters):
                        continue
                    spans.append(
                        Span(
                            trace_id=_string(span, "traceId"),
                            span_id=_string(span, "spanId"),
                            parent_span_id=_string(span, "parentSpanId"),
                            name=name,
                            method=get_attr(span, "method"),
                            uri=get_attr(span, "uri"),
                            project=project,
                            busy_ns=get_attr(span, "busy_ns"),
                            idle_ns=get_attr(span, "idle_ns"),
                            start_unix_nano=_nanos(span.get("startTimeUnixNano")),
                            end_unix_nano=_nanos(span.get("endTimeUnixNano")),
                            duration_ms=parse_duration_ms(span),
                            source=source,
                            raw=span,
                        )
                    )
    return spans


def read_spans(path: Path, filters: Filters) -> list[Span]:
    """`parse_spans` over a file; errors name the path."""
    try:
        content = path.read_text(encoding="utf-8")
    except (OSError, ValueError) as error:
        raise EvidenceError(f"reading trace file {path}: {error}") from error
    return parse_spans(content, filters, str(path))


# Boot decomposition (ADR-0165, spec D5/D8).
#
# Every decomposed segment is document-relative (`performance.timeOrigin`), so
# the target is `mount_done.startTime`, never the Node-side `commitToMountMs`:
# mixing the two frames charges event-delivery latency to the app's boot.

# The two exclusive pre-Rust-boot segments, as (label, navigation field).
EXCLUSIVE_SEGMENTS = (
    ("wasm_init_start", "wasmInitStartMs"),
    ("wasm_init_start_to_boot_entry", "wasmInitStartToBootEntryMs"),
)
# The fewest finite `boot.*` intervals a decomposition uses: a floor, so new
# marks extend it rather than break it.
MIN_BOOT_PHASES = 3
# How far the segments may miss the boot total before a navigation is a closure
# violation. They close exactly by construction, so this is float slack.
CLOSURE_TOLERANCE_MS = 1.0
# The mark ending the boot total, matched by suffix: only the tail is
# contractual (the names live in `client::perf`).
MOUNT_DONE_SUFFIX = ".boot.mount_done"
# A `bootPhases` key belongs to the decomposition iff it mentions a boot mark:
# selected by substring, never by position or count.
BOOT_INTERVAL_KEY = "boot."
# The timing schema of a navigation that can be decomposed.
CURRENT_TIMING_SCHEMA = "direct-init-v1"


class BootOutcome(Enum):
    """Whether a navigation has a complete, closing document-frame decomposition."""

    INCOMPLETE = "incomplete"
    CLOSURE_VIOLATION = "closure-violation"
    COMPLETE = "complete"


@dataclass(frozen=True)
class Decomposition:
    """One navigation's ordered segments and the boot total they close on,
    with its wall-clock `commitToMountMs`, which is reported, never decomposed."""

    segments: list[tuple[str, float]]
    boot_total_ms: float
    commit_to_mount_ms: float | None


def plain_sum(values: Sequence[float]) -> float:
    """`values` added plainly left to right.

    Python's `sum` compensates float rounding (3.12+), while the Rust tools
    these verdicts and reports port summed naively: the boot closure verdict at
    its 1 ms tolerance edge and a reported mean's last digit must not move.
    """
    total = 0.0
    for value in values:
        total += value
    return total


def decompose(
    navigation: Any, marks: list[Any], page_total: float | None = None
) -> Decomposition | BootOutcome:
    """Decompose one navigation against its own boot marks.

    Returns the decomposition, or `INCOMPLETE` without a full mark set and
    `CLOSURE_VIOLATION` when the segments miss the boot total. `page_total`,
    supplied only for page-span evidence, stands in for a missing `mount_done`
    mark.
    """
    if member(navigation, "wasmTimingSchema") != CURRENT_TIMING_SCHEMA:
        return BootOutcome.INCOMPLETE
    starts: dict[str, float] = {}
    for mark in marks:
        name, start = member(mark, "name"), field_f64(mark, "startTime")
        if isinstance(name, str) and start is not None:
            starts[name] = start
    boot_total = next(
        (starts[name] for name in sorted(starts) if name.endswith(MOUNT_DONE_SUFFIX)),
        page_total,
    )
    if boot_total is None:
        return BootOutcome.INCOMPLETE
    segments = []
    for label, field in EXCLUSIVE_SEGMENTS:
        value = field_f64(navigation, field)
        if value is None:
            return BootOutcome.INCOMPLETE
        segments.append((label, value))
    phases = member(navigation, "bootPhases")
    if not isinstance(phases, dict):
        return BootOutcome.INCOMPLETE
    intervals = [
        (key, value)
        for key in phases
        if BOOT_INTERVAL_KEY in key and (value := field_f64(phases, key)) is not None
    ]
    if len(intervals) < MIN_BOOT_PHASES:
        return BootOutcome.INCOMPLETE

    # In `startTime` order from the marks, since an object's key order is no
    # timeline, and on both endpoints: sub-millisecond phases report as zero
    # length, so two intervals can share a start.
    def bounds(interval: tuple[str, float]) -> tuple[float, float]:
        ends = interval[0].split("->")
        end = starts.get(ends[1], math.inf) if len(ends) > 1 else math.inf
        return starts.get(ends[0], math.inf), end

    segments.extend(sorted(intervals, key=bounds))
    total = plain_sum([value for _, value in segments])
    if abs(total - boot_total) > CLOSURE_TOLERANCE_MS:
        return BootOutcome.CLOSURE_VIOLATION
    return Decomposition(segments, boot_total, field_f64(navigation, "commitToMountMs"))


def _outcome(result: Decomposition | BootOutcome) -> BootOutcome:
    return BootOutcome.COMPLETE if isinstance(result, Decomposition) else result


def boot_decomposition_outcome(navigation: Any, marks: list[Any]) -> BootOutcome:
    """Classifies test-span navigation evidence, which must close on its marks."""
    return _outcome(decompose(navigation, marks))


def page_boot_decomposition_outcome(navigation: Any, marks: list[Any]) -> BootOutcome:
    """Classifies page-span navigation evidence, which may carry its document
    total (`documentBootTotalMs`) without the test span's boot marks."""
    total = field_f64(navigation, "documentBootTotalMs")
    return _outcome(decompose(navigation, marks, total))
