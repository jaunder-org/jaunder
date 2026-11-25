"""`dev trace-analysis analyze` and `run`: the analysis sections, their
rendering, the Playwright duration join and the commands' exits."""

import json
from pathlib import Path
from typing import Any

import pytest
from conftest import FIXTURES, Checkout, write_lane_capture

from dev import report
from dev.trace_analysis import (
    LIFECYCLE_SPAN_NAME,
    Analysis,
    AttemptKey,
    ReportedDurations,
    Section,
    analyze,
    analyze_spans,
    lanes,
    render,
)
from dev.traces import EvidenceError, Filters, MalformedJsonAttr, Span, parse_spans

SAMPLE = FIXTURES / "trace-analysis" / "otel-traces-sample.jsonl"
NONE = ReportedDurations()


def line(spans: list[dict[str, Any]]) -> str:
    return json.dumps({"resourceSpans": [{"scopeSpans": [{"spans": spans}]}]})


def attr(key: str, value: Any) -> dict[str, Any]:
    return {"key": key, "value": value}


def text(key: str, value: str) -> dict[str, Any]:
    return attr(key, {"stringValue": value})


def blob(key: str, value: Any) -> dict[str, Any]:
    return text(key, json.dumps(value))


def spans_of(spans: list[dict[str, Any]], source: str = "inline") -> list[Span]:
    return parse_spans(line(spans), Filters(), source)


def fixture_spans() -> list[Span]:
    """The sample's spans with chromium's malformed navigation JSON dropped."""
    spans = parse_spans(SAMPLE.read_text(), Filters(), "sample")
    for span in spans:
        if span.project == "chromium":
            span.raw["attributes"] = [
                a
                for a in span.raw["attributes"]
                if a["key"] != "e2e.navigation_top_json"
            ]
    return spans


def sections(analysis: Analysis) -> dict[str, Section]:
    return {section.title: section for section in analysis.sections}


def rows(analysis: Analysis, title: str) -> list[list[str]]:
    return sections(analysis)[title].rows


def analysis_of(spans: list[Span], reported: ReportedDurations = NONE) -> Analysis:
    return analyze_spans(spans, None, reported, 25)


def playwright(*attempts: tuple[str, str, int | None, float | None]) -> dict[str, Any]:
    """A report of (title, project, retry, duration) attempts, one spec each."""
    return {
        "suites": [
            {
                "specs": [
                    {
                        "title": title,
                        "tests": [
                            {
                                "projectName": project,
                                "results": [
                                    {
                                        key: value
                                        for key, value in (
                                            ("retry", retry),
                                            ("duration", duration),
                                        )
                                        if value is not None
                                    }
                                ],
                            }
                        ],
                    }
                    for title, project, retry, duration in attempts
                ]
            }
        ]
    }


def write(path: Path, value: Any) -> Path:
    path.write_text(json.dumps(value))
    return path


# Playwright's per-attempt durations.

NESTED = {
    "suites": [
        {
            "title": "auth.spec.ts",
            "specs": [
                {
                    "title": "logs in",
                    "tests": [
                        {
                            "projectName": "chromium",
                            "results": [
                                {"retry": 0, "duration": 1200.0},
                                {"retry": 1, "duration": 900.0},
                            ],
                        }
                    ],
                }
            ],
            "suites": [
                {
                    "title": "nested describe",
                    "specs": [
                        {
                            "title": "deep test",
                            "tests": [
                                {
                                    "projectName": "firefox",
                                    "results": [{"retry": 0, "duration": 42.0}],
                                }
                            ],
                        }
                    ],
                }
            ],
        }
    ]
}


def test_each_attempt_and_nested_suite_is_read_and_an_unknown_one_is_absent(
    tmp_path: Path,
) -> None:
    durations = ReportedDurations.from_paths([write(tmp_path / "r.json", NESTED)])
    assert durations.get("any", AttemptKey("logs in", "chromium", 0)) == 1200.0
    assert durations.get("any", AttemptKey("logs in", "chromium", 1)) == 900.0
    assert durations.get("any", AttemptKey("deep test", "firefox", 0)) == 42.0
    # Absent stays absent: a zero denominator would read as 100% uncovered.
    assert durations.get("any", AttemptKey("never ran", "chromium", 0)) is None
    assert len(durations) == 3


def test_per_source_scoping_keeps_two_backends_apart(tmp_path: Path) -> None:
    sqlite = write(tmp_path / "s.json", NESTED)
    postgres = write(tmp_path / "p.json", playwright(("logs in", "chromium", 0, 5555)))
    durations = ReportedDurations.from_labeled(
        [("sqlite-chromium", sqlite), ("postgres-chromium", postgres)]
    )
    key = AttemptKey("logs in", "chromium", 0)
    assert durations.get("sqlite-chromium", key) == 1200.0
    assert durations.get("postgres-chromium", key) == 5555.0
    assert durations.get("other", key) is None


def test_unpaired_reports_that_disagree_are_refused_not_overwritten(
    tmp_path: Path,
) -> None:
    one = write(tmp_path / "a.json", NESTED)
    two = write(tmp_path / "b.json", playwright(("logs in", "chromium", 0, 9999.5)))
    with pytest.raises(EvidenceError) as refused:
        ReportedDurations.from_paths([one, two])
    assert str(refused.value).startswith(
        "playwright reports disagree on logs in [chromium] retry 0: "
        "1200 ms vs 9999.5 ms."
    )


def test_identical_duplicates_across_reports_are_fine(tmp_path: Path) -> None:
    one = write(tmp_path / "a.json", NESTED)
    assert len(ReportedDurations.from_paths([one, one])) == 3


def test_a_result_without_duration_or_retry_and_an_empty_report(
    tmp_path: Path,
) -> None:
    report_ = playwright(("t", "chromium", 0, None), ("u", "", None, 7.0))
    durations = ReportedDurations.from_paths([write(tmp_path / "r.json", report_)])
    assert len(durations) == 1
    assert durations.get("x", AttemptKey("u", "", 0)) == 7.0
    assert not len(ReportedDurations.from_paths([write(tmp_path / "e.json", {})]))


def test_an_unreadable_or_unparseable_report_is_refused(tmp_path: Path) -> None:
    with pytest.raises(EvidenceError, match="failed to read playwright report"):
        ReportedDurations.from_paths([tmp_path / "missing.json"])
    (tmp_path / "bad.json").write_text("{")
    with pytest.raises(EvidenceError, match="failed to parse playwright report"):
        ReportedDurations.from_paths([tmp_path / "bad.json"])


# The summary sections.


def test_slowest_spans_are_complete_and_sorted_descending() -> None:
    spans = fixture_spans()
    found = rows(analysis_of(spans), "Top 25 slowest spans")
    assert [row[0] for row in found] == ["5000.000", "3000.000", "200.000", "150.000"]
    # Empty strings render as `-`; the source never does.
    assert found[0][1:] == ["1" * 32, "e2e.test", "-", "-", "-", "-", "sample"]


def test_slowest_e2e_tests_are_only_e2e_test_spans() -> None:
    found = rows(analysis_of(fixture_spans()), "Top 25 slowest e2e.test spans")
    assert found == [
        ["5000.000", "firefox", "40", "12", "1" * 32, "timeline heavy"],
        ["3000.000", "chromium", "20", "6", "2" * 32, "timeline light"],
    ]


def test_by_project_groups_and_averages_and_trace_totals_sum_per_trace() -> None:
    analysis = analysis_of(fixture_spans())
    assert rows(analysis, "E2E test duration by project") == [
        ["firefox", "1", "5000.000", "5000.000", "40.00", "12.00"],
        ["chromium", "1", "3000.000", "3000.000", "20.00", "6.00"],
    ]
    assert rows(analysis, "Trace totals (sum of span durations)") == [
        ["5200.000", "2", "1" * 32],
        ["3150.000", "2", "2" * 32],
    ]


def test_unset_labels_and_counts_read_as_dashes_and_zeros() -> None:
    spans = spans_of(
        [
            {"name": "e2e.test", "attributes": [text("e2e.action_count", "-1")]},
            {"name": "e2e.test", "attributes": [text("e2e.action_count", "+3")]},
            {"name": "e2e.test", "attributes": [text("e2e.action_count", "1" * 21)]},
        ]
    )
    found = rows(analysis_of(spans), "Top 25 slowest e2e.test spans")
    assert [row[1:4] for row in found] == [
        ["-", "0", "0"],
        ["-", "3", "0"],
        ["-", "0", "0"],
    ]
    assert [row[5] for row in found] == ["-", "-", "-"]


def test_equal_keys_keep_their_first_seen_order() -> None:
    spans = spans_of([{"name": name, "traceId": name} for name in ["b", "a", "c"]])
    found = rows(analysis_of(spans), "Trace totals (sum of span durations)")
    assert [row[2] for row in found] == ["b", "a", "c"]


# Per-test span coverage.


def timed(
    name: str, span_id: str, parent: str, start_ms: int, end_ms: int, *extra: Any
) -> dict[str, Any]:
    return {
        "name": name,
        "spanId": span_id,
        "parentSpanId": parent,
        "startTimeUnixNano": str(start_ms * 1_000_000),
        "endTimeUnixNano": str(end_ms * 1_000_000),
        "attributes": [text("e2e.project", "chromium"), *extra],
    }


def lifecycle_tree(retry: str = "0") -> list[Span]:
    """An envelope whose two children overlap by 50 ms: their union is 300,
    a naive sum 350."""
    identity = (text("e2e.test", "logs in"), attr("e2e.retry", {"intValue": retry}))
    return spans_of(
        [
            timed(LIFECYCLE_SPAN_NAME, "aa", "", 1_000, 1_500, *identity),
            timed("e2e.context_mint", "bb", "aa", 1_000, 1_200),
            timed("e2e.test", "cc", "aa", 1_150, 1_300),
            # A child without timestamps covers nothing.
            {"name": "e2e.page", "parentSpanId": "aa"},
        ]
    )


def reported(ms: float, retry: int = 0) -> ReportedDurations:
    return ReportedDurations(global_={AttemptKey("logs in", "chromium", retry): ms})


COVERAGE = "Top 25 tests by unattributed time (reported vs lifecycle-tree union)"


def test_span_coverage_unions_overlapping_children_not_the_envelope() -> None:
    analysis = analysis_of(lifecycle_tree(), reported(500.0))
    assert analysis.coverage_note is None
    assert rows(analysis, COVERAGE) == [
        ["chromium", "500.000", "300.000", "200.000", "40.0", "logs in"]
    ]


def test_span_coverage_clamps_a_negative_remainder_and_a_zero_report() -> None:
    assert rows(analysis_of(lifecycle_tree(), reported(100.0)), COVERAGE)[0][3:5] == [
        "0.000",
        "0.0",
    ]
    assert rows(analysis_of(lifecycle_tree(), reported(0.0)), COVERAGE)[0][4] == "0.0"


def test_retry_attempts_join_separately_and_coverage_sorts_by_remainder() -> None:
    assert not rows(analysis_of(lifecycle_tree(), reported(500.0, retry=1)), COVERAGE)
    spans = [*lifecycle_tree(), *lifecycle_tree("1")]
    durations = ReportedDurations(
        global_={
            AttemptKey("logs in", "chromium", 0): 400.0,
            AttemptKey("logs in", "chromium", 1): 900.0,
        }
    )
    found = rows(analysis_of(spans, durations), COVERAGE)
    assert [row[1] for row in found] == ["900.000", "400.000"]


def test_the_coverage_note_says_why_the_section_is_empty() -> None:
    no_report = analysis_of(lifecycle_tree())
    assert no_report.coverage_note == (
        "no --playwright-report supplied; per-test coverage needs Playwright's "
        "own durations as the denominator"
    )
    no_lifecycle = analysis_of(fixture_spans(), reported(500.0))
    assert no_lifecycle.coverage_note == (
        "no `e2e.test.lifecycle` spans in the capture (pre-#794 traces have none)"
    )
    unmatched = analysis_of(lifecycle_tree(), reported(500.0, retry=3))
    assert unmatched.coverage_note == (
        "no lifecycle span matched a report entry (1 attempt(s) in the report)"
    )


# Boot decomposition coverage.


def navigation(
    commit_to_mount_ms: float | None = 120.0,
    phases: int | None = 3,
    init_start_ms: float | None = 40.0,
    schema: str | None = "direct-init-v1",
) -> dict[str, Any]:
    entry: dict[str, Any] = {
        "commitToMountMs": commit_to_mount_ms,
        "bootPhases": None
        if phases is None
        else {f"a{index}->a{index + 1}": 1.0 for index in range(phases)},
        "wasmInitStartMs": init_start_ms,
        "wasmInitStartToBootEntryMs": init_start_ms,
    }
    if schema is not None:
        entry["wasmTimingSchema"] = schema
    return entry


def navigation_span(
    source: str,
    project: str,
    dropped: int,
    navigations: list[Any],
    name: str = "e2e.test",
) -> list[Span]:
    return spans_of(
        [
            {
                "name": name,
                "attributes": [
                    text("e2e.project", project),
                    blob("e2e.navigation_top_json", navigations),
                    attr("e2e.navigation_top_dropped", {"intValue": str(dropped)}),
                ],
            }
        ],
        source,
    )


BOOT = "Boot decomposition coverage (from e2e.test/e2e.page navigation JSON)"


def boot_rows(spans: list[Span]) -> dict[tuple[str, str], list[str]]:
    return {(row[0], row[1]): row[2:] for row in rows(analysis_of(spans), BOOT)}


def test_boot_coverage_reports_legacy_without_counting_it_as_current_loss() -> None:
    spans = [
        *navigation_span(
            "sqlite",
            "firefox",
            0,
            [navigation(300.0, None, None, None), navigation(280.0, None, None, None)],
        ),
        *navigation_span(
            "sqlite", "chromium", 0, [navigation(), navigation(None, None, None, None)]
        ),
    ]
    assert boot_rows(spans) == {
        ("sqlite", "firefox"): ["2", "0", "2", "0", "0", "0"],
        ("sqlite", "chromium"): ["2", "1", "1", "1", "1", "0"],
    }


def test_boot_coverage_separates_sources_and_keeps_empty_pairs() -> None:
    spans = [
        *navigation_span("sqlite", "firefox", 0, [navigation()]),
        *navigation_span("postgres", "firefox", 0, [navigation()]),
        *spans_of([{"name": "e2e.test"}], "sqlite"),
    ]
    assert boot_rows(spans) == {
        ("sqlite", "firefox"): ["1", "1", "0", "1", "1", "0"],
        ("postgres", "firefox"): ["1", "1", "0", "1", "1", "0"],
        ("sqlite", "-"): ["0", "0", "0", "0", "0", "0"],
    }


def test_mounted_needs_commit_to_mount_and_full_marks_needs_every_mark() -> None:
    spans = [
        *navigation_span("s", "firefox", 0, [navigation(None)]),
        *navigation_span("s", "chromium", 0, [navigation(1.0, 4)]),
        *navigation_span("s", "webkit", 0, [navigation(1.0, 2)]),
        *navigation_span("s", "edge", 0, [navigation(1.0, 3, None)]),
    ]
    found = boot_rows(spans)
    # Boot phases alone are not mounted; a fourth phase is still full marks.
    assert found[("s", "firefox")][3:5] == ["0", "1"]
    assert found[("s", "chromium")][4] == "1"
    assert found[("s", "webkit")][4] == "0"
    assert found[("s", "edge")][4] == "0"


def test_boot_coverage_reconciles_test_and_page_sinks_and_sums_dropped() -> None:
    spans = [
        *navigation_span("s", "firefox", 2, [navigation()]),
        *navigation_span("s", "firefox", 3, [navigation()], "e2e.page"),
        # A page span without navigation JSON is not a navigation sink.
        *spans_of(
            [
                {
                    "name": "e2e.page",
                    "attributes": [
                        text("e2e.project", "firefox"),
                        attr("e2e.navigation_top_dropped", {"intValue": "9"}),
                    ],
                }
            ],
            "s",
        ),
    ]
    assert boot_rows(spans) == {("s", "firefox"): ["2", "2", "0", "2", "2", "5"]}


# The browser sections.


def test_navigation_sections_include_secondary_page_navigation_json() -> None:
    def target(url: str, total: float, mount: float) -> dict[str, Any]:
        return {"url": url, "totalMs": total, "commitToMountMs": mount}

    spans = [
        *navigation_span(
            "s", "c", 0, [target("http://jaunder.local:8080/default", 100.0, 40.0)]
        ),
        *navigation_span(
            "s",
            "c",
            0,
            [
                target("http://jaunder.local:8080/secondary", 250.0, 90.0),
                # No URL: phases only. A negative total: neither.
                {"totalMs": 5.0},
                {"url": "http://x/", "totalMs": -1.0},
            ],
            "e2e.page",
        ),
    ]
    analysis = analysis_of(spans)
    phases = rows(
        analysis,
        "Top 25 navigation phase hotspots (from e2e.test/e2e.page navigation JSON)",
    )
    assert phases[0] == ["250.000", "118.333", "355.000", "3", "navigation.total"]
    assert rows(analysis, "Top 25 slow navigation targets") == [
        ["250.000", "250.000", "250.000", "1", "jaunder.local:8080/secondary"],
        ["100.000", "100.000", "100.000", "1", "jaunder.local:8080/default"],
    ]


def test_action_long_task_and_resource_sections_over_the_fixture() -> None:
    analysis = analysis_of(fixture_spans())
    assert rows(analysis, "Top 25 e2e action hotspots (from e2e.action_top_json)") == [
        ["120.500", "90.250", "180.500", "2", "click"],
        ["80.000", "80.000", "80.000", "1", "fill"],
    ]
    # The chromium task of -10 ms is dropped.
    assert rows(analysis, "Top 25 long-task hotspots (from e2e.long_tasks_json)") == [
        ["90.000", "80.000", "160.000", "2", "longtask"],
        ["50.000", "50.000", "50.000", "1", "self"],
    ]
    assert rows(analysis, "Long-task totals by project") == [
        ["firefox", "1", "2", "140.000", "90.000"],
        ["chromium", "1", "1", "70.000", "70.000"],
    ]
    initiators = "Top 25 resource initiator hotspots (from e2e.resource_summary_json)"
    assert rows(analysis, initiators) == [
        ["300.000", "300.000", "300.000", "1", "fetch"],
        ["120.000", "115.000", "230.000", "2", "script"],
    ]
    assert [row[4:] for row in rows(analysis, "Top 25 slow resource assets")] == [
        ["fetch", "jaunder.local:8080/pkg/jaunder.wasm"],
        ["script", "cdn.example.com/x.js"],
        ["script", "jaunder.local:8080/pkg/jaunder.js"],
    ]


def test_malformed_and_unnamed_entries_take_their_defaults() -> None:
    spans = spans_of(
        [
            {
                "name": "e2e.test",
                "attributes": [
                    blob(
                        "e2e.action_top_json",
                        [{"name": "", "durationMs": 1}, {"name": "x"}, "junk"],
                    ),
                    blob("e2e.long_tasks_json", [{"duration": 4}, {"duration": -1}]),
                    blob(
                        "e2e.resource_summary_json",
                        {"topSlow": [{"durationMs": 2}, {"durationMs": -2}, {}]},
                    ),
                ],
            },
            {
                "name": "e2e.test",
                "attributes": [
                    blob("e2e.action_top_json", {"not": "an array"}),
                    blob("e2e.resource_summary_json", {"topSlow": "none"}),
                ],
            },
        ]
    )
    analysis = analysis_of(spans)
    assert not rows(analysis, "Top 25 e2e action hotspots (from e2e.action_top_json)")
    assert rows(analysis, "Top 25 long-task hotspots (from e2e.long_tasks_json)") == [
        ["4.000", "4.000", "4.000", "1", "longtask"]
    ]
    assert rows(analysis, "Top 25 slow resource assets") == [
        ["2.000", "2.000", "2.000", "1", "unknown", "unknown"]
    ]


def test_a_malformed_json_attribute_is_its_own_failure_naming_the_file(
    tmp_path: Path,
) -> None:
    path = tmp_path / "otel-traces.jsonl"
    path.write_text(SAMPLE.read_text())
    with pytest.raises(MalformedJsonAttr) as malformed:
        analyze([path], Filters(), NONE, 25)
    assert "e2e.navigation_top_json" in str(malformed.value)
    assert str(path) in str(malformed.value)


def test_the_project_filter_keeps_http_spans_and_titles_the_report() -> None:
    analysis = analyze([SAMPLE], Filters(project="firefox"), NONE, 25)
    assert analysis.project_filter == "firefox"
    assert [row[1] for row in rows(analysis, "Top 25 slowest e2e.test spans")] == [
        "firefox"
    ]
    names = [row[2] for row in rows(analysis, "Top 25 slowest spans")]
    assert "GET" in names
    assert "POST" in names
    assert render(analysis, 25).startswith("Project filter: firefox\n\n")


# Rendering.


def slow(count: int) -> list[Span]:
    return spans_of(
        [
            {
                "name": "e2e.test",
                "startTimeUnixNano": "0",
                "endTimeUnixNano": str(index * 1_000_000),
            }
            for index in range(count)
        ]
    )


def test_render_of_nothing_says_so() -> None:
    assert (
        render(analysis_of([]), 25) == "No spans found in the provided trace files.\n"
    )


def test_ranked_sections_are_cut_to_top_and_the_others_are_not() -> None:
    spans = [
        *slow(30),
        *navigation_span("a", "x", 0, []),
        *navigation_span("b", "x", 0, []),
    ]
    out = render(analyze_spans(spans, None, NONE, 5), 5)
    slowest = out.split("Top 5 slowest spans\n")[1].split("\n\n")[0]
    assert slowest.count(".000") == 5
    # Boot coverage is not ranked: every source renders.
    boot = out.split(f"{BOOT}\n")[1].split("\n\n")[0]
    assert "│ a " in boot
    assert "│ b " in boot


def test_render_emits_every_section_in_canonical_order(tmp_path: Path) -> None:
    durations = reported(500.0)
    spans = [
        *fixture_spans(),
        *lifecycle_tree(),
        *navigation_span("s", "c", 0, [navigation()]),
    ]
    out = render(analyze_spans(spans, None, durations, 25), 25)
    order = [
        COVERAGE,
        BOOT,
        "Top 25 slowest spans",
        "Top 25 slowest e2e.test spans",
        "Top 25 e2e action hotspots",
        "Top 25 navigation phase hotspots",
        "Top 25 slow navigation targets",
        "Top 25 long-task hotspots",
        "Long-task totals by project",
        "Top 25 resource initiator hotspots",
        "Top 25 slow resource assets",
        "E2E test duration by project",
        "Trace totals (sum of span durations)",
    ]
    positions = [out.index(f"{title}") for title in order]
    assert positions == sorted(positions)
    assert out.endswith("┘\n\n")


def test_a_skipped_coverage_section_renders_its_note_first() -> None:
    out = render(analysis_of(fixture_spans()), 25)
    assert out.startswith(
        "== Per-test span coverage ==\n(skipped: no --playwright-report supplied;"
    )


def test_tables_keep_formatted_cells_verbatim() -> None:
    table = report.table(("max_ms", "count"), [["1.000", "007"], ["-", "1e3"]])
    assert table.splitlines() == [
        "┌──────────┬─────────┐",
        "│ max_ms   │ count   │",
        "├──────────┼─────────┤",
        "│ 1.000    │ 007     │",
        "│ -        │ 1e3     │",
        "└──────────┴─────────┘",
    ]


# The commands.


def dev_output(result: Any) -> tuple[int, str, str]:
    return result.returncode, result.stdout.decode(), result.stderr.decode()


def test_analyze_shows_its_report_then_its_step(checkout: Checkout) -> None:
    code, out, _ = dev_output(
        checkout.dev(
            "trace-analysis",
            "analyze",
            "--top",
            "1",
            "--project",
            "firefox",
            str(SAMPLE),
        )
    )
    assert code == 0
    assert out.startswith("Project filter: firefox\n\n== Per-test span coverage ==")
    assert "Top 1 slowest spans" in out
    assert out.endswith("[ ok ] traces-analyze — 3 span(s)\n")


def test_analyze_pairs_its_playwright_reports(
    checkout: Checkout, tmp_path: Path
) -> None:
    trace = tmp_path / "trace.jsonl"
    trace.write_text("\n".join(line([span.raw]) for span in lifecycle_tree()))
    report_ = write(tmp_path / "r.json", playwright(("logs in", "chromium", 0, 500.0)))
    code, out, _ = dev_output(
        checkout.dev(
            "trace-analysis", "analyze", "--playwright-report", str(report_), str(trace)
        )
    )
    assert code == 0
    assert out.startswith(f"{COVERAGE}\n")


def test_a_malformed_attribute_fails_the_step_with_status_1(checkout: Checkout) -> None:
    code, out, _ = dev_output(checkout.dev("trace-analysis", "analyze", str(SAMPLE)))
    assert code == 1
    assert out.startswith("[FAIL] traces-analyze (")
    assert f"e2e.navigation_top_json from {SAMPLE}" in out


@pytest.mark.parametrize("subcommand", ["analyze", "boot-phases"])
def test_an_unreadable_trace_is_a_refusal_with_status_2(
    checkout: Checkout, subcommand: str
) -> None:
    code, out, _ = dev_output(
        checkout.dev("trace-analysis", subcommand, "/no/such.jsonl")
    )
    assert code == 2
    assert "reading trace file /no/such.jsonl" in out


def test_conflicting_playwright_reports_are_a_refusal_with_status_2(
    checkout: Checkout, tmp_path: Path
) -> None:
    one = write(tmp_path / "a.json", NESTED)
    two = write(tmp_path / "b.json", playwright(("logs in", "chromium", 0, 1.0)))
    code, out, _ = dev_output(
        checkout.dev(
            "trace-analysis",
            "analyze",
            "--playwright-report",
            str(one),
            "--playwright-report",
            str(two),
            str(SAMPLE),
        )
    )
    assert code == 2
    assert "playwright reports disagree on logs in [chromium] retry 0" in out


@pytest.mark.parametrize(
    "arguments",
    [
        ["analyze"],
        ["boot-phases"],
        ["analyze", "--top", "0", "x.jsonl"],
        ["run", "--top", "0"],
        ["run", "--browser", "webkit"],
    ],
)
def test_usage_errors_are_clicks(checkout: Checkout, arguments: list[str]) -> None:
    code, out, err = dev_output(checkout.dev("trace-analysis", *arguments))
    assert (code, out) == (2, "")
    assert "Usage: dev trace-analysis" in err


@pytest.mark.parametrize("subcommand", [[], ["analyze"], ["run"], ["boot-phases"]])
def test_help_is_shown(checkout: Checkout, subcommand: list[str]) -> None:
    code, out, _ = dev_output(checkout.dev("trace-analysis", *subcommand, "--help"))
    assert code == 0
    assert out.startswith(f"Usage: dev trace-analysis {' '.join(subcommand)}".rstrip())


# `run`: builds through fake Nix, then analyzes the outputs' captures.

# A build prints $RUN_OUTPUTS/<attribute> and exits $RUN_FAIL_CODE for the
# attribute $RUN_FAIL; evaluation and queries behave as a fake store's would.
RUN_NIX = """\
command = sys.argv[1]
if command == "path-info":
    if "--derivation" in sys.argv:
        print("/nix/store/fake.drv")
        sys.exit()
    sys.exit(1)
attribute = sys.argv[-1].rpartition(".")[2]
if attribute == os.environ.get("RUN_FAIL"):
    sys.exit(int(os.environ["RUN_FAIL_CODE"]))
print(pathlib.Path(os.environ["RUN_OUTPUTS"], attribute))
"""


@pytest.fixture
def run_checkout(checkout: Checkout) -> Checkout:
    checkout.tool("nix", RUN_NIX)
    catalog = checkout.root / "end2end" / "e2e-lanes.json"
    catalog.parent.mkdir()
    catalog.write_text((FIXTURES / "e2e-lanes.json").read_text())
    return checkout


def lane_output(checkout: Checkout, attribute: str, identity: str, trace: str) -> Path:
    output = checkout.root.parent / "outputs" / attribute
    output.mkdir(parents=True)
    write_lane_capture(output / f"capture-{identity}.tar.gz", trace)
    return output


def run(checkout: Checkout, *arguments: str, **extra: str) -> tuple[int, str, str]:
    outputs = str(checkout.root.parent / "outputs")
    environment = checkout.environment(RUN_OUTPUTS=outputs, **extra)
    return dev_output(
        checkout.run(checkout.command("trace-analysis", "run", *arguments), environment)
    )


def test_lanes_are_every_enabled_catalog_check_or_each_single_worker_package() -> None:
    catalog = FIXTURES / "e2e-lanes.json"
    selected = lanes(catalog, False, "chromium")
    assert [lane.installable for lane in selected] == [
        *(f".#checks.x86_64-linux.e2e-sqlite-chromium-{i}-of-3" for i in (1, 2, 3)),
        *(f".#checks.x86_64-linux.e2e-postgres-chromium-{i}-of-3" for i in (1, 2, 3)),
    ]
    assert len(lanes(catalog, False, None)) == 16
    single = lanes(catalog, True, None)
    assert [(lane.identity, lane.installable) for lane in single] == [
        (
            f"{backend}-{browser}-unsplit",
            f".#packages.x86_64-linux.e2e-{backend}-{browser}-single-worker",
        )
        for backend in ("sqlite", "postgres")
        for browser in ("chromium", "firefox")
    ]


def test_run_labels_each_lane_by_identity_and_pairs_its_report(
    run_checkout: Checkout,
) -> None:
    trace = "\n".join(line([span.raw]) for span in lifecycle_tree())
    for backend in ("sqlite", "postgres"):
        identity = f"{backend}-firefox-unsplit"
        output = lane_output(
            run_checkout, f"e2e-{backend}-firefox-single-worker", identity, trace
        )
        if backend == "sqlite":
            write(
                output / f"playwright-report-{identity}.json",
                playwright(("logs in", "chromium", 0, 800.0)),
            )
    code, out, _ = run(run_checkout, "--single-worker", "--browser", "firefox")
    assert code == 0, out
    assert out.startswith(f"{COVERAGE}\n")
    # Only SQLite's report pairs: its trace alone has a coverage row.
    coverage = out.split("\n\n")[0]
    assert coverage.count("logs in") == 1
    assert "800.000" in coverage
    assert "│ sqlite-firefox-unsplit " in out
    assert "│ postgres-firefox-unsplit " in out
    assert out.endswith("[ ok ] traces-run — 2 trace file(s)\n")
    spans = run_checkout.merged()
    assert spans["e2e-sqlite-firefox-single-worker"]["args"]["jaunder.status"] == "ok"


def test_run_applies_the_trace_filter(run_checkout: Checkout) -> None:
    trace = line([{"name": "GET", "traceId": "aa"}, {"name": "POST", "traceId": "bb"}])
    for lane in lanes(FIXTURES / "e2e-lanes.json", False, "chromium"):
        lane_output(run_checkout, lane.attribute, lane.identity, trace)
    code, out, _ = run(
        run_checkout, "--browser", "chromium", "--trace", "bb", "--top", "2"
    )
    assert code == 0, out
    assert "│ GET " not in out
    assert "Top 2 slowest spans" in out
    assert out.endswith("[ ok ] traces-run — 6 trace file(s)\n")


def test_a_failed_build_stops_run_with_nix_status(run_checkout: Checkout) -> None:
    code, out, _ = run(
        run_checkout,
        "--single-worker",
        RUN_FAIL="e2e-sqlite-chromium-single-worker",
        RUN_FAIL_CODE="7",
    )
    assert code == 7
    assert "traces-run" not in out


def test_a_missing_capture_is_a_refusal_with_status_2(run_checkout: Checkout) -> None:
    for attribute in [
        "e2e-sqlite-chromium-single-worker",
        "e2e-postgres-chromium-single-worker",
    ]:
        (run_checkout.root.parent / "outputs" / attribute).mkdir(parents=True)
    code, out, _ = run(run_checkout, "--single-worker", "--browser", "chromium")
    assert code == 2
    capture = run_checkout.root.parent / "outputs" / "e2e-sqlite-chromium-single-worker"
    assert (
        f"capture tarball not found: {capture}/capture-sqlite-chromium-unsplit.tar.gz"
        in out
    )
