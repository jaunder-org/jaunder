"""`dev coverage` and `dev doctests`: real Git admission, then fail-fast steps.

Their full success populations are in test_gates.
"""

import json
import subprocess
from pathlib import Path
from typing import Any

import pytest
from conftest import Checkout, Workflow

CLEAN = ["git", "status", "--porcelain", "--untracked-files=all"]
FLAGS = "--no-update-lock-file --no-write-lock-file"
NIX_EVAL = f"nix path-info --derivation {FLAGS}"
NIX_BUILD = f"nix build -L {FLAGS}"
STEPS = {
    "coverage": [
        f"{NIX_EVAL} .#checks.x86_64-linux.coverage",
        f"{NIX_BUILD} --print-out-paths --out-link .xtask/gcroots/coverage .#checks.x86_64-linux.coverage",
        f"{NIX_EVAL} .#checks.x86_64-linux.coverage-gate",
        f"{NIX_BUILD} --print-out-paths --no-link .#checks.x86_64-linux.coverage-gate",
        "nix path-info --derivation --accept-flake-config .#coverage-check",
        "nix build -L --accept-flake-config --print-out-paths --no-link .#coverage-check",
        "coverage-check policy --out .xtask/gcroots/coverage",
    ],
    "doctests": [
        f"{NIX_EVAL} .#checks.x86_64-linux.doctests",
        f"{NIX_BUILD} --print-out-paths --out-link .xtask/gcroots/doctests .#checks.x86_64-linux.doctests",
        f"{NIX_EVAL} .#checks.x86_64-linux.doctests-gate",
        f"{NIX_BUILD} --print-out-paths --no-link .#checks.x86_64-linux.doctests-gate",
    ],
}


@pytest.mark.parametrize(
    ("workflow_name", "failing"),
    [(name, index) for name, lines in STEPS.items() for index in range(len(lines))],
)
def test_workflows_stop_at_each_failing_boundary(
    workflow: Workflow, workflow_name: str, failing: int
) -> None:
    lines = STEPS[workflow_name]
    result = workflow.dev(workflow_name, failure=lines[failing])
    assert result.returncode == 17
    assert workflow.commands() == [
        CLEAN,
        *(line.split(" ") for line in lines[: failing + 1]),
    ]


@pytest.mark.parametrize("workflow_name", ["coverage", "doctests"])
@pytest.mark.parametrize("staged", [False, True])
def test_public_workflows_refuse_a_really_dirty_checkout(
    checkout: Checkout, workflow_name: str, staged: bool
) -> None:
    for tool in ["nix", "cargo"]:
        checkout.tool(tool, "")
    # `dev` writes its trace under .xtask, which Jaunder's checkout ignores.
    (checkout.root / ".git" / "info" / "exclude").write_text(".xtask/\n")
    (checkout.root / "dirty").write_text("changed")
    if staged:
        subprocess.run(["git", "-C", str(checkout.root), "add", "dirty"], check=True)
    result = checkout.dev(workflow_name)
    assert result.returncode == 1
    assert result.stderr.startswith(b"verify workflows require a clean checkout\n")
    assert checkout.order() == ""


@pytest.mark.parametrize(
    ("extra", "cache"), [({"NIX_VALID": "1"}, "reused"), ({}, "substituted")]
)
def test_producer_and_gate_record_where_their_outputs_came_from(
    checkout: Checkout, extra: dict[str, str], cache: str
) -> None:
    checkout.nix_store()
    (checkout.root / ".git" / "info" / "exclude").write_text(".xtask/\nbuilt\n")
    result = checkout.run(checkout.command("doctests"), checkout.environment(**extra))
    assert result.returncode == 0, result
    slices = checkout.merged()
    assert slices["doctests"]["args"]["jaunder.cache"] == cache
    # The fake store holds one output, which the producer step leaves valid.
    assert slices["doctests-gate"]["args"]["jaunder.cache"] == "reused"
    # The evaluation and the build both leave the lock file untouched.
    assert checkout.order().splitlines()[:2] == STEPS["doctests"][:2]


# The coverage producer's `status.json` stages, as `emit` records them: each
# started stage carries its wall-clock start and duration; `not-run` neither.
# The fake build writes them with T0, its own clock (conftest's NIX_BUILD_STATUS).
T0 = "@T0_MS@"
STAGES: list[dict[str, Any]] = [
    {
        "stage": "workspace-resolution",
        "outcome": "success",
        "started_unix_ms": T0,
        "duration_ms": 5,
    },
    {
        "stage": "test-census",
        "outcome": {"exit-code": {"exit_code": 101}},
        "started_unix_ms": T0,
        "duration_ms": 10,
    },
    {"stage": "instrumented-test-run", "outcome": "not-run"},
]


def _status(stages: object) -> str:
    return json.dumps({"stages": stages}).replace(f'"{T0}"', T0)


def _coverage_output(checkout: Checkout) -> Path:
    """The fake store's one output as the coverage producer, held by its GC
    root, and as the policy tool's package; returns the output."""
    (checkout.root / ".git" / "info" / "exclude").write_text(
        ".xtask/\nbuilt\nbuild-out/\n"
    )
    out = checkout.root / "build-out"
    (out / "bin").mkdir(parents=True)
    policy = out / "bin" / "coverage-check"
    policy.write_text("#!/bin/sh\nprintf '{\"steps\":[]}\\n'\n")
    policy.chmod(0o755)
    gcroots = checkout.root / ".xtask" / "gcroots"
    gcroots.mkdir(parents=True)
    (gcroots / "coverage").symlink_to(out)
    return out


def _stage_spans(checkout: Checkout) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """The coverage producer step's `build` and the spans under it."""
    events = checkout.events()
    coverage = checkout.merged()["coverage"]["args"]["span_id"]
    (build,) = (
        event
        for event in events
        if event["name"] == "build" and event["args"]["parent_id"] == coverage
    )
    children = [
        event
        for event in events
        if event["args"]["parent_id"] == build["args"]["span_id"]
    ]
    return build, children


def test_a_coverage_build_traces_the_stages_its_producer_timed_under_it(
    checkout: Checkout,
) -> None:
    checkout.nix_store()
    _coverage_output(checkout)
    result = checkout.dev(
        "coverage", NIX_ULTIMATE="1", NIX_BUILD_STATUS=_status(STAGES)
    )
    assert result.returncode == 0, result
    build, children = _stage_spans(checkout)
    stages = {event["name"]: event for event in children}
    assert sorted(stages) == ["test-census", "workspace-resolution"]
    census, resolution = stages["test-census"], stages["workspace-resolution"]
    assert census["ts"] == resolution["ts"] >= build["ts"]
    assert (census["dur"], census["args"]["jaunder.status"]) == (10_000, "failed")
    assert (resolution["dur"], resolution["args"]["jaunder.status"]) == (5_000, "ok")


@pytest.mark.parametrize(
    "status",
    [
        "not json",
        _status("none"),
        # One malformed stage withholds them all.
        _status(
            [*STAGES, {"stage": "text-report", "outcome": "success", "duration_ms": 1}]
        ),
        _status([{**STAGES[0], "duration_ms": -1}]),
        _status([{**STAGES[0], "started_unix_ms": True}]),
    ],
    ids=["not-json", "not-a-list", "duration-alone", "negative", "bool"],
)
def test_a_malformed_status_traces_no_stages(checkout: Checkout, status: str) -> None:
    checkout.nix_store()
    _coverage_output(checkout)
    result = checkout.dev("coverage", NIX_ULTIMATE="1", NIX_BUILD_STATUS=status)
    assert result.returncode == 0, result
    assert _stage_spans(checkout)[1] == []
    assert checkout.merged()["coverage"]["args"]["jaunder.status"] == "ok"


@pytest.mark.parametrize("earlier", [True, False], ids=["earlier-run", "missing"])
def test_stages_this_build_did_not_time_are_not_traced(
    checkout: Checkout, earlier: bool
) -> None:
    """A reused output's stages ran before this step, as an earlier run's did."""
    checkout.nix_store()
    out = _coverage_output(checkout)
    if earlier:
        (out / "status.json").write_text(_status(STAGES).replace(T0, "1700000000000"))
    result = checkout.dev("coverage", NIX_VALID="1")
    assert result.returncode == 0, result
    assert _stage_spans(checkout)[1] == []
