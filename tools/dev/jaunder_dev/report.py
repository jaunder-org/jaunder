"""The human view of a host-proof JSON report.

A diagnostic's rendered `report` (trace tables) comes first, verbatim, then one
line per step. The producing executable's exit status remains the verdict.
"""

import json
from typing import Any


def _jq_truthy(value: Any) -> bool:
    # As in step-report.jq: an optional field counts unless null or false.
    return value is not None and value is not False


def _render(report: Any) -> str:
    lines = []
    if _jq_truthy(report.get("report")):
        lines.append(report["report"])
    for step in report["steps"]:
        skipped = _jq_truthy(step.get("skipped"))
        mark = "skip" if skipped else " ok " if step["ok"] else "FAIL"
        line = f"[{mark}] {step['name']}"
        if not step["ok"] or step["duration_ms"] >= 1000:
            line += f" ({step['duration_ms']} ms)"
        if _jq_truthy(step.get("detail")):
            line += f" — {step['detail']}"
        lines.append(line)
    return "".join(f"{line}\n" for line in lines)


def render(document: bytes) -> str:
    """Renders the report on host-proof's stdout; anything else passes through.

    Output that is not a step report, whatever its shape, is shown as it came,
    so the step's own status and diagnostics always survive.
    """
    try:
        return _render(json.loads(document))
    except ValueError, LookupError, TypeError, AttributeError:
        return document.decode(errors="replace")
