"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran. The verdicts themselves run here, in-process
(`e2e_evidence`, `server_fn_coverage`); none builds or runs a VM.
"""

import json
import os
import sys
from collections import defaultdict
from collections.abc import Callable
from pathlib import Path

from opentelemetry.trace import Span

from dev import annotations, e2e_evidence, server_fn_coverage, steps, trace, traces
from dev.commands import SOURCE_CONTRACT
from dev.e2e_evidence import GATE_PHASE
from dev.evidence import CATALOG, EvidenceError
from dev.server_fn_coverage import Coverage
from dev.traces import LIFECYCLE_SPAN, OTEL_STATUS_CODE

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


def capture_tree(spans: list[traces.Span]) -> list[trace.Imported]:
    """The lane's test tree: every capture span below the parent span the VM
    driver injects into Playwright, which the capture itself never holds.

    The host never sees that parent, so it is the one every
    `e2e.test.lifecycle` span names. Background spans no test owns, and spans
    whose parent was lost, are not in the tree. A capture without exactly one
    such parent, or with a tree span lacking times, is malformed.
    """
    parents = {span.parent_span_id for span in spans if span.name == LIFECYCLE_SPAN}
    known = {span.span_id for span in spans}
    if len(parents) != 1 or "" in parents or parents & known:
        raise EvidenceError(
            f"capture has no single injected parent for its {LIFECYCLE_SPAN} spans"
        )
    children: dict[str, list[traces.Span]] = defaultdict(list)
    for span in spans:
        children[span.parent_span_id].append(span)
    seen: set[str] = set()

    def imported(span: traces.Span) -> trace.Imported:
        if span.span_id in seen:
            raise EvidenceError(f"capture span id {span.span_id} repeats in its tree")
        seen.add(span.span_id)
        if span.start_unix_nano is None or span.end_unix_nano is None:
            raise EvidenceError(f"capture span {span.name} has no start or end time")
        attributes = traces.attributes(span.raw)
        code = traces.status_code(span.raw)
        if code is not None:
            attributes[OTEL_STATUS_CODE] = code
        return trace.Imported(
            span.name,
            span.start_unix_nano,
            span.end_unix_nano,
            attributes,
            [imported(child) for child in children[span.span_id]],
        )

    return [imported(span) for span in children[parents.pop()]]


def _capture_spans(capture: Path, gate: trace.Phase, span: Span) -> None:
    """Adds the lane's test tree, from its lifted capture, under the
    `gate-execution` span this run wrote.

    The capture is on the VM's clock and the phase on the host's, so a tree
    whose top-level spans do not all fit inside the phase is left out whole
    rather than misplaced. A failed lane may lift no capture; any other read
    failure is reported, and none changes the step.
    """
    if not capture.exists():
        return
    try:
        content = traces.read_trace_member(capture)
        tree = capture_tree(traces.parse_spans(content, traces.Filters(), str(capture)))
    except (EvidenceError, RecursionError) as error:
        print(f"dev: {capture}: no capture spans: {error}", file=sys.stderr, flush=True)
        return
    outside = [
        top
        for top in tree
        if not gate.start_ns <= top.start_ns <= top.end_ns <= gate.end_ns
    ]
    if outside:
        print(
            f"dev: {capture}: no capture spans: {len(outside)} of {len(tree)}"
            f" top-level spans fall outside {GATE_PHASE}",
            file=sys.stderr,
            flush=True,
        )
        return
    trace.record_spans(span, tree)


def _phase_spans(check: str, lane: str) -> Callable[[Span, int], None]:
    """Adds the lane's VM phases, from its lifted manifest, under the step,
    and its capture's test tree under the `gate-execution` phase.

    Only phases this step's own run measured are shown: a reused output's
    manifest, or one from another run, falls outside the step. Anything
    unmeasured or unreadable is left out, never shown as zero, and never
    changes the step. A failed lane may leave no manifest at all; any other
    read failure is reported.
    """
    lifted = e2e_evidence.lane_evidence(lane, Path(DIAGNOSTICS, check))

    def add(step: Span, started_ns: int) -> None:
        try:
            entries = json.loads(lifted.phase.read_bytes()).get("phases")
        except FileNotFoundError:
            return
        except (OSError, ValueError, AttributeError, RecursionError) as error:
            print(
                f"dev: {lifted.phase}: no phase spans: {error}",
                file=sys.stderr,
                flush=True,
            )
            return
        measured = map(_measured, entries if isinstance(entries, list) else [])
        written = trace.record_phases(
            step, started_ns, [phase for phase in measured if phase is not None]
        )
        for phase, span in written:
            if phase.name == GATE_PHASE:
                _capture_spans(lifted.capture, phase, span)

    return add


def _report_pressure(lane: str, budget: e2e_evidence.DurationBudget) -> None:
    """Under GitHub Actions, the lane's duration pressure as one warning
    annotation and its every attempt in the step summary, so a green run still
    shows it. One annotation per lane: GitHub drops a step's annotations past
    ten. A summary that cannot be written is said, never a failure."""
    if not budget.pressured or not annotations.in_github_actions():
        return
    title = annotations.DURATION_PRESSURE
    worst = budget.pressured[0].describe()
    annotations.warning(title, f"{lane}: {budget.headline()}; worst {worst}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary:
        return
    lines = "".join(f"- {pressure.describe()}\n" for pressure in budget.pressured)
    try:
        with open(summary, "a", encoding="utf-8") as out:
            out.write(f"### {title}: {lane}\n\n{lines}\n")
    except OSError as error:
        print(f"dev: {summary}: no pressure summary: {error}", file=sys.stderr)


def producer(lane: str) -> None:
    """One lane's producer, then its verification, whose result is shown.

    The script builds the lane's check and lifts its artifacts, and the step
    records where that output came from and the VM phases its run measured;
    only a successful lift is verified. The lane is a verdict that must execute
    here (ADR-0192), so an output this machine did not build fails the step.
    A verified lane's duration pressure is reported, never refused.
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

    def decide() -> str:
        verdict, budget = e2e_evidence.verify_lane(lane, Path(CATALOG), output)
        _report_pressure(lane, budget)
        return verdict

    steps.verdict("e2e-evidence", decide)


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
