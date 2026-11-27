"""Trace-derived server-fn flow coverage: extraction, span-loss refusal, the
committed snapshot's bytes, and the real seed capture's signals."""

import dataclasses
import json
from pathlib import Path
from typing import Any

import pytest
from conftest import FIXTURES, REPOSITORY, require, write_capture

from dev.evidence import EvidenceError
from dev.server_fn_coverage import (
    CRATE_PREFIX,
    MAX_DEPTH,
    REGENERATE_CMD,
    SNAPSHOT_PATH,
    Coverage,
    ServerFn,
    coverage_from_capture,
    coverage_from_captures,
    coverage_from_jsonl,
    extract,
    parse_inventory,
    refuse_span_loss,
    regenerate,
    render,
    span_module,
    uri_endpoint,
    verify,
)
from dev.traces import (
    TRACE_MEMBER,
    Filters,
    Span,
    instance_diag_member,
    parse_spans,
    read_spans,
)

COVERAGE = FIXTURES / "server-fn-coverage"
SAMPLE = (COVERAGE / "coverage-sample.jsonl").read_text()


def fn(ident: str, module: str, endpoint: str | None = "") -> ServerFn:
    """A fn as the inventory lists it; its endpoint is its ident unless given."""
    vertical = module.split("::")[0]
    return ServerFn(
        f"{vertical}::{ident}", module, ident, ident if endpoint == "" else endpoint
    )


def sample_spans() -> list[Span]:
    return parse_spans(SAMPLE, Filters(), "sample")


SAMPLE_INVENTORY = [
    fn("create_post", "posts::api"),
    fn("update_post", "posts::api"),
    fn("get_post", "posts::api"),
    fn("list_tags", "tags::api"),
    fn("register", "registration::api"),
]


def titles(coverage: Coverage, qualified: str) -> list[str]:
    return sorted(coverage.covered.get(qualified, set()))


# Extraction.


def test_a_uri_hit_and_an_instrument_span_attribute_to_their_test() -> None:
    coverage = extract(sample_spans(), SAMPLE_INVENTORY)
    # The request span is one hop from the test; the instrument span, which
    # carries no `uri`, is two, so only its name can find it.
    assert titles(coverage, "posts::create_post") == ["creates a post"]
    assert titles(coverage, "posts::update_post") == ["creates a post"]
    # Its query string stripped, and in absolute as well as origin form.
    assert "posts::get_post" in coverage.covered
    assert "tags::list_tags" in coverage.covered


def test_an_unattributable_hit_is_an_orphan_with_its_reason() -> None:
    coverage = extract(sample_spans(), SAMPLE_INVENTORY)
    assert "registration::register" not in coverage.covered
    (reason,) = coverage.orphans["registration::register"]
    assert reason.startswith("unknown-parent:")


def test_a_parentless_request_hit_is_a_no_parent_orphan() -> None:
    spans = sample_spans()
    request = next(
        span
        for span in spans
        if span.name == "request" and span.uri.startswith("/api/create_post")
    )
    request.parent_span_id = ""
    coverage = extract(spans, SAMPLE_INVENTORY)
    assert "posts::create_post" not in coverage.covered
    assert coverage.orphans["posts::create_post"] == {"no-parent"}


def one_named_span(name: str, module: str, attr: str = "code.namespace") -> list[Span]:
    """One `e2e.test` span with a single child span of `name` declaring `module`."""
    spans = [
        {
            "traceId": "aa",
            "spanId": "t1",
            "name": "e2e.test",
            "attributes": [{"key": "e2e.test", "value": {"stringValue": "t"}}],
        },
        {
            "traceId": "aa",
            "spanId": "i1",
            "parentSpanId": "t1",
            "name": name,
            "attributes": [{"key": attr, "value": {"stringValue": module}}],
        },
    ]
    line = json.dumps({"resourceSpans": [{"scopeSpans": [{"spans": spans}]}]})
    return parse_spans(line, Filters(), "t")


def test_a_chain_that_never_reaches_a_test_is_depth_exceeded() -> None:
    spans = one_named_span("web.posts.update_post", "web::posts::api")
    spans[1].parent_span_id = "i1"
    coverage = extract(spans, [fn("update_post", "posts::api")])
    assert coverage.orphans == {"posts::update_post": {"depth-exceeded"}}


def test_static_assets_resolve_to_no_fn() -> None:
    coverage = extract(sample_spans(), SAMPLE_INVENTORY)
    assert not any("wasm" in name for name in [*coverage.covered, *coverage.orphans])


def test_a_foreign_legacy_module_never_confirms_a_name() -> None:
    coverage = extract(sample_spans(), [fn("update_post", "storage::posts")])
    assert coverage == Coverage()


def test_the_legacy_module_crate_prefix_is_stripped_before_comparing() -> None:
    coverage = extract(sample_spans(), [fn("update_post", "posts::api")])
    assert titles(coverage, "posts::update_post") == ["creates a post"]


def test_a_name_hit_without_a_module_is_not_counted() -> None:
    spans = one_named_span("__server_update_post", "")
    assert extract(spans, [fn("update_post", "posts::api")]) == Coverage()


def test_a_fn_hit_by_both_signals_is_counted_once_per_test() -> None:
    coverage = extract(sample_spans(), SAMPLE_INVENTORY)
    assert len(titles(coverage, "posts::create_post")) == 1


def test_the_current_module_attribute_identifies_a_span() -> None:
    spans = one_named_span(
        "__server_update_post", "web::posts::api", "code.module.name"
    )
    coverage = extract(spans, [fn("update_post", "posts::api")])
    assert titles(coverage, "posts::update_post") == ["t"]
    assert span_module(spans[1]) == "web::posts::api"


@pytest.mark.parametrize(
    "name", ["web.posts.update_post", "__server_update_post", "update_post"]
)
def test_every_naming_regime_needs_and_takes_the_right_module(name: str) -> None:
    inventory = [fn("update_post", "posts::api")]
    coverage = extract(one_named_span(name, "web::posts::api"), inventory)
    assert titles(coverage, "posts::update_post") == ["t"]
    assert extract(one_named_span(name, "web::elsewhere::api"), inventory) == Coverage()


def test_an_endpoint_is_matched_by_its_declared_value_not_the_fn_name() -> None:
    coverage = extract(sample_spans(), [fn("fetch_post", "posts::api", "get_post")])
    assert titles(coverage, "posts::fetch_post") == ["creates a post"]


def test_a_fn_without_an_endpoint_matches_no_uri() -> None:
    coverage = extract(sample_spans(), [fn("create_post", "elsewhere::api", None)])
    assert coverage == Coverage()


def test_one_verticals_hit_never_covers_another_verticals_namesake() -> None:
    creates = [
        fn("create", "posts::api", "posts/create"),
        fn("create", "audiences::api", "audiences/create"),
    ]
    by_name = one_named_span("web.posts.create", "web::posts::api")
    by_uri = one_named_span("masked", "")
    by_uri[1].uri = "/api/posts/create"
    for spans in [by_name, by_uri]:
        coverage = extract(spans, creates)
        assert titles(coverage, "posts::create") == ["t"]
        assert "audiences::create" not in coverage.covered


def test_a_uri_without_a_server_fn_path_names_no_endpoint() -> None:
    spans = one_named_span("request", "")
    for uri in ["/api/", "/static/app.wasm", "/api?x=/api/create_post"]:
        spans[1].uri = uri
        assert extract(spans, [fn("create_post", "posts::api")]) == Coverage()


def test_the_inventory_is_read_as_listed() -> None:
    listing = json.dumps(
        [
            {
                "qualified": "posts::create",
                "module": "posts::api",
                "ident": "create",
                "endpoint": "posts/create",
            }
        ]
    ).encode()
    assert parse_inventory(listing) == [
        ServerFn("posts::create", "posts::api", "create", "posts/create")
    ]
    assert parse_inventory(b"[]") == []
    assert ServerFn(
        "posts::create", "posts::api", "create", None
    ).candidate_span_names() == (
        "web.posts.create",
        "__server_create",
        "create",
    )


@pytest.mark.parametrize(
    "listing",
    [b"", b"{", b'[{"qualified": "posts::create"}]', b"[1]", b'"text"', b"{}x"],
)
def test_a_listing_that_is_not_an_inventory_is_refused(listing: bytes) -> None:
    with pytest.raises(EvidenceError, match="inventory listing is malformed"):
        parse_inventory(listing)


# Span loss (#2029).

STARTED = json.dumps(
    {
        "level": "WARN",
        "fields": {"name": "BatchSpanProcessor.SpanDroppingStarted"},
        "target": "opentelemetry_sdk",
    }
)
DROPPED = json.dumps(
    {
        "level": "WARN",
        "fields": {
            "name": "BatchSpanProcessor.SpansDropped",
            "dropped_span_count": 280,
        },
        "target": "opentelemetry_sdk",
    }
)
EXPORT_ERROR = json.dumps(
    {
        "level": "ERROR",
        "fields": {"name": "BatchSpanProcessor.ExportError", "error": "timed out"},
        "target": "opentelemetry_sdk",
    }
)


def test_a_queue_full_drop_is_refused_with_its_lower_bound() -> None:
    with pytest.raises(EvidenceError) as refused:
        refuse_span_loss(f"{STARTED}\n{DROPPED}\n")
    assert "at least 280" in str(refused.value)
    assert (
        "BatchSpanProcessor.SpanDroppingStarted, BatchSpanProcessor.SpansDropped"
        in (str(refused.value))
    )


def test_an_export_error_is_refused_without_a_count() -> None:
    with pytest.raises(EvidenceError, match="an unknown number of"):
        refuse_span_loss(EXPORT_ERROR)


def test_a_clean_or_unrelated_log_passes() -> None:
    refuse_span_loss("")
    unrelated = [
        {"level": "WARN", "fields": {"message": "slow"}, "target": "sqlx::query"},
        {
            "level": "INFO",
            "fields": {"name": "BatchSpanProcessor.X"},
            "target": "opentelemetry_sdk",
        },
        {"level": "WARN", "fields": "flat", "target": "opentelemetry_sdk"},
        ["not", "a", "record"],
    ]
    refuse_span_loss("\n".join([*map(json.dumps, unrelated), "not json"]))


# Captures and the snapshot.


def sample_capture(path: Path, diag_log: str | None = "") -> Path:
    members = [(TRACE_MEMBER, SAMPLE.encode())]
    if diag_log is not None:
        members.append((instance_diag_member(0), diag_log.encode()))
    write_capture(path, members)
    return path


def test_empty_or_unparseable_jsonl_fails_closed() -> None:
    for jsonl, message in [
        ("", "no spans"),
        ("\n\n", "no spans"),
        ("{not json\n", "parsing the e2e capture's otel-traces.jsonl"),
    ]:
        with pytest.raises(EvidenceError, match=message):
            coverage_from_jsonl(jsonl, [])


def test_a_capture_with_a_matching_snapshot_verifies(tmp_path: Path) -> None:
    capture = sample_capture(tmp_path / "capture-sqlite.tar.gz")
    snapshot = tmp_path / "server-fns.json"
    coverage = coverage_from_captures([capture], SAMPLE_INVENTORY)
    regenerate(coverage, snapshot)
    assert verify(coverage, snapshot) == "4 covered; snapshot current"


def test_a_stale_snapshot_names_itself_and_the_remedy(tmp_path: Path) -> None:
    capture = sample_capture(tmp_path / "capture-sqlite.tar.gz")
    coverage = coverage_from_capture(capture, SAMPLE_INVENTORY)
    snapshot = tmp_path / "server-fns.json"
    for committed in [None, "stale\n", render(coverage).replace("  ", "    ")]:
        if committed is not None:
            snapshot.write_text(committed)
        with pytest.raises(EvidenceError) as refused:
            verify(coverage, snapshot)
        assert str(snapshot) in str(refused.value)
        assert REGENERATE_CMD in str(refused.value)
    snapshot.write_bytes(b"\xff")
    with pytest.raises(EvidenceError, match="out of date"):
        verify(coverage, snapshot)
    # A checkout that rewrote the line endings is drift too.
    snapshot.write_bytes(render(coverage).replace("\n", "\r\n").encode())
    with pytest.raises(EvidenceError, match="out of date"):
        verify(coverage, snapshot)


def test_regeneration_writes_only_the_snapshot(tmp_path: Path) -> None:
    coverage_dir = tmp_path / "docs" / "coverage"
    snapshot = coverage_dir / "server-fns.json"
    coverage = Coverage({"posts::create_post": {"t"}}, {"posts::x": {"no-parent"}})
    detail = regenerate(coverage, snapshot)
    assert detail == f"1 covered, 1 fn(s) with unattributed hits → {snapshot}"
    retired = coverage_dir / "server-fns-evidence.json"
    retired.write_text("retired sentinel")
    regenerate(coverage, snapshot)
    assert json.loads(snapshot.read_text()) == {
        "covered": ["posts::create_post"],
        "orphans": {"posts::x": ["no-parent"]},
    }
    assert retired.read_text() == "retired sentinel"
    assert sorted(path.name for path in coverage_dir.iterdir()) == [
        "server-fns-evidence.json",
        "server-fns.json",
    ]


def test_a_snapshot_that_cannot_be_written_is_refused(tmp_path: Path) -> None:
    (tmp_path / "docs").write_text("not a directory")
    snapshot = tmp_path / "docs" / "coverage" / "server-fns.json"
    with pytest.raises(EvidenceError, match=f"writing {snapshot}: "):
        regenerate(Coverage(), snapshot)


def test_unusable_captures_are_named_failures(tmp_path: Path) -> None:
    empty = tmp_path / "empty.tar.gz"
    write_capture(empty, [])
    malformed = tmp_path / "malformed.tar.gz"
    malformed.write_text("not a gzip archive")
    for capture in [tmp_path / "missing.tar.gz", empty, malformed, tmp_path]:
        with pytest.raises(EvidenceError):
            coverage_from_captures([capture], SAMPLE_INVENTORY)
    with pytest.raises(EvidenceError, match="no e2e capture at .*missing"):
        coverage_from_capture(tmp_path / "missing.tar.gz", SAMPLE_INVENTORY)
    trace_only = tmp_path / "trace-only.tar.gz"
    write_capture(trace_only, [(instance_diag_member(0), b"")])
    with pytest.raises(EvidenceError, match="reading the otel trace from"):
        coverage_from_capture(trace_only, SAMPLE_INVENTORY)


@pytest.mark.parametrize(
    ("diag_log", "message"),
    [(DROPPED, "refusing .*at least 280"), (None, "diag.log")],
)
def test_a_capture_with_reported_or_unprovable_span_loss_is_refused(
    tmp_path: Path, diag_log: str | None, message: str
) -> None:
    capture = sample_capture(tmp_path / "capture-sqlite.tar.gz", diag_log)
    with pytest.raises(EvidenceError, match=message):
        coverage_from_captures([capture], SAMPLE_INVENTORY)


def test_coverage_is_the_union_of_every_lane_capture(tmp_path: Path) -> None:
    first = sample_capture(tmp_path / "capture-a.tar.gz")
    second = tmp_path / "capture-b.tar.gz"
    trace = one_named_span("web.posts.update_post", "web::posts::api")
    record = {
        "resourceSpans": [{"scopeSpans": [{"spans": [span.raw for span in trace]}]}]
    }
    write_capture(
        second,
        [(TRACE_MEMBER, json.dumps(record).encode()), (instance_diag_member(0), b"")],
    )
    union = coverage_from_captures([first, second], SAMPLE_INVENTORY)
    assert titles(union, "posts::update_post") == ["creates a post", "t"]
    assert "registration::register" in union.orphans
    with pytest.raises(EvidenceError, match="no e2e captures supplied"):
        coverage_from_captures([], SAMPLE_INVENTORY)


# The snapshot's bytes. The determinism fixtures are three real regenerate
# inputs from one tree, in the pre-#745 combined shape: their covered keys and
# orphans agree while their test titles differ.


def run_coverage(name: str) -> Coverage:
    run = json.loads((COVERAGE / "determinism" / f"run-{name}.json").read_text())
    return Coverage(
        {key: set(value) for key, value in run["covered"].items()},
        {key: set(value) for key, value in run["orphans"].items()},
    )


def test_runs_that_disagree_on_titles_render_one_snapshot() -> None:
    runs = [run_coverage(name) for name in "abc"]
    # Without this the projection below would prove nothing about titles.
    assert (
        len({json.dumps(sorted(map(sorted, run.covered.values()))) for run in runs})
        == 3
    )
    assert len({render(run) for run in runs}) == 1


def test_the_snapshot_holds_no_titles_and_renders_stably() -> None:
    coverage = Coverage({"b::fn": {"t"}, "a::fn": {"authenticated user can post"}})
    rendered = render(coverage)
    assert "authenticated user" not in rendered
    assert (
        rendered
        == '{\n  "covered": [\n    "a::fn",\n    "b::fn"\n  ],\n  "orphans": {}\n}\n'
    )
    assert render(Coverage({"a::fn": {"t"}, "b::fn": {"u"}})) == rendered
    assert render(Coverage()) == '{\n  "covered": [],\n  "orphans": {}\n}\n'


def test_the_snapshot_bytes_sort_every_list_and_keep_unicode() -> None:
    coverage = Coverage(
        {"posts::update": {"t"}, "auth::login": {"u"}},
        {
            "themes::é": {"unknown-parent:b2", "no-parent"},
            "media::upload": {"depth-exceeded"},
        },
    )
    assert render(coverage).encode() == (
        b"{\n"
        b'  "covered": [\n'
        b'    "auth::login",\n'
        b'    "posts::update"\n'
        b"  ],\n"
        b'  "orphans": {\n'
        b'    "media::upload": [\n'
        b'      "depth-exceeded"\n'
        b"    ],\n"
        b'    "themes::\xc3\xa9": [\n'
        b'      "no-parent",\n'
        b'      "unknown-parent:b2"\n'
        b"    ]\n"
        b"  }\n"
        b"}\n"
    )


def committed_snapshot() -> Path:
    path = REPOSITORY / SNAPSHOT_PATH
    require(path.is_file(), f"the repository's {SNAPSHOT_PATH}")
    return path


@pytest.mark.repository
def test_the_committed_snapshot_renders_back_byte_for_byte() -> None:
    raw = committed_snapshot().read_text()
    committed = json.loads(raw)
    coverage = Coverage(
        {name: {"t"} for name in committed["covered"]},
        {name: set(reasons) for name, reasons in committed["orphans"].items()},
    )
    assert render(coverage) == raw
    for name in committed["covered"]:
        vertical, _, ident = name.partition("::")
        assert vertical and ident and "::" not in ident, name


# The real seed capture: the `sqlite × chromium` capture the snapshot was first
# derived from, reduced to the spans extraction reads, with the inventory
# recorded at the same time.


@pytest.fixture(scope="module")
def seed() -> list[Span]:
    return read_spans(COVERAGE / "otel-traces-seed.jsonl", Filters())


@pytest.fixture(scope="module")
def seed_inventory() -> list[ServerFn]:
    return parse_inventory((COVERAGE / "seed-inventory.json").read_bytes())


def test_each_signal_alone_covers_everything_the_union_does(
    seed: list[Span], seed_inventory: list[ServerFn]
) -> None:
    by_name = [dataclasses.replace(span, uri="") for span in seed]
    by_uri = [
        span
        if span.name == "e2e.test"
        else dataclasses.replace(span, name=f"masked.{span.name}")
        for span in seed
    ]
    union = extract(seed, seed_inventory)
    assert union.covered, "a fixture covering nothing makes every check vacuous"
    assert sorted(extract(by_name, seed_inventory).covered) == sorted(union.covered)
    only_uri = extract(by_uri, seed_inventory)
    assert sorted(only_uri.covered) == sorted(union.covered)
    # `media::upload` takes multipart input, the sharpest case for the uri signal.
    assert "media::upload" in only_uri.covered


def test_the_instrument_spans_carry_the_module_the_name_signal_compares(
    seed: list[Span], seed_inventory: list[ServerFn]
) -> None:
    verticals = {server_fn.qualified.partition("::")[0] for server_fn in seed_inventory}
    instrument = [
        span
        for span in seed
        if any(span.name.startswith(f"web.{v}.") for v in verticals)
    ]
    assert instrument, "the reduction dropped the name signal's evidence entirely"
    for span in instrument:
        assert span_module(span).startswith("web::"), span.name


def test_the_seed_keeps_non_api_traffic_as_the_negative_case(seed: list[Span]) -> None:
    assert any(span.uri and "/api/" not in span.uri for span in seed)


@pytest.mark.repository
def test_the_seed_covers_every_fn_the_committed_snapshot_does(
    seed: list[Span], seed_inventory: list[ServerFn]
) -> None:
    committed: dict[str, Any] = json.loads(committed_snapshot().read_text())
    covered = extract(seed, seed_inventory).covered
    assert [name for name in committed["covered"] if name not in covered] == []


# `repository-policy`'s endpoint cross-check (ADR-0120) reads the seed's
# observed hits from its own fixture, so it needs no trace reader.
SEED_ENDPOINTS = "tools/repository-policy/src/testdata/server-fn-seed-endpoints.json"


def seed_endpoints(spans: list[Span]) -> bytes:
    """Each hit's `(module, span name, URI path)`, as that fixture's bytes:
    every span carrying a `web::` module, which is all the cross-check can
    locate a fn by, with the request path its nearest `uri`-bearing span,
    itself included, served."""
    by_id = {span.span_id: span for span in spans if span.span_id}
    observed = set()
    for span in spans:
        module = span_module(span)
        current: Span | None = span
        for _ in range(MAX_DEPTH):
            if current is None or not module.startswith(CRATE_PREFIX):
                break
            if current.uri:
                observed.add((module, span.name, f"/api/{uri_endpoint(current.uri)}"))
                break
            current = by_id.get(current.parent_span_id)
    hits = [
        {"module": module, "span": name, "uri_path": path}
        for module, name, path in sorted(observed)
    ]
    return (json.dumps(hits, indent=2) + "\n").encode()


@pytest.mark.repository
def test_the_seed_endpoint_fixture_is_derived_from_the_seed(seed: list[Span]) -> None:
    """To refresh it after re-recording the seed, write `seed_endpoints(seed)`
    to that fixture."""
    path = REPOSITORY / SEED_ENDPOINTS
    require(path.is_file(), f"the repository's {SEED_ENDPOINTS}")
    assert path.read_bytes() == seed_endpoints(seed)
