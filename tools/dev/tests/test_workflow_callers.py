"""The Git hooks and CI aggregates keep a failure a failure.

Each test guards a way the verification could pass when it should fail: a
hook that loses `dev`'s exit status, a required aggregate that is skipped
(which branch protection accepts) or that passes a failed lane, and a
measurement step whose failure is continued. The hooks and the aggregates'
verdict scripts run for real under bash. It needs the repository's
`.githooks/` and `.github/`, so only the filtered `checks.dev-python` source
may skip it.
"""

import os
import subprocess
from pathlib import Path

import pytest
from conftest import REPOSITORY, load_workflow, require


@pytest.mark.parametrize(
    ("hook", "workflow"), [("pre-commit", "precommit"), ("pre-push", "prepush")]
)
@pytest.mark.parametrize("status", [0, 17])
def test_hooks_invoke_only_their_dev_workflow_and_preserve_its_status(
    tmp_path: Path, hook: str, workflow: str, status: int
) -> None:
    script = REPOSITORY / ".githooks" / hook
    require(script.is_file(), f"the repository's .githooks/{hook}")
    fake = tmp_path / "dev"
    fake.write_text(
        '#!/bin/sh\nprintf \'%s\\n\' "$@" > "$HOOK_ARGS"\nexit "$HOOK_STATUS"\n',
        encoding="utf-8",
    )
    fake.chmod(0o755)
    arguments = tmp_path / "args"
    result = subprocess.run(
        ["bash", str(script)],
        cwd=tmp_path,
        env=os.environ
        | {
            "PATH": f"{tmp_path}{os.pathsep}{os.environ['PATH']}",
            "HOOK_ARGS": str(arguments),
            "HOOK_STATUS": str(status),
            "SKIP_PRE_COMMIT": "0",
            "SKIP_PRE_PUSH": "0",
        },
        capture_output=True,
        check=False,
    )
    assert result.returncode == status, result
    assert arguments.read_text() == f"{workflow}\n"


def test_required_aggregates_run_even_when_a_lane_fails() -> None:
    jobs = load_workflow("ci.yml")["jobs"]
    # A job whose `needs` failed is skipped unless its `if:` holds a status
    # function, and branch protection accepts a skipped required check.
    for name in ["validate-no-e2e", "e2e-gate"]:
        assert "!cancelled()" in jobs[name]["if"], name
    # The aggregates skip a push of a merge-queue-tested commit; a pull
    # request or merge-queue run must never count as tested.
    tested = jobs["merge-queue-verdict"]["outputs"]["tested"]
    assert tested.startswith("${{ github.event_name == 'push' && "), tested


# The jobs an aggregate's script reads the results of.
AGGREGATED = [
    "validate-docs",
    "validate-host",
    "validate-hermetic",
    "validate-test-checks",
    "validate-coverage",
    "e2e-lane",
    "e2e-reconcile",
]


def verdict(script: str, merge_queue: str, docs_only: bool, results: list[str]) -> bool:
    """Whether an aggregate's script passes under bash, as the runner runs it,
    given the verdict job's result and `docs_only` output and the other jobs'
    results (`job=result` by the job's name without `validate-`; unlisted
    ones skipped)."""
    run = script.replace("${{ needs.merge-queue-verdict.result }}", merge_queue)
    run = run.replace(
        "${{ needs.merge-queue-verdict.outputs.docs_only }}",
        "true" if docs_only else "false",
    )
    given = dict(entry.split("=") for entry in results)
    for job in AGGREGATED:
        result = given.get(job.removeprefix("validate-"), "skipped")
        run = run.replace(f"${{{{ needs.{job}.result }}}}", result)
    assert "${{" not in run, f"unsubstituted: {run}"
    output = subprocess.run(["bash", "-e", "-c", run], capture_output=True, check=False)
    return output.returncode == 0


EVERY_LANE = ["host=success", "test-checks=success", "coverage=success"]


@pytest.mark.parametrize(
    ("merge_queue", "docs_only", "results", "passes"),
    [
        ("success", True, ["docs=success"], True),
        ("success", True, ["docs=failure"], False),
        ("success", True, ["docs=skipped"], False),
        ("success", False, [*EVERY_LANE, "hermetic=success"], True),
        ("success", False, [*EVERY_LANE, "hermetic=skipped"], False),
        ("failure", False, ["host=success"], False),
    ],
)
def test_validate_no_e2e_passes_only_when_its_lanes_succeed(
    merge_queue: str, docs_only: bool, results: list[str], passes: bool
) -> None:
    script = load_workflow("ci.yml")["jobs"]["validate-no-e2e"]["steps"][0]["run"]
    assert verdict(script, merge_queue, docs_only, results) == passes


@pytest.mark.parametrize(
    ("merge_queue", "docs_only", "results", "passes"),
    [
        ("success", True, ["e2e-lane=skipped", "e2e-reconcile=skipped"], True),
        ("success", True, ["e2e-lane=success", "e2e-reconcile=skipped"], False),
        ("success", False, ["e2e-lane=success", "e2e-reconcile=success"], True),
        ("success", False, ["e2e-lane=skipped", "e2e-reconcile=skipped"], False),
        ("failure", False, [], False),
    ],
)
def test_e2e_gate_passes_only_when_its_lanes_succeed(
    merge_queue: str, docs_only: bool, results: list[str], passes: bool
) -> None:
    script = load_workflow("ci.yml")["jobs"]["e2e-gate"]["steps"][0]["run"]
    assert verdict(script, merge_queue, docs_only, results) == passes


def test_a_failed_performance_measurement_fails_the_job() -> None:
    steps = load_workflow("performance.yml")["jobs"]["performance"]["steps"]
    measurements = [step for step in steps if " dev perf" in step.get("run", "")]
    assert measurements, "no `dev perf` steps"
    for step in measurements:
        assert "continue-on-error" not in step, step["run"]


@pytest.mark.parametrize(
    ("dependencies", "arguments"),
    [("true", ["ci-validate-host", "--advisories"]), ("false", ["ci-validate-host"])],
)
def test_the_host_lane_checks_advisories_only_for_a_dependency_change(
    tmp_path: Path, dependencies: str, arguments: list[str]
) -> None:
    jobs = load_workflow("ci.yml")["jobs"]
    scope = jobs["merge-queue-verdict"]["outputs"]["dependencies"]
    assert scope == "${{ steps.scope.outputs.dependencies }}", scope
    (step,) = [
        step
        for step in jobs["validate-host"]["steps"]
        if "dev ci-validate-host" in step.get("run", "")
    ]
    assert step["env"]["DEPENDENCIES"] == (
        "${{ needs.merge-queue-verdict.outputs.dependencies }}"
    )
    # The step's script runs for real; a fake `nix` records what `dev` gets.
    fake = tmp_path / "nix"
    fake.write_text(
        '#!/bin/sh\nshift 5\nprintf \'%s\\n\' "$@" > "$NIX_ARGS"\n', encoding="utf-8"
    )
    fake.chmod(0o755)
    recorded = tmp_path / "args"
    result = subprocess.run(
        ["bash", "-e", "-c", step["run"]],
        env=os.environ
        | {
            "PATH": f"{tmp_path}{os.pathsep}{os.environ['PATH']}",
            "NIX_ARGS": str(recorded),
            "DEPENDENCIES": dependencies,
        },
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result
    assert recorded.read_text().splitlines() == arguments
