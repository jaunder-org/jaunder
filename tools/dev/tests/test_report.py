"""The human view of a host tool's JSON report, and its absence."""

import json

import pytest
from conftest import Checkout

from dev import report

# Reports and their exact rendering.
REPORT = {
    "command": "wasm-audit",
    "ok": False,
    "duration_ms": 2042,
    "steps": [
        {"name": "fast", "ok": True, "skipped": False, "duration_ms": 999},
        {
            "name": "slow",
            "ok": True,
            "skipped": False,
            "duration_ms": 1000,
            "detail": "verified CSR bundle /nix/store/x",
        },
        {"name": "blank", "ok": True, "skipped": False, "duration_ms": 3, "detail": ""},
        {"name": "skipped", "ok": False, "skipped": True, "duration_ms": 0},
        {
            "name": "broken",
            "ok": False,
            "skipped": False,
            "duration_ms": 12,
            "detail": "line one\nline two — ü",
        },
    ],
    "report": "table\n  row",
}
RENDERED = (
    "table\n"
    "  row\n"
    "[ ok ] fast\n"
    "[ ok ] slow (1000 ms) — verified CSR bundle /nix/store/x\n"
    "[ ok ] blank\n"
    "[skip] skipped (0 ms)\n"
    "[FAIL] broken (12 ms) — line one\n"
    "line two — ü\n"
)
PLAIN = {
    "command": "wasm-audit",
    "ok": True,
    "duration_ms": 5,
    "steps": [{"name": "wasm-audit", "ok": True, "skipped": False, "duration_ms": 5}],
}


@pytest.mark.parametrize(
    ("document", "rendered"),
    [(REPORT, RENDERED), (PLAIN, "[ ok ] wasm-audit\n")],
)
def test_a_report_renders_its_table_then_one_line_per_step(
    document: dict[str, object], rendered: str
) -> None:
    assert report.render(json.dumps(document).encode()) == rendered


# ADR-0247: a tool prints exactly one report, so empty output is not one.
NOT_REPORTS = [
    b"",
    b" \n",
    b"not a report\n",
    b"1\n",
    b'"text"\n',
    b"[]\n",
    b'{"command": "wasm-audit"}\n',
    b'{"steps": 5}\n',
    b'{"steps": [{"name": "no outcome"}]}\n',
]


@pytest.mark.parametrize("output", NOT_REPORTS)
def test_output_that_is_not_a_report_cannot_be_rendered(output: bytes) -> None:
    assert report.render(output) is None


def test_a_failed_tool_keeps_its_own_status_without_a_report(
    checkout: Checkout,
) -> None:
    checkout.tool("cargo", "print('not a report'); sys.exit(3)")
    result = checkout.dev("wasm-audit", "report")
    assert result.returncode == 3
    assert result.stdout == b"not a report\n"
    assert result.stderr == b""
