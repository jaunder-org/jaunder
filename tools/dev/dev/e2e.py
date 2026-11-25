"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran. The verdicts themselves run here, in-process
(`e2e_evidence`, `server_fn_coverage`); none builds or runs a VM.
"""

import json
import sys
import time
from collections.abc import Callable
from pathlib import Path

from opentelemetry.trace import Span

from dev import e2e_evidence, report, server_fn_coverage, steps, trace
from dev.commands import SOURCE_CONTRACT
from dev.traces import EvidenceError

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


def _verdict(name: str, decide: Callable[[], str]) -> None:
    """Runs one in-process verdict as the step `name` and shows its result.

    A refusal fails the step with status 1, like a failed tool would.
    """
    started_ns = time.time_ns()
    with steps.phase(name):
        try:
            detail, ok = decide(), True
        except EvidenceError as error:
            detail, ok = str(error), False
        duration_ms = (time.time_ns() - started_ns) // 1_000_000
        step = {"name": name, "ok": ok, "duration_ms": duration_ms, "detail": detail}
        sys.stdout.write(report.render_report({"steps": [step]}))
        sys.stdout.flush()
        if not ok:
            raise steps.StepFailed(1, signalled=False)


def producer(lane: str) -> None:
    """One lane's producer, then its verification, whose result is shown.

    The script builds the lane's check and lifts its artifacts, and the step
    records where that output came from and the VM phases its run measured;
    only a successful lift is verified. The lane is a verdict that must execute
    here (ADR-0192), so an output this machine did not build fails the step.
    """
    check = f"e2e-{lane}"
    steps.run(
        "e2e-producer",
        ["bash", "scripts/e2e-workflow", "producer", lane],
        realizes=lambda: _selected_output(check),
        finished=_phase_spans(check, lane),
        verdict=True,
    )
    output = Path(_selected_output(check))
    _verdict(
        "e2e-evidence",
        lambda: e2e_evidence.verify_lane(lane, Path(CATALOG), output),
    )


def _coverage(regenerate: bool, artifacts: str) -> None:
    """Server-function flow coverage over the coverage pair's reconciled lanes.

    Reconciliation runs first and fails closed, so no capture of an
    unreconciled lane is ever read. The inventory comes from this checkout's
    `source-contracts`, run from the checkout root.
    """
    name = f"server-fn-coverage-{'regenerate' if regenerate else 'verify'}"

    def decide() -> str:
        try:
            _, lanes = e2e_evidence.reconciled_lanes(
                *COVERAGE_PAIR, Path(CATALOG), Path(artifacts)
            )
        except EvidenceError as error:
            raise EvidenceError(
                f"the SQLite/Chromium lanes must reconcile before flow coverage: {error}"
            ) from error
        inventory = server_fn_coverage.parse_inventory(
            steps.output(
                "server-fn-inventory",
                [*steps.tool(SOURCE_CONTRACT), "server-fn-inventory"],
            )
        )
        coverage = server_fn_coverage.coverage_from_captures(
            [lane.capture for lane in lanes], inventory
        )
        snapshot = Path(server_fn_coverage.SNAPSHOT_PATH)
        if regenerate:
            return server_fn_coverage.regenerate(coverage, snapshot)
        return server_fn_coverage.verify(coverage, snapshot)

    _verdict(name, decide)


def regenerate_coverage() -> None:
    """The intentional snapshot update from freshly produced coverage lanes."""
    steps.run(
        "e2e-coverage-producers",
        ["bash", "scripts/e2e-workflow", "pair", *COVERAGE_PAIR],
    )
    _coverage(True, DIAGNOSTICS)


def reconcile(backend: str, browser: str, artifacts: str) -> None:
    """Artifact-only reconciliation of one pair's lifted lanes.

    The coverage pair's verdict also includes server-function flow coverage.
    """
    _verdict(
        "e2e-reconcile",
        lambda: e2e_evidence.reconciled_lanes(
            backend, browser, Path(CATALOG), Path(artifacts)
        )[0],
    )
    if (backend, browser) == COVERAGE_PAIR:
        _coverage(False, artifacts)
