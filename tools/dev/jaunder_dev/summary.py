"""`dev trace summary`: a run's slowest spans, read from its span records."""

from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

import click

from jaunder_dev import spans
from jaunder_dev.spans import RunSpan

LIMIT = 20


@dataclass(frozen=True)
class Row:
    name: str
    depth: int
    duration_ns: int
    status: str
    cache: str
    # Time no child covers; None for a span without children.
    untraced_ns: int | None


def latest(runs: Path) -> Path:
    found = sorted(runs.glob("*.trace.d"))
    if not found:
        raise click.ClickException(f"no traced runs in {runs}")
    return found[-1]


def _covered(parent: RunSpan, children: list[RunSpan]) -> int:
    """Length of the union of the children's intervals inside the parent.

    Children that run in parallel overlap; summing them would overstate how
    much of the parent they explain.
    """
    total = 0
    cursor = parent.start_ns
    for child in sorted(children, key=lambda span: span.start_ns):
        start = max(child.start_ns, cursor)
        end = min(child.end_ns, parent.end_ns)
        if end > start:
            total += end - start
            cursor = end
    return total


def rows(directory: Path) -> list[Row]:
    run = spans.load(directory)
    known = {span.span_id: span for span in run}
    children: dict[str, list[RunSpan]] = defaultdict(list)
    for span in run:
        if span.parent_id in known:
            children[span.parent_id].append(span)

    def depth(span: RunSpan) -> int:
        parent = known.get(span.parent_id) if span.parent_id is not None else None
        return 0 if parent is None else depth(parent) + 1

    table = [
        Row(
            name=span.name,
            depth=depth(span),
            duration_ns=span.end_ns - span.start_ns,
            status=span.status,
            cache=str(span.attributes.get(spans.CACHE, "")),
            untraced_ns=(
                span.end_ns - span.start_ns - _covered(span, children[span.span_id])
                if children[span.span_id]
                else None
            ),
        )
        for span in run
    ]
    return sorted(table, key=lambda row: row.duration_ns, reverse=True)[:LIMIT]


def _seconds(nanoseconds: int | None) -> str:
    return "" if nanoseconds is None else f"{nanoseconds / 1e9:.3f}s"


def render(table: list[Row], markdown: bool) -> str:
    header = ("Duration", "Untraced", "Depth", "Status", "Cache", "Span")
    cells = [
        (
            _seconds(row.duration_ns),
            _seconds(row.untraced_ns),
            str(row.depth),
            row.status,
            row.cache,
            row.name,
        )
        for row in table
    ]
    if markdown:
        lines = [
            "| " + " | ".join(header) + " |",
            "| ---: | ---: | ---: | --- | --- | --- |",
            *(
                "| " + " | ".join(cell.replace("|", "\\|") for cell in line) + " |"
                for line in cells
            ),
        ]
    else:
        widths = [max(len(line[i]) for line in [header, *cells]) for i in range(5)]
        lines = [
            "  ".join(
                [
                    *(line[i].rjust(widths[i]) for i in range(3)),
                    *(line[i].ljust(widths[i]) for i in range(3, 5)),
                    line[5],
                ]
            )
            for line in [header, *cells]
        ]
    return "\n".join(lines) + "\n"
