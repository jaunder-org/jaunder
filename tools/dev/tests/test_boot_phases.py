"""`dev trace-analysis boot-phases`: per-population medians of the
document-frame boot decomposition, and their rendering."""

import json
from pathlib import Path
from typing import Any

import pytest
from conftest import Checkout, cells

from dev.boot_phases import (
    BRIDGE_SCHEMA,
    Population,
    boot_phase_rows,
    boot_phases,
    median,
    render,
)
from dev.traces import (
    CLOSURE_TOLERANCE_MS,
    Filters,
    MalformedJsonAttr,
    Span,
    parse_spans,
)

# The canonical timeline: initialization starts at 10 and reaches `boot.entry`
# at 35; the Rust phases then reach `mount_done`.
INIT_START = 10.0
ENTRY = 35.0
SEED_PARSED = 50.0
RENDER_START = 75.0


def shape(imports: int, exports: int) -> dict[str, int]:
    return {
        "imports": imports,
        "importedFunctions": imports,
        "importedTables": 0,
        "importedMemories": 0,
        "exports": exports,
        "exportedFunctions": exports,
        "exportedTables": 0,
        "exportedMemories": 1,
        "customSections": 0,
    }


def mark(name: str, start: float) -> dict[str, Any]:
    return {"name": name, "startTime": start}


def boot_marks(mount_done_ms: float) -> list[dict[str, Any]]:
    return [
        mark("jaunder.boot.entry", ENTRY),
        mark("jaunder.boot.seed_parsed", SEED_PARSED),
        mark("jaunder.boot.render_start", RENDER_START),
        mark("jaunder.boot.mount_done", mount_done_ms),
    ]


def nav(
    id: int, warmth: str, mount_done_ms: float, commit_to_mount_ms: float | None
) -> dict[str, Any]:
    return {
        "id": id,
        "cacheWarmth": warmth,
        "wasmTimingSchema": "direct-init-v1",
        "commitToMountMs": commit_to_mount_ms,
        "wasmInitStartMs": INIT_START,
        "wasmInitStartToBootEntryMs": ENTRY - INIT_START,
        "bootPhases": {
            "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
            "jaunder.boot.seed_parsed->jaunder.boot.render_start": RENDER_START
            - SEED_PARSED,
            "jaunder.boot.render_start->jaunder.boot.mount_done": mount_done_ms
            - RENDER_START,
        },
    }


def add_bridge(
    navigation: dict[str, Any], start: float, binding: float, remainder: float
) -> None:
    navigation.update(
        frameSkewSchema=BRIDGE_SCHEMA,
        commitToDocumentStartMs=start,
        mountDoneToBindingMs=binding,
        frameSkewRemainderMs=remainder,
    )


def add_style(
    navigation: dict[str, Any],
    module_before_init: float,
    css: float,
    themes: float,
    fetch_start: float,
) -> None:
    latest = max(css, themes)
    navigation.update(
        moduleBeforeInitMs=module_before_init,
        jaunderCssResponseEndMs=css,
        jaunderThemesCssResponseEndMs=themes,
        styleMaxResponseEndMs=latest,
        styleToModuleBeforeInitMs=module_before_init - latest,
        moduleBeforeInitToWasmFetchStartMs=fetch_start - module_before_init,
        wasmFetchStartMs=fetch_start,
    )


def dark_nav(id: int, warmth: str) -> dict[str, Any]:
    """An unversioned navigation with no boot marks: raw legacy input."""
    return {
        "id": id,
        "cacheWarmth": warmth,
        "commitToMountMs": 300.0,
        "bootPhases": None,
    }


def raw_span(
    project: str, navs: list[Any], marks: list[tuple[int, list[Any]]]
) -> dict[str, Any]:
    return {
        "name": "e2e.test",
        "attributes": [
            {"key": "e2e.project", "value": {"stringValue": project}},
            {
                "key": "e2e.navigation_top_json",
                "value": {"stringValue": json.dumps(navs)},
            },
            {
                "key": "e2e.boot_marks_json",
                "value": {
                    "stringValue": json.dumps(
                        [{"id": id, "marks": found} for id, found in marks]
                    )
                },
            },
        ],
    }


def line(raw: dict[str, Any]) -> str:
    return json.dumps({"resourceSpans": [{"scopeSpans": [{"spans": [raw]}]}]})


def nav_span(
    source: str, project: str, navs: list[Any], marks: list[tuple[int, list[Any]]]
) -> list[Span]:
    return parse_spans(line(raw_span(project, navs, marks)), Filters(), source)


def one_nav_span(source: str, project: str, mount_done_ms: float) -> list[Span]:
    return nav_span(
        source,
        project,
        [nav(1, "warm", mount_done_ms, mount_done_ms + 50.0)],
        [(1, boot_marks(mount_done_ms))],
    )


def rows_of(spans: list[Span]) -> list[Population]:
    return boot_phase_rows(spans)


def test_segments_sum_to_the_boot_total_within_a_millisecond() -> None:
    (row,) = rows_of(one_nav_span("sqlite", "chromium", 105.0))
    assert (row.navigations, len(row.decomposed), row.closure_violations) == (1, 1, 0)
    # Two exclusive segments and three intervals.
    assert len(row.segments) == 5
    assert row.boot_total_ms == 105.0
    total = sum(segment.median_ms for segment in row.segments)
    assert abs(total - 105.0) <= CLOSURE_TOLERANCE_MS


def test_commit_to_mount_and_frame_skew_are_reported_never_decomposed() -> None:
    (row,) = rows_of(one_nav_span("sqlite", "chromium", 105.0))
    assert (row.commit_to_mount_ms, row.frame_skew_ms) == (155.0, 50.0)
    assert not any(
        "commit" in segment.label or "skew" in segment.label for segment in row.segments
    )


def test_complete_bridge_coverage_within_bounds_is_decisive() -> None:
    first = nav(1, "warm", 105.0, 155.0)
    add_bridge(first, 15.0, 35.0, 0.5)
    second = nav(2, "warm", 95.0, 145.0)
    add_bridge(second, 18.0, 31.0, -0.5)
    (row,) = rows_of(
        nav_span(
            "sqlite",
            "chromium",
            [first, second],
            [(1, boot_marks(105.0)), (2, boot_marks(95.0))],
        )
    )
    assert (row.mounted, len(row.bridge), row.bridge_decisive) == (2, 2, True)
    assert row.bridge_estimate(0).mean == 16.5
    assert row.bridge_estimate(1).mean == 33.0
    assert row.bridge_estimate(2).mean == 0.0
    assert row.remainder_max_abs_ms == 0.5
    assert "bridge certification: decisive" in render([row])


def test_incomplete_bridge_coverage_is_non_decisive() -> None:
    complete = nav(1, "warm", 105.0, 155.0)
    add_bridge(complete, 15.0, 35.0, 0.0)
    rows = rows_of(
        nav_span(
            "sqlite",
            "chromium",
            [complete, nav(2, "warm", 95.0, 145.0)],
            [(1, boot_marks(105.0)), (2, boot_marks(95.0))],
        )
    )
    assert (rows[0].mounted, len(rows[0].bridge), rows[0].bridge_decisive) == (
        2,
        1,
        False,
    )
    assert "bridge certification: non-decisive (coverage incomplete)" in render(rows)


@pytest.mark.parametrize(
    ("remainders", "reasons"),
    [
        ([2.5], "mean remainder out of bounds, max remainder out of bounds"),
        ([2.5, -2.5], "max remainder out of bounds"),
        ([2.0, 0.0, 2.0], "mean remainder out of bounds"),
    ],
)
def test_an_out_of_bounds_remainder_is_non_decisive(
    remainders: list[float], reasons: str
) -> None:
    navs = []
    for index, remainder in enumerate(remainders, 1):
        navigation = nav(index, "warm", 105.0, 155.0)
        add_bridge(navigation, 15.0, 35.0, remainder)
        navs.append(navigation)
    rows = rows_of(nav_span("sqlite", "chromium", navs, [(1, boot_marks(105.0))]))
    assert not rows[0].bridge_decisive
    assert f"bridge certification: non-decisive ({reasons})" in render(rows)


def test_a_single_remainder_renders_its_mean_without_an_error() -> None:
    navigation = nav(1, "warm", 105.0, 155.0)
    add_bridge(navigation, 15.0, 35.0, 2.5)
    out = render(rows_of(nav_span("s", "c", [navigation], [(1, boot_marks(105.0))])))
    assert (
        "mean frameSkewRemainderMs ± SE (n=1): 2.500 ± -  max |remainder|: 2.500" in out
    )


def test_historical_navigations_without_bridge_fields_are_non_decisive() -> None:
    other = nav(2, "warm", 105.0, None)
    other["frameSkewSchema"] = "bridge-v0"
    unmounted = nav(3, "cold", 105.0, None)
    add_bridge(unmounted, 1.0, 1.0, 0.0)
    unmounted.pop("mountDoneToBindingMs")
    rows = rows_of(
        nav_span(
            "s",
            "c",
            [nav(1, "warm", 105.0, 155.0), other, unmounted],
            [(1, boot_marks(105.0))],
        )
    )
    cold, warm = rows
    assert (warm.mounted, len(warm.bridge), warm.bridge_decisive) == (1, 0, False)
    assert warm.bridge_estimate(0).mean is None
    out = render(rows)
    assert (
        "bridge certification: non-decisive (no mounted navigations, mean remainder"
        " out of bounds, max remainder out of bounds)"
    ) in out
    assert "mean commitToDocumentStartMs ± SE (n=0): - ± -" in out
    assert cold.cache_warmth == "cold"


def test_stylesheet_diagnostics_report_coverage_and_ordering_rate() -> None:
    first = nav(1, "warm", 105.0, 155.0)
    add_style(first, 120.0, 80.0, 90.0, 150.0)
    second = nav(2, "warm", 95.0, 145.0)
    add_style(second, 95.0, 100.0, 90.0, 140.0)
    rows = rows_of(
        nav_span(
            "s", "c", [first, second], [(1, boot_marks(105.0)), (2, boot_marks(95.0))]
        )
    )
    row = rows[0]
    assert (len(row.style), row.style_decisive, row.style_ordering_pass) == (2, True, 1)
    assert row.style_estimate(1).mean == 12.5
    assert row.style_estimate(2).mean == 37.5
    assert row.style_estimate(0).mean == 95.0
    assert row.style_estimate(3).mean == 145.0
    out = render(rows)
    assert "stylesheet certification: decisive" in out
    assert (
        "stylesheet ordering pass (styleToModuleBeforeInitMs >= 0): 1/2 (50.0%)" in out
    )
    assert "mean styleToModuleBeforeInitMs ± SE (n=2): 12.500 ± 17.500" in out


def test_historical_stylesheet_rows_are_non_decisive() -> None:
    complete = nav(1, "warm", 105.0, 155.0)
    add_style(complete, 120.0, 80.0, 90.0, 150.0)
    partial = nav(2, "warm", 95.0, 145.0)
    add_style(partial, 95.0, 100.0, 90.0, 140.0)
    partial.pop("jaunderThemesCssResponseEndMs")
    rows = rows_of(
        nav_span(
            "s",
            "c",
            [complete, partial, nav(3, "warm", 95.0, 145.0)],
            [(1, boot_marks(105.0))],
        )
    )
    assert (len(rows[0].style), rows[0].style_decisive) == (1, False)
    assert rows[0].style_estimate(1).mean == 30.0
    assert "stylesheet certification: non-decisive (coverage incomplete)" in render(
        rows
    )


def test_an_ordering_failure_is_reported_decisively() -> None:
    navigation = nav(1, "warm", 105.0, 155.0)
    add_style(navigation, 95.0, 100.0, 90.0, 140.0)
    rows = rows_of(nav_span("s", "c", [navigation], [(1, boot_marks(105.0))]))
    assert (rows[0].style_decisive, rows[0].style_ordering_pass) == (True, 0)
    assert rows[0].style_estimate(1).mean == -5.0
    assert "(styleToModuleBeforeInitMs >= 0): 0/1 (0.0%)" in render(rows)


def test_stylesheet_diagnostics_render_before_the_missing_decomposition() -> None:
    navigation = dark_nav(1, "warm")
    add_style(navigation, 120.0, 80.0, 90.0, 150.0)
    rows = rows_of(nav_span("s", "c", [navigation], [(1, [])]))
    assert not rows[0].decomposed
    out = render(rows)
    assert out.index("stylesheet certification: decisive") < out.index(
        "no decomposed navigations"
    )


def test_a_navigation_failing_closure_is_counted_not_included() -> None:
    (row,) = rows_of(
        nav_span("s", "c", [nav(1, "warm", 105.0, 160.0)], [(1, boot_marks(200.0))])
    )
    assert (row.navigations, row.closure_violations, len(row.decomposed)) == (1, 1, 0)
    assert "closure violations: 1" in render([row])


def test_cold_and_warm_are_never_pooled_and_cold_sorts_first() -> None:
    rows = rows_of(
        nav_span(
            "s",
            "c",
            [
                nav(1, "warm", 95.0, 145.0),
                nav(2, "cold", 105.0, 155.0),
                {**nav(3, "", 105.0, 155.0), "cacheWarmth": 4},
                nav(4, "lukewarm", 105.0, 155.0),
            ],
            [(1, boot_marks(95.0)), (2, boot_marks(105.0))],
        )
    )
    assert [(row.cache_warmth, row.boot_total_ms) for row in rows] == [
        ("cold", 105.0),
        ("warm", 95.0),
        ("-", None),
        ("lukewarm", None),
    ]


def test_backends_never_pool() -> None:
    rows = rows_of(
        [
            *one_nav_span("sqlite", "firefox", 105.0),
            *one_nav_span("postgres", "firefox", 205.0),
        ]
    )
    assert [(row.source, row.project) for row in rows] == [
        ("postgres", "firefox"),
        ("sqlite", "firefox"),
    ]


def test_a_fourth_mark_extends_the_table_in_timeline_order() -> None:
    marks = [
        mark("jaunder.boot.entry", ENTRY),
        mark("jaunder.boot.seed_parsed", SEED_PARSED),
        mark("jaunder.boot.render_start", RENDER_START),
        mark("jaunder.boot.hydrated", 90.0),
        mark("jaunder.boot.mount_done", 105.0),
    ]
    navigation = nav(1, "warm", 105.0, 155.0)
    navigation["bootPhases"] = {
        "jaunder.boot.hydrated->jaunder.boot.mount_done": 105.0 - 90.0,
        "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
        "jaunder.boot.seed_parsed->jaunder.boot.render_start": RENDER_START
        - SEED_PARSED,
        "jaunder.boot.render_start->jaunder.boot.hydrated": 90.0 - RENDER_START,
    }
    (row,) = rows_of(nav_span("s", "c", [navigation], [(1, marks)]))
    assert (len(row.decomposed), row.closure_violations) == (1, 0)
    labels = [segment.label for segment in row.segments]
    assert len(labels) == 6
    assert labels[2] == "jaunder.boot.entry->jaunder.boot.seed_parsed"
    assert labels[5] == "jaunder.boot.hydrated->jaunder.boot.mount_done"


def test_medians_take_the_lower_middle_value_on_even_counts() -> None:
    assert median([]) is None
    assert median([3.0, 1.0, 2.0, 4.0]) == 2.0
    totals = [100.0, 200.0, 300.0, 400.0]
    (row,) = rows_of(
        nav_span(
            "s",
            "c",
            [
                nav(index, "warm", total, total + 10.0)
                for index, total in enumerate(totals, 1)
            ],
            [(index, boot_marks(total)) for index, total in enumerate(totals, 1)],
        )
    )
    assert (len(row.decomposed), row.boot_total_ms) == (4, 200.0)
    assert (row.segments[-1].median_ms, row.segments[-1].count) == (
        200.0 - RENDER_START,
        4,
    )


def test_a_population_with_no_decomposed_navigations_says_so() -> None:
    rows = rows_of(
        nav_span(
            "s",
            "firefox",
            [dark_nav(1, "cold"), dark_nav(2, "warm")],
            [(1, []), (2, [])],
        )
    )
    assert len(rows) == 2
    assert all(not row.decomposed and row.boot_total_ms is None for row in rows)
    assert all(not row.segments for row in rows)
    assert (
        "no decomposed navigations (0 of 1) — nothing to decompose for this population\n\n"
        in render(rows)
    )


def test_render_names_every_population_and_its_table() -> None:
    navigation = nav(1, "warm", 105.0, 155.0)
    add_bridge(navigation, 15.0, 35.0, 0.0)
    spans = [
        *nav_span("sqlite", "chromium", [navigation], [(1, boot_marks(105.0))]),
        *one_nav_span("postgres", "firefox", 205.0),
    ]
    out = render(rows_of(spans))
    assert out.startswith(
        "Boot-phase decomposition (medians; even counts take the lower middle value).\n"
    )
    assert "== sqlite / chromium / warm / arm:- ==\n" in out
    assert "== postgres / firefox / warm / arm:- ==\n" in out
    assert "median bootTotalMs: 105.000\nmedian commitToMountMs: 155.000\n" in out
    assert "median frame skew (commitToMountMs - bootTotalMs): 50.000\n" in out
    assert ("wasm_init_start", "10.000", "9.5", "1") in cells(out)
    assert out.endswith("┘\n\n")


def test_a_zero_boot_total_has_no_share() -> None:
    navigation = nav(1, "warm", 0.0, 1.0)
    navigation.update(wasmInitStartMs=0.0, wasmInitStartToBootEntryMs=0.0)
    navigation["bootPhases"] = {f"a.boot.{i}": 0.0 for i in range(3)}
    rows = rows_of(
        nav_span("s", "c", [navigation], [(1, [mark("x.boot.mount_done", 0.0)])])
    )
    assert ("wasm_init_start", "0.000", "-", "1") in cells(render(rows))


def test_render_of_nothing_says_so() -> None:
    assert render([]) == "No navigations found in the provided trace files.\n"


def test_direct_diagnostics_count_only_valid_current_completions() -> None:
    streaming = nav(1, "warm", 105.0, 155.0)
    streaming.update(
        wasmApiMs=10.0,
        wasmInitMs=20.0,
        wasmInitPath="streaming",
        wasmExperimentArm="shape",
        wasmModuleShape=shape(2, 3),
    )
    malformed = nav(2, "warm", 105.0, 155.0)
    malformed.update(
        wasmApiMs=30.0,
        wasmInitMs=20.0,
        wasmInitPath="buffered",
        wasmExperimentArm="shape",
    )
    buffered = nav(4, "warm", 105.0, 155.0)
    buffered.update(
        wasmApiMs=1.0,
        wasmInitMs=2.0,
        wasmInitPath="buffered",
        wasmExperimentArm="shape",
        wasmModuleShape={**shape(2, 3), "imports": -1},
    )
    legacy = {**dark_nav(3, "warm"), "wasmExperimentArm": "shape"}
    (row,) = rows_of(
        nav_span(
            "s",
            "c",
            [streaming, malformed, legacy, buffered],
            [(1, boot_marks(105.0)), (2, boot_marks(105.0)), (3, [])],
        )
    )
    assert (row.navigations, row.current, row.direct_complete, row.direct_missing) == (
        4,
        3,
        2,
        1,
    )
    assert (row.streaming, row.buffered, row.legacy) == (1, 1, 1)
    assert median(row.wasm_api_ms) == 1.0
    assert row.experiment_arm == "shape"
    assert (row.shape_complete, row.shape_missing, len(row.shapes)) == (1, 1, 1)
    out = render([row])
    assert "median wasmApiMs (n=2): 1.000  median wasmInitMs (n=2): 2.000" in out
    assert (
        "mean wasmApiMs ± SE: 5.500 ± 4.500  mean wasmInitMs ± SE: 11.000 ± 9.000"
        in out
    )


def test_experiment_arms_group_and_report_shape_integrity() -> None:
    baseline = nav(1, "warm", 105.0, 155.0)
    baseline.update(
        wasmApiMs=10.0,
        wasmInitMs=20.0,
        wasmInitPath="streaming",
        wasmExperimentArm="baseline",
        wasmModuleShape=shape(2, 3),
    )
    changed = nav(2, "warm", 110.0, 160.0)
    changed.update(
        wasmApiMs=15.0,
        wasmInitMs=25.0,
        wasmInitPath="streaming",
        wasmExperimentArm="shape",
        wasmModuleShape=shape(2, 5),
    )
    rows = rows_of(
        nav_span(
            "s",
            "firefox",
            [changed, baseline],
            [(1, boot_marks(105.0)), (2, boot_marks(110.0))],
        )
    )
    assert [row.experiment_arm for row in rows] == ["baseline", "shape"]
    assert [(r.shape_complete, r.shape_missing, len(r.shapes)) for r in rows] == [
        (1, 0, 1),
        (1, 0, 1),
    ]
    out = render(rows)
    assert "arm:baseline" in out
    assert "arm:shape" in out
    assert "shape complete: 1" in out


def test_spans_without_navigations_and_unusable_marks_are_skipped() -> None:
    spans = [
        *parse_spans(line({"name": "e2e.page"}), Filters(), "s"),
        *parse_spans(line({"name": "e2e.test"}), Filters(), "s"),
        *nav_span("s", "", [{**nav(1, "warm", 105.0, None), "id": 1.0}, "junk"], []),
    ]
    raw = raw_span("c", [nav(1, "warm", 105.0, None)], [])
    raw["attributes"][2]["value"]["stringValue"] = json.dumps(
        [
            {"id": 1, "marks": "none"},
            {"marks": []},
            {"id": 1, "marks": boot_marks(105.0)},
        ]
    )
    spans.extend(parse_spans(line(raw), Filters(), "t"))
    first, junk, second = rows_of(spans)
    assert (first.project, len(first.decomposed)) == ("-", 0)
    # A navigation that is not an object still counts, as legacy.
    assert (junk.cache_warmth, junk.navigations, junk.legacy) == ("-", 1, 1)
    assert len(second.decomposed) == 1


def test_boot_phases_reads_files(tmp_path: Path) -> None:
    path = tmp_path / "otel-traces.jsonl"
    path.write_text(line(one_nav_span("inline", "chromium", 105.0)[0].raw) + "\n")
    (row,) = boot_phases([path])
    assert (row.source, len(row.decomposed)) == (str(path), 1)


@pytest.mark.parametrize("key", ["e2e.navigation_top_json", "e2e.boot_marks_json"])
def test_a_malformed_attribute_is_never_hidden(key: str) -> None:
    raw = raw_span("c", [nav(1, "warm", 105.0, None)], [])
    for attribute in raw["attributes"]:
        if attribute["key"] == key:
            attribute["value"]["stringValue"] = "{not json"
    if key == "e2e.boot_marks_json":
        raw["attributes"] = [
            a for a in raw["attributes"] if a["key"] != "e2e.navigation_top_json"
        ]
    with pytest.raises(MalformedJsonAttr) as malformed:
        boot_phase_rows(parse_spans(line(raw), Filters(), "inline"))
    assert f"{key} from inline" in str(malformed.value)


def test_boot_phases_command_shows_its_report_and_population_count(
    checkout: Checkout, tmp_path: Path
) -> None:
    path = tmp_path / "otel-traces.jsonl"
    path.write_text(line(one_nav_span("inline", "chromium", 105.0)[0].raw) + "\n")
    result = checkout.dev("trace-analysis", "boot-phases", str(path))
    out = result.stdout.decode()
    assert result.returncode == 0
    assert out.startswith("Boot-phase decomposition")
    assert out.endswith("[ ok ] traces-boot-phases — 1 population(s)\n")
    path.write_text(line(raw_span("c", [], [])).replace("[]", "{bad", 1) + "\n")
    result = checkout.dev("trace-analysis", "boot-phases", str(path))
    assert result.returncode == 1
    assert result.stdout.decode().startswith("[FAIL] traces-boot-phases (")
