"""The daily advisory scan's reading of cargo-deny's report.

`fixtures/cargo-deny/url-2.5.8-advisories.jsonl` is the pinned cargo-deny's
stderr for `--format json check advisories` on this workspace with `url` held
at 2.5.8 (RUSTSEC-2026-0592). Filing and commenting on issues is proved by a
live run against the tracker, not here.
"""

import json
from pathlib import Path

from dev.advisories import Advisory, findings

REPORT = (
    Path(__file__).parent / "fixtures" / "cargo-deny" / "url-2.5.8-advisories.jsonl"
).read_bytes()
URL = Advisory(
    "RUSTSEC-2026-0592",
    "url",
    "2.5.8",
    "Stack overflow in `Url::origin()` on nested `blob:` URLs",
)


def test_a_vulnerability_is_one_finding() -> None:
    assert findings(REPORT) == [URL]


def test_the_title_names_the_advisory_crate_and_problem() -> None:
    assert URL.title == (
        "fix(deps): RUSTSEC-2026-0592 url: "
        "Stack overflow in `Url::origin()` on nested `blob:` URLs"
    )


def test_each_advisory_is_one_finding_however_often_it_is_reported() -> None:
    """cargo-deny reports an advisory once per affected crate version."""
    diagnostic = REPORT.splitlines()[0]
    other = json.loads(diagnostic)
    other["fields"]["advisory"]["id"] = "RUSTSEC-2026-9999"
    report = b"\n".join([diagnostic, diagnostic, json.dumps(other).encode(), b""])
    assert [finding.id for finding in findings(report)] == [
        "RUSTSEC-2026-0592",
        "RUSTSEC-2026-9999",
    ]


def test_only_errors_with_an_advisory_are_findings() -> None:
    """A warning, a note, the summary and a non-advisory error file nothing;
    the scan's red status still reports them."""
    diagnostic = json.loads(REPORT.splitlines()[0])
    warning = json.loads(REPORT.splitlines()[0])
    warning["fields"]["severity"] = "warning"
    unrelated = {"type": "diagnostic", "fields": {"severity": "error"}}
    del diagnostic["fields"]["advisory"]
    report = b"\n".join(
        json.dumps(record).encode() for record in [warning, unrelated, diagnostic]
    )
    assert findings(report + b"\n" + REPORT.splitlines()[1]) == []
