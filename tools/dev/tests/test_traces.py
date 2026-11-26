"""The trace core: OTLP span parsing, attribute reads, the capture reader and
the boot decomposition outcome."""

import json
from pathlib import Path
from typing import Any

import pytest
from conftest import write_capture

from dev.evidence import EvidenceError
from dev.traces import (
    TRACE_MEMBER,
    BootOutcome,
    Decomposition,
    Filters,
    MalformedJsonAttr,
    decompose,
    field_f64,
    get_attr,
    instance_diag_member,
    instance_directory,
    instance_index,
    interval_union_ms,
    page_boot_decomposition_outcome,
    parse_duration_ms,
    parse_json_attr,
    parse_spans,
    read_capture_member,
    read_instance_diag_logs,
    read_spans,
    read_trace_member,
    span_interval_ms,
    to_url_path,
)


def line(spans: Any) -> str:
    return json.dumps({"resourceSpans": [{"scopeSpans": [{"spans": spans}]}]})


def attr(key: str, value: str) -> dict[str, Any]:
    return {"key": key, "value": {"stringValue": value}}


# Span parsing.


@pytest.mark.parametrize("malformed", [None, {}, {"name": ""}, {"name": 3}])
def test_a_malformed_span_beside_valid_e2e_evidence_is_rejected(
    malformed: Any,
) -> None:
    content = line(
        [malformed, {"name": "e2e.test", "attributes": [attr("e2e.project", "c")]}]
    )
    with pytest.raises(EvidenceError, match="span entry"):
        parse_spans(content, Filters(), "capture")


@pytest.mark.parametrize(
    "record",
    [{}, [], {"resourceSpans": [{}]}, {"resourceSpans": [{"scopeSpans": [{}]}]}],
)
def test_a_record_without_its_nested_arrays_is_rejected(record: Any) -> None:
    with pytest.raises(EvidenceError, match="JSON line 1 in t has no .* array"):
        parse_spans(json.dumps(record), Filters(), "t")


def test_spans_are_walked_from_every_resource_and_scope() -> None:
    spans = parse_spans(
        line([{"traceId": "aa", "name": "a"}, {"traceId": "aa", "name": "b"}]),
        Filters(),
        "sample",
    )
    assert [span.name for span in spans] == ["a", "b"]
    assert all(span.source == "sample" for span in spans)


def test_a_span_keeps_its_identity_edges_and_unix_nanoseconds() -> None:
    (span,) = parse_spans(
        line(
            [
                {
                    "traceId": "aa",
                    "spanId": "bb",
                    "parentSpanId": "cc",
                    "name": "request",
                    "startTimeUnixNano": "1000000",
                    "endTimeUnixNano": 3000000,
                    "attributes": [attr("method", "GET"), attr("uri", "/x")],
                }
            ]
        ),
        Filters(),
        "t",
    )
    assert (span.trace_id, span.span_id, span.parent_span_id) == ("aa", "bb", "cc")
    assert (span.start_unix_nano, span.end_unix_nano) == (1_000_000, 3_000_000)
    assert (span.method, span.uri, span.duration_ms) == ("GET", "/x", 2.0)


def test_a_root_span_has_empty_parent_and_no_timestamps() -> None:
    (span,) = parse_spans(line([{"spanId": "bb", "name": "request"}]), Filters(), "t")
    assert (span.parent_span_id, span.trace_id) == ("", "")
    assert (span.start_unix_nano, span.end_unix_nano) == (None, None)


def test_a_malformed_line_is_an_error_naming_its_source() -> None:
    with pytest.raises(EvidenceError, match="failed to parse JSON line 2 in t"):
        parse_spans("\n{bad json\n", Filters(), "t")


def test_blank_content_has_no_spans() -> None:
    assert parse_spans("", Filters(), "t") == []
    assert parse_spans("\n  \r\n", Filters(), "t") == []


def test_the_trace_filter_keeps_only_its_trace() -> None:
    content = line([{"traceId": "aa", "name": "a"}, {"traceId": "bb", "name": "b"}])
    spans = parse_spans(content, Filters(trace="aa"), "t")
    assert [span.trace_id for span in spans] == ["aa"]


def test_the_project_filter_drops_only_other_projects_e2e_spans() -> None:
    def e2e(project: str) -> dict[str, Any]:
        return {"name": "e2e.test", "attributes": [attr("e2e.project", project)]}

    content = line([e2e("firefox"), e2e("chromium"), {"name": "GET"}])
    spans = parse_spans(content, Filters(project="firefox"), "t")
    assert [(span.name, span.project) for span in spans] == [
        ("e2e.test", "firefox"),
        ("GET", ""),
    ]


def test_reading_spans_names_a_missing_file(tmp_path: Path) -> None:
    with pytest.raises(EvidenceError, match="no-such-trace.jsonl"):
        read_spans(tmp_path / "no-such-trace.jsonl", Filters())
    trace = tmp_path / "trace.jsonl"
    trace.write_text(line([{"name": "a"}]))
    assert [span.source for span in read_spans(trace, Filters())] == [str(trace)]


# Attributes.


def test_an_attribute_is_its_string_then_its_int_then_empty() -> None:
    span = {
        "attributes": [
            "not an attribute",
            attr("method", "GET"),
            {"key": "n", "value": {"intValue": 42}},
            {"key": "s", "value": {"intValue": "99"}},
            {"key": "f", "value": {"intValue": 1.5}},
            {"key": "v"},
        ]
    }
    assert [get_attr(span, key) for key in ["method", "n", "s", "f", "v", "x"]] == [
        "GET",
        "42",
        "99",
        "",
        "",
        "",
    ]
    assert get_attr({"attributes": "none"}, "method") == ""


def test_a_json_attribute_is_absent_malformed_or_parsed() -> None:
    def span(text: str) -> dict[str, Any]:
        return {"attributes": [attr("e2e.x", text)]}

    assert parse_json_attr({}, "e2e.x", "source") is None
    with pytest.raises(MalformedJsonAttr) as malformed:
        parse_json_attr(span("{not json"), "e2e.x", "source.jsonl")
    assert malformed.value.span_source == "source.jsonl"
    assert malformed.value.key == "e2e.x"
    assert str(malformed.value).startswith(
        "parsing JSON trace attribute e2e.x from source.jsonl: "
    )
    assert parse_json_attr(span("[1,2]"), "e2e.x", "source") == [1, 2]


def test_a_finite_number_field() -> None:
    value = {"a": 1, "b": 1.5, "c": True, "d": "1", "e": float("inf")}
    assert [field_f64(value, key) for key in "abcdef"] == [
        1.0,
        1.5,
        None,
        None,
        None,
        None,
    ]
    assert field_f64([], "a") is None


# Timing.


def test_duration_is_milliseconds_between_the_unix_nanos() -> None:
    span = {"startTimeUnixNano": "1000000", "endTimeUnixNano": "2500000"}
    assert parse_duration_ms(span) == 1.5


@pytest.mark.parametrize(
    "start",
    [None, "x", " 1", True, 1.0, 2**63, "1" + "0" * 40],
)
def test_an_unreadable_start_counts_as_zero(start: Any) -> None:
    assert parse_duration_ms({"startTimeUnixNano": start, "endTimeUnixNano": "1"}) == (
        1e-6
    )


def test_an_extreme_duration_saturates_instead_of_overflowing() -> None:
    span = {
        "endTimeUnixNano": str(2**127 - 1),
        "startTimeUnixNano": str(-(2**127)),
    }
    assert parse_duration_ms(span) == (2**127 - 1) / 1_000_000


def test_an_interval_reads_string_and_numeric_nanos() -> None:
    expected = (1.0, 3.0)
    assert (
        span_interval_ms({"startTimeUnixNano": "1e6", "endTimeUnixNano": "3000000"})
        == expected
    )
    assert (
        span_interval_ms({"startTimeUnixNano": 1e6, "endTimeUnixNano": 3_000_000})
        == expected
    )


@pytest.mark.parametrize(
    "raw",
    [
        {},
        {"startTimeUnixNano": "1"},
        {"startTimeUnixNano": "x", "endTimeUnixNano": "1"},
        {"startTimeUnixNano": True, "endTimeUnixNano": "1"},
        {"startTimeUnixNano": "nan", "endTimeUnixNano": "1"},
        {"startTimeUnixNano": "3000000", "endTimeUnixNano": "1000000"},
    ],
)
def test_a_missing_unreadable_or_inverted_interval_is_excluded(raw: Any) -> None:
    assert span_interval_ms(raw) is None


@pytest.mark.parametrize(
    ("intervals", "union"),
    [
        ([(0.0, 100.0), (50.0, 150.0)], 150.0),
        ([(200.0, 250.0), (0.0, 100.0)], 150.0),
        ([(0.0, 100.0), (25.0, 50.0)], 100.0),
        ([], 0.0),
    ],
    ids=["overlap", "disjoint", "contained", "empty"],
)
def test_an_interval_union_counts_overlap_once(
    intervals: list[tuple[float, float]], union: float
) -> None:
    assert interval_union_ms(intervals) == union


@pytest.mark.parametrize(
    ("url", "path"),
    [
        ("https://h:8080/a/b?q=1", "h:8080/a/b"),
        ("https://h:443", "h/"),
        ("mailto:someone", "someone"),
        ("https://h:port/", "https://h:port/"),
        ("not a url", "not a url"),
        ("", ""),
    ],
)
def test_a_url_becomes_host_port_and_path(url: str, path: str) -> None:
    assert to_url_path(url) == path


# The capture reader.


def test_the_trace_member_reader_rejects_incomplete_or_ambiguous_evidence(
    tmp_path: Path,
) -> None:
    capture = tmp_path / "capture.tar.gz"
    for members, directories, message in [
        ([], (), "missing"),
        ([(TRACE_MEMBER, b"  \n")], (), "empty"),
        ([], (TRACE_MEMBER,), "regular"),
        ([(TRACE_MEMBER, b"\xff")], (), "reading capture member"),
        ([(TRACE_MEMBER, b"one"), (TRACE_MEMBER, b"two")], (), "ambiguous"),
    ]:
        write_capture(capture, members, directories)
        with pytest.raises(EvidenceError, match=message):
            read_trace_member(capture)
    write_capture(capture, [(TRACE_MEMBER, b"trace")])
    assert read_trace_member(capture) == "trace"


def test_an_unopenable_or_malformed_capture_is_an_error(tmp_path: Path) -> None:
    with pytest.raises(EvidenceError, match="opening capture"):
        read_trace_member(tmp_path / "missing.tar.gz")
    with pytest.raises(EvidenceError, match="opening capture"):
        read_trace_member(tmp_path)
    garbage = tmp_path / "garbage.tar.gz"
    garbage.write_bytes(b"not a gzip archive")
    with pytest.raises(EvidenceError, match="malformed capture archive"):
        read_trace_member(garbage)


def test_a_capture_member_may_be_empty(tmp_path: Path) -> None:
    capture = tmp_path / "capture.tar.gz"
    write_capture(capture, [(instance_diag_member(0), b"")])
    assert read_capture_member(capture, instance_diag_member(0)) == ""


def test_instance_diag_logs_are_read_in_instance_order(tmp_path: Path) -> None:
    capture = tmp_path / "capture.tar.gz"
    write_capture(
        capture,
        [
            (instance_diag_member(1), b"two"),
            (TRACE_MEMBER, b"trace"),
            ("capture/diag.log", b"unscoped"),
            ("other/instance-0/diag.log", b"foreign root"),
            ("capture/instance-0/sub/diag.log", b"nested"),
            ("capture/instance-x/diag.log", b"not an instance"),
            (instance_diag_member(0), b""),
        ],
    )
    assert read_instance_diag_logs(capture) == ["", "two"]


def test_missing_or_gapped_instance_logs_are_rejected(tmp_path: Path) -> None:
    capture = tmp_path / "capture.tar.gz"
    write_capture(capture, [(TRACE_MEMBER, b"trace")])
    with pytest.raises(EvidenceError, match="capture/instance-0/diag.log"):
        read_instance_diag_logs(capture)
    write_capture(
        capture, [(instance_diag_member(0), b""), (instance_diag_member(2), b"")]
    )
    with pytest.raises(EvidenceError, match="capture/instance-1/diag.log"):
        read_instance_diag_logs(capture)


def test_an_instance_index_inverts_only_its_directory() -> None:
    assert instance_directory(0) == "instance-0"
    assert instance_index(instance_directory(12)) == 12
    for name in [
        "instance-",
        "instance-x",
        "instance--1",
        "instance-1/",
        "capture",
        "instance-١",
    ]:
        assert instance_index(name) is None, name


# The boot decomposition outcome.

INIT_START, ENTRY, SEED_PARSED, RENDER_START = 10.0, 35.0, 50.0, 75.0


def mark(name: str, start: float) -> dict[str, Any]:
    return {"name": name, "startTime": start}


def boot_marks(mount_done: float) -> list[Any]:
    return [
        mark("jaunder.boot.entry", ENTRY),
        mark("jaunder.boot.seed_parsed", SEED_PARSED),
        mark("jaunder.boot.render_start", RENDER_START),
        mark("jaunder.boot.mount_done", mount_done),
    ]


def navigation(mount_done: float, **extra: Any) -> dict[str, Any]:
    return {
        "id": 1,
        "wasmTimingSchema": "direct-init-v1",
        "commitToMountMs": mount_done + 50,
        "wasmInitStartMs": INIT_START,
        "wasmInitStartToBootEntryMs": ENTRY - INIT_START,
        "bootPhases": {
            "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
            "jaunder.boot.seed_parsed->jaunder.boot.render_start": RENDER_START
            - SEED_PARSED,
            "jaunder.boot.render_start->jaunder.boot.mount_done": mount_done
            - RENDER_START,
        },
        **extra,
    }


def test_the_segments_close_on_the_mount_done_mark() -> None:
    decomposition = decompose(navigation(105.0), boot_marks(105.0))
    assert isinstance(decomposition, Decomposition)
    labels = [label for label, _ in decomposition.segments]
    assert labels[:2] == ["wasm_init_start", "wasm_init_start_to_boot_entry"]
    assert len(labels) == 5
    assert sum(value for _, value in decomposition.segments) == 105.0
    assert (decomposition.boot_total_ms, decomposition.commit_to_mount_ms) == (
        105.0,
        155.0,
    )


def test_segments_missing_the_boot_total_are_a_closure_violation() -> None:
    assert (
        decompose(navigation(105.0), boot_marks(200.0)) is BootOutcome.CLOSURE_VIOLATION
    )


@pytest.mark.parametrize(
    "change",
    [
        {"wasmTimingSchema": "legacy-v1"},
        {"wasmInitStartMs": None},
        {"bootPhases": None},
        {"bootPhases": {"jaunder.boot.entry->jaunder.boot.mount_done": 95.0}},
        {
            "bootPhases": {
                "jaunder.boot.entry->jaunder.boot.mount_done": 40.0,
                "notBoot": 1.0,
                "boot.not_an_interval": "not finite",
                "diagnostic": 3.0,
            }
        },
    ],
    ids=["legacy", "no-init-start", "no-phases", "too-few-phases", "junk-phases"],
)
def test_a_navigation_without_a_full_decomposition_is_incomplete(
    change: dict[str, Any],
) -> None:
    assert decompose({**navigation(105.0), **change}, boot_marks(105.0)) is (
        BootOutcome.INCOMPLETE
    )


def test_without_a_mount_done_mark_only_a_page_total_closes() -> None:
    marks = boot_marks(105.0)[:-1] + [{"name": 7, "startTime": 1.0}, "not a mark"]
    assert decompose(navigation(105.0), marks) is BootOutcome.INCOMPLETE
    page = navigation(105.0, documentBootTotalMs=105.0)
    assert page_boot_decomposition_outcome(page, []) is BootOutcome.COMPLETE
    page["documentBootTotalMs"] = 107.0
    assert page_boot_decomposition_outcome(page, []) is BootOutcome.CLOSURE_VIOLATION


def test_a_fourth_mark_extends_the_decomposition_in_timeline_order() -> None:
    marks = [*boot_marks(105.0)[:3], mark("jaunder.boot.hydrated", 90.0)]
    marks.append(mark("jaunder.boot.mount_done", 105.0))
    nav = navigation(105.0)
    nav["bootPhases"] = {
        "jaunder.boot.hydrated->jaunder.boot.mount_done": 15.0,
        "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
        "jaunder.boot.seed_parsed->jaunder.boot.render_start": RENDER_START
        - SEED_PARSED,
        "jaunder.boot.render_start->jaunder.boot.hydrated": 90.0 - RENDER_START,
    }
    decomposition = decompose(nav, marks)
    assert isinstance(decomposition, Decomposition)
    labels = [label for label, _ in decomposition.segments]
    assert labels[2] == "jaunder.boot.entry->jaunder.boot.seed_parsed"
    assert labels[5] == "jaunder.boot.hydrated->jaunder.boot.mount_done"


def test_zero_length_phases_stay_in_timeline_order() -> None:
    marks = [
        mark("jaunder.boot.entry", ENTRY),
        mark("jaunder.boot.seed_parsed", SEED_PARSED),
        mark("jaunder.boot.render_start", SEED_PARSED),
        mark("jaunder.boot.mount_done", 105.0),
    ]
    nav = navigation(105.0)
    nav["bootPhases"] = {
        "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
        "jaunder.boot.render_start->jaunder.boot.mount_done": 105.0 - SEED_PARSED,
        "jaunder.boot.seed_parsed->jaunder.boot.render_start": 0.0,
        "boot.unanchored": 0.0,
    }
    decomposition = decompose(nav, marks)
    assert isinstance(decomposition, Decomposition)
    assert [label for label, _ in decomposition.segments][3:] == [
        "jaunder.boot.seed_parsed->jaunder.boot.render_start",
        "jaunder.boot.render_start->jaunder.boot.mount_done",
        "boot.unanchored",
    ]
