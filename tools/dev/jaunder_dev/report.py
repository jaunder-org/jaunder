"""The human view of a host-proof JSON report.

A diagnostic's rendered `report` (trace tables) comes first, verbatim, then one
line per step. The producing executable's exit status remains the verdict.
"""

import json
from typing import Any


def _set(value: Any) -> bool:
    # The report's optional fields count as present unless null or false.
    return value is not None and value is not False


def render(document: bytes) -> str:
    """Renders the report on host-proof's stdout; anything else passes through."""
    try:
        report = json.loads(document)
    except ValueError:
        return document.decode(errors="replace")
    lines = []
    if _set(report.get("report")):
        lines.append(report["report"])
    for step in report["steps"]:
        mark = "skip" if _set(step.get("skipped")) else " ok " if step["ok"] else "FAIL"
        line = f"[{mark}] {step['name']}"
        if not step["ok"] or step["duration_ms"] >= 1000:
            line += f" ({step['duration_ms']} ms)"
        if _set(step.get("detail")):
            line += f" — {step['detail']}"
        lines.append(line)
    return "".join(f"{line}\n" for line in lines)
