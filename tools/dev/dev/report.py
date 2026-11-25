"""The human view of a step report: a host-proof tool's JSON, or `dev`'s own.

A diagnostic's rendered `report` (trace tables) comes first, verbatim, then one
line per step. The producing executable's exit status remains the verdict.
"""

import json
import sys
import time
from collections.abc import Callable, Sequence
from typing import Any

from tabulate import tabulate

from dev import steps
from dev.traces import EvidenceError


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


def table(headers: Sequence[str], rows: Sequence[Sequence[str]]) -> str:
    """`rows` under `headers` as a box-drawn table, without a final newline.

    Cells arrive already formatted: number parsing is off, so `1.000` stays
    `1.000` rather than becoming `1`.
    """
    return tabulate(
        rows, headers=headers, tablefmt="simple_outline", disable_numparse=True
    )


def verdict(
    name: str,
    decide: Callable[[], str],
    refused: Callable[[EvidenceError], int] = lambda _error: 1,
) -> None:
    """Runs one in-process verdict as the step `name` and shows its result.

    `decide` returns the step's detail, having written any report of its own
    first. Evidence it refuses fails the step with the status `refused` gives
    that refusal, as a failed tool's status would.
    """
    started_ns = time.time_ns()
    with steps.phase(name):
        try:
            detail, code = decide(), 0
        except EvidenceError as error:
            detail, code = str(error), refused(error)
        duration_ms = (time.time_ns() - started_ns) // 1_000_000
        step = {
            "name": name,
            "ok": not code,
            "duration_ms": duration_ms,
            "detail": detail,
        }
        sys.stdout.write(render_report({"steps": [step]}))
        sys.stdout.flush()
        if code:
            raise steps.StepFailed(code, signalled=False)
