"""Trace-derived `#[server]` fn flow coverage (#681, ADR-0081).

Which server entry points a real browser session drives, from the reconciled
SQLite/Chromium lanes' captures. The committed snapshot
(`docs/coverage/server-fns.json`) is the one generated artifact; `verify`
compares it byte for byte with what this run's traces derive, and
`regenerate` rewrites it. Everything fails closed: a missing, empty or
unparseable capture is an error, never "nothing uncovered".

**Two signals, unioned.** A span identifies a fn when its name is one of the
fn's candidate span names *and* its module attribute is the fn's module, or
else when its `uri` path is the fn's endpoint. The name is matched forward
from the inventory under every naming regime this repo has used, because an
inverted pattern that stopped matching once went unnoticed while `uri` carried
every hit. The inventory comes from `source-contracts server-fn-inventory`,
which owns the endpoint derivation; its seed cross-check against real traffic
lives with it (ADR-0120).

**Attribution walks ancestors.** An instrument span's parent is its request
span, whose parent is the `e2e.test` span, so a hit walks `parent_span_id`
upward to a known test. A walk that reaches none is an orphan, recorded by
reason rather than dropped, since a silent drop would understate coverage.
"""

import json
import re
from dataclasses import dataclass, field
from pathlib import Path

from dev.traces import (
    EvidenceError,
    Filters,
    Span,
    get_attr,
    parse_spans,
    read_instance_diag_logs,
    read_trace_member,
)

SNAPSHOT_PATH = "docs/coverage/server-fns.json"
# Named in every failure, so an author who hits the gate needs no other context.
REGENERATE_CMD = "dev e2e-regenerate-coverage"
# The route prefix every server-fn request lands under.
API_PREFIX = "/api/"
# Module attributes, current first; earlier captures carry only the legacy one.
MODULE_ATTRS = ("code.module.name", "code.namespace")
# The crate prefix module attributes carry and inventory modules do not.
CRATE_PREFIX = "web::"
TEST_SPAN = "e2e.test"
# The `e2e.test` span's attribute holding the test's title.
TEST_TITLE_ATTR = "e2e.test"
# A cycle guard: a malformed capture must not hang a gate.
MAX_DEPTH = 32
# The `tracing` target of the OpenTelemetry SDK's own diagnostics, and the
# prefix of every span-loss diagnostic its batch span processor emits.
SDK_TARGET = "opentelemetry_sdk"
PROCESSOR_PREFIX = "BatchSpanProcessor."


@dataclass(frozen=True)
class ServerFn:
    """One `#[macros::server]` fn, as `source-contracts` lists it."""

    qualified: str
    module: str
    ident: str
    endpoint: str | None

    def candidate_span_names(self) -> tuple[str, str, str]:
        """Every span name that could denote this fn: `web.<vertical>.<ident>`,
        which `#[macros::server]` derives (#714); `__server_<ident>`, which
        `#[tracing::instrument]` derives from the body `#[server]` relocates;
        and the bare ident. The last two omit the vertical, so the module
        match is what disambiguates them."""
        vertical = self.qualified.partition("::")[0]
        return (f"web.{vertical}.{self.ident}", f"__server_{self.ident}", self.ident)


def parse_inventory(listing: bytes) -> list[ServerFn]:
    """`source-contracts server-fn-inventory`'s listing, which it already
    derived and validated; read, not checked again."""
    return [
        ServerFn(entry["qualified"], entry["module"], entry["ident"], entry["endpoint"])
        for entry in json.loads(listing)
    ]


@dataclass
class Coverage:
    """Which tests drove each fn, and why each unattributed hit reached none.

    Both are keyed by qualified name (`<vertical>::<ident>`): bare idents
    collide across verticals (#684). Orphans are reasons, not counts, since a
    count tracks how many tests ran and would churn the byte-compared snapshot.
    """

    covered: dict[str, set[str]] = field(default_factory=dict)
    orphans: dict[str, set[str]] = field(default_factory=dict)

    def merge(self, other: Coverage) -> None:
        for name, tests in other.covered.items():
            self.covered.setdefault(name, set()).update(tests)
        for name, reasons in other.orphans.items():
            self.orphans.setdefault(name, set()).update(reasons)


def span_module(span: Span) -> str:
    """The module a span records: the current attribute, else the legacy one.

    An empty current attribute counts as absent, so legacy captures stay usable.
    """
    return get_attr(span.raw, MODULE_ATTRS[0]) or get_attr(span.raw, MODULE_ATTRS[1])


def _endpoint(uri: str) -> str | None:
    """The server-fn endpoint a request `uri` names, origin-form or absolute,
    without its query or fragment; None for anything not under `/api/`."""
    path = re.split("[?#]", uri, maxsplit=1)[0]
    _, prefix, rest = path.partition(API_PREFIX)
    return rest if prefix and rest else None


def _identify(span: Span, inventory: list[ServerFn]) -> ServerFn | None:
    # A module that is absent cannot confirm a name match; fall through to `uri`
    # rather than guess.
    module = span_module(span)
    if module:
        relative = module.removeprefix(CRATE_PREFIX)
        for fn in inventory:
            if relative == fn.module and span.name in fn.candidate_span_names():
                return fn
    endpoint = _endpoint(span.uri)
    if endpoint is None:
        return None
    return next((fn for fn in inventory if fn.endpoint == endpoint), None)


def _attribute(start: Span, by_id: dict[str, Span], tests: dict[str, str]) -> str:
    """The title of the test `start` descends from, or raises its orphan reason."""
    current = start
    for _ in range(MAX_DEPTH):
        parent = current.parent_span_id
        if not parent:
            raise _Orphan("no-parent")
        if parent in tests:
            return tests[parent]
        if parent not in by_id:
            raise _Orphan(f"unknown-parent:{parent}")
        current = by_id[parent]
    raise _Orphan("depth-exceeded")


class _Orphan(Exception):
    """Why a hit reached no test: `no-parent`, `unknown-parent:<id>` (the id is
    the evidence of where the chain came from) or `depth-exceeded`."""


def extract(spans: list[Span], inventory: list[ServerFn]) -> Coverage:
    """Coverage of `inventory` by one lane's spans."""
    by_id = {span.span_id: span for span in spans if span.span_id}
    tests = {
        span.span_id: get_attr(span.raw, TEST_TITLE_ATTR)
        for span in spans
        if span.name == TEST_SPAN and span.span_id
    }
    coverage = Coverage()
    for span in spans:
        fn = _identify(span, inventory)
        if fn is None:
            continue
        try:
            title = _attribute(span, by_id, tests)
        except _Orphan as orphan:
            coverage.orphans.setdefault(fn.qualified, set()).add(str(orphan))
        else:
            coverage.covered.setdefault(fn.qualified, set()).add(title)
    return coverage


def refuse_span_loss(diag_log: str) -> None:
    """Refuses a capture whose server's OpenTelemetry SDK reported losing spans.

    A lost request span orphans its surviving server-fn child, so a truncated
    capture looks exactly like an attribution leak (#2029). The SDK records
    every loss in the server's WARN+ `diag.log`; lines that are not JSON are
    not SDK records. The reported count is a lower bound: export errors and
    shutdown timeouts lose spans the SDK's count omits.
    """
    names = []
    # The SDK's count is cumulative, so the largest reported is the total.
    dropped: int | None = None
    for line in diag_log.split("\n"):
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if not isinstance(record, dict):
            continue
        fields = record.get("fields")
        fields = fields if isinstance(fields, dict) else {}
        name = fields.get("name")
        if (
            record.get("target") == SDK_TARGET
            and record.get("level") in ("WARN", "ERROR")
            and isinstance(name, str)
            and name.startswith(PROCESSOR_PREFIX)
        ):
            names.append(name)
            count = fields.get("dropped_span_count")
            if type(count) is int and 0 <= count < 2**64:
                dropped = max(count, dropped or 0)
    if names:
        lost = "an unknown number of" if dropped is None else f"at least {dropped}"
        raise EvidenceError(
            f"the capture is incomplete: the server's OpenTelemetry SDK lost {lost} "
            f"span(s) ({', '.join(names)}), which would surface as misattributed "
            "orphans — re-run the lane"
        )


def coverage_from_jsonl(jsonl: str, inventory: list[ServerFn]) -> Coverage:
    """Coverage from raw OTLP JSONL; empty content is an error, not no coverage."""
    try:
        spans = parse_spans(jsonl, Filters(), "capture")
    except EvidenceError as error:
        raise EvidenceError(
            f"parsing the e2e capture's otel-traces.jsonl: {error}"
        ) from error
    if not spans:
        raise EvidenceError(
            "the e2e capture contains no spans — refusing to treat an empty capture "
            "as full coverage; re-run `dev e2e`"
        )
    return extract(spans, inventory)


def coverage_from_capture(capture: Path, inventory: list[ServerFn]) -> Coverage:
    """Coverage from one lane's capture, refused first when any server instance
    reported losing spans; every instance's `diag.log` is required, since
    without it the loss cannot be ruled out (#2076)."""
    if not capture.exists():
        raise EvidenceError(
            f"no e2e capture at {capture} — run `dev e2e` first (coverage is "
            "derived from the reconciled SQLite/Chromium lanes' traces)"
        )
    for diag_log in read_instance_diag_logs(capture):
        try:
            refuse_span_loss(diag_log)
        except EvidenceError as error:
            raise EvidenceError(f"refusing {capture}: {error}") from error
    try:
        jsonl = read_trace_member(capture)
    except EvidenceError as error:
        raise EvidenceError(
            f"reading the otel trace from {capture}: {error}"
        ) from error
    return coverage_from_jsonl(jsonl, inventory)


def coverage_from_captures(captures: list[Path], inventory: list[ServerFn]) -> Coverage:
    """The union of several lanes' coverage. Each lane is extracted alone,
    since span ids, and so attribution chains, are unique only within one."""
    if not captures:
        raise EvidenceError(
            "no e2e captures supplied — refusing to treat no evidence as coverage"
        )
    union = Coverage()
    for capture in captures:
        union.merge(coverage_from_capture(capture, inventory))
    return union


def render(coverage: Coverage) -> str:
    """The snapshot's exact committed bytes: the covered names and each fn's
    orphan reasons, sorted, as pretty JSON with a trailing newline.

    Test titles stay out: which test drove a fn is timing-dependent, while the
    covered set reproduces. Nor is there any provenance, which would differ
    on every commit.
    """
    snapshot = {
        "covered": sorted(coverage.covered),
        "orphans": {
            name: sorted(reasons) for name, reasons in sorted(coverage.orphans.items())
        },
    }
    return json.dumps(snapshot, indent=2, ensure_ascii=False) + "\n"


def verify(coverage: Coverage, snapshot: Path) -> str:
    """The committed snapshot is byte for byte what `coverage` renders.

    Bytes, not parsed values: a hand edit that parses equal is still drift. An
    unreadable snapshot reads as empty, which never matches.
    """
    try:
        committed = snapshot.read_text(encoding="utf-8")
    except OSError, ValueError:
        committed = ""
    if committed != render(coverage):
        raise EvidenceError(
            f"{snapshot} is out of date with this run's traces — regenerate it "
            f"with `{REGENERATE_CMD}` and commit the result"
        )
    return f"{len(coverage.covered)} covered; snapshot current"


def regenerate(coverage: Coverage, snapshot: Path) -> str:
    """Writes the snapshot `coverage` renders, its one generated artifact."""
    snapshot.parent.mkdir(parents=True, exist_ok=True)
    snapshot.write_text(render(coverage), encoding="utf-8")
    return (
        f"{len(coverage.covered)} covered, {len(coverage.orphans)} fn(s) with "
        f"unattributed hits → {snapshot}"
    )
