"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran. The verdicts themselves run here, in-process
(`e2e_evidence`, `server_fn_coverage`); none builds or runs a VM.
"""

import json
import sys
from collections.abc import Callable
from pathlib import Path

from opentelemetry.trace import Span

from dev import e2e_evidence, server_fn_coverage, steps, trace
from dev.commands import SOURCE_CONTRACT
from dev.evidence import CATALOG, EvidenceError
from dev.server_fn_coverage import Coverage

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
    steps.verdict(
        "e2e-evidence",
        lambda: e2e_evidence.verify_lane(lane, Path(CATALOG), output),
    )


def _inventory() -> list[server_fn_coverage.ServerFn]:
    """This checkout's server fns, from its own `source-contracts`, run from
    the checkout root. A listing that cannot be had refuses the coverage
    verdict; only a termination request still ends the gate."""
    try:
        listing = steps.output(
            "server-fn-inventory", [*steps.tool(SOURCE_CONTRACT), "server-fn-inventory"]
        )
    except steps.StepFailed as failure:
        if failure.signalled:
            raise
        raise EvidenceError(
            f"listing the server-fn inventory failed with status {failure.code}"
        ) from failure
    return server_fn_coverage.parse_inventory(listing)


def _coverage(
    name: str,
    lanes: Callable[[], list[e2e_evidence.LaneEvidence]],
    judge: Callable[[Coverage, Path], str],
) -> None:
    """Server-function flow coverage over the coverage pair's reconciled
    `lanes`, which `judge` holds against the committed snapshot.

    The lanes are reconciled before any capture is read, so no capture of an
    unreconciled lane ever is.
    """

    def decide() -> str:
        captures = [lane.capture for lane in lanes()]
        coverage = server_fn_coverage.coverage_from_captures(captures, _inventory())
        return judge(coverage, Path(server_fn_coverage.SNAPSHOT_PATH))

    steps.verdict(name, decide)


def _reconciled_coverage_lanes() -> list[e2e_evidence.LaneEvidence]:
    """The freshly produced coverage pair's lanes, reconciled."""
    try:
        return e2e_evidence.reconciled_lanes(
            *COVERAGE_PAIR, Path(CATALOG), Path(DIAGNOSTICS)
        )[1]
    except EvidenceError as error:
        raise EvidenceError(
            f"the SQLite/Chromium lanes must reconcile before flow coverage: {error}"
        ) from error


def regenerate_coverage() -> None:
    """The intentional snapshot update from freshly produced coverage lanes."""
    steps.run(
        "e2e-coverage-producers",
        ["bash", "scripts/e2e-workflow", "pair", *COVERAGE_PAIR],
    )
    _coverage(
        "server-fn-coverage-regenerate",
        _reconciled_coverage_lanes,
        server_fn_coverage.regenerate,
    )


def reconcile(backend: str, browser: str, artifacts: str) -> None:
    """Artifact-only reconciliation of one pair's lifted lanes.

    The coverage pair's verdict also includes server-function flow coverage,
    over the lanes this reconciliation verified.
    """
    lanes: list[e2e_evidence.LaneEvidence] = []

    def decide() -> str:
        verdict, reconciled = e2e_evidence.reconciled_lanes(
            backend, browser, Path(CATALOG), Path(artifacts)
        )
        lanes.extend(reconciled)
        return verdict

    steps.verdict("e2e-reconcile", decide)
    if (backend, browser) == COVERAGE_PAIR:
        _coverage("server-fn-coverage-verify", lambda: lanes, server_fn_coverage.verify)
