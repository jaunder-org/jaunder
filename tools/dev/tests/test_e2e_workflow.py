"""The E2E workflow through `dev`, against fake Nix.

`scripts/e2e-workflow` settles every producer, lifts only its own evidence and
reconciles every backend/browser pair; SQLite/Chromium also verifies coverage.
`dev` decides each verdict in-process. It needs the repository's script, lane
catalog and jq programs, a real Nix store for immutable outputs, and jq, so
only the filtered `checks.dev-python` source may skip it.
"""

import json
import re
import shutil
import subprocess
import tempfile
from dataclasses import dataclass, fields
from pathlib import Path
from typing import Any

import pytest
from conftest import Checkout, lift_lanes, nests, require, write_lane_capture
from opentelemetry.trace import INVALID_SPAN

from dev import trace, traces
from dev.e2e import _capture_spans, capture_tree
from dev.evidence import EvidenceError

BASH = shutil.which("bash") or "/bin/sh"
NIX = shutil.which("nix")
FAKE_NIX = f"""#!{BASH}
set -eu
if [[ $1 == path-info ]]; then
  # The selected output was built here long before this run: reused, unless
  # STORE says it was substituted or cannot be read.
  [[ ${{STORE-}} != unreadable ]] || exit 1
  ultimate=true; [[ ${{STORE-}} != substituted ]] || ultimate=false
  printf '{{"%s":{{"registrationTime":0,"ultimate":%s}}}}\\n' "${{!#}}" "$ultimate"
  exit 0
fi
if [[ $* == *".#repository-policy"* ]]; then
  echo "tool repository-policy" >> "$EVENTS"
  printf '%s\\n' "$TOOL_OUT"
  exit 0
fi
if [[ $1 == build ]]; then
  lane=${{!#}}; lane=${{lane##*.e2e-}}
  echo "producer $lane" >> "$EVENTS"
  echo "producer exit ${{BUILD_STATUS:-23}}" >&2
  if [[ -n ${{PHASES-}} ]]; then
    # A VM run during this step: its phase manifest, and any CAPTURE trace,
    # start now. "@NOW+<ms>@" is that many milliseconds later.
    fresh=$(mktemp -d)
    [[ -z ${{LANE_EVIDENCE-}} ]] || cp "$LANE_EVIDENCE"/* "$fresh/"
    now=$(date +%s%N)
    stamp() {{
      local text=$1 later='@NOW\\+([0-9]+)@'
      while [[ $text =~ $later ]]; do
        text=${{text//"${{BASH_REMATCH[0]}}"/$((now + BASH_REMATCH[1] * 1000000))}}
      done
      printf '%s' "${{text//@NOW@/$now}}"
    }}
    stamp "$PHASES" > "$fresh/e2e-phase-$lane.json"
    if [[ -n ${{CAPTURE-}} ]]; then
      mkdir -p "$fresh/vm/capture"
      stamp "$CAPTURE" > "$fresh/vm/capture/otel-traces.jsonl"
      tar -czf "$fresh/capture-$lane.tar.gz" -C "$fresh/vm" capture
      rm -r "$fresh/vm"
    fi
    OUTPUT=$("$REAL_NIX" store add-path --name e2e-phase-fixture "$fresh")
    rm -r "$fresh"
    echo "$OUTPUT" > "$EVENTS.output"
    echo "$now" > "$EVENTS.now"
  fi
  if [[ -n ${{PRODUCER_SECONDS-}} ]]; then
    sleep "$PRODUCER_SECONDS"
    echo "finished $lane" >> "$EVENTS"
  fi
  if [[ ${{BUILD_STATUS:-23}} == 0 ]]; then
    while [[ $1 != --out-link ]]; do shift; done
    ln -sfn "${{GCROOT_DECOY:-$OUTPUT}}" "$2"
    printf '[{{"outputs":{{"out":"%s"}}}}]\\n' "$OUTPUT"
  fi
  exit "${{BUILD_STATUS:-23}}"
fi
if [[ -f $EVENTS.output ]]; then OUTPUT=$(<"$EVENTS.output"); fi
[[ -n ${{OUTPUT-}} ]] || exit 1
printf '%s' "$OUTPUT"
"""
# This checkout's server-fn inventory: no fns, so the fixture lanes cover all.
FAKE_INVENTORY = f"""#!{BASH}
echo "inventory $*" >> "$EVENTS"
printf '[]\\n'
"""
LANE = "sqlite-chromium-1-of-3"
COVERAGE_LANES = 3
LANES = 16
PAIRS = ["sqlite chromium", "sqlite firefox", "postgres chromium", "postgres firefox"]
MARKER = f"producer-input-{LANE}.json"


@dataclass
class E2E:
    checkout: Checkout

    @property
    def root(self) -> Path:
        return self.checkout.root

    @property
    def diagnostics(self) -> Path:
        return self.root / ".xtask" / "diagnostics" / f"e2e-{LANE}"

    @property
    def capture(self) -> Path:
        return self.diagnostics / f"capture-{LANE}.tar.gz"

    def run(
        self, *args: str, output: Path | None = None, status: str, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        if output is not None:
            extra["OUTPUT"] = str(output)
        return self.checkout.run(
            self.checkout.command(*args),
            self.checkout.environment(
                EVENTS=str(self.root / "events"),
                REAL_NIX=str(NIX),
                BUILD_STATUS=status,
                TOOL_OUT=str(self.root.parent / "tool-out"),
                **extra,
            ),
        )

    def events(self) -> str:
        path = self.root / "events"
        return path.read_text() if path.exists() else ""

    def fail_copy(self, name: str) -> None:
        cp = shutil.which("cp")
        self.checkout.executable(
            "cp",
            f"""#!{BASH}
set -eu
if [[ ${{@: -2:1}} == */{name} ]]; then
  echo 'injected copy failure: {name}' >&2
  exit 37
fi
exec '{cp}' "$@"
""",
        )


@pytest.fixture
def e2e(checkout: Checkout) -> E2E:
    require(NIX is not None, "Nix")
    require(shutil.which("jq") is not None, "jq")
    checkout.from_repository(
        "scripts/e2e-workflow",
        "scripts/e2e-matrix.jq",
        "end2end/e2e-lanes.json",
    )
    checkout.executable("nix", FAKE_NIX)
    checkout.dev_on_path()
    inventory = checkout.root.parent / "tool-out" / "bin" / "source-contracts"
    inventory.parent.mkdir(parents=True)
    inventory.write_text(FAKE_INVENTORY)
    inventory.chmod(0o755)
    return E2E(checkout)


def immutable(artifacts: dict[str, bytes]) -> Path:
    """A store path holding `artifacts`, as a producer's output would be."""
    assert NIX is not None
    with tempfile.TemporaryDirectory() as directory:
        for name, content in artifacts.items():
            Path(directory, name).write_bytes(content)
        added = subprocess.run(
            [NIX, "store", "add-path", "--name", "e2e-workflow-fixture", directory],
            capture_output=True,
            text=True,
            check=True,
        )
    return Path(added.stdout.strip())


def capture_only() -> Path:
    return immutable({f"capture-{LANE}.tar.gz": b"current diagnostic"})


def passing_lanes() -> dict[str, bytes]:
    """Every SQLite/Chromium lane's passing evidence, lane-qualified, so one
    output can serve each lane's producer."""
    with tempfile.TemporaryDirectory() as directory:
        return {
            path.name: path.read_bytes()
            for lane in lift_lanes(Path(directory), "chromium")
            for path in (getattr(lane, field.name) for field in fields(lane)[1:])
        }


def verdicts(result: subprocess.CompletedProcess[bytes], mark: str, name: str) -> int:
    """How many `name` verdicts the run showed with `mark` (`[ ok ]`, `[FAIL]`)."""
    return result.stdout.decode().count(f"{mark} {name} ")


@pytest.mark.parametrize(
    ("jobs", "failure"),
    [
        ("", "23"),
        ("1", "23"),
        ("2", "255"),
        ("12", "23"),
        ("18446744073709551615", "23"),
    ],
)
def test_an_early_producer_failure_still_settles_every_lane_and_every_reconciliation(
    e2e: E2E, jobs: str, failure: str
) -> None:
    e2e.diagnostics.mkdir(parents=True)
    e2e.capture.write_text("stale")
    (e2e.root / ".xtask" / "not-owned-keep").write_text("keep")
    result = e2e.run("e2e", *([jobs] if jobs else []), status=failure)
    assert result.returncode != 0
    lines = e2e.events().splitlines()
    assert sum(line.startswith("producer ") for line in lines) == LANES, lines
    assert "producer postgres-firefox-5-of-5" in lines
    # Every pair is still reconciled, and fails without its lanes' evidence;
    # coverage never runs over unreconciled lanes.
    output = result.stdout.decode()
    assert verdicts(result, "[FAIL]", "e2e-reconcile") == len(PAIRS), output
    for pair in PAIRS:
        assert f"e2e-{pair.replace(' ', '-')}-1-of-" in output, pair
    assert "server-fn-coverage" not in output
    assert not e2e.capture.exists(), "a failed producer reused stale evidence"
    assert (e2e.root / ".xtask" / "not-owned-keep").read_text() == "keep"
    for lane in [LANE, "postgres-firefox-5-of-5"]:
        log = e2e.root / ".xtask" / "diagnostics" / f"e2e-{lane}" / "build.log"
        assert f"producer exit {failure}" in log.read_text()


def most_at_once(lines: list[str]) -> int:
    running = peak = 0
    for line in lines:
        running += line.startswith("producer ") - line.startswith("finished ")
        peak = max(peak, running)
    return peak


@pytest.mark.parametrize(("jobs", "capped"), [("", True), ("8", False)])
def test_local_e2e_runs_at_most_three_vms_unless_told_otherwise(
    e2e: E2E, jobs: str, capped: bool
) -> None:
    e2e.run("e2e", *([jobs] if jobs else []), status="23", PRODUCER_SECONDS="1")
    assert (most_at_once(e2e.events().splitlines()) <= 3) == capped


@pytest.mark.parametrize("identity", ["unknown", "sqlite-firefox-unsplit", "all"])
def test_a_producer_refuses_unknown_disabled_and_internal_names_without_building(
    e2e: E2E, identity: str
) -> None:
    assert e2e.run("e2e-producer", identity, status="23").returncode != 0
    assert e2e.events() == ""


def test_every_built_lane_is_verified_and_every_pair_reconciled(e2e: E2E) -> None:
    result = e2e.run("e2e", output=capture_only(), status="0")
    assert result.returncode != 0
    lines = e2e.events().splitlines()
    assert sum(line.startswith("producer ") for line in lines) == LANES
    # Each build's output lacks its evidence, so every lane fails verification
    # and every pair reconciliation; coverage never runs over unreconciled lanes.
    assert verdicts(result, "[FAIL]", "e2e-evidence") == LANES
    assert verdicts(result, "[FAIL]", "e2e-reconcile") == len(PAIRS)
    assert b"server-fn-coverage" not in result.stdout
    assert "tool repository-policy" not in lines


def test_a_successful_producer_keeps_its_root_and_lifts_the_selected_output(
    e2e: E2E,
) -> None:
    artifacts = passing_lanes()
    selected = immutable(artifacts)
    e2e.diagnostics.mkdir(parents=True)
    e2e.capture.write_text("stale")
    e2e.capture.chmod(0o444)
    result = e2e.run("e2e-producer", LANE, output=selected, status="0")
    assert result.returncode == 0, result
    assert e2e.capture.read_bytes() == artifacts[e2e.capture.name]
    assert (e2e.root / ".xtask" / "gcroots" / f"e2e-{LANE}").readlink() == selected
    assert result.stdout.decode().startswith("[ ok ] e2e-evidence")
    assert f"{LANE}: population, retry-duration and boot evidence verified\n" in (
        result.stdout.decode()
    ), "the verdict is shown"
    assert "server-fn-coverage" not in result.stdout.decode(), "never a producer's"


@pytest.mark.parametrize("status", ["0", "23"])
def test_regeneration_freshly_produces_the_coverage_lanes_and_needs_their_success(
    e2e: E2E, status: str
) -> None:
    output = immutable(passing_lanes())
    result = e2e.run("e2e-regenerate-coverage", output=output, status=status)
    assert (result.returncode == 0) == (status == "0"), result
    lines = e2e.events().splitlines()
    produced = [line for line in lines if line.startswith("producer ")]
    assert len(produced) == COVERAGE_LANES, lines
    assert all(line.startswith("producer sqlite-chromium-") for line in produced)
    snapshot = e2e.root / "docs" / "coverage" / "server-fns.json"
    regenerated = verdicts(result, "[ ok ]", "server-fn-coverage-regenerate")
    assert regenerated == (1 if status == "0" else 0), result.stdout
    assert snapshot.exists() == (status == "0")
    if status == "0":
        # The inventory's tool is built, then run, only once the lanes reconcile.
        assert lines[COVERAGE_LANES:] == [
            "tool repository-policy",
            "inventory server-fn-inventory",
        ]
        assert snapshot.read_text() == '{\n  "covered": [],\n  "orphans": {}\n}\n'
    assert b"server-fn-coverage-verify" not in result.stdout


def test_a_producer_records_its_output_cache_then_verifies_in_process(
    e2e: E2E,
) -> None:
    result = e2e.run(
        "e2e-producer", LANE, output=immutable(passing_lanes()), status="0"
    )
    assert result.returncode == 0, result
    spans = e2e.checkout.merged()
    producer = spans["e2e-producer"]["args"]
    assert producer["jaunder.cache"] == "reused", "an already valid output"
    assert (
        producer["process.command_line"] == f"bash scripts/e2e-workflow producer {LANE}"
    )
    assert spans["e2e-evidence"]["args"]["jaunder.status"] == "ok"
    assert spans["e2e-producer"]["ts"] < spans["e2e-evidence"]["ts"]
    assert "tool repository-policy" not in e2e.events(), "verification builds nothing"


def test_the_verifier_reads_this_builds_selected_output_never_the_gc_root(
    e2e: E2E,
) -> None:
    # The decoy holds no evidence, so only the selected output verifies.
    result = e2e.run(
        "e2e-producer",
        LANE,
        output=immutable(passing_lanes()),
        status="0",
        GCROOT_DECOY=str(e2e.root / "decoy"),
    )
    assert result.returncode == 0, result
    assert verdicts(result, "[ ok ]", "e2e-evidence") == 1


@pytest.mark.parametrize("store", ["substituted", "unreadable"])
def test_a_lane_not_built_here_fails_naming_it_and_is_not_verified(
    e2e: E2E, store: str
) -> None:
    result = e2e.run(
        "e2e-producer", LANE, output=capture_only(), status="0", STORE=store
    )
    assert result.returncode == 1, result
    assert b"dev: e2e-producer: verdict not executed here" in result.stderr
    spans = e2e.checkout.merged()
    assert spans["e2e-producer"]["args"]["jaunder.status"] == "failed"
    assert "e2e-evidence" not in spans


def test_a_failed_producer_records_no_cache_and_runs_no_verifier(e2e: E2E) -> None:
    result = e2e.run("e2e-producer", LANE, status="23")
    assert result.returncode == 23
    spans = e2e.checkout.merged()
    assert "jaunder.cache" not in spans["e2e-producer"]["args"]
    assert "e2e-evidence" not in spans


PHASE_MANIFEST = f"e2e-phase-{LANE}.json"


def phase_entry(
    name: str, outcome: str, duration_ms: object, start: int | str | None
) -> dict[str, object]:
    measured = {} if start is None else {"started_at_unix_ns": start}
    return {
        "name": name,
        **measured,
        "duration_ms": duration_ms,
        "outcome": outcome,
        "detail": "",
    }


def manifest(*entries: object) -> str:
    """A phase manifest; a start of "@NOW@" is the fake build's own clock."""
    text = json.dumps({"schema_version": 1, "lane": LANE, "phases": list(entries)})
    return text.replace('"@NOW@"', "@NOW@")


def phase_children(e2e: E2E) -> dict[str, dict[str, Any]]:
    """The producer step's child spans, keyed by name."""
    events = e2e.checkout.events()
    (producer,) = [event for event in events if event["name"] == "e2e-producer"]
    parent = producer["args"]["span_id"]
    return {
        event["name"]: event for event in events if event["args"]["parent_id"] == parent
    }


@pytest.mark.parametrize(("status", "gate"), [("0", "success"), ("23", "failed")])
def test_a_producer_shows_this_runs_measured_phases_as_its_children(
    e2e: E2E, status: str, gate: str
) -> None:
    phases = manifest(
        phase_entry("nix-evaluation", "unavailable", None, None),
        phase_entry("vm-startup-readiness", "success", 3, "@NOW@"),
        phase_entry("gate-execution", gate, 2, "@NOW@"),
        # Written before the start field existed: no place on the timeline.
        phase_entry("result-lift", "success", 1, None),
        # Never written by the driver; skipped, not guessed at.
        "not a phase",
        phase_entry("bool-duration", "success", True, "@NOW@"),
        phase_entry("negative-duration", "success", -1, "@NOW@"),
    )
    evidence = e2e.root.parent / "lane-evidence"
    evidence.mkdir()
    for name, content in passing_lanes().items():
        (evidence / name).write_bytes(content)
    result = e2e.run(
        "e2e-producer",
        LANE,
        status=status,
        PHASES=phases,
        LANE_EVIDENCE=str(evidence),
    )
    assert str(result.returncode) == status, result
    assert b"no phase spans" not in result.stderr
    children = phase_children(e2e)
    assert set(children) == {"vm-startup-readiness", "gate-execution"}
    now_us = int((e2e.root / "events.now").read_text()) // 1000
    for name, duration_us in [("vm-startup-readiness", 3000), ("gate-execution", 2000)]:
        assert (children[name]["ts"], children[name]["dur"]) == (now_us, duration_us)
    assert children["vm-startup-readiness"]["args"]["jaunder.status"] == "ok"
    expected = "ok" if gate == "success" else "failed"
    assert children["gate-execution"]["args"]["jaunder.status"] == expected
    step = e2e.checkout.merged()["e2e-producer"]
    assert step["args"]["jaunder.status"] == ("ok" if status == "0" else "failed")


@pytest.mark.parametrize("status", ["0", "23"])
@pytest.mark.parametrize(
    ("lifted", "warned"),
    [
        # A reused output's manifest describes an earlier run.
        (manifest(phase_entry("gate-execution", "success", 1, 1_000_000_000)), False),
        ("{", True),
        ("[]", True),
        ("[" * 100_000, True),
        # A failed or timed-out lane may leave no manifest (#2020).
        (None, False),
    ],
    ids=["another-run", "malformed", "not-an-object", "too-deep", "missing"],
)
def test_a_producer_without_this_runs_phases_shows_no_children(
    e2e: E2E, lifted: str | None, warned: bool, status: str
) -> None:
    artifacts = passing_lanes()
    artifacts.pop(PHASE_MANIFEST)
    if lifted is not None:
        artifacts[PHASE_MANIFEST] = lifted.encode()
    result = e2e.run("e2e-producer", LANE, output=immutable(artifacts), status=status)
    assert str(result.returncode) == status, result
    assert (b"no phase spans" in result.stderr) == warned, result.stderr
    assert phase_children(e2e) == {}
    step = e2e.checkout.merged()["e2e-producer"]
    assert step["args"]["jaunder.status"] == ("ok" if status == "0" else "failed")


INJECTED = "2b2b2b2b2b2b2b2b"
# A gate lasting 50 ms from the fake build's clock, for capture spans to fit in.
GATE_PHASES = manifest(phase_entry("gate-execution", "success", 50, "@NOW@"))


def otlp(
    name: str,
    span_id: str,
    parent: str,
    start_ms: int,
    end_ms: int,
    *attributes: dict[str, Any],
    status: object = None,
) -> dict[str, Any]:
    """One OTLP span, timed in milliseconds after the fake build's clock."""
    return {
        "traceId": "2b" * 16,
        "spanId": span_id,
        "parentSpanId": parent,
        "name": name,
        "startTimeUnixNano": f"@NOW+{start_ms}@",
        "endTimeUnixNano": f"@NOW+{end_ms}@",
        "attributes": list(attributes),
        **({} if status is None else {"status": {"code": status}}),
    }


def otlp_capture(*spans: dict[str, Any]) -> str:
    """OTLP JSONL as the collector writes it, one span per line."""
    return "".join(
        json.dumps({"resourceSpans": [{"scopeSpans": [{"spans": [span]}]}]}) + "\n"
        for span in spans
    )


# Two overlapping tests, one with a failed request and its storage span, plus
# a request made before any test owned it, a background root and an orphan.
TEST_TREE = otlp_capture(
    otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 40),
    otlp("e2e.test", "t1", "l1", 6, 30),
    otlp(
        "request",
        "r1",
        "t1",
        10,
        20,
        {"key": "method", "value": {"stringValue": "GET"}},
        {"key": "status", "value": {"intValue": "500"}},
        {"key": "busy_ns", "value": {"intValue": 7}},
        {"key": "cached", "value": {"boolValue": False}},
        {"key": "ratio", "value": {"doubleValue": 0.5}},
        {"key": "tags", "value": {"arrayValue": {"values": []}}},
        status=2,
    ),
    otlp("storage.posts.get", "s1", "r1", 11, 12),
    otlp("e2e.test.lifecycle", "l2", INJECTED, 10, 35),
    otlp("unattributed", "p1", INJECTED, 8, 9, status=1),
    otlp("storage.write_scope", "b1", "", 1, 2),
    otlp("lost-parent", "o1", "gone", 12, 13),
)


def capture_events(e2e: E2E) -> dict[str, dict[str, Any]]:
    """The merged slices of the gate's test tree, keyed by span name."""
    events = e2e.checkout.events()
    by_id = {event["args"]["span_id"]: event for event in events}
    (gate,) = [event for event in events if event["name"] == "gate-execution"]

    def under_gate(event: dict[str, Any]) -> bool:
        parent = by_id.get(event["args"]["parent_id"])
        return parent is not None and (parent is gate or under_gate(parent))

    return {event["name"]: event for event in events if under_gate(event)}


@pytest.mark.parametrize("status", ["0", "23"])
def test_a_lanes_test_tree_is_shown_under_its_gate_execution_phase(
    e2e: E2E, status: str
) -> None:
    result = e2e.run(
        "e2e-producer",
        LANE,
        status=status,
        PHASES=GATE_PHASES,
        CAPTURE=TEST_TREE,
        PRODUCER_SECONDS="0.1",
    )
    # A built lane goes on to e2e-evidence, which this lane's fixture fails.
    assert result.returncode == (1 if status == "0" else 23), result
    assert b"no capture spans" not in result.stderr, result.stderr
    tree = capture_events(e2e)
    assert set(tree) == {
        "e2e.test.lifecycle",
        "e2e.test",
        "request",
        "storage.posts.get",
        "unattributed",
    }
    lifecycles = [
        event
        for event in e2e.checkout.events()
        if event["name"] == "e2e.test.lifecycle"
    ]
    (gate,) = [e for e in e2e.checkout.events() if e["name"] == "gate-execution"]
    assert len(lifecycles) == 2
    for top in [*lifecycles, tree["unattributed"]]:
        assert top["args"]["parent_id"] == gate["args"]["span_id"]
    assert tree["e2e.test"]["args"]["parent_id"] in {
        event["args"]["span_id"] for event in lifecycles
    }
    assert tree["request"]["args"]["parent_id"] == tree["e2e.test"]["args"]["span_id"]
    child = tree["storage.posts.get"]["args"]["parent_id"]
    assert child == tree["request"]["args"]["span_id"]
    now_us = int((e2e.root / "events.now").read_text()) // 1000
    assert (tree["request"]["ts"], tree["request"]["dur"]) == (now_us + 10_000, 10_000)
    request = tree["request"]["args"]
    assert {key: request[key] for key in ["method", "status", "busy_ns"]} == {
        "method": "GET",
        "status": 500,
        "busy_ns": 7,
    }
    assert (request["cached"], request["ratio"]) == (False, 0.5)
    assert "tags" not in request
    assert request["otel.status_code"] == "ERROR"
    assert tree["unattributed"]["args"]["otel.status_code"] == "OK"
    assert "otel.status_code" not in tree["e2e.test"]["args"]
    assert {event["args"]["jaunder.status"] for event in tree.values()} == {""}
    # The overlapping tests share no track they would partly overlap on.
    assert nests(e2e.checkout.events())
    step = e2e.checkout.merged()["e2e-producer"]
    assert step["args"]["jaunder.status"] == ("ok" if status == "0" else "failed")


@pytest.mark.parametrize(
    ("phases", "capture", "warned"),
    [
        # A reused output's gate ran before this step: nothing to place under.
        (
            manifest(phase_entry("gate-execution", "success", 50, 1_000_000_000)),
            TEST_TREE,
            False,
        ),
        # A failed lane may lift no capture.
        (GATE_PHASES, None, False),
        (GATE_PHASES, "not json\n", True),
        # Lifecycle spans naming different parents: no one injected parent.
        (
            GATE_PHASES,
            otlp_capture(
                otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 10),
                otlp("e2e.test.lifecycle", "l2", "other", 12, 20),
            ),
            True,
        ),
        # A tree span without a start time.
        (GATE_PHASES, TEST_TREE.replace('"@NOW+11@"', "null"), True),
        # The VM's clock put a test past the end of the gate.
        (
            GATE_PHASES,
            otlp_capture(otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 500)),
            True,
        ),
    ],
    ids=["reused", "no-capture", "malformed", "no-single-parent", "untimed", "skewed"],
)
def test_a_capture_that_cannot_be_placed_shows_no_spans(
    e2e: E2E, phases: str, capture: str | None, warned: bool
) -> None:
    result = e2e.run(
        "e2e-producer",
        LANE,
        status="23",
        PHASES=phases,
        # An empty CAPTURE lifts none.
        CAPTURE=capture or "",
        PRODUCER_SECONDS="0.1",
    )
    assert result.returncode == 23, result
    assert (b"no capture spans" in result.stderr) == warned, result.stderr
    names = {event["name"] for event in e2e.checkout.events()}
    assert not names & {"e2e.test.lifecycle", "e2e.test", "request"}
    assert e2e.checkout.merged()["e2e-producer"]["args"]["jaunder.status"] == "failed"


# The same captures in-process, without Nix: what `checks.dev-python` runs.
GATE_START_NS = 1_795_000_000_000_000_000
IN_GATE = trace.Phase("gate-execution", GATE_START_NS, GATE_START_NS + 50_000_000, True)


def stamped(capture: str) -> str:
    """`capture` with each "@NOW+<ms>@" that many milliseconds into the gate."""
    return re.sub(
        r"@NOW\+([0-9]+)@",
        lambda found: str(GATE_START_NS + int(found[1]) * 1_000_000),
        capture,
    )


def parsed(capture: str) -> list[traces.Span]:
    return traces.parse_spans(stamped(capture), traces.Filters(), "capture")


def flattened(tree: list[trace.Imported]) -> dict[str, trace.Imported]:
    found: dict[str, trace.Imported] = {}
    for span in tree:
        found[span.name] = span
        found.update(flattened(list(span.children)))
    return found


def test_a_capture_tree_is_everything_below_the_injected_parent() -> None:
    tree = capture_tree(parsed(TEST_TREE))
    assert [span.name for span in tree] == [
        "e2e.test.lifecycle",
        "e2e.test.lifecycle",
        "unattributed",
    ]
    spans = flattened(tree)
    assert set(spans) == {
        "e2e.test.lifecycle",
        "e2e.test",
        "request",
        "storage.posts.get",
        "unattributed",
    }
    request = spans["request"]
    assert [child.name for child in request.children] == ["storage.posts.get"]
    assert (request.start_ns, request.end_ns) == (
        GATE_START_NS + 10_000_000,
        GATE_START_NS + 20_000_000,
    )
    assert request.attributes["otel.status_code"] == "ERROR"
    assert request.attributes["status"] == 500
    assert "otel.status_code" not in spans["e2e.test"].attributes


@pytest.mark.parametrize(
    ("spans", "error"),
    [
        (
            [
                otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 10),
                otlp("e2e.test.lifecycle", "l2", "other", 12, 20),
            ],
            "no single injected parent",
        ),
        ([otlp("e2e.test.lifecycle", "l1", "", 5, 10)], "no single injected parent"),
        (
            [
                otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 10),
                otlp("injected", INJECTED, "", 1, 20),
            ],
            "no single injected parent",
        ),
        ([otlp("storage.write_scope", "b1", "", 1, 2)], "no single injected parent"),
        (
            [
                otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 10),
                otlp("e2e.test", "t1", "l1", 6, 9),
                otlp("e2e.test", "t1", "l1", 6, 9),
            ],
            "repeats in its tree",
        ),
        (
            [
                otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 10),
                {**otlp("e2e.test", "t1", "l1", 6, 9), "endTimeUnixNano": None},
            ],
            "has no start or end time",
        ),
    ],
    ids=[
        "two-parents",
        "no-parent",
        "parent-captured",
        "no-tests",
        "repeat",
        "untimed",
    ],
)
def test_a_malformed_capture_tree_is_an_error(
    spans: list[dict[str, Any]], error: str
) -> None:
    with pytest.raises(EvidenceError, match=error):
        capture_tree(parsed(otlp_capture(*spans)))


@pytest.mark.parametrize(
    ("capture", "written", "warning"),
    [
        (None, False, ""),
        ("not json\n", False, "no capture spans: failed to parse"),
        (
            otlp_capture(otlp("e2e.test.lifecycle", "l1", INJECTED, 5, 500)),
            False,
            "no capture spans: 1 of 1 top-level spans fall outside gate-execution",
        ),
        (TEST_TREE, True, ""),
    ],
    ids=["missing", "malformed", "skewed", "placed"],
)
def test_a_capture_is_placed_under_its_gate_whole_or_not_at_all(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    capture: str | None,
    written: bool,
    warning: str,
) -> None:
    path = tmp_path / f"capture-{LANE}.tar.gz"
    if capture is not None:
        write_lane_capture(path, stamped(capture))
    recorded: list[tuple[object, list[trace.Imported]]] = []

    def record(parent: object, spans: list[trace.Imported]) -> list[object]:
        recorded.append((parent, list(spans)))
        return []

    monkeypatch.setattr(trace, "record_spans", record)
    _capture_spans(path, IN_GATE, INVALID_SPAN)
    assert [parent for parent, _ in recorded] == ([INVALID_SPAN] if written else [])
    if written:
        assert len(recorded[0][1]) == 3
    stderr = capsys.readouterr().err
    assert (warning in stderr) if warning else stderr == ""


def test_the_script_takes_no_coverage_action(e2e: E2E) -> None:
    argv = ["bash", "scripts/e2e-workflow", "producer", LANE, "regenerate"]
    result = e2e.checkout.run(argv, e2e.checkout.environment())
    assert result.returncode == 2
    assert e2e.events() == ""


def test_an_optional_provenance_copy_failure_only_warns(e2e: E2E) -> None:
    artifacts = passing_lanes()
    selected = immutable({**artifacts, MARKER: b"attribution only"})
    e2e.fail_copy(MARKER)
    result = e2e.run("e2e-producer", LANE, output=selected, status="0")
    assert result.returncode == 0, result
    assert b"warning: optional attribution" in result.stderr
    assert MARKER.encode() in result.stderr
    assert e2e.capture.read_bytes() == artifacts[e2e.capture.name]
    assert verdicts(result, "[ ok ]", "e2e-evidence") == 1
    assert (selected / MARKER).is_file()


REQUIRED = [
    f"capture-{LANE}.tar.gz",
    f"playwright-report-{LANE}.json",
    f"duration-budget-manifest-{LANE}.json",
]


@pytest.mark.parametrize(
    ("status", "artifact"),
    [("0", name) for name in REQUIRED] + [("23", MARKER), ("23", REQUIRED[0])],
)
def test_a_required_copy_failure_blocks_success_but_keeps_a_producer_failure(
    e2e: E2E, status: str, artifact: str
) -> None:
    selected = immutable(
        {
            REQUIRED[0]: b"current diagnostic",
            REQUIRED[1]: b"report",
            REQUIRED[2]: b"duration",
            MARKER: b"attribution only",
        }
    )
    e2e.fail_copy(artifact)
    result = e2e.run("e2e-producer", LANE, output=selected, status=status)
    assert result.returncode != 0, "a failed copy became green"
    if status == "23":
        assert result.returncode == 23, "the producer's own status was replaced"
    assert b"e2e-evidence" not in result.stdout
    assert f"injected copy failure: {artifact}".encode() in result.stderr
    retained = REQUIRED[1] if artifact == REQUIRED[0] else REQUIRED[0]
    assert (e2e.diagnostics / retained).is_file()


def test_a_failed_producer_lifts_available_evidence_but_never_verifies_it(
    e2e: E2E,
) -> None:
    result = e2e.run("e2e-producer", LANE, output=capture_only(), status="23")
    assert result.returncode == 23
    assert e2e.capture.read_text() == "current diagnostic"
    assert b"warning" not in result.stderr, "a successful lift was reported missing"
    assert b"e2e-evidence" not in result.stdout


@pytest.mark.parametrize("output", ["unevaluable", "missing", "empty"])
def test_a_failed_producer_that_lifts_nothing_says_so_and_keeps_its_status(
    e2e: E2E, output: str
) -> None:
    selected = {
        "unevaluable": None,
        "missing": e2e.root / "missing-output",
        "empty": immutable({}),
    }[output]
    result = e2e.run("e2e-producer", LANE, output=selected, status="23")
    assert result.returncode == 23, "the producer failure was replaced"
    warning = f"no lane artifacts were lifted from the failed build of e2e-{LANE}"
    assert warning.encode() in result.stderr
    assert b"unavailable or ambiguous" not in result.stderr
    assert b"e2e-evidence" not in result.stdout


def test_missing_successful_evidence_fails_lane_verification(e2e: E2E) -> None:
    result = e2e.run("e2e-producer", LANE, output=capture_only(), status="0")
    assert result.returncode == 1
    assert verdicts(result, "[FAIL]", "e2e-evidence") == 1
    assert b"reading report" in result.stdout


def test_regeneration_never_reaches_unverified_lanes(e2e: E2E) -> None:
    # Lane verification rejects the evidence-less producers, so the pair fails
    # before regeneration, which would itself refuse unreconciled lanes.
    result = e2e.run("e2e-regenerate-coverage", output=capture_only(), status="0")
    assert result.returncode != 0
    assert verdicts(result, "[FAIL]", "e2e-evidence") == COVERAGE_LANES
    assert b"server-fn-coverage" not in result.stdout
    assert "tool repository-policy" not in e2e.events()
    assert not (e2e.root / "docs" / "coverage" / "server-fns.json").exists()
