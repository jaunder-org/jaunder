"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran.
"""

import json
from pathlib import Path

from dev import report, steps
from dev.commands import E2E_EVIDENCE

CATALOG = "end2end/e2e-lanes.json"
# Where `scripts/e2e-workflow` keeps each producer's build record and artifacts.
DIAGNOSTICS = ".xtask/diagnostics"
# Server-function flow coverage is derived from this pair's reconciled lanes.
COVERAGE_PAIR = ("sqlite", "chromium")


def run_all(jobs: str) -> None:
    """Every producer, then reconciliation of every backend/browser pair.

    `jobs` bounds concurrent producers; empty means the script's default of 3.
    """
    steps.run("e2e", ["bash", "scripts/e2e-workflow", "all", jobs])


def _selected_output(check: str) -> str:
    """The output this invocation's build selected, from its retained `build.json`.

    Never the GC root, which an overlapping invocation of the same lane could
    have repointed.
    """
    selected = json.loads(Path(DIAGNOSTICS, check, "build.json").read_text())
    output: str = selected[0]["outputs"]["out"]
    return output


def producer(lane: str) -> None:
    """One lane's producer, then its verification, whose report is rendered.

    The script builds the lane's check and lifts its artifacts, and the step
    records where that output came from; only a successful lift is verified.
    """
    check = f"e2e-{lane}"
    steps.run(
        "e2e-producer",
        ["bash", "scripts/e2e-workflow", "producer", lane],
        realizes=lambda: _selected_output(check),
    )
    output = _selected_output(check)
    steps.run(
        "e2e-evidence",
        [
            *steps.tool(E2E_EVIDENCE),
            "verify",
            "--lane",
            lane,
            "--catalog",
            CATALOG,
            "--artifacts",
            output,
        ],
        report.render,
    )


def _coverage(action: str, artifacts: str) -> None:
    steps.run(
        f"server-fn-coverage-{action}",
        [
            *steps.tool(E2E_EVIDENCE),
            "server-fn-coverage",
            action,
            "--catalog",
            CATALOG,
            "--diagnostics-root",
            artifacts,
        ],
        report.render,
    )


def regenerate_coverage() -> None:
    """The intentional snapshot update from freshly produced coverage lanes."""
    steps.run(
        "e2e-coverage-producers",
        ["bash", "scripts/e2e-workflow", "pair", *COVERAGE_PAIR],
    )
    _coverage("regenerate", DIAGNOSTICS)


def reconcile(backend: str, browser: str, artifacts: str) -> None:
    """Artifact-only reconciliation by this checkout's Nix-built verifier.

    The coverage pair's verdict also includes server-function flow coverage.
    """
    steps.run(
        "e2e-reconcile",
        [
            *steps.tool(E2E_EVIDENCE),
            "reconcile",
            backend,
            browser,
            "--catalog",
            CATALOG,
            "--diagnostics-root",
            artifacts,
        ],
        report.render,
    )
    if (backend, browser) == COVERAGE_PAIR:
        _coverage("verify", artifacts)
