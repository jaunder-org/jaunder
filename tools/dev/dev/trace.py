"""Span records for one traced `dev` run.

Every process in a run appends a start and an end record for each of its spans
to its own JSON Lines file in the run directory named by `JAUNDER_TRACE_DIR`.
The root, the process that finds that variable unset, creates the directory and
merges every file into `<run_id>.trace.json` when it finishes (`chrome.py`).
The record format is locked by `tests/golden/span-records.jsonl`.
"""

import json
import os
import secrets
import sys
import time
from collections.abc import Iterable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from opentelemetry import context as context_api
from opentelemetry.sdk.trace import ReadableSpan, Span, SpanProcessor, TracerProvider
from opentelemetry.sdk.trace.export import (
    SimpleSpanProcessor,
    SpanExporter,
    SpanExportResult,
)
from opentelemetry.sdk.trace.id_generator import IdGenerator
from opentelemetry.trace import NoOpTracer, Tracer, set_span_in_context
from opentelemetry.trace import Span as ApiSpan
from opentelemetry.trace.propagation.tracecontext import TraceContextTextMapPropagator
from opentelemetry.util.types import AttributeValue

from dev import chrome, runner
from dev.spans import STATUS, Status

TRACE_DIR = "JAUNDER_TRACE_DIR"
CARRIER_KEYS = ("TRACEPARENT", "TRACESTATE")
# The longest text attribute, in UTF-8 bytes, unless a caller names less.
ATTRIBUTE_LIMIT = 1024


class Warner:
    """Reports the first tracing failure of this process, and only that one."""

    def __init__(self) -> None:
        self._warned = False

    def __call__(self, message: str) -> None:
        if not self._warned:
            self._warned = True
            print(f"dev: tracing disabled: {message}", file=sys.stderr, flush=True)


class RecordWriter:
    """Appends this process's span records to its own file in the run directory.

    Each record reaches the file as it is written, so a SIGKILL loses only records
    that had not been written yet. A failure disables the writer after one
    warning: tracing never changes a step's outcome.
    """

    def __init__(self, directory: Path | None, warn: Warner) -> None:
        self._path = (
            None
            if directory is None
            else directory / f"{os.getpid()}-{secrets.token_hex(4)}.jsonl"
        )
        self._warn = warn

    def write(self, record: Mapping[str, object]) -> None:
        if self._path is None:
            return
        try:
            with open(self._path, "a", encoding="utf-8") as file:
                file.write(json.dumps(record, separators=(",", ":")) + "\n")
        except OSError as error:
            self._path = None
            self._warn(str(error))


def _record(kind: str, span: ReadableSpan, time_ns: int | None) -> dict[str, object]:
    context = span.context
    return {
        "kind": kind,
        "trace_id": f"{context.trace_id:032x}",
        "span_id": f"{context.span_id:016x}",
        "parent_id": None if span.parent is None else f"{span.parent.span_id:016x}",
        "name": span.name,
        "time_ns": time_ns,
    }


class StartRecorder(SpanProcessor):
    """Writes a start record as each span opens, so a killed span still shows."""

    def __init__(self, writer: RecordWriter) -> None:
        self._writer = writer

    def on_start(
        self, span: Span, parent_context: context_api.Context | None = None
    ) -> None:
        self._writer.write(_record("start", span, span.start_time))


class EndRecorder(SpanExporter):
    """Writes an end record, with the span's attributes, for each ended span."""

    def __init__(self, writer: RecordWriter) -> None:
        self._writer = writer

    def export(self, spans: Sequence[ReadableSpan]) -> SpanExportResult:
        for span in spans:
            record = _record("end", span, span.end_time)
            record["attributes"] = dict(span.attributes or {})
            self._writer.write(record)
        return SpanExportResult.SUCCESS


def recording_provider(
    writer: RecordWriter, id_generator: IdGenerator | None = None
) -> TracerProvider:
    # The provider is shut down explicitly when the gate ends, before the merge.
    provider = TracerProvider(id_generator=id_generator, shutdown_on_exit=False)
    provider.add_span_processor(StartRecorder(writer))
    provider.add_span_processor(SimpleSpanProcessor(EndRecorder(writer)))
    return provider


_PROPAGATOR = TraceContextTextMapPropagator()


class _Run:
    """The tracing state a step needs to start spans and launch children."""

    def __init__(self, tracer: Tracer, directory: Path | None) -> None:
        self.tracer = tracer
        # An empty value tells children that the root disabled tracing, so
        # they stay silent instead of each warning or starting a run of their own.
        self.directory = "" if directory is None else str(directory)
        self.gate: ApiSpan | None = None


_current = _Run(NoOpTracer(), None)


def tracer() -> Tracer:
    return _current.tracer


def annotate(attributes: Mapping[str, str | int]) -> None:
    """Records what the gate learned about its own run on this process's gate span."""
    if _current.gate is not None:
        _current.gate.set_attributes(attributes)


def child_environment() -> dict[str, str]:
    """The environment for a step's child: this run, with the current span as parent."""
    environment = dict(os.environ)
    for key in CARRIER_KEYS:
        environment.pop(key, None)
    environment[TRACE_DIR] = _current.directory
    if _current.directory:
        # W3C trace context travels as upper-case environment variables.
        carrier: dict[str, str] = {}
        _PROPAGATOR.inject(carrier)
        environment.update({key.upper(): value for key, value in carrier.items()})
    return environment


@dataclass(frozen=True)
class Phase:
    """Work a step's child timed itself where it could not trace, as in a Nix
    sandbox: placed in the run afterwards, on the child's own clock."""

    name: str
    start_ns: int
    end_ns: int
    ok: bool


@dataclass(frozen=True)
class Imported:
    """A finished span, with its subtree, that a step's child timed where it
    could not trace (a producer's phase, a span of its OTLP capture): placed in
    the run afterwards, on the child's own clock."""

    name: str
    start_ns: int
    end_ns: int
    attributes: Mapping[str, AttributeValue]
    children: Sequence[Imported] = ()


def record_spans(parent: ApiSpan, spans: Iterable[Imported]) -> list[ApiSpan]:
    """Writes `spans` and their subtrees as finished descendants of `parent`,
    each parent before its children, and returns the top-level spans written."""
    written = []
    for imported in spans:
        span = tracer().start_span(
            imported.name,
            set_span_in_context(parent),
            attributes=imported.attributes,
            start_time=imported.start_ns,
        )
        record_spans(span, imported.children)
        span.end(end_time=imported.end_ns)
        written.append(span)
    return written


def record_phases(
    step: ApiSpan, started_ns: int, phases: Iterable[Phase]
) -> list[tuple[Phase, ApiSpan]]:
    """Writes `phases` as finished children of the still-open `step`, and
    returns each phase written with its span.

    `started_ns` is the wall-clock nanosecond the step started. Only phases
    inside the step so far are its own work: one outside it, as from a reused
    output or an earlier run, is left out rather than misplaced.
    """
    ended_ns = time.time_ns()
    inside = [
        phase
        for phase in phases
        if started_ns <= phase.start_ns <= phase.end_ns <= ended_ns
    ]
    written = record_spans(
        step,
        [
            Imported(
                phase.name,
                phase.start_ns,
                phase.end_ns,
                {STATUS: Status.OK if phase.ok else Status.FAILED},
            )
            for phase in inside
        ],
    )
    return list(zip(inside, written, strict=True))


def cap(text: str, limit: int = ATTRIBUTE_LIMIT) -> str:
    return text.encode()[:limit].decode(errors="ignore")


@contextmanager
def run(root: Path, name: str) -> Iterator[ApiSpan]:
    """Opens the gate span `name`, joining the caller's run or starting one under `root`."""
    global _current
    warn = Warner()
    inherited = os.environ.get(TRACE_DIR)
    directory: Path | None = None
    merge: tuple[Path, Path] | None = None
    if inherited is None:
        # Outside tools may set TRACEPARENT; only the run directory marks a
        # child, so the root starts its own trace.
        run_id = f"{time.time_ns()}-{os.getpid()}"
        candidate = root / ".xtask" / "run" / f"{run_id}.trace.d"
        parent = context_api.Context()
        try:
            candidate.mkdir(parents=True)
        except OSError as error:
            warn(str(error))
        else:
            directory = candidate
            merge = (candidate, candidate.with_name(f"{run_id}.trace.json"))
    else:
        directory = Path(inherited) if inherited else None
        parent = _PROPAGATOR.extract(
            {key.lower(): os.environ[key] for key in CARRIER_KEYS if key in os.environ}
        )
    writer = RecordWriter(directory, warn)
    provider = recording_provider(writer)
    _current = _Run(provider.get_tracer("dev"), directory)
    try:
        with _current.tracer.start_as_current_span(name, context=parent) as span:
            _current.gate = span
            if inherited is None:
                span.set_attributes(runner.attributes(os.environ))
                span.set_attributes(runner.head(root))
            yield span
    finally:
        _current.gate = None
        provider.shutdown()
        if merge is not None:
            # The merged file is written even for a failed or partial run. The
            # merge accepts any record content, so only I/O can fail it, and
            # that only warns: tracing never changes the gate's outcome.
            try:
                chrome.write(*merge)
            except OSError as error:
                warn(str(error))
