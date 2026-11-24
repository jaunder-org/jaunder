"""The E2E workflow through `dev`, against fake Nix and a fake verifier.

`scripts/e2e-workflow` settles every producer, lifts only its own evidence and
reconciles every backend/browser pair; SQLite/Chromium also verifies coverage. It needs the repository's script, lane catalog and
jq programs, a real Nix store for immutable outputs, and jq, so only the
filtered `checks.dev-python` source may skip it.
"""

import json
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from conftest import REPOSITORY, Checkout, require

BASH = shutil.which("bash") or "/bin/sh"
NIX = shutil.which("nix")
FAKE_NIX = f"""#!{BASH}
set -eu
if [[ $1 == path-info ]]; then
  # The selected output was registered long before this run: reused.
  printf '{{"%s":{{"registrationTime":0,"ultimate":true}}}}\\n' "${{!#}}"
  exit 0
fi
if [[ $* == *".#host-proof"* ]]; then
  printf '%s\\n' "$EVIDENCE_OUT"
  exit 0
fi
if [[ $1 == build ]]; then
  lane=${{!#}}; lane=${{lane##*.e2e-}}
  echo "producer $lane" >> "$EVENTS"
  echo "producer exit ${{BUILD_STATUS:-23}}" >&2
  if [[ -n ${{PHASES-}} ]]; then
    # A VM run during this step: its phase manifest starts now.
    fresh=$(mktemp -d)
    now=$(date +%s%N)
    printf '%s' "${{PHASES//@NOW@/$now}}" > "$fresh/e2e-phase-$lane.json"
    OUTPUT=$("$REAL_NIX" store add-path --name e2e-phase-fixture "$fresh")
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
FAKE_VERIFIER = f"""#!{BASH}
set -eu
echo "verifier $*" >> "$EVENTS"
if [[ -e $EVENTS.fail-reconcile && $1 == reconcile ]]; then exit 31; fi
if [[ -n ${{REAL_VERIFIER-}} ]]; then exec "$REAL_VERIFIER" "$@"; fi
printf '{{"steps":[{{"name":"%s","ok":true,"duration_ms":1}}]}}\\n' "$1"
"""
LANE = "sqlite-chromium-1-of-3"
COVERAGE_LANES = 3
LANES = 12
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
                EVIDENCE_OUT=str(self.root.parent / "evidence-out"),
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
    verifier = checkout.root.parent / "evidence-out" / "bin" / "e2e-evidence"
    verifier.parent.mkdir(parents=True)
    verifier.write_text(FAKE_VERIFIER)
    verifier.chmod(0o755)
    return E2E(checkout)


def immutable(artifacts: dict[str, str]) -> Path:
    """A store path holding `artifacts`, as a producer's output would be."""
    assert NIX is not None
    with tempfile.TemporaryDirectory() as directory:
        for name, text in artifacts.items():
            Path(directory, name).write_text(text)
        added = subprocess.run(
            [NIX, "store", "add-path", "--name", "e2e-workflow-fixture", directory],
            capture_output=True,
            text=True,
            check=True,
        )
    return Path(added.stdout.strip())


def capture_only() -> Path:
    return immutable({f"capture-{LANE}.tar.gz": "current diagnostic"})


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
    assert "producer postgres-firefox-3-of-3" in lines
    for pair in PAIRS:
        assert any(f"reconcile {pair}" in line for line in lines[LANES:]), pair
    # Only the SQLite/Chromium reconciliation goes on to coverage, after it.
    coverage = [
        i for i, line in enumerate(lines) if "server-fn-coverage verify" in line
    ]
    reconcile = lines.index(next(l for l in lines if "reconcile sqlite chromium" in l))
    assert len(coverage) == 1 and coverage[0] == reconcile + 1, lines
    assert not e2e.capture.exists(), "a failed producer reused stale evidence"
    assert (e2e.root / ".xtask" / "not-owned-keep").read_text() == "keep"
    for lane in [LANE, "postgres-firefox-3-of-3"]:
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


def test_a_reconciliation_failure_blocks_even_when_every_producer_succeeds(
    e2e: E2E,
) -> None:
    (e2e.root / "events.fail-reconcile").write_text("")
    assert e2e.run("e2e", output=capture_only(), status="0").returncode != 0
    lines = e2e.events().splitlines()
    assert sum(line.startswith("producer ") for line in lines) == LANES
    assert sum("verifier reconcile" in line for line in lines) == len(PAIRS)
    # Coverage never runs over unreconciled lanes.
    assert "server-fn-coverage" not in e2e.events()
    assert sum("verifier verify" in line for line in lines) == LANES


def test_a_successful_producer_keeps_its_root_and_lifts_the_selected_output(
    e2e: E2E,
) -> None:
    selected = capture_only()
    e2e.diagnostics.mkdir(parents=True)
    e2e.capture.write_text("stale")
    e2e.capture.chmod(0o444)
    result = e2e.run("e2e-producer", LANE, output=selected, status="0")
    assert result.returncode == 0, result
    assert e2e.capture.read_text() == "current diagnostic"
    assert (e2e.root / ".xtask" / "gcroots" / f"e2e-{LANE}").readlink() == selected
    assert f"verify --lane {LANE}" in e2e.events()
    assert f"--artifacts {selected}" in e2e.events()
    assert "server-fn-coverage" not in e2e.events(), "producers never own coverage"
    assert result.stdout == b"[ ok ] verify\n", "the report is rendered"


@pytest.mark.parametrize("status", ["0", "23"])
def test_regeneration_freshly_produces_the_coverage_lanes_and_needs_their_success(
    e2e: E2E, status: str
) -> None:
    result = e2e.run("e2e-regenerate-coverage", output=capture_only(), status=status)
    assert (result.returncode == 0) == (status == "0"), result
    lines = e2e.events().splitlines()
    produced = [line for line in lines if line.startswith("producer ")]
    assert len(produced) == COVERAGE_LANES, lines
    assert all(line.startswith("producer sqlite-chromium-") for line in produced)
    regenerated = sum("server-fn-coverage regenerate" in line for line in lines)
    assert regenerated == (1 if status == "0" else 0), lines
    assert "server-fn-coverage verify" not in e2e.events()


def test_a_producer_records_its_output_cache_then_builds_and_runs_the_verifier(
    e2e: E2E,
) -> None:
    result = e2e.run("e2e-producer", LANE, output=capture_only(), status="0")
    assert result.returncode == 0, result
    spans = e2e.checkout.merged()
    producer = spans["e2e-producer"]["args"]
    assert producer["jaunder.cache"] == "reused", "an already valid output"
    assert (
        producer["process.command_line"] == f"bash scripts/e2e-workflow producer {LANE}"
    )
    assert spans["host-proof-build"]["args"]["jaunder.status"] == "ok"
    evidence = spans["e2e-evidence"]["args"]["process.command_line"]
    assert evidence.startswith(f"{e2e.root.parent}/evidence-out/bin/e2e-evidence ")
    starts = [
        spans[name]["ts"]
        for name in ["e2e-producer", "host-proof-build", "e2e-evidence"]
    ]
    assert starts == sorted(starts)


def test_the_verifier_reads_this_builds_selected_output_never_the_gc_root(
    e2e: E2E,
) -> None:
    selected = capture_only()
    result = e2e.run(
        "e2e-producer",
        LANE,
        output=selected,
        status="0",
        GCROOT_DECOY=str(e2e.root / "decoy"),
    )
    assert result.returncode == 0, result
    assert f"--artifacts {selected}\n" in e2e.events()
    assert "decoy" not in e2e.events()


def test_a_failed_producer_records_no_cache_and_builds_no_verifier(e2e: E2E) -> None:
    result = e2e.run("e2e-producer", LANE, status="23")
    assert result.returncode == 23
    spans = e2e.checkout.merged()
    assert "jaunder.cache" not in spans["e2e-producer"]["args"]
    assert "host-proof-build" not in spans


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
    result = e2e.run(
        "e2e-producer", LANE, output=capture_only(), status=status, PHASES=phases
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
    artifacts = {f"capture-{LANE}.tar.gz": "current diagnostic"}
    if lifted is not None:
        artifacts[PHASE_MANIFEST] = lifted
    result = e2e.run("e2e-producer", LANE, output=immutable(artifacts), status=status)
    assert str(result.returncode) == status, result
    assert (b"no phase spans" in result.stderr) == warned, result.stderr
    assert phase_children(e2e) == {}
    step = e2e.checkout.merged()["e2e-producer"]
    assert step["args"]["jaunder.status"] == ("ok" if status == "0" else "failed")


def test_the_script_takes_no_coverage_action(e2e: E2E) -> None:
    argv = ["bash", "scripts/e2e-workflow", "producer", LANE, "regenerate"]
    result = e2e.checkout.run(argv, e2e.checkout.environment())
    assert result.returncode == 2
    assert e2e.events() == ""


def test_an_optional_provenance_copy_failure_only_warns(e2e: E2E) -> None:
    selected = immutable(
        {f"capture-{LANE}.tar.gz": "current diagnostic", MARKER: "attribution only"}
    )
    e2e.fail_copy(MARKER)
    result = e2e.run("e2e-producer", LANE, output=selected, status="0")
    assert result.returncode == 0, result
    assert b"warning: optional attribution" in result.stderr
    assert MARKER.encode() in result.stderr
    assert e2e.capture.read_text() == "current diagnostic"
    assert "verifier verify" in e2e.events()
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
            REQUIRED[0]: "current diagnostic",
            REQUIRED[1]: "report",
            REQUIRED[2]: "duration",
            MARKER: "attribution only",
        }
    )
    e2e.fail_copy(artifact)
    result = e2e.run("e2e-producer", LANE, output=selected, status=status)
    assert result.returncode != 0, "a failed copy became green"
    if status == "23":
        assert result.returncode == 23, "the producer's own status was replaced"
    assert "verifier" not in e2e.events()
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
    assert "verifier" not in e2e.events()


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
    assert "verifier" not in e2e.events()


# The gates build the tools workspace's binaries (`cargo test` in host_tests)
# before they run pytest; run alone, pytest names the build it needs.
VERIFIER = REPOSITORY / "tools" / "target" / "debug" / "e2e-evidence"
BUILD_VERIFIER = (
    "cargo build --locked --manifest-path tools/Cargo.toml -p host-proof "
    "--bin e2e-evidence"
)


def test_missing_successful_evidence_fails_the_real_lane_verifier(e2e: E2E) -> None:
    require(VERIFIER.is_file(), f"{VERIFIER}; build it with `{BUILD_VERIFIER}`")
    result = e2e.run(
        "e2e-producer",
        LANE,
        output=capture_only(),
        status="0",
        REAL_VERIFIER=str(VERIFIER),
    )
    assert result.returncode != 0
    assert "verifier verify" in e2e.events()
    assert b"reading report" in result.stdout


def test_regeneration_never_reaches_unverified_lanes_with_the_real_verifier(
    e2e: E2E,
) -> None:
    # The real lane verifier rejects the evidence-less producers, so the pair
    # fails before regeneration; the Rust verifier separately refuses
    # unreconciled lanes (`coverage_refuses_unreconciled_lane_evidence`).
    require(VERIFIER.is_file(), f"{VERIFIER}; build it with `{BUILD_VERIFIER}`")
    result = e2e.run(
        "e2e-regenerate-coverage",
        output=capture_only(),
        status="0",
        REAL_VERIFIER=str(VERIFIER),
    )
    assert result.returncode != 0
    assert "verifier verify" in e2e.events()
    assert "server-fn-coverage" not in e2e.events()
    assert not (e2e.root / "docs" / "coverage" / "server-fns.json").exists()
