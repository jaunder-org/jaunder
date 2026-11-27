"""kache's hit and miss counts on the Cargo steps that compile under it."""

import json
import sys
from pathlib import Path
from typing import Any

import pytest
from conftest import Workflow

from dev.steps import kache_counts

SUMMARY = {
    "local_hits": 27,
    "prefetch_hits": 2,
    "remote_hits": 1,
    "misses": 22,
    "time_saved_ms": 49123,
}
COUNTS = {
    "jaunder.kache.hits": 30,
    "jaunder.kache.misses": 22,
    "jaunder.kache.avoided_compile_ms": 49123,
}


def stats(summary: Any) -> bytes:
    return json.dumps({"success": True, "report": {"summary": summary}}).encode()


def test_counts_sum_every_kind_of_hit() -> None:
    assert kache_counts(stats(SUMMARY)) == COUNTS


def test_a_session_of_probes_alone_counts_zero() -> None:
    zeros = dict.fromkeys(SUMMARY, 0)
    assert kache_counts(stats(zeros)) == dict.fromkeys(COUNTS, 0)


@pytest.mark.parametrize(
    "report",
    [
        b"not json",
        b"\xff",
        b"[]",
        b'{"report": null}',
        json.dumps({"report": {}}).encode(),
        stats("summary"),
        stats({key: value for key, value in SUMMARY.items() if key != "misses"}),
        stats(SUMMARY | {"misses": True}),
        stats(SUMMARY | {"time_saved_ms": 1.5}),
        stats(SUMMARY | {"remote_hits": None}),
    ],
)
def test_a_report_of_any_other_shape_counts_nothing(report: bytes) -> None:
    assert kache_counts(report) is None


# A kache whose `stats` prints KACHE_REPORT, recording each query's argv in
# `kache-queries` at its working directory.
KACHE = f"""#!{sys.executable}
import json, os, sys
with open("kache-queries", "a") as queries:
    queries.write(json.dumps(sys.argv[1:]) + "\\n")
print(os.environ["KACHE_REPORT"])
"""


def queries(root: Path) -> list[list[str]]:
    path = root / "kache-queries"
    lines = path.read_text().splitlines() if path.exists() else []
    return [json.loads(line) for line in lines]


def trees(root: Path) -> list[str]:
    """The KACHE_EVENT_ROOT each fake Cargo command saw, in order."""
    return (root / "kache-trees").read_text().splitlines()


def kache_attributes(step: dict[str, Any]) -> dict[str, Any]:
    return {
        key: value for key, value in step.items() if key.startswith("jaunder.kache")
    }


@pytest.fixture
def reporting(workflow: Workflow) -> Workflow:
    workflow.checkout.executable("kache", KACHE)
    return workflow


def test_a_compiling_step_records_the_counts_of_its_own_build_tree(
    reporting: Workflow,
) -> None:
    result = reporting.dev(
        "static", "tools-clippy", KACHE_REPORT=stats(SUMMARY).decode()
    )
    assert result.returncode == 0, result
    step = reporting.checkout.merged()["tools-clippy"]["args"]
    assert kache_attributes(step) == COUNTS
    tree = f"{reporting.root}/.xtask/kache/{step['trace_id']}-{step['span_id']}"
    assert trees(reporting.root) == [tree]
    assert queries(reporting.root) == [
        ["stats", "--json", "--last-build", "--root", tree]
    ]
    # The tree is a label on kache's events, never a directory.
    assert not Path(tree).exists()


def test_each_compiling_step_of_a_gate_has_a_tree_of_its_own(
    reporting: Workflow,
) -> None:
    result = reporting.dev("check-no-test", KACHE_REPORT=stats(SUMMARY).decode())
    assert result.returncode == 0, result
    slices = reporting.checkout.merged()
    compiling = ["tools-tests-build", "tools-tests", "tools-clippy"]
    for name in compiling:
        assert kache_attributes(slices[name]["args"]) == COUNTS, name
    labelled = [tree for tree in trees(reporting.root) if tree != "unset"]
    assert len(labelled) == len(set(labelled))
    assert set(labelled) == {query[-1] for query in queries(reporting.root)}
    for name in ["fmt", "tools-fmt", "cargo-deny"]:
        assert kache_attributes(slices[name]["args"]) == {}, name


@pytest.mark.parametrize("leaf", ["fmt", "tools-fmt", "cargo-deny"])
def test_a_cargo_step_that_compiles_nothing_is_not_labelled_or_queried(
    reporting: Workflow, leaf: str
) -> None:
    result = reporting.dev("static", leaf, KACHE_REPORT=stats(SUMMARY).decode())
    assert result.returncode == 0, result
    assert trees(reporting.root) == ["unset"]
    assert queries(reporting.root) == []


def test_a_sandboxed_compiling_step_has_no_kache_to_ask(reporting: Workflow) -> None:
    result = reporting.dev(
        "static",
        "tools-clippy",
        "--sandbox",
        JAUNDER_TOOLS_CARGO_HOME="home",
        RUSTC_WRAPPER="caller",
        KACHE_REPORT=stats(SUMMARY).decode(),
    )
    assert result.returncode == 0, result
    assert trees(reporting.root) == ["unset"]
    assert queries(reporting.root) == []


def test_a_failed_build_still_records_what_it_compiled(reporting: Workflow) -> None:
    result = reporting.dev(
        "static",
        "tools-clippy",
        failure="cargo clippy --manifest-path tools/Cargo.toml --workspace "
        "--all-targets --features coverage/host -- -D warnings",
        KACHE_REPORT=stats(SUMMARY).decode(),
    )
    assert result.returncode == 17
    step = reporting.checkout.merged()["tools-clippy"]["args"]
    assert step["jaunder.status"] == "failed"
    assert step["process.exit.code"] == 17
    assert kache_attributes(step) == COUNTS


def test_a_signalled_build_is_not_queried(reporting: Workflow) -> None:
    reporting.checkout.tool("cargo", "os.kill(os.getpid(), signal.SIGKILL)")
    result = reporting.dev(
        "static", "tools-clippy", KACHE_REPORT=stats(SUMMARY).decode()
    )
    assert result.returncode == 137
    step = reporting.checkout.merged()["tools-clippy"]["args"]
    assert step["jaunder.status"] == "signalled"
    assert queries(reporting.root) == []
    assert kache_attributes(step) == {}


@pytest.mark.parametrize("report", ["not json", json.dumps({"report": {}})])
def test_an_unreadable_report_records_nothing_and_keeps_the_outcome(
    reporting: Workflow, report: str
) -> None:
    result = reporting.dev("static", "tools-clippy", KACHE_REPORT=report)
    assert result.returncode == 0, result
    step = reporting.checkout.merged()["tools-clippy"]["args"]
    assert step["jaunder.status"] == "ok"
    assert kache_attributes(step) == {}


def test_a_failed_query_records_nothing_and_keeps_the_outcome(
    workflow: Workflow,
) -> None:
    # The suite's default kache fails, as kache does for a tree with no
    # events: a build that compiled nothing, or ran with KACHE_DISABLED.
    result = workflow.dev("static", "tools-clippy")
    assert result.returncode == 0, result
    step = workflow.checkout.merged()["tools-clippy"]["args"]
    assert step["jaunder.status"] == "ok"
    assert kache_attributes(step) == {}
