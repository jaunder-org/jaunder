"""Strict decoding of lifted e2e evidence: standard JSON, typed fields, the
Playwright report and the lane catalog."""

import json
from pathlib import Path
from typing import Any

import pytest
from conftest import FIXTURES, REPOSITORY, require

from dev.evidence import (
    CATALOG,
    EvidenceError,
    finite,
    loads,
    pair_lanes,
    read_catalog,
    report_specs,
)
from dev.traces import Filters, MalformedJsonAttr, parse_json_attr, parse_spans

CATALOG_FIXTURE = FIXTURES / "e2e-lanes.json"


# Standard JSON only.


@pytest.mark.parametrize("token", ["NaN", "Infinity", "-Infinity"])
def test_a_nonstandard_number_token_is_malformed_evidence(token: str) -> None:
    with pytest.raises(ValueError, match=f"`{token}` is not standard JSON"):
        loads(f"[{token}]")
    report = f'{{"suites": [{{"specs": [{{"line": {token}, "tests": []}}]}}]}}'
    with pytest.raises(EvidenceError, match="parsing report: `.*` is not standard"):
        report_specs(report, "parsing report")
    trace = {
        "resourceSpans": [
            {
                "scopeSpans": [
                    {
                        "spans": [
                            {
                                "name": "e2e.test",
                                "attributes": [
                                    {
                                        "key": "e2e.navigation_top_json",
                                        "value": {"stringValue": f"[{token}]"},
                                    }
                                ],
                            }
                        ]
                    }
                ]
            }
        ]
    }
    (span,) = parse_spans(json.dumps(trace), Filters(), "trace")
    with pytest.raises(MalformedJsonAttr):
        parse_json_attr(span.raw, "e2e.navigation_top_json", span.source)
    with pytest.raises(EvidenceError, match="failed to parse JSON line 1"):
        parse_spans(f'{{"resourceSpans": {token}}}', Filters(), "t")


def test_only_a_finite_number_is_finite() -> None:
    assert finite(2) == 2.0
    assert finite(-1.5) == -1.5
    for value in [True, "1", None, float("inf"), 10**400]:
        assert finite(value) is None


# The Playwright report.


@pytest.mark.parametrize(
    ("document", "message"),
    [
        ([], "invalid type: expected an object"),
        ({"suites": [{"specs": [{}]}]}, "missing field `tests`"),
        ({"suites": [{}]}, "missing field `specs`"),
        ({"suites": None}, "missing field `suites`"),
        ({"suites": {}}, "`suites`: expected an array"),
        ({"suites": [1]}, "invalid type: expected an object"),
        ({"suites": [{"specs": [{"id": 1, "tests": []}]}]}, "`id`: expected a string"),
        (
            {"suites": [{"specs": [{"line": -1, "tests": []}]}]},
            "`line`: expected a non-negative integer",
        ),
        (
            {"suites": [{"specs": [{"line": True, "tests": []}]}]},
            "`line`: expected a non-negative integer",
        ),
        (
            {"suites": [{"specs": [{"tests": [{"projectName": 7}]}]}]},
            "`projectName`: expected a string",
        ),
        (
            {"suites": [{"specs": [{"tests": [{"results": [{"duration": "1"}]}]}]}]},
            "`duration`: expected a finite number",
        ),
        (
            {"suites": [{"specs": [{"tests": [{"results": [{"retry": 1.5}]}]}]}]},
            "`retry`: expected a non-negative integer",
        ),
    ],
)
def test_a_report_is_decoded_strictly(document: Any, message: str) -> None:
    with pytest.raises(EvidenceError, match=f"^report: .*{message}"):
        report_specs(json.dumps(document), "report")


@pytest.mark.parametrize("duration", ["1e999", "1" + "0" * 400])
def test_a_duration_beyond_a_float_is_not_a_finite_number(duration: str) -> None:
    report = (
        '{"suites": [{"specs": [{"tests": [{"results": [{"duration": '
        + duration
        + "}]}]}]}]}"
    )
    with pytest.raises(EvidenceError, match="`duration`: expected a finite number"):
        report_specs(report, "report")


def test_large_counts_are_counts() -> None:
    # Only the kind is checked; no writer bounds a count's width.
    report = {
        "suites": [
            {
                "specs": [
                    {
                        "line": 2**70,
                        "tests": [{"results": [{"retry": 2**40, "duration": 1}]}],
                    }
                ]
            }
        ]
    }
    (spec,) = report_specs(json.dumps(report), "report")
    assert spec.line == 2**70
    assert spec.tests[0].results[0].retry == 2**40


def test_report_specs_are_visited_in_reporter_order() -> None:
    document = {
        "suites": [
            {
                "suites": [{"suites": [], "specs": [{"id": "nested", "tests": []}]}],
                "specs": [{"id": "outer", "tests": []}],
            }
        ]
    }
    assert [spec.id for spec in report_specs(json.dumps(document), "report")] == [
        "outer",
        "nested",
    ]
    assert report_specs("{}", "report") == []


# The lane catalog.


@pytest.mark.parametrize(
    ("catalog", "message"),
    [
        (None, "reading lane catalog"),
        ("{", "parsing"),
        ('{"schemaVersion": 2, "lanes": [{}]}', "parsing .*missing field `backend`"),
        ('{"schemaVersion": 2, "lanes": []}', "unsupported or empty"),
        ('{"schemaVersion": 1, "lanes": []}', "unsupported or empty"),
        ("[]", "expected an object"),
    ],
)
def test_an_unusable_catalog_is_refused(
    tmp_path: Path, catalog: str | None, message: str
) -> None:
    path = tmp_path / "lanes.json"
    if catalog is not None:
        path.write_text(catalog)
    with pytest.raises(EvidenceError, match=message):
        read_catalog(path)


def test_a_catalog_with_a_duplicate_identity_is_refused(tmp_path: Path) -> None:
    path = tmp_path / "lanes.json"
    edit_catalog = json.loads(CATALOG_FIXTURE.read_text())
    edit_catalog["lanes"].append(edit_catalog["lanes"][0])
    path.write_text(json.dumps(edit_catalog))
    with pytest.raises(EvidenceError, match="duplicate ownership catalog identity"):
        read_catalog(path)


def test_a_pairs_lanes_are_its_enabled_ones_in_catalog_order() -> None:
    catalog = read_catalog(CATALOG_FIXTURE)
    assert [lane.identity for lane in pair_lanes(catalog, "sqlite", "firefox")] == [
        f"sqlite-firefox-{index}-of-5" for index in range(1, 6)
    ]
    assert pair_lanes(catalog, "sqlite", "webkit") == []


def test_the_catalog_fixture_is_the_repository_catalog() -> None:
    """The verdict tests read the catalog the lanes really run under, as the
    Rust tests read it in place; refresh the fixture by copying it."""
    catalog = REPOSITORY / CATALOG
    require(catalog.is_file(), f"the repository's {CATALOG}")
    assert CATALOG_FIXTURE.read_bytes() == catalog.read_bytes()
