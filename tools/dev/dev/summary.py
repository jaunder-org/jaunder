"""`dev trace summary`: a run's slowest spans, read from its span records.

Every span that did not end `ok` is listed too, however short: the step that
stopped a run is the row a reader looks for first. So is the time a CI job
spent before its root `dev` started, and the root's runner heads the table.
"""

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import click

from dev import runner, spans
from dev.spans import RunSpan, Status

# The slowest spans shown; unsuccessful spans are shown beyond it.
LIMIT = 20
_UNSUCCESSFUL = {Status.FAILED, Status.SIGNALLED, Status.INCOMPLETE}
# The row for a CI job's time between its start and its root `dev`.
BEFORE_DEV = "before dev"


@dataclass(frozen=True)
class Row:
    name: str
    depth: int
    duration_ns: int
    status: str
    cache: str
    # Time no child covers; None for a span without children.
    untraced_ns: int | None
    # Bytes a Nix step fetched from substituters; None where not recorded.
    fetched_bytes: int | None = None


@dataclass(frozen=True)
class Summary:
    # The root's runner, or None when it recorded none.
    runner: str | None
    rows: list[Row]


def _integer(value: object) -> int | None:
    # bool is an int to Python, never to a record writer.
    return value if type(value) is int else None


def _runner(root: RunSpan) -> str | None:
    attributes = root.attributes
    parts = [
        str(attributes[runner.CPU_MODEL]) if runner.CPU_MODEL in attributes else "",
        f"nproc {attributes[runner.NPROC]}" if runner.NPROC in attributes else "",
        f"cloud {attributes[runner.CLOUD]}" if runner.CLOUD in attributes else "",
    ]
    shown = [part for part in parts if part]
    return "Runner: " + ", ".join(shown) if shown else None


def _before_dev(root: RunSpan) -> Row | None:
    """The CI job's time before `root` started, if it recorded a positive one."""
    job_start = _integer(root.attributes.get(runner.JOB_START))
    if job_start is None or root.start_ns <= job_start:
        return None
    return Row(BEFORE_DEV, 0, root.start_ns - job_start, "", "", None)


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


def summarize(directory: Path) -> Summary:
    run = spans.load(directory)
    known = {span.span_id: span for span in run}
    children = spans.children(run)

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
            fetched_bytes=_integer(span.attributes.get(spans.FETCHED_BYTES)),
        )
        for span in run
    ]
    roots = [span for span in run if span.parent_id is None]
    before = [row for row in map(_before_dev, roots) if row is not None]
    slowest_first = sorted(
        [*table, *before], key=lambda row: row.duration_ns, reverse=True
    )
    pinned = {id(row) for row in before}
    shown = [
        row
        for rank, row in enumerate(slowest_first)
        if rank < LIMIT or row.status in _UNSUCCESSFUL or id(row) in pinned
    ]
    described = [line for line in map(_runner, roots) if line is not None]
    return Summary(described[0] if described else None, shown)


def _seconds(nanoseconds: int | None) -> str:
    return "" if nanoseconds is None else f"{nanoseconds / 1e9:.3f}s"


def _mebibytes(count: int | None) -> str:
    return "" if count is None else f"{count / 2**20:.1f}MiB"


@dataclass(frozen=True)
class _Column:
    heading: str
    # Numbers align right, text left.
    numeric: bool
    cell: Callable[[Row], str]


_COLUMNS = (
    _Column("Duration", True, lambda row: _seconds(row.duration_ns)),
    _Column("Untraced", True, lambda row: _seconds(row.untraced_ns)),
    _Column("Depth", True, lambda row: str(row.depth)),
    _Column("Status", False, lambda row: row.status),
    _Column("Cache", False, lambda row: row.cache),
    _Column("Fetched", True, lambda row: _mebibytes(row.fetched_bytes)),
    # Last, so a long span name needs no padding.
    _Column("Span", False, lambda row: row.name),
)


def render(summary: Summary, markdown: bool) -> str:
    grid = [
        [column.heading for column in _COLUMNS],
        *([column.cell(row) for column in _COLUMNS] for row in summary.rows),
    ]
    if markdown:
        rule = ["---:" if column.numeric else "---" for column in _COLUMNS]
        lines = [
            "| " + " | ".join(cell.replace("|", "\\|") for cell in line) + " |"
            for line in [grid[0], rule, *grid[1:]]
        ]
    else:
        *padded, _span = _COLUMNS
        widths = [max(len(cell) for cell in cells) for cells in zip(*grid)]
        lines = [
            "  ".join(
                [
                    *(
                        cell.rjust(width) if column.numeric else cell.ljust(width)
                        for column, cell, width in zip(padded, line, widths)
                    ),
                    line[-1],
                ]
            )
            for line in grid
        ]
    if summary.runner is not None:
        # A blank line keeps Markdown from reading the heading into the table.
        lines = [summary.runner, *([""] if markdown else []), *lines]
    return "\n".join(lines) + "\n"
