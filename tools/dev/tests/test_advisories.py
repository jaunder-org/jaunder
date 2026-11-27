"""The daily advisory scan: its reading of cargo-deny's report, and what it
asks `gh` to do with each finding.

`fixtures/cargo-deny/url-2.5.8-advisories.jsonl` is the pinned cargo-deny's
stderr for `--format json check advisories` on this workspace with `url` held
at 2.5.8 (RUSTSEC-2026-0592). `dev advisory-scan` runs for real against fake
`cargo` and `gh`, which replay that report and record their arguments; a live
run against the tracker proved the filing and commenting themselves (#2147).
"""

import json
from collections.abc import Sequence
from pathlib import Path

from conftest import Checkout

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


def test_the_title_names_the_advisory_crate_and_summary() -> None:
    assert URL.title == (
        "fix(deps): RUSTSEC-2026-0592 url: "
        "Stack overflow in `Url::origin()` on nested `blob:` URLs"
    )


def test_each_advisory_is_one_finding_however_often_it_is_reported() -> None:
    """cargo-deny reports an advisory once per affected crate version; a blank
    line is not a record."""
    diagnostic = REPORT.splitlines()[0]
    other = json.loads(diagnostic)
    other["fields"]["advisory"]["id"] = "RUSTSEC-2026-9999"
    report = b"\n".join([diagnostic, b"", diagnostic, json.dumps(other).encode()])
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


# Fakes that record each call's argv as one JSON line in `calls`. cargo replays
# DENY_REPORT on stderr and exits DENY_STATUS; gh answers `issue list` with
# GH_OPEN.
RECORD = """
with open("calls", "a") as calls:
    calls.write(json.dumps(sys.argv[1:]) + "\\n")
"""
CARGO = f"""import json
{RECORD}
sys.stderr.buffer.write(pathlib.Path(os.environ["DENY_REPORT"]).read_bytes())
sys.exit(int(os.environ["DENY_STATUS"]))
"""
GH = f"""import json
{RECORD}
if sys.argv[1:3] == ["issue", "list"]:
    print(os.environ["GH_OPEN"])
"""
DENY = ["deny", "--format", "json", "check", "advisories"]
SEARCH = [
    "issue",
    "list",
    "--repo",
    "jaunder-org/jaunder",
    "--state",
    "open",
    "--search",
    "RUSTSEC-2026-0592 in:title",
    "--json",
    "number,title",
]


def scan(
    checkout: Checkout,
    report: bytes,
    status: int,
    open_issues: Sequence[dict[str, object]] = (),
    run_id: str = "42",
) -> tuple[int, bytes, list[list[str]]]:
    checkout.tool("cargo", CARGO)
    checkout.tool("gh", GH)
    (checkout.root / "report").write_bytes(report)
    result = checkout.dev(
        "advisory-scan",
        DENY_REPORT=str(checkout.root / "report"),
        DENY_STATUS=str(status),
        GH_OPEN=json.dumps(list(open_issues)),
        GITHUB_SERVER_URL="https://github.com",
        GITHUB_REPOSITORY="jaunder-org/jaunder",
        GITHUB_RUN_ID=run_id,
    )
    calls = (checkout.root / "calls").read_text().splitlines()
    return result.returncode, result.stderr, [json.loads(call) for call in calls]


RUN = "https://github.com/jaunder-org/jaunder/actions/runs/42"


def test_a_new_advisory_opens_one_labelled_issue(checkout: Checkout) -> None:
    code, _, calls = scan(checkout, REPORT, 1)
    assert code == 1
    assert calls[:2] == [DENY, SEARCH]
    (create,) = calls[2:]
    assert create[:9] == [
        "issue",
        "create",
        "--repo",
        "jaunder-org/jaunder",
        "--title",
        URL.title,
        "--label",
        "security,dependencies,needs-triage",
        "--body",
    ]
    body = create[9]
    assert "[RUSTSEC-2026-0592](https://rustsec.org/advisories/" in body
    assert "`url` 2.5.8 on `main`" in body
    assert f"by {RUN}." in body


def test_an_open_issue_for_the_advisory_gets_a_comment_instead(
    checkout: Checkout,
) -> None:
    """Only a title that carries the ID matches the fuzzy search; of two, the
    oldest is the scan's."""
    open_issues = [
        {"number": 9, "title": "RUSTSEC-2026-0593 something else"},
        {"number": 7, "title": "fix(deps): RUSTSEC-2026-0592 url: older"},
        {"number": 8, "title": "fix(deps): RUSTSEC-2026-0592 url: newer"},
    ]
    code, _, calls = scan(checkout, REPORT, 1, open_issues)
    assert code == 1
    assert calls[:2] == [DENY, SEARCH]
    (comment,) = calls[2:]
    assert comment[:6] == [
        "issue",
        "comment",
        "7",
        "--repo",
        "jaunder-org/jaunder",
        "--body",
    ]
    assert comment[6].startswith("Still reported on ")
    assert comment[6].endswith(f" by {RUN}.")


def test_a_clean_scan_files_nothing(checkout: Checkout) -> None:
    assert scan(checkout, REPORT.splitlines()[1] + b"\n", 0)[::2] == (0, [DENY])


def test_a_failure_without_advisories_is_red_and_shows_its_report(
    checkout: Checkout,
) -> None:
    report = b'{"type":"log","fields":{"level":"ERROR","message":"fetch failed"}}\n'
    code, stderr, calls = scan(checkout, report, 1)
    assert (code, calls) == (1, [DENY])
    assert report in stderr


def test_a_local_run_says_so(checkout: Checkout) -> None:
    """Outside Actions there is no run to link; an empty run ID counts as none,
    which also keeps a CI job's own ID out of this test."""
    body = scan(checkout, REPORT, 1, run_id="")[2][-1][9]
    assert "Found on " in body
    assert " by a local run." in body
