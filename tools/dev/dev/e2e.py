"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran.
"""

import json
import time
from collections.abc import Callable
from pathlib import Path

from opentelemetry.trace import Span, set_span_in_context

from dev import report, steps, trace
from dev.commands import E2E_EVIDENCE
from dev.spans import STATUS, Status

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


def _measured(phase: object) -> tuple[str, int, int, bool] | None:
    """A manifest phase's name, start, end and failure, if it was measured."""
    if not isinstance(phase, dict):
        return None
    name, start = phase.get("name"), phase.get("started_at_unix_ns")
    duration, outcome = phase.get("duration_ms"), phase.get("outcome")
    # bool is an int to Python, never to the manifest's writer.
    if (
        not isinstance(name, str)
        or type(start) is not int
        or type(duration) is not int
        or outcome not in ("success", "failed")
    ):
        return None
    return name, start, start + duration * 1_000_000, outcome == "failed"


def _phase_spans(check: str, lane: str) -> Callable[[Span, int], None]:
    """Adds the lane's VM phases, from its lifted manifest, under the step.

    Only phases this step's own run measured are shown: a reused output's
    manifest, or one from another run, falls outside the step. Anything
    unmeasured or unreadable is left out, never shown as zero, and never
    changes the step.
    """

    def add(step: Span, started_ns: int) -> None:
        exited_ns = time.time_ns()
        try:
            manifest = Path(DIAGNOSTICS, check, f"e2e-phase-{lane}.json").read_bytes()
            phases = json.loads(manifest).get("phases")
        except OSError, ValueError, AttributeError:
            return
        parent = set_span_in_context(step)
        for measured in map(_measured, phases if isinstance(phases, list) else []):
            if measured is None:
                continue
            name, start, end, failed = measured
            if started_ns <= start <= end <= exited_ns:
                span = trace.tracer().start_span(name, parent, start_time=start)
                span.set_attribute(STATUS, Status.FAILED if failed else Status.OK)
                span.end(end_time=end)

    return add


def producer(lane: str) -> None:
    """One lane's producer, then its verification, whose report is rendered.

    The script builds the lane's check and lifts its artifacts, and the step
    records where that output came from and the VM phases its run measured;
    only a successful lift is verified.
    """
    check = f"e2e-{lane}"
    steps.run(
        "e2e-producer",
        ["bash", "scripts/e2e-workflow", "producer", lane],
        realizes=lambda: _selected_output(check),
        finished=_phase_spans(check, lane),
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
