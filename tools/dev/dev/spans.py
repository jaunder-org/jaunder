"""Spans reassembled from a run directory's start and end records."""

import json
from collections import defaultdict
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any

COMMAND_LINE = "process.command_line"
EXIT_CODE = "process.exit.code"
STATUS = "jaunder.status"
CACHE = "jaunder.cache"


class Status(StrEnum):
    """`jaunder.status`: how a span ended."""

    OK = "ok"
    FAILED = "failed"
    SIGNALLED = "signalled"
    # Set by the merge, never by a writer: the span's process was killed.
    INCOMPLETE = "incomplete"


class Cache(StrEnum):
    """`jaunder.cache`: where a Nix step's top-level output came from."""

    REUSED = "reused"
    BUILT = "built"
    SUBSTITUTED = "substituted"


@dataclass(frozen=True)
class RunSpan:
    span_id: str
    parent_id: str | None
    trace_id: str
    name: str
    start_ns: int
    end_ns: int
    attributes: dict[str, Any]
    # No end record, or a parent that left no record: a killed process.
    incomplete: bool

    @property
    def status(self) -> str:
        if self.incomplete:
            return Status.INCOMPLETE
        return str(self.attributes.get(STATUS, ""))


# Each field a record must carry, with the types it may take.
_FIELDS: dict[str, tuple[type, ...]] = {
    "trace_id": (str,),
    "span_id": (str,),
    "parent_id": (str, type(None)),
    "name": (str,),
    "time_ns": (int,),
}


def _record(line: bytes) -> dict[str, Any] | None:
    """The span record on `line`, or None for anything else.

    A writer killed mid-line leaves a partial last record, possibly cut inside
    a multibyte character; other tools may leave lines that are not records.
    Neither may stop the merge.
    """
    try:
        record = json.loads(line.decode("utf-8"))
    except ValueError:
        return None
    if not isinstance(record, dict) or record.get("kind") not in ("start", "end"):
        return None
    for field, types in _FIELDS.items():
        if not isinstance(record.get(field), types):
            return None
    if record["kind"] == "end" and not isinstance(record.get("attributes"), dict):
        return None
    return record


def _cycles(span_id: str, starts: dict[str, dict[str, Any]]) -> bool:
    """Whether the chain of parents from `span_id` loops."""
    seen: set[str] = set()
    current: str | None = span_id
    while current in starts:
        if current in seen:
            return True
        seen.add(current)
        current = starts[current]["parent_id"]
    return False


def load(directory: Path) -> list[RunSpan]:
    """Spans in start order.

    An unfinished span ends with its parent, or with the run's last record.
    """
    starts: dict[str, dict[str, Any]] = {}
    ends: dict[str, dict[str, Any]] = {}
    for path in sorted(directory.glob("*.jsonl")):
        for line in path.read_bytes().splitlines():
            record = _record(line)
            if record is not None:
                kind = starts if record["kind"] == "start" else ends
                kind[record["span_id"]] = record
    # No writer produces a parent cycle, and nothing could place one in the
    # span tree: its spans are dropped, leaving any children orphaned.
    for span_id in [span_id for span_id in starts if _cycles(span_id, starts)]:
        del starts[span_id]
    last = max(
        (record["time_ns"] for record in [*starts.values(), *ends.values()]),
        default=0,
    )

    def end_ns(span_id: str) -> int:
        # A killed process's span cannot outlive its parent, which ended later.
        end = ends.get(span_id)
        if end is not None:
            return int(end["time_ns"])
        parent_id = starts[span_id]["parent_id"]
        return end_ns(parent_id) if parent_id in starts else last

    spans = []
    for span_id, start in starts.items():
        end = ends.get(span_id)
        parent_id = start["parent_id"]
        spans.append(
            RunSpan(
                span_id=span_id,
                parent_id=parent_id,
                trace_id=start["trace_id"],
                name=start["name"],
                start_ns=start["time_ns"],
                end_ns=end_ns(span_id),
                attributes={} if end is None else end["attributes"],
                incomplete=end is None
                or (parent_id is not None and parent_id not in starts),
            )
        )
    return sorted(spans, key=lambda span: (span.start_ns, -span.end_ns))


def children(run: list[RunSpan]) -> dict[str | None, list[RunSpan]]:
    """Each span's children in start order; roots and orphans under None."""
    known = {span.span_id for span in run}
    index: dict[str | None, list[RunSpan]] = defaultdict(list)
    for span in run:
        index[span.parent_id if span.parent_id in known else None].append(span)
    return index
