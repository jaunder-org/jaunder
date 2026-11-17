"""Merges a run directory into one Chrome trace JSON file for Perfetto."""

import json
from pathlib import Path

from dev import spans
from dev.spans import RunSpan


def tracks(run: list[RunSpan]) -> dict[str, int]:
    """Assigns each span a track from the span tree.

    Perfetto nests slices on one track by time, so a child shares its parent's
    track only when it fits inside the parent and after its earlier siblings;
    an overlapping sibling, or one outside its parent, opens a new track.
    """
    children = spans.children(run)
    assigned: dict[str, int] = {}
    opened = 1

    def place(parent: str | None, track: int, start: int, end: int) -> None:
        nonlocal opened
        cursor = start
        for child in children[parent]:
            if child.start_ns >= cursor and child.end_ns <= end:
                assigned[child.span_id] = track
                cursor = child.end_ns
            else:
                assigned[child.span_id] = opened
                opened += 1
            place(child.span_id, assigned[child.span_id], child.start_ns, child.end_ns)

    place(None, 0, 0, max((span.end_ns for span in run), default=0))
    return assigned


def write(directory: Path, output: Path) -> None:
    run = spans.load(directory)
    assigned = tracks(run)
    events: list[dict[str, object]] = []
    named: set[int] = set()
    for span in run:
        track = assigned[span.span_id]
        if track not in named:
            named.add(track)
            events.append(
                {
                    "ph": "M",
                    "name": "thread_name",
                    "pid": 1,
                    "tid": track,
                    "args": {"name": span.name},
                }
            )
        events.append(
            {
                "ph": "X",
                "name": span.name,
                "pid": 1,
                "tid": track,
                "ts": span.start_ns // 1000,
                "dur": span.end_ns // 1000 - span.start_ns // 1000,
                "args": {
                    **span.attributes,
                    spans.STATUS: span.status,
                    "trace_id": span.trace_id,
                    "span_id": span.span_id,
                    "parent_id": span.parent_id,
                },
            }
        )
    output.write_text(
        json.dumps({"traceEvents": events, "displayTimeUnit": "ms"}) + "\n",
        encoding="utf-8",
    )
