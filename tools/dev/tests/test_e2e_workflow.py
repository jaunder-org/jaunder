"""The E2E workflow through `dev`, against fake Nix and a fake verifier.

`scripts/e2e-workflow` settles every producer, lifts only its own evidence and
reconciles both backends. It needs the repository's script, lane catalog and
jq programs, a real Nix store for immutable outputs, and jq, so only the
filtered `checks.dev-python` source may skip it.
"""

import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

import pytest
from conftest import REPOSITORY, Checkout, require

BASH = shutil.which("bash") or "/bin/sh"
NIX = shutil.which("nix")
FAKE_NIX = f"""#!{BASH}
set -eu
if [[ $* == *".#host-proof"* ]]; then
  printf '%s\\n' "$EVIDENCE_OUT"
  exit 0
fi
if [[ $1 == build ]]; then
  lane=${{!#}}; lane=${{lane##*.e2e-}}
  echo "producer $lane" >> "$EVENTS"
  echo "producer exit ${{BUILD_STATUS:-23}}" >&2
  if [[ -f $EVENTS.kept-note ]]; then
    while IFS= read -r note; do printf '%s\\n' "$note" >&2; done < "$EVENTS.kept-note"
  fi
  if [[ -n ${{PRODUCER_SECONDS-}} ]]; then
    sleep "$PRODUCER_SECONDS"
    echo "finished $lane" >> "$EVENTS"
  fi
  if [[ ${{BUILD_STATUS:-23}} == 0 ]]; then
    while [[ $1 != --out-link ]]; do shift; done
    ln -sfn "$OUTPUT" "$2"
    printf '[{{"outputs":{{"out":"%s"}}}}]\\n' "$OUTPUT"
  fi
  exit "${{BUILD_STATUS:-23}}"
fi
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
LANE = "sqlite-chromium-unsplit"
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
def test_an_early_producer_failure_still_settles_every_lane_and_both_reconciliations(
    e2e: E2E, jobs: str, failure: str
) -> None:
    e2e.diagnostics.mkdir(parents=True)
    e2e.capture.write_text("stale")
    (e2e.root / ".xtask" / "not-owned-keep").write_text("keep")
    result = e2e.run("e2e", *([jobs] if jobs else []), status=failure)
    assert result.returncode != 0
    lines = e2e.events().splitlines()
    assert sum(line.startswith("producer ") for line in lines) == 8, lines
    assert "producer postgres-firefox-serial-special" in lines
    assert any("reconcile sqlite" in line for line in lines[8:])
    assert any("reconcile postgres" in line for line in lines[8:])
    assert "server-fn-coverage" not in e2e.events()
    assert not e2e.capture.exists(), "a failed producer reused stale evidence"
    assert (e2e.root / ".xtask" / "not-owned-keep").read_text() == "keep"
    for lane in [LANE, "postgres-firefox-serial-special"]:
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
    assert sum(line.startswith("producer ") for line in lines) == 8
    assert sum("verifier reconcile" in line for line in lines) == 2
    assert sum("server-fn-coverage verify" in line for line in lines) == 1
    assert "regenerate" not in e2e.events()
    assert sum("verifier verify" in line for line in lines) == 7


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
    assert (
        f"--capture {selected}/capture-{LANE}.tar.gz --producer-output {selected}"
        in e2e.events()
    )
    assert "--capture .xtask/diagnostics" not in e2e.events()
    assert result.stdout == b"[ ok ] server-fn-coverage\n", "the report is rendered"


@pytest.mark.parametrize(
    ("args", "status", "action"),
    [
        (["e2e-regenerate-coverage"], "0", "regenerate"),
        (["e2e-regenerate-coverage"], "23", ""),
        (["e2e-producer", LANE], "0", "verify"),
    ],
)
def test_regeneration_needs_success_and_the_authoritative_lane_verifies_once(
    e2e: E2E, args: list[str], status: str, action: str
) -> None:
    result = e2e.run(*args, output=capture_only(), status=status)
    assert (result.returncode == 0) == (status == "0"), result
    events = e2e.events()
    assert "verifier verify" not in events
    if action:
        assert events.count(f"server-fn-coverage {action}") == 1, events
    else:
        assert "server-fn-coverage" not in events
    if action != "regenerate":
        assert "regenerate" not in events


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
    assert "server-fn-coverage verify" in e2e.events()
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
    assert "verifier" not in e2e.events()


def kept(root: Path, build: str) -> Path:
    directory = root / "nix" / "var" / "nix" / "builds" / build / "build"
    (directory / "emit-out" / "diagnostics").mkdir(parents=True)
    return directory


def test_a_failed_producer_recovers_only_its_logged_build_and_lane(e2e: E2E) -> None:
    current = kept(e2e.root, "nix-184917-2991942680")
    unrelated = kept(e2e.root, "nix-999999-1234567890")
    diagnostics = current / "emit-out" / "diagnostics"
    (diagnostics / f"capture-{LANE}.tar.gz").write_text("current failed diagnostic")
    (diagnostics / "capture-postgres-chromium-unsplit.tar.gz").write_text("other lane")
    stale = unrelated / "emit-out" / "diagnostics" / f"playwright-report-{LANE}.json"
    stale.write_text("stale unrelated report")
    e2e.diagnostics.mkdir(parents=True)
    (e2e.diagnostics / "build.log").write_text(
        f'note: keeping build directory "{unrelated}"\n'
    )
    (e2e.root / "events.kept-note").write_text(
        f'note: keeping build directory "{current}"\n'
    )
    result = e2e.run("e2e-producer", LANE, status="23")
    assert result.returncode == 23
    assert e2e.capture.read_text() == "current failed diagnostic"
    assert not (e2e.diagnostics / f"playwright-report-{LANE}.json").exists()
    assert not (e2e.diagnostics / "capture-postgres-chromium-unsplit.tar.gz").exists()
    assert stale.is_file()
    assert "verifier" not in e2e.events()


@pytest.mark.parametrize(
    "variant", ["absent", "missing", "relative", "unquoted", "traversal", "ambiguous"]
)
def test_unusable_kept_paths_never_replace_the_failure_or_borrow_old_evidence(
    e2e: E2E, variant: str
) -> None:
    current = kept(e2e.root, "nix-184917-2991942680")
    (current / "emit-out" / "diagnostics" / f"capture-{LANE}.tar.gz").write_text(
        "unselected stale diagnostic"
    )
    e2e.diagnostics.mkdir(parents=True)
    (e2e.diagnostics / "build.log").write_text(
        f'note: keeping build directory "{current}"\n'
    )
    note = {
        "absent": "",
        "missing": f'note: keeping build directory "{current}/missing"\n',
        "relative": 'note: keeping build directory "relative/build"\n',
        "unquoted": f"note: keeping build directory {current}\n",
        "traversal": f'note: keeping build directory "{current}/../build"\n',
        "ambiguous": f'note: keeping build directory "{current}"\n'
        'note: keeping build directory "/another/build"\n',
    }[variant]
    (e2e.root / "events.kept-note").write_text(note)
    result = e2e.run("e2e-producer", LANE, status="23")
    assert result.returncode == 23, "the producer failure was replaced"
    assert not e2e.capture.exists(), "borrowed unselected evidence"
    assert "verifier" not in e2e.events()


# The gates build the tools workspace's binaries (`cargo test` in host_tests)
# before they run pytest; run alone, pytest names the build it needs.
VERIFIER = REPOSITORY / "tools" / "target" / "debug" / "e2e-evidence"
BUILD_VERIFIER = (
    "cargo build --locked --manifest-path tools/Cargo.toml -p host-proof "
    "--bin e2e-evidence"
)


@pytest.mark.parametrize("args", [["e2e-producer", LANE], ["e2e-regenerate-coverage"]])
def test_missing_successful_evidence_fails_the_real_verifier(
    e2e: E2E, args: list[str]
) -> None:
    require(VERIFIER.is_file(), f"{VERIFIER}; build it with `{BUILD_VERIFIER}`")
    result = e2e.run(
        *args, output=capture_only(), status="0", REAL_VERIFIER=str(VERIFIER)
    )
    assert result.returncode != 0
    assert "verifier verify" not in e2e.events()
    assert "server-fn-coverage" in e2e.events()
    assert b"reading report" in result.stdout
    assert not (e2e.root / "docs" / "coverage" / "server-fns.json").exists()
