"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran.
"""

import json
import sys
from collections.abc import Callable
from pathlib import Path

from opentelemetry.trace import Span

from dev import report, steps, trace
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


def _measured(entry: object) -> trace.Phase | None:
    """A manifest phase entry as a `trace.Phase`, if the VM measured it."""
    if not isinstance(entry, dict):
        return None
    name, start = entry.get("name"), entry.get("started_at_unix_ns")
    duration, outcome = entry.get("duration_ms"), entry.get("outcome")
    # bool is an int to Python, never to the manifest's writer.
    if (
        not isinstance(name, str)
        or type(start) is not int
        or type(duration) is not int
        or outcome not in ("success", "failed")
    ):
        return None
    return trace.Phase(name, start, start + duration * 1_000_000, outcome == "success")


def _phase_spans(check: str, lane: str) -> Callable[[Span, int], None]:
    """Adds the lane's VM phases, from its lifted manifest, under the step.

    Only phases this step's own run measured are shown: a reused output's
    manifest, or one from another run, falls outside the step. Anything
    unmeasured or unreadable is left out, never shown as zero, and never
    changes the step. A failed lane may leave no manifest at all; any other
    read failure is reported.
    """

    def add(step: Span, started_ns: int) -> None:
        path = Path(DIAGNOSTICS, check, f"e2e-phase-{lane}.json")
        try:
            entries = json.loads(path.read_bytes()).get("phases")
        except FileNotFoundError:
            return
        except (OSError, ValueError, AttributeError, RecursionError) as error:
            print(f"dev: {path}: no phase spans: {error}", file=sys.stderr, flush=True)
            return
        measured = map(_measured, entries if isinstance(entries, list) else [])
        trace.record_phases(
            step, started_ns, [phase for phase in measured if phase is not None]
        )

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
