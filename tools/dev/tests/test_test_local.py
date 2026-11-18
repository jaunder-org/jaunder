"""`dev test-local`: its steps, in order, under the checkout's artifact lock."""

import fcntl
import json
import os
import signal
import subprocess
import time
from pathlib import Path
from typing import Any

import pytest
from conftest import Checkout

CARGO_RUN = "cargo run --locked --quiet --manifest-path tools/Cargo.toml"
LOCK = ".xtask/host-artifacts.lock"
STEPS = [
    "host-artifacts-lock",
    "csr-admission",
    "csr-bundle",
    "csr-prerequisite",
    "image-worker",
    "nextest",
]

# Host-proof's helpers, the image worker build and nextest (behind the fake
# pg-run). The helpers fail as ADMIT_CODE and PREREQUISITE_CODE say; with
# DAEMON set, admission first leaves behind a daemon that keeps every
# inheritable descriptor, its pid in `daemon.pid`. The prerequisite prints its
# resolved root. The build records CFLAGS. nextest records what it was given,
# and while `hold` exists it removes it, says it is `holding` and waits for
# `release`.
CARGO = """\
import json
args = sys.argv[1:]
if args[0] == "run":
    binary = args[args.index("--bin") + 1]
    root = args[args.index("--") + 1:]
    if binary == "admit-inputs" and os.environ.get("DAEMON"):
        daemon = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=False,
            start_new_session=True,
        )
        pathlib.Path("daemon.pid").write_text(str(daemon.pid))
    code = int(os.environ.get(
        "ADMIT_CODE" if binary == "admit-inputs" else "PREREQUISITE_CODE", "0"))
    if code:
        print(f"{binary}: refused", file=sys.stderr)
        sys.exit(code)
    if binary == "csr-prerequisite":
        print(os.path.realpath(root[0]))
elif args[0] == "build":
    pathlib.Path("cflags.json").write_text(json.dumps(os.environ.get("CFLAGS")))
else:
    lock = os.stat(".xtask/host-artifacts.lock")
    inherited = False
    for fd in range(3, 256):
        try:
            found = os.fstat(fd)
        except OSError:
            continue
        inherited |= (found.st_dev, found.st_ino) == (lock.st_dev, lock.st_ino)
    pathlib.Path("nextest.json").write_text(json.dumps({
        "args": args,
        "bundle": os.environ.get("JAUNDER_CSR_BUNDLE_DIR"),
        "executable": os.environ.get("JAUNDER_TEST_EXECUTABLE"),
        "wrapper": os.environ.get("RUSTC_WRAPPER"),
        "inherited": inherited,
    }))
    if os.path.exists("hold"):
        os.remove("hold")
        pathlib.Path("holding").touch()
        while not os.path.exists("release"):
            time.sleep(0.02)
"""
# A build links its out-link to $BUNDLE, or fails with NIX_BUILD_CODE; cache
# probes fail, so the step records no cache source.
NIX = """\
if sys.argv[1] != "build":
    sys.exit(1)
code = int(os.environ.get("NIX_BUILD_CODE", "0"))
if code:
    sys.exit(code)
link = sys.argv[sys.argv.index("--out-link") + 1]
if os.path.lexists(link):
    os.remove(link)
os.symlink(os.environ["BUNDLE"], link)
"""
PG_RUN = "os.execvp(sys.argv[1], sys.argv[1:])"


@pytest.fixture
def local(checkout: Checkout) -> Checkout:
    checkout.tool("cargo", CARGO)
    checkout.tool("nix", NIX)
    checkout.tool("pg-run", PG_RUN)
    (checkout.root.parent / "bundle").mkdir()
    return checkout


def environment(checkout: Checkout, **extra: str) -> dict[str, str]:
    """The run's environment; CFLAGS and the bundle root only as `extra` sets them."""
    environment = checkout.environment(
        BUNDLE=str(checkout.root.parent / "bundle"), **extra
    )
    for key in ("CFLAGS", "JAUNDER_CSR_BUNDLE_DIR"):
        if key not in extra:
            environment.pop(key, None)
    return environment


def run_test_local(
    checkout: Checkout, *args: str, **extra: str
) -> subprocess.CompletedProcess[bytes]:
    return checkout.run(
        checkout.command("test-local", *args), environment(checkout, **extra)
    )


def order(checkout: Checkout) -> list[str]:
    return checkout.order().splitlines()


def nextest(checkout: Checkout) -> dict[str, Any]:
    observed: dict[str, Any] = json.loads((checkout.root / "nextest.json").read_text())
    return observed


BUNDLE_BUILD = (
    "nix build -L --accept-flake-config --print-out-paths --out-link .xtask/gcroots/csr-bundle "
    ".#csrBundle"
)
PREREQUISITE = f"{CARGO_RUN} -p host-proof --bin csr-prerequisite --"


def expected_order(*nextest_args: str) -> list[str]:
    return [
        f"{CARGO_RUN} -p host-proof --bin admit-inputs -- .#csrBundle.sourceAdmits",
        BUNDLE_BUILD,
        f"{PREREQUISITE} .xtask/gcroots/csr-bundle",
        "cargo build --locked -p jaunder --bin jaunder",
        " ".join(["pg-run", "cargo", "nextest", "run", *nextest_args]),
        " ".join(["cargo", "nextest", "run", *nextest_args]),
    ]


def test_each_step_runs_in_order_as_a_span_of_the_gate(local: Checkout) -> None:
    result = run_test_local(local)
    assert result.returncode == 0, result
    # Nix's cache probes run before its build and are not part of the order.
    assert [line for line in order(local) if " eval " not in line] == expected_order(
        "--workspace"
    )
    slices = local.merged()
    gate = slices["dev test-local"]["args"]["span_id"]
    for name in STEPS:
        assert slices[name]["args"]["parent_id"] == gate, name
    starts = [slices[name]["ts"] for name in STEPS]
    assert starts == sorted(starts)


def test_nextest_gets_the_verified_bundle_and_the_built_worker(
    local: Checkout,
) -> None:
    result = run_test_local(local)
    assert result.returncode == 0, result
    assert nextest(local) == {
        "args": ["nextest", "run", "--workspace"],
        "bundle": os.path.realpath(local.root.parent / "bundle"),
        "executable": str(local.root.resolve() / "target" / "debug" / "jaunder"),
        "wrapper": "sccache",
        "inherited": False,
    }


SELECTORS = ["-p", "storage", "a b", "$(touch pwned)", "--", "--nocapture"]


@pytest.mark.parametrize("separator", [[], ["--"]])
def test_selectors_pass_through_unchanged_without_the_workspace_default(
    local: Checkout, separator: list[str]
) -> None:
    # Only a leading `--` is dropped; a later one belongs to nextest.
    result = run_test_local(local, *separator, *SELECTORS)
    assert result.returncode == 0, result
    assert nextest(local)["args"] == ["nextest", "run", *SELECTORS]
    assert not (local.root / "pwned").exists()


def test_an_explicit_bundle_root_is_verified_without_building_one(
    local: Checkout,
) -> None:
    explicit = local.root.parent / "explicit"
    explicit.mkdir()
    result = run_test_local(local, JAUNDER_CSR_BUNDLE_DIR=str(explicit))
    assert result.returncode == 0, result
    lines = order(local)
    assert not any(line.startswith("nix ") for line in lines), lines
    assert lines[0] == f"{PREREQUISITE} {explicit}"
    assert nextest(local)["bundle"] == os.path.realpath(explicit)
    assert "csr-bundle" not in local.merged()


@pytest.mark.parametrize(
    ("inherited", "expected"),
    [
        (None, ""),
        ("", ""),
        ("-O2 -fstack-protector", "-O2 -fstack-protector"),
    ],
)
def test_the_worker_build_sets_missing_cflags_empty_and_keeps_an_existing_value(
    local: Checkout, inherited: str | None, expected: str
) -> None:
    extra = {} if inherited is None else {"CFLAGS": inherited}
    result = run_test_local(local, **extra)
    assert result.returncode == 0, result
    assert json.loads((local.root / "cflags.json").read_text()) == expected


@pytest.mark.parametrize(
    ("failure", "last"),
    [
        ({"ADMIT_CODE": "3"}, "admit-inputs"),
        ({"NIX_BUILD_CODE": "4"}, "nix build"),
        ({"PREREQUISITE_CODE": "1"}, "csr-prerequisite"),
    ],
)
def test_a_refused_bundle_stops_before_any_compilation(
    local: Checkout, failure: dict[str, str], last: str
) -> None:
    result = run_test_local(local, **failure)
    assert result.returncode == int(next(iter(failure.values()))), result
    lines = [line for line in order(local) if " eval " not in line]
    assert last in lines[-1], lines
    assert not any(line.startswith("cargo build") for line in lines)
    assert not (local.root / "nextest.json").exists()


def wait_for(path: Path) -> None:
    deadline = time.monotonic() + 30
    while not path.exists():
        assert time.monotonic() < deadline, f"{path} never appeared"
        time.sleep(0.02)


def test_the_lock_is_held_for_the_whole_run_so_a_second_run_waits(
    local: Checkout,
) -> None:
    (local.root / "hold").touch()
    command = local.command("test-local", "-p", "common")
    first = subprocess.Popen(command, cwd=local.root, env=environment(local))
    wait_for(local.root / "holding")
    with open(local.root / LOCK) as probe, pytest.raises(BlockingIOError):
        fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)

    second = subprocess.Popen(command, cwd=local.root, env=environment(local))
    time.sleep(0.5)
    assert second.poll() is None
    assert len([line for line in order(local) if "admit-inputs" in line]) == 1

    (local.root / "release").touch()
    assert first.wait(timeout=60) == 0
    assert second.wait(timeout=60) == 0
    lines = [line for line in order(local) if " eval " not in line]
    assert lines == expected_order("-p", "common") * 2


def test_the_lock_is_released_after_a_failed_step_despite_a_lingering_daemon(
    local: Checkout,
) -> None:
    result = run_test_local(local, ADMIT_CODE="3", DAEMON="1")
    daemon = int((local.root / "daemon.pid").read_text())
    try:
        assert result.returncode == 3, result
        with open(local.root / LOCK) as probe:
            fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)
    finally:
        os.kill(daemon, signal.SIGKILL)


def test_a_termination_request_while_waiting_for_the_lock_closes_it_signalled(
    local: Checkout,
) -> None:
    (local.root / LOCK).parent.mkdir(parents=True)
    with open(local.root / LOCK, "a") as held:
        fcntl.flock(held, fcntl.LOCK_EX)
        dev = subprocess.Popen(
            local.command("test-local"),
            cwd=local.root,
            env=environment(local),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        deadline = time.monotonic() + 30
        while not any(
            '"host-artifacts-lock"' in records.read_text()
            for records in (local.root / ".xtask" / "run").glob("*.trace.d/*.jsonl")
        ):
            assert time.monotonic() < deadline, "the lock wait never started"
            time.sleep(0.02)
        dev.send_signal(signal.SIGTERM)
        dev.communicate(timeout=30)
    assert dev.returncode == 128 + signal.SIGTERM
    assert "admit-inputs" not in local.order()
    waited = local.merged()["host-artifacts-lock"]["args"]
    assert (waited["jaunder.status"], waited["process.exit.code"]) == (
        "signalled",
        128 + signal.SIGTERM,
    )
