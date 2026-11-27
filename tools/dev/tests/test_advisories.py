"""The daily advisory scan: its reading of cargo-deny's reports, and what it
asks `gh` to do with each finding.

The fixtures are the pinned cargo-deny's stderr for
`--format json check advisories`: `url-2.5.8-advisories.jsonl` on the product
workspace with `url` held at 2.5.8 (RUSTSEC-2026-0592), and
`zip-9.0.4-tools-advisories.jsonl` on the tools workspace as `main` had it
(RUSTSEC-2026-0548). `dev advisory-scan` runs for real against fake `cargo` and
`gh`, which replay those reports and record their arguments; live runs against
the tracker proved the filing and commenting themselves (#2147, #2163).
"""

import json
from collections.abc import Sequence
from pathlib import Path

import pytest
from conftest import Checkout

from dev.advisories import Advisory, Place, findings, merged

FIXTURES = Path(__file__).parent / "fixtures" / "cargo-deny"
REPORT = (FIXTURES / "url-2.5.8-advisories.jsonl").read_bytes()
TOOLS_REPORT = (FIXTURES / "zip-9.0.4-tools-advisories.jsonl").read_bytes()
# A report with nothing to find: its summary record alone.
CLEAN = REPORT.splitlines()[1] + b"\n"
URL_PLACE = Place("url", "2.5.8", "product")
URL = Advisory(
    "RUSTSEC-2026-0592",
    "url",
    "Stack overflow in `Url::origin()` on nested `blob:` URLs",
    (URL_PLACE,),
)


def at_version(report: bytes, version: bytes) -> bytes:
    """`report` with its affected `url` at `version`."""
    return report.replace(b'"2.5.8"', b'"' + version + b'"')


def test_a_vulnerability_is_one_finding_in_its_workspace() -> None:
    assert findings(REPORT, "product") == [URL]
    (zip_advisory,) = findings(TOOLS_REPORT, "tools")
    assert (zip_advisory.id, zip_advisory.found) == (
        "RUSTSEC-2026-0548",
        (Place("zip", "9.0.4", "tools"),),
    )
    assert URL_PLACE.described() == "`url` 2.5.8 in the product workspace"


def test_the_title_names_the_advisory_crate_and_summary() -> None:
    assert URL.title == (
        "fix(deps): RUSTSEC-2026-0592 url: "
        "Stack overflow in `Url::origin()` on nested `blob:` URLs"
    )


def test_each_advisory_is_one_issue_at_each_distinct_version() -> None:
    """cargo-deny reports an advisory once per affected crate version, possibly
    more than once in a workspace; a blank line is not a record."""
    diagnostic = REPORT.splitlines()[0]
    older = at_version(diagnostic, b"2.5.7")
    other = json.loads(diagnostic)
    other["fields"]["advisory"]["id"] = "RUSTSEC-2026-9999"
    report = b"\n".join(
        [diagnostic, b"", older, diagnostic, json.dumps(other).encode()]
    )
    first, second = merged(findings(report, "product"))
    assert (first.id, first.found) == (
        "RUSTSEC-2026-0592",
        (URL_PLACE, Place("url", "2.5.7", "product")),
    )
    assert second.id == "RUSTSEC-2026-9999"


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
    assert findings(report + b"\n" + CLEAN, "product") == []


def test_an_advisory_in_both_workspaces_is_one_found_in_each() -> None:
    tools = findings(at_version(REPORT, b"2.5.7"), "tools")
    (both,) = merged([*findings(REPORT, "product"), *tools])
    assert both.found == (URL_PLACE, Place("url", "2.5.7", "tools"))


# Fakes that record each call's argv as one JSON line in `calls`. cargo replays
# the product or tools workspace's report (DENY_<WORKSPACE>_REPORT) on stderr
# and exits its status (DENY_<WORKSPACE>_STATUS); gh answers `issue list` with
# GH_OPEN.
RECORD = """
with open("calls", "a") as calls:
    calls.write(json.dumps(sys.argv[1:]) + "\\n")
"""
CARGO = f"""import json
{RECORD}
workspace = "TOOLS" if "--manifest-path" in sys.argv else "PRODUCT"
report = os.environ[f"DENY_{{workspace}}_REPORT"]
sys.stderr.buffer.write(pathlib.Path(report).read_bytes())
sys.exit(int(os.environ[f"DENY_{{workspace}}_STATUS"]))
"""
GH = f"""import json
{RECORD}
if sys.argv[1:3] == ["issue", "list"]:
    print(os.environ["GH_OPEN"])
"""
DENY = ["deny", "--format", "json", "check", "advisories"]
TOOLS_DENY = ["deny", "--manifest-path", "tools/Cargo.toml", *DENY[1:]]


def search(advisory: str) -> list[str]:
    return [
        "issue",
        "list",
        "--repo",
        "jaunder-org/jaunder",
        "--state",
        "open",
        "--search",
        f"{advisory} in:title",
        "--json",
        "number,title",
    ]


def scan(
    checkout: Checkout,
    product: tuple[bytes, int] = (CLEAN, 0),
    tools: tuple[bytes, int] = (CLEAN, 0),
    open_issues: Sequence[dict[str, object]] = (),
    run_id: str = "42",
) -> tuple[int, bytes, list[list[str]]]:
    checkout.tool("cargo", CARGO)
    checkout.tool("gh", GH)
    (checkout.root / "product-report").write_bytes(product[0])
    (checkout.root / "tools-report").write_bytes(tools[0])
    result = checkout.dev(
        "advisory-scan",
        DENY_PRODUCT_REPORT=str(checkout.root / "product-report"),
        DENY_PRODUCT_STATUS=str(product[1]),
        DENY_TOOLS_REPORT=str(checkout.root / "tools-report"),
        DENY_TOOLS_STATUS=str(tools[1]),
        GH_OPEN=json.dumps(list(open_issues)),
        GITHUB_SERVER_URL="https://github.com",
        GITHUB_REPOSITORY="jaunder-org/jaunder",
        GITHUB_RUN_ID=run_id,
    )
    calls = (checkout.root / "calls").read_text().splitlines()
    return result.returncode, result.stderr, [json.loads(call) for call in calls]


RUN = "https://github.com/jaunder-org/jaunder/actions/runs/42"


def test_a_new_advisory_opens_one_labelled_issue(checkout: Checkout) -> None:
    code, _, calls = scan(checkout, product=(REPORT, 1))
    assert code == 1
    assert calls[:3] == [DENY, TOOLS_DENY, search("RUSTSEC-2026-0592")]
    (create,) = calls[3:]
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
    assert "- `url` 2.5.8 in the product workspace\n" in body
    assert f"by {RUN}." in body


def test_a_tools_advisory_is_reported_like_a_product_one(
    checkout: Checkout,
) -> None:
    code, _, calls = scan(checkout, tools=(TOOLS_REPORT, 1))
    assert code == 1
    assert calls[:3] == [DENY, TOOLS_DENY, search("RUSTSEC-2026-0548")]
    (create,) = calls[3:]
    assert create[5].startswith("fix(deps): RUSTSEC-2026-0548 zip: ")
    assert "- `zip` 9.0.4 in the tools workspace\n" in create[9]


def test_an_advisory_in_both_workspaces_is_one_issue(checkout: Checkout) -> None:
    tools = at_version(REPORT, b"2.5.7")
    code, _, calls = scan(checkout, product=(REPORT, 1), tools=(tools, 1))
    assert code == 1
    assert calls[:3] == [DENY, TOOLS_DENY, search("RUSTSEC-2026-0592")]
    (create,) = calls[3:]
    assert (
        "- `url` 2.5.8 in the product workspace\n- `url` 2.5.7 in the tools workspace\n"
    ) in create[9]


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
    code, _, calls = scan(checkout, product=(REPORT, 1), open_issues=open_issues)
    assert code == 1
    assert calls[:3] == [DENY, TOOLS_DENY, search("RUSTSEC-2026-0592")]
    (comment,) = calls[3:]
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
    assert scan(checkout)[::2] == (0, [DENY, TOOLS_DENY])


FETCH_FAILED = b'{"type":"log","fields":{"level":"ERROR","message":"fetch failed"}}\n'


@pytest.mark.parametrize(
    ("product", "tools"),
    [((FETCH_FAILED, 1), (CLEAN, 0)), ((CLEAN, 0), (FETCH_FAILED, 1))],
    ids=["product", "tools"],
)
def test_a_failure_in_either_workspace_is_red_and_shows_its_report(
    checkout: Checkout, product: tuple[bytes, int], tools: tuple[bytes, int]
) -> None:
    code, stderr, calls = scan(checkout, product=product, tools=tools)
    assert (code, calls) == (1, [DENY, TOOLS_DENY])
    assert FETCH_FAILED in stderr


def test_a_local_run_says_so(checkout: Checkout) -> None:
    """Outside Actions there is no run to link; an empty run ID counts as none,
    which also keeps a CI job's own ID out of this test."""
    body = scan(checkout, product=(REPORT, 1), run_id="")[2][-1][9]
    assert "Found on " in body
    assert " by a local run." in body
