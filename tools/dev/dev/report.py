"""The human view of a step report: a host tool's JSON, or `dev`'s own.

A diagnostic's rendered `report` (trace tables) comes first, verbatim, then one
line per step. The producing executable's exit status remains the verdict. The
cell helpers format the trace diagnostics' tables.
"""

import json
from collections.abc import Sequence
from typing import Any

from tabulate import tabulate


def render_report(report: Any) -> str:
    """A report already in hand, as a dict, rendered."""
    lines = []
    if report.get("report"):
        lines.append(report["report"])
    for step in report["steps"]:
        mark = "skip" if step.get("skipped") else " ok " if step["ok"] else "FAIL"
        line = f"[{mark}] {step['name']}"
        if not step["ok"] or step["duration_ms"] >= 1000:
            line += f" ({step['duration_ms']} ms)"
        if step.get("detail"):
            line += f" — {step['detail']}"
        lines.append(line)
    return "".join(f"{line}\n" for line in lines)


def render(document: bytes) -> str | None:
    """The report on a host tool's stdout, rendered; None if it is not a report.

    ADR-0247 has a host tool print exactly one report, even when it refuses,
    so empty output is not a report either.
    """
    try:
        return render_report(json.loads(document))
    except ValueError, LookupError, TypeError, AttributeError:
        return None


def table(
    headers: Sequence[str],
    rows: Sequence[Sequence[str]],
    align: Sequence[str] | None = None,
    markdown: bool = False,
) -> str:
    """`rows` under `headers` as a box-drawn table, without a final newline.

    Cells arrive already formatted: number parsing is off, so `1.000` stays
    `1.000` rather than becoming `1`. `align` gives each column `"left"` or
    `"right"`; every column is left-aligned without it. `markdown` draws a
    GitHub pipe table instead, whose rule carries the alignment; tabulate does
    not escape a `|` in a cell, so it is escaped here.
    """
    if markdown:
        rows = [[cell.replace("|", "\\|") for cell in row] for row in rows]
    return tabulate(
        rows,
        headers=headers,
        tablefmt="pipe" if markdown else "simple_outline",
        colalign=align,
        disable_numparse=True,
    )


def ms(value: float | None) -> str:
    """A millisecond value as a report cell: three decimals, `-` when unset."""
    return "-" if value is None else f"{value:.3f}"
