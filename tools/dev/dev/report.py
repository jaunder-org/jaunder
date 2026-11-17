"""The human view of a host-proof JSON report.

A diagnostic's rendered `report` (trace tables) comes first, verbatim, then one
line per step. The producing executable's exit status remains the verdict.
"""

import json
from typing import Any


def _render(report: Any) -> str:
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
        return _render(json.loads(document))
    except ValueError, LookupError, TypeError, AttributeError:
        return None
