"""`dev coverage` and `dev doctests`: real Git admission, then fail-fast steps.

Their full success populations are in test_gates.
"""

import json
import subprocess
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
STARTED_MS = 1_700_000_000_000
STAGES: list[dict[str, Any]] = [
    {
        "stage": "workspace-resolution",
        "outcome": "success",
        "started_unix_ms": STARTED_MS,
        "duration_ms": 5,
    },
    {
        "stage": "test-census",
        "outcome": {"exit-code": {"exit_code": 101}},
        "started_unix_ms": STARTED_MS + 10,
        "duration_ms": 2000,
    },
    {"stage": "instrumented-test-run", "outcome": "not-run"},
]


def _coverage_output(checkout: Checkout, status: object | None) -> None:
    """The fake store's one output as a coverage producer and its policy tool.

    `status`, unless None, is written as the output's `status.json`.
    """
    (checkout.root / ".git" / "info" / "exclude").write_text(
        ".xtask/\nbuilt\nbuild-out/\n"
    )
    out = checkout.root / "build-out"
    (out / "bin").mkdir(parents=True)
    policy = out / "bin" / "coverage-check"
    policy.write_text("#!/bin/sh\nprintf '{\"steps\":[]}\\n'\n")
    policy.chmod(0o755)
    if status is not None:
        (out / "status.json").write_text(
            status if isinstance(status, str) else json.dumps(status)
        )


def _stage_spans(checkout: Checkout) -> list[dict[str, Any]]:
    """The spans under the coverage producer step's `build`."""
    events = checkout.events()

    def child(parent: str, name: str) -> str:
        (span_id,) = (
            event["args"]["span_id"]
            for event in events
            if event["name"] == name and event["args"]["parent_id"] == parent
        )
        return str(span_id)

    build = child(checkout.merged()["coverage"]["args"]["span_id"], "build")
    return [event for event in events if event["args"]["parent_id"] == build]


def test_a_built_coverage_output_traces_its_started_stages_under_its_build(
    checkout: Checkout,
) -> None:
    checkout.nix_store()
    _coverage_output(checkout, {"stages": STAGES})
    result = checkout.run(
        checkout.command("coverage"), checkout.environment(NIX_ULTIMATE="1")
    )
    assert result.returncode == 0, result
    assert checkout.merged()["coverage"]["args"]["jaunder.cache"] == "built"
    stages = {event["name"]: event for event in _stage_spans(checkout)}
    assert sorted(stages) == ["test-census", "workspace-resolution"]
    census = stages["test-census"]
    assert census["ts"] == (STARTED_MS + 10) * 1000
    assert census["dur"] == 2000 * 1000
    assert census["args"]["jaunder.status"] == "failed"
    assert stages["workspace-resolution"]["dur"] == 5 * 1000
    assert stages["workspace-resolution"]["args"]["jaunder.status"] == "ok"


@pytest.mark.parametrize(
    ("status", "extra"),
    [
        # Reused: its stages ran in an earlier build.
        ({"stages": STAGES}, {"NIX_VALID": "1"}),
        (None, {"NIX_ULTIMATE": "1"}),
        ("not json", {"NIX_ULTIMATE": "1"}),
        ({"stages": "none"}, {"NIX_ULTIMATE": "1"}),
        # One malformed stage withholds them all.
        (
            {
                "stages": [
                    *STAGES,
                    {"stage": "text-report", "outcome": "success", "duration_ms": 1},
                ]
            },
            {"NIX_ULTIMATE": "1"},
        ),
        (
            {"stages": [{**STAGES[0], "duration_ms": -1}]},
            {"NIX_ULTIMATE": "1"},
        ),
        (
            {"stages": [{**STAGES[0], "started_unix_ms": True}]},
            {"NIX_ULTIMATE": "1"},
        ),
    ],
)
def test_coverage_stages_are_traced_only_from_a_readable_status_it_built(
    checkout: Checkout, status: object | None, extra: dict[str, str]
) -> None:
    checkout.nix_store()
    _coverage_output(checkout, status)
    result = checkout.run(checkout.command("coverage"), checkout.environment(**extra))
    assert result.returncode == 0, result
    assert _stage_spans(checkout) == []
    assert checkout.merged()["coverage"]["args"]["jaunder.status"] == "ok"
