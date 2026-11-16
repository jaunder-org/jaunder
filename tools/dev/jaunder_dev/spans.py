"""Spans reassembled from a run directory's start and end records."""

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

COMMAND_LINE = "process.command_line"
EXIT_CODE = "process.exit.code"
STATUS = "jaunder.status"
CACHE = "jaunder.cache"
INCOMPLETE = "incomplete"


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
        return INCOMPLETE if self.incomplete else str(self.attributes.get(STATUS, ""))


def load(directory: Path) -> list[RunSpan]:
    """Spans in start order.

    An unfinished span ends with its parent, or with the run's last record.
    """
    starts: dict[str, dict[str, Any]] = {}
    ends: dict[str, dict[str, Any]] = {}
    for path in sorted(directory.glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                # A writer killed mid-line leaves a partial last record.
                continue
            (starts if record["kind"] == "start" else ends)[record["span_id"]] = record
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
