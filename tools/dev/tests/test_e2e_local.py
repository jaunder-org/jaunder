"""`dev e2e-local`: its plan, its refusals and its whole lifecycle with fakes.

The process tests drive the real CLI against fake `nix`, `admit-inputs`,
`jaunder`, `otelcol-contrib`, `test-support` and `playwright`. Each fake notes
its pid and what it saw in the checkout root, and behaves as its `FAKE_*`
environment variable asks.
"""

import json
import os
import re
import signal
import socket
import subprocess
import sys
import time
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import psutil
import pytest
from conftest import Checkout

from dev import e2e_local
from dev.e2e_local import Instance, Provenance
from dev.supervision import Child, Signals

HEAD = (
    f"#!{sys.executable}\n"
    + """\
import json, os, pathlib, signal, socket, subprocess, sys, time
state = pathlib.Path(os.environ["FAKE_STATE"])
def note(name, text=""):
    with open(state / name, "a") as file:
        file.write(text + "\\n")
def mode(name):
    return os.environ.get(name, "")
def wait():
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        time.sleep(0.05)
note("pids", str(os.getpid()))
"""
)
NIX = """\
args = sys.argv[1:]
note("nix", json.dumps(args))
if args[0] == "path-info":
    if "--derivation" in args:
        print("/nix/store/fake.drv")
        sys.exit()
    sys.exit(1)
if args[-1] != ".#host-proof" and mode("FAKE_BUILD") == "fail":
    sys.exit(9)
if "--out-link" in args:
    os.symlink(os.environ["FAKE_TOOL_OUT"], args[args.index("--out-link") + 1])
print(os.environ["FAKE_TOOL_OUT"])
"""
ADMIT = """\
note("admitted", json.dumps(sys.argv[1:]))
"""
TEST_SUPPORT = """\
command = sys.argv[1]
if command == "capture-path":
    capture = pathlib.Path(os.environ["JAUNDER_CAPTURE_DIR"])
    unset = ("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_EXPORTER_OTLP_ENDPOINT")
    note("capture-path-env", json.dumps([key for key in unset if key in os.environ]))
    if mode("FAKE_CAPTURE") == "fail":
        sys.exit(4)
    if mode("FAKE_CAPTURE") == "outside":
        print("/elsewhere/otel-traces.jsonl")
        sys.exit()
    if mode("FAKE_CAPTURE") == "file":
        capture.parent.mkdir(parents=True)
        capture.touch()
    print(capture / "otel-traces.jsonl")
elif command == "seed-e2e":
    environment = [os.environ[key] for key in (
        "JAUNDER_STORAGE_PATH", "JAUNDER_CAPTURE_DIR",
        "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
    )]
    note("seeds", json.dumps(sys.argv[1:] + environment))
    sys.exit(1 if mode("FAKE_SEED") == "fail" else 0)
else:
    note("verified", json.dumps(sys.argv[1:]))
    sys.exit(1 if pathlib.Path(sys.argv[3]).name == mode("FAKE_PANIC") else 0)
"""
ORPHAN = """\
import os, pathlib, sys, time
if os.fork():
    sys.exit()
os.setsid()
pathlib.Path(os.environ["FAKE_STATE"], "orphan.pid").write_text(str(os.getpid()))
time.sleep(60)
"""
JAUNDER = (
    f"ORPHAN = {ORPHAN!r}\n"
    + """\
import http.server
capture = pathlib.Path(os.environ["JAUNDER_CAPTURE_DIR"])
target, _, behaviour = mode("FAKE_SERVER").rpartition(":")
if target and target != capture.name:
    behaviour = ""
print(f"jaunder serving {capture.name}", file=sys.stderr, flush=True)
keys = ("JAUNDER_BIND", "JAUNDER_STORAGE_PATH", "JAUNDER_DB", "JAUNDER_RUNTIME_FILE",
        "JAUNDER_CAPTURE_DIR", "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", "RUST_LOG")
note("servers", json.dumps([sys.argv[1:], {key: os.environ[key] for key in keys}]))
if behaviour == "exit":
    sys.exit(3)
if behaviour == "hang":
    note("server-ready")
    wait()
if behaviour == "ignore-term":
    signal.signal(signal.SIGTERM, lambda *_: note("server-term"))
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
port = server.server_address[1]
if behaviour == "http-hang":
    server.server_close()
runtime = pathlib.Path(os.environ["JAUNDER_RUNTIME_FILE"])
partial = runtime.with_name("partial")
partial.write_text(json.dumps({"ip": "127.0.0.1", "port": port, "pid": os.getpid()}))
partial.replace(runtime)
if behaviour == "http-hang":
    note("server-ready")
    wait()
if behaviour == "exit-after-runtime":
    sys.exit(0)
if behaviour == "orphan":
    # A grandchild that leaves this group and is orphaned, holding stderr.
    subprocess.run([sys.executable, "-c", ORPHAN], check=True)
server.serve_forever()
"""
)
COLLECTOR = """\
behaviour = mode("FAKE_COLLECTOR")
note("collector-attempts")
attempts = len((state / "collector-attempts").read_text().splitlines())
capture = pathlib.Path(os.environ["JAUNDER_CAPTURE_DIR"])
if behaviour == "collide" or (behaviour == "collide-once" and attempts == 1):
    print("listen tcp: bind: address already in use", file=sys.stderr, flush=True)
    sys.exit(1)
listeners = []
for key in ("OTELCOL_GRPC_ENDPOINT", "OTELCOL_HTTP_ENDPOINT"):
    host, port = os.environ[key].rsplit(":", 1)
    listeners.append(socket.create_server((host, int(port))))
note("collector-args", json.dumps(sys.argv[1:]))
(capture / "otel-traces.jsonl").write_text("{}\\n")
if behaviour == "instance-file":
    (capture / "instance-0").touch()
if behaviour == "readonly-tmp":
    os.chmod(os.environ["TMPDIR"], 0o500)
code = 2 if behaviour == "fail-shutdown" else 0
signal.signal(signal.SIGTERM, lambda *_: sys.exit(code))
while not (behaviour == "premature" and (state / "playwright-ran").exists()):
    time.sleep(0.05)
"""
PLAYWRIGHT_KEYS = (
    "JAUNDER_E2E_INSTANCES",
    "JAUNDER_E2E_PRODUCER_INPUT",
    "JAUNDER_TEST_EXECUTABLE",
    "JAUNDER_E2E_OTLP_HTTP_ENDPOINT",
    "JAUNDER_E2E_TRACE_ID",
    "JAUNDER_E2E_WORKERS",
    "PLAYWRIGHT_HTML_OPEN",
    "PATH",
)
PLAYWRIGHT = (
    f"KEYS = {PLAYWRIGHT_KEYS!r}\n"
    + """\
environment = {key: os.environ[key] for key in KEYS}
note("playwright", json.dumps({"argv": sys.argv[1:], "cwd": os.getcwd(), "env": environment}))
behaviour = mode("FAKE_PLAYWRIGHT")
if behaviour == "flood":
    sys.stdout.write(("x" * 99 + "\\n") * 10600)
if behaviour == "grandchild":
    grandchild = subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(60)"], start_new_session=True
    )
    note("grandchild.pid", str(grandchild.pid))
if behaviour == "ignore-term":
    signal.signal(signal.SIGTERM, lambda *_: note("playwright-term"))
if behaviour in ("wait", "grandchild", "ignore-term"):
    note("playwright-ready")
    wait()
note("playwright-ran")
sys.exit(1 if behaviour == "fail" else 0)
"""
)
# What `--browser`'s default selects, as each step of its lifecycle is named.
CHROMIUM = "e2e-local-chromium-"


def executable(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")
    path.chmod(0o755)


def alive(pid: int) -> bool:
    try:
        return psutil.Process(pid).status() != psutil.STATUS_ZOMBIE
    except psutil.NoSuchProcess:
        return False


def wait_for(path: Path) -> None:
    deadline = time.monotonic() + 30
    while not path.exists():
        assert time.monotonic() < deadline, f"{path} never appeared"
        time.sleep(0.02)


@dataclass
class E2E:
    """A checkout whose e2e-local artifacts and tools are fakes."""

    checkout: Checkout
    tmp: Path

    @property
    def root(self) -> Path:
        return self.checkout.root

    @property
    def bundle(self) -> Path:
        """The fake artifact set every build selects (and the tool package)."""
        return self.checkout.tool_out / "bin"

    def environment(self, **extra: str) -> dict[str, str]:
        environment = self.checkout.environment(
            FAKE_STATE=str(self.root), TMPDIR=str(self.tmp)
        )
        for key in [key for key in environment if key.startswith("FAKE_")]:
            if key not in ("FAKE_STATE", "FAKE_TOOL_OUT"):
                del environment[key]
        environment.pop("JAUNDER_CSR_BUNDLE_DIR", None)
        environment.pop("JAUNDER_E2E_WORKERS", None)
        environment.update(extra)
        return environment

    def without(self, tool: str) -> str:
        """A PATH on which `tool` is found nowhere, not even in the devShell."""
        (self.checkout.bin / tool).unlink()
        directories = self.environment()["PATH"].split(os.pathsep)
        return os.pathsep.join(
            directory
            for directory in directories
            if not (Path(directory) / tool).exists()
        )

    def run(self, *args: str, **extra: str) -> subprocess.CompletedProcess[bytes]:
        return self.checkout.run(
            self.checkout.command("e2e-local", *args), self.environment(**extra)
        )

    def start(self, *args: str, **extra: str) -> subprocess.Popen[bytes]:
        with (
            open(self.root.parent / "stdout", "wb") as stdout,
            open(self.root.parent / "stderr", "wb") as stderr,
        ):
            return subprocess.Popen(
                self.checkout.command("e2e-local", *args),
                cwd=self.root,
                env=self.environment(**extra),
                stdout=stdout,
                stderr=stderr,
            )

    def finished(self, dev: subprocess.Popen[bytes]) -> int:
        return dev.wait(timeout=60)

    @property
    def stdout(self) -> bytes:
        return (self.root.parent / "stdout").read_bytes()

    def notes(self, name: str) -> list[Any]:
        path = self.root / name
        if not path.exists():
            return []
        return [json.loads(line) for line in path.read_text().splitlines()]

    def pids(self) -> list[int]:
        return [int(line) for line in (self.root / "pids").read_text().split()]

    def gcroots(self) -> list[Path]:
        return sorted((self.root / ".xtask" / "gcroots").glob("e2e-local-*"))


@pytest.fixture
def e2e(checkout: Checkout, tmp_path: Path) -> Iterator[E2E]:
    fake = E2E(checkout, tmp_path / "tmp")
    fake.tmp.mkdir()
    (checkout.root / "end2end").mkdir()
    executable(checkout.bin / "nix", HEAD + NIX)
    executable(checkout.bin / "otelcol-contrib", HEAD + COLLECTOR)
    executable(checkout.bin / "playwright", HEAD + PLAYWRIGHT)
    executable(fake.bundle / "admit-inputs", HEAD + ADMIT)
    executable(fake.bundle / "jaunder", HEAD + JAUNDER)
    executable(fake.bundle / "test-support", HEAD + TEST_SUPPORT)
    yield fake
    fake.tmp.chmod(0o755)


def summary(output: bytes) -> list[tuple[str, bool]]:
    """The summary's steps, as (name, passed)."""
    lines = output.decode(errors="replace").splitlines()
    return [
        (match[2], match[1] == " ok ")
        for line in lines
        if (match := re.match(r"\[(.{4})\] (\S+)", line))
    ]


def details(output: bytes) -> dict[str, str]:
    lines = output.decode(errors="replace").splitlines()
    return {
        match[1]: match[2]
        for line in lines
        if (match := re.match(r"\[.{4}\] (\S+)(?: \(\d+ ms\))?(?: — (.*))?$", line))
    }


def lifecycle(*names: str, browser: str = "chromium") -> list[tuple[str, bool]]:
    """Steps that passed, `!name` for one that failed."""
    return [
        (f"e2e-local-{browser}-{name.lstrip('!')}", not name.startswith("!"))
        for name in names
    ]


PASSING_TWO_WORKERS = lifecycle(
    "collector",
    "instance-0-server",
    "instance-0-seed",
    "instance-1-server",
    "instance-1-seed",
    "collector-shutdown",
    "capture",
    "playwright",
    "instance-0-panic-gate",
    "instance-1-panic-gate",
)


# Units.


def test_the_default_run_selects_chromium_with_html_and_line_reporters() -> None:
    assert e2e_local.plan(None, None, False) == [
        e2e_local.Lifecycle(
            "chromium",
            "browser",
            ("test", "--project", "chromium", "--reporter=html,line"),
        )
    ]


@pytest.mark.parametrize("browser", ["chromium", "firefox", "webkit"])
def test_a_filter_follows_the_selected_browser_project(browser: str) -> None:
    (only,) = e2e_local.plan(browser, "theme.spec.ts:63", False)
    assert (only.browser, only.args) == (
        browser,
        ("test", "--project", browser, "--reporter=html,line", "theme.spec.ts:63"),
    )
    # Playwright must be free to fail a filter that matches nothing.
    assert "--pass-with-no-tests" not in only.args


def test_visual_updates_run_chromium_then_firefox_lifecycles() -> None:
    assert [
        (each.browser, each.label, each.args)
        for each in e2e_local.plan(None, None, True)
    ] == [
        (
            browser,
            "visual",
            (
                "test",
                "--project",
                browser,
                "--grep",
                "@visual",
                "--update-snapshots",
                "--reporter=html,line",
            ),
        )
        for browser in ("chromium", "firefox")
    ]


def instance(url: str, storage: str, capture: str) -> Instance:
    found = Instance.__new__(Instance)
    found.url, found.storage, found.capture = url, Path(storage), Path(capture)
    return found


def test_the_instance_map_lists_each_worker_with_a_localhost_origin() -> None:
    instances = [
        instance("http://127.0.0.1:4100", "/tmp/a", "/tmp/capture/instance-0"),
        instance("http://127.0.0.1:4200", "/tmp/ü", "/tmp/capture/instance-1"),
    ]
    assert e2e_local.instance_map(instances) == (
        '[{"baseUrl":"http://localhost:4100","db":"sqlite:/tmp/a/jaunder.db",'
        '"storagePath":"/tmp/a","captureDir":"/tmp/capture/instance-0"},'
        '{"baseUrl":"http://localhost:4200","db":"sqlite:/tmp/ü/jaunder.db",'
        '"storagePath":"/tmp/ü","captureDir":"/tmp/capture/instance-1"}]'
    )


def test_the_instance_map_refuses_a_path_that_is_not_utf8() -> None:
    storage = os.fsdecode(b"/tmp/\xff")
    with pytest.raises(UnicodeEncodeError):
        e2e_local.instance_map([instance("http://127.0.0.1:1", storage, "/c")])


@pytest.mark.parametrize(
    ("environment", "refused"),
    [
        ({}, None),
        ({"JAUNDER_E2E_WORKERS": "2"}, None),
        *(
            ({"JAUNDER_E2E_WORKERS": value}, "e2e-local-env")
            for value in ["0", "two", "-1", "", "+1", " 2", "²"]
        ),
        ({"JAUNDER_CSR_BUNDLE_DIR": ""}, "e2e-local-env"),
        ({"JAUNDER_CSR_BUNDLE_DIR": "caller-owned-bundle"}, "e2e-local-env"),
    ],
)
def test_workers_must_be_a_positive_integer_and_no_bundle_may_be_named(
    environment: dict[str, str], refused: str | None
) -> None:
    found = e2e_local.refusal(False, "linux", environment)
    assert (found and found[0]) == refused
    if environment.get("JAUNDER_E2E_WORKERS"):
        assert found is None or "positive integer" in found[1]


@pytest.mark.parametrize(
    ("update", "platform", "refused"),
    [
        (True, "darwin", "e2e-local-platform"),
        (True, "linux", None),
        (False, "darwin", None),
        (False, "linux", None),
    ],
)
def test_canonical_visual_updates_are_linux_only(
    update: bool, platform: str, refused: str | None
) -> None:
    found = e2e_local.refusal(update, platform, {})
    assert (found and found[0]) == refused


@pytest.mark.parametrize(
    ("runtime", "url"),
    [
        (b'{"ip":"127.0.0.1","pid":42,"port":4312}', "http://127.0.0.1:4312"),
        (b'{"ip":"127.0.0.1","pid":41,"port":4312}', None),
        (b'{"ip":"127.0.0.1","pid":true,"port":4312}', None),
        (b'{"ip":"127.0.0.1","port":4312}', None),
        (b'{"pid":42,"port":4312}', None),
        (b'{"ip":"127.0.0.1","pid":42}', None),
        (b'{"ip":"127.0.0.1","pid":42,"port":"4312"}', None),
        (b'[{"ip":"127.0.0.1","pid":42,"port":4312}]', None),
        (b'{"ip":"127.0.0.1","pid":42,', None),
        (b"not json", None),
    ],
)
def test_runtime_info_names_the_server_only_for_the_spawned_pid(
    runtime: bytes, url: str | None
) -> None:
    assert e2e_local.base_url(runtime, 42) == url


CLEAN = {"head": "h", "tree": "t", "parents": ["p"], "reference": None, "clean": True}


@pytest.mark.parametrize(
    ("before", "after", "cleared", "attested"),
    [
        (CLEAN, CLEAN, False, "marker-clearing-failed"),
        (None, CLEAN, True, "checkout-unavailable"),
        (CLEAN, None, True, "checkout-unavailable"),
        (CLEAN | {"clean": False}, CLEAN, True, "dirty-committed-base-only"),
        (CLEAN, CLEAN | {"clean": False}, True, "dirty-committed-base-only"),
        (CLEAN, CLEAN | {"tree": "other"}, True, "checkout-changed"),
        (CLEAN, CLEAN, True, "selection-unavailable"),
    ],
)
def test_attestation_takes_the_first_reason_that_applies(
    before: dict[str, Any] | None,
    after: dict[str, Any] | None,
    cleared: bool,
    attested: str,
) -> None:
    assert e2e_local.attestation(before, after, cleared) == attested


# Provenance, against a real repository.


def git(*args: str) -> str:
    done = subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args],
        check=True,
        capture_output=True,
        text=True,
    )
    return done.stdout.strip()


@pytest.fixture
def repository(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    root = tmp_path / "repository"
    root.mkdir()
    monkeypatch.chdir(root)
    git("init", "-q", "-b", "main")
    (root / ".gitignore").write_text(".xtask/\n")
    git("add", ".gitignore")
    git("commit", "-q", "-m", "one")
    git("commit", "-q", "--allow-empty", "-m", "two")
    return root


def record(root: Path) -> dict[str, Any]:
    path = root / e2e_local.PRODUCER_INPUT
    found: dict[str, Any] = json.loads(path.read_text())
    return found


def test_provenance_records_a_clean_unchanged_checkout(
    repository: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("GITHUB_RUN_ID", "77")
    monkeypatch.delenv("GITHUB_SHA", raising=False)
    marker = repository / e2e_local.MARKER
    marker.parent.mkdir(parents=True)
    marker.write_text("stale")
    provenance = Provenance()
    assert not marker.exists()
    started = record(repository)
    assert started["build_succeeded"] is None
    assert started["checkout_after"] is None
    provenance.close(True)
    closed = record(repository)
    head = git("rev-parse", "HEAD")
    identity = {
        "head": head,
        "tree": git("rev-parse", "HEAD^{tree}"),
        "parents": [git("rev-parse", "HEAD^")],
        "reference": "refs/heads/main",
        "clean": True,
    }
    assert closed == closed | {
        "schema_version": 1,
        "lane": "host-local",
        "requesting_checkout": identity,
        "checkout_after": identity,
        "ci_request_annotations": {"GITHUB_RUN_ID": "77"},
        "acquisition_errors": [],
        "build_succeeded": True,
        "marker_destination_cleared": True,
        "input_attestation": "selection-unavailable",
    }
    assert closed["request_started_unix_ms"] <= closed["recorded_unix_ms"]


def test_provenance_sees_a_detached_dirty_or_changed_checkout(
    repository: Path,
) -> None:
    git("checkout", "-q", "--detach")
    provenance = Provenance()
    assert record(repository)["requesting_checkout"]["reference"] is None
    git("commit", "-q", "--allow-empty", "-m", "three")
    provenance.close(False)
    assert record(repository)["input_attestation"] == "checkout-changed"
    assert record(repository)["build_succeeded"] is False
    (repository / "untracked").touch()
    Provenance().close(True)
    assert record(repository)["input_attestation"] == "dirty-committed-base-only"


def test_provenance_without_a_commit_is_unavailable_with_bounded_errors(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)
    git("init", "-q")
    provenance = Provenance()
    provenance.close(True)
    closed = record(tmp_path)
    assert closed["input_attestation"] == "checkout-unavailable"
    assert [error.split(":")[0] for error in closed["acquisition_errors"]] == [
        "checkout-before",
        "checkout-after",
    ]
    assert "git rev-parse: exit" in closed["acquisition_errors"][0]
    for _ in range(20):
        provenance._error("probe", "x" * 300)
    assert len(provenance.errors) == 16
    assert {len(error) for error in provenance.errors[2:]} == {256}


def test_provenance_that_cannot_clear_or_write_only_warns(
    repository: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # A file where the record's directory belongs: nothing can be removed
    # from it or written into it.
    (repository / e2e_local.DIAGNOSTICS).parent.mkdir(parents=True)
    (repository / e2e_local.DIAGNOSTICS).write_text("")
    provenance = Provenance()
    assert not provenance.cleared
    assert [error.split(":")[0] for error in provenance.errors] == [
        "clear-marker",
        "clear-checkout-sidecar",
    ]
    provenance.close(True)
    warnings = capsys.readouterr().err
    assert (
        "e2e-local.e2e.provenance: clear-marker: remove NotADirectoryError" in warnings
    )
    assert "persist-sidecar: write" in warnings


# Readiness limits, too long to wait out through the CLI.


@pytest.fixture
def sleeper(tmp_path: Path) -> Iterator[Instance]:
    child = Child([sys.executable, "-c", "import time; time.sleep(30)"])
    yield Instance(child, tmp_path, tmp_path)
    child.stop(0)


def test_a_server_without_runtime_info_in_time_is_not_ready(
    sleeper: Instance, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(e2e_local, "RUNTIME_READY", 0.1)
    assert e2e_local.ready(sleeper, Signals()) == (
        "server runtime file not ready within 15s"
    )


def test_a_server_that_never_answers_is_not_ready(
    sleeper: Instance, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(e2e_local, "HTTP_ATTEMPTS", 2)
    monkeypatch.setattr(e2e_local, "HTTP_INTERVAL", 0.01)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    # Nothing listens there now.
    runtime = {"ip": "127.0.0.1", "port": port, "pid": sleeper.child.popen.pid}
    (sleeper.storage / "runtime.json").write_text(json.dumps(runtime))
    assert e2e_local.ready(sleeper, Signals()) == (
        "server not reachable via runtime.json within 15s"
    )


# The command line.


def test_help_redirects_iteration_without_replacing_gates(checkout: Checkout) -> None:
    result = checkout.dev("e2e-local", "--help")
    assert result.returncode == 0
    text = " ".join(result.stdout.decode().split())
    for phrase in [
        "Focused browser proof",
        "same focused command",
        "hermetic",
        "parity",
        "dev e2e-producer",
        "--browser [chromium|firefox|webkit]",
        "--update-visual-snapshots",
    ]:
        assert phrase in text, phrase


@pytest.mark.parametrize(
    ("args", "conflict"),
    [
        (["--update-visual-snapshots", "auth.spec.ts"], "TEST"),
        (["--update-visual-snapshots", "--browser", "webkit"], "--browser"),
    ],
)
def test_visual_update_mode_takes_no_browser_or_filter(
    e2e: E2E, args: list[str], conflict: str
) -> None:
    result = e2e.run(*args)
    assert result.returncode == 2
    assert (
        f"--update-visual-snapshots conflicts with {conflict}".encode() in result.stderr
    )
    assert not (e2e.root / "nix").exists()


@pytest.mark.parametrize(
    ("environment", "step"),
    [
        ({"JAUNDER_CSR_BUNDLE_DIR": "bundle"}, "e2e-local-env"),
        ({"JAUNDER_E2E_WORKERS": "0"}, "e2e-local-env"),
    ],
)
def test_a_refused_run_prints_one_failed_step_and_builds_nothing(
    e2e: E2E, environment: dict[str, str], step: str
) -> None:
    result = e2e.run(**environment)
    assert result.returncode == 1
    assert summary(result.stdout) == [(step, False)]
    assert not (e2e.root / "nix").exists()


# The lifecycle.


def test_two_workers_each_get_a_seeded_server_and_one_playwright_run(
    e2e: E2E,
) -> None:
    result = e2e.run("--browser", "chromium", "auth.spec.ts", JAUNDER_E2E_WORKERS="2")
    assert result.returncode == 0, result.stderr.decode()
    assert summary(result.stdout) == PASSING_TWO_WORKERS
    # stdout carries the summary alone.
    assert len(result.stdout.splitlines()) == len(PASSING_TWO_WORKERS)
    assert b"jaunder serving instance-1" in result.stderr

    (admitted,) = e2e.notes("admitted")
    assert admitted == [".#sandboxArtifacts.sourceAdmits"]
    build = [args for args in e2e.notes("nix") if args[-1] == ".#sandboxArtifacts"]
    assert build[0] == [
        "path-info",
        "--derivation",
        "--accept-flake-config",
        "--no-update-lock-file",
        "--no-write-lock-file",
        ".#sandboxArtifacts",
    ]
    link = build[1][build[1].index("--out-link") + 1]
    assert re.fullmatch(r"\.xtask/gcroots/e2e-local-\d+-[0-9a-f]{8}", link)
    assert e2e.gcroots() == []

    (collector,) = e2e.notes("collector-args")
    assert collector == [
        "--config",
        "end2end/otel-collector.yaml",
        "--config",
        "nix/otel-processors.yaml",
        "--set=service.telemetry.metrics.level=none",
    ]
    assert e2e.notes("capture-path-env") == [[]]

    servers = e2e.notes("servers")
    assert [argv for argv, _ in servers] == [["serve"], ["serve"]]
    for index, (_, server) in enumerate(servers):
        storage = server["JAUNDER_STORAGE_PATH"]
        assert server["JAUNDER_BIND"] == "127.0.0.1:0"
        assert server["JAUNDER_DB"] == f"sqlite:{storage}/jaunder.db"
        assert server["JAUNDER_RUNTIME_FILE"] == f"{storage}/runtime.json"
        assert server["JAUNDER_CAPTURE_DIR"].endswith(f"/instance-{index}")
        assert re.fullmatch(
            r"http://127\.0\.0\.1:\d+", server["JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT"]
        )
        assert server["RUST_LOG"] == "info"
        # Storage is temporary: gone once the run ends.
        assert not Path(storage).exists()

    jaunder = str((e2e.bundle / "jaunder").resolve())
    test_support = str((e2e.bundle / "test-support").resolve())
    seeds = e2e.notes("seeds")
    assert seeds == [
        [
            "seed-e2e",
            "--db",
            server["JAUNDER_DB"],
            "--test-support-bin",
            test_support,
            "--jaunder-bin",
            jaunder,
            server["JAUNDER_STORAGE_PATH"],
            server["JAUNDER_CAPTURE_DIR"],
            server["JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT"],
        ]
        for _, server in servers
    ]

    (playwright,) = e2e.notes("playwright")
    assert playwright["argv"] == [
        "test",
        "--project",
        "chromium",
        "--reporter=html,line",
        "auth.spec.ts",
    ]
    assert playwright["cwd"] == str(e2e.root / "end2end")
    environment = playwright["env"]
    instances = json.loads(environment["JAUNDER_E2E_INSTANCES"])
    assert [entry["db"] for entry in instances] == [
        server["JAUNDER_DB"] for _, server in servers
    ]
    assert [entry["storagePath"] for entry in instances] == [
        server["JAUNDER_STORAGE_PATH"] for _, server in servers
    ]
    assert [entry["captureDir"] for entry in instances] == [
        server["JAUNDER_CAPTURE_DIR"] for _, server in servers
    ]
    assert all(
        re.fullmatch(r"http://localhost:\d+", entry["baseUrl"]) for entry in instances
    )
    assert environment["JAUNDER_E2E_PRODUCER_INPUT"] == str(
        e2e.root / ".xtask/diagnostics/e2e-host-local/producer-checkout-host-local.json"
    )
    assert environment["JAUNDER_TEST_EXECUTABLE"] == jaunder
    assert re.fullmatch(
        r"http://127\.0\.0\.1:\d+/v1/traces",
        environment["JAUNDER_E2E_OTLP_HTTP_ENDPOINT"],
    )
    assert re.fullmatch("[0-9a-f]{32}", environment["JAUNDER_E2E_TRACE_ID"])
    assert int(environment["JAUNDER_E2E_TRACE_ID"], 16)
    assert environment["JAUNDER_E2E_WORKERS"] == "2"
    assert environment["PLAYWRIGHT_HTML_OPEN"] == "never"
    assert environment["PATH"].startswith(str(e2e.root / link / "bin") + os.pathsep)

    (run,) = (e2e.root / ".xtask/e2e-local").glob("run-*")
    capture = run / "chromium" / "capture"
    assert details(result.stdout)[f"{CHROMIUM}capture"] == (
        f"trace retained at {capture / 'otel-traces.jsonl'}"
    )
    for index in range(2):
        log = capture / f"instance-{index}" / "server-stderr.log"
        assert f"jaunder serving instance-{index}" in log.read_text()
    assert (capture / "otelcol-contrib.stderr.log").exists()
    assert e2e.notes("verified") == [
        [
            "verify-no-panics",
            "--capture-dir",
            str(capture / f"instance-{index}"),
            "--server-log",
            str(capture / f"instance-{index}" / "server-stderr.log"),
        ]
        for index in range(2)
    ]
    # The collector's temporary capture is gone once copied.
    assert list(e2e.tmp.iterdir()) == []

    provenance = record(e2e.root)
    assert provenance["build_succeeded"] is True
    assert provenance["input_attestation"] == "checkout-unavailable"

    slices = e2e.checkout.merged()
    for name, _ in PASSING_TWO_WORKERS:
        assert slices[name]["args"]["jaunder.status"] == "ok"
    for name in ("e2e-local-admission", "e2e-local-build"):
        assert slices[name]["args"]["jaunder.status"] == "ok"
    assert not any(alive(pid) for pid in e2e.pids())


def test_visual_updates_build_release_csr_and_run_both_baselines(e2e: E2E) -> None:
    result = e2e.run("--update-visual-snapshots")
    assert result.returncode == 0, result.stderr.decode()
    one = ["collector", "instance-0-server", "instance-0-seed", "collector-shutdown"]
    rest = ["capture", "playwright", "instance-0-panic-gate"]
    assert summary(result.stdout) == [
        *lifecycle(*one, *rest),
        *lifecycle(*one, *rest, browser="firefox"),
    ]
    assert e2e.notes("admitted") == [[".#sandboxArtifactsReleaseCsr.sourceAdmits"]]
    assert ["build", ".#sandboxArtifactsReleaseCsr"] in [
        [args[0], args[-1]] for args in e2e.notes("nix")
    ]
    assert [run["argv"][2] for run in e2e.notes("playwright")] == [
        "chromium",
        "firefox",
    ]
    assert len(list((e2e.root / ".xtask/e2e-local").glob("run-*/*/capture"))) == 2


def test_a_consumer_writing_more_than_a_pipe_holds_runs_to_exit(e2e: E2E) -> None:
    result = e2e.run(FAKE_PLAYWRIGHT="flood")
    assert result.returncode == 0
    assert result.stderr.count(b"x" * 99 + b"\n") == 10600


@pytest.mark.parametrize("sent", [signal.SIGINT, signal.SIGTERM, signal.SIGHUP])
def test_a_signal_during_playwright_stops_everything_and_exits_128_plus_n(
    e2e: E2E, sent: signal.Signals
) -> None:
    dev = e2e.start(FAKE_PLAYWRIGHT="wait")
    wait_for(e2e.root / "playwright-ready")
    dev.send_signal(sent)
    assert e2e.finished(dev) == 128 + sent
    steps = summary(e2e.stdout)
    assert steps == lifecycle(
        "collector",
        "instance-0-server",
        "instance-0-seed",
        "collector-shutdown",
        "capture",
        "!playwright",
        "instance-0-panic-gate",
    )
    assert not any(alive(pid) for pid in e2e.pids())
    assert e2e.gcroots() == []
    assert record(e2e.root)["build_succeeded"] is False
    gate = e2e.checkout.merged()["dev e2e-local"]["args"]
    assert (gate["jaunder.status"], gate["process.exit.code"]) == (
        "signalled",
        128 + sent,
    )


def test_a_signal_ends_visual_updates_before_the_next_browser(e2e: E2E) -> None:
    dev = e2e.start("--update-visual-snapshots", FAKE_PLAYWRIGHT="wait")
    wait_for(e2e.root / "playwright-ready")
    dev.send_signal(signal.SIGINT)
    assert e2e.finished(dev) == 130
    assert {name.split("-")[2] for name, _ in summary(e2e.stdout)} == {"chromium"}


@pytest.mark.parametrize("behaviour", ["hang", "http-hang"])
def test_a_signal_during_readiness_aborts_startup_and_stops_the_server(
    e2e: E2E, behaviour: str
) -> None:
    dev = e2e.start(FAKE_SERVER=behaviour)
    wait_for(e2e.root / "server-ready")
    dev.send_signal(signal.SIGTERM)
    assert e2e.finished(dev) == 143
    assert summary(e2e.stdout) == lifecycle(
        "collector",
        "!instance-0-server",
        "collector-shutdown",
        "capture",
        "instance-0-panic-gate",
    )
    assert details(e2e.stdout)[f"{CHROMIUM}instance-0-server"] == (
        "E2E server startup interrupted"
    )
    assert not any(alive(pid) for pid in e2e.pids())


def test_a_second_signal_kills_a_consumer_that_ignores_termination(e2e: E2E) -> None:
    dev = e2e.start(FAKE_PLAYWRIGHT="ignore-term")
    wait_for(e2e.root / "playwright-ready")
    started = time.monotonic()
    dev.send_signal(signal.SIGINT)
    wait_for(e2e.root / "playwright-term")
    dev.send_signal(signal.SIGINT)
    assert e2e.finished(dev) == 130
    assert time.monotonic() - started < 5
    assert not any(alive(pid) for pid in e2e.pids())


def test_a_signal_during_a_server_stop_kills_the_server(e2e: E2E) -> None:
    dev = e2e.start(FAKE_SERVER="ignore-term")
    wait_for(e2e.root / "server-term")
    started = time.monotonic()
    dev.send_signal(signal.SIGTERM)
    assert e2e.finished(dev) == 143
    assert time.monotonic() - started < 5
    # Every step itself passed; the request alone sets the status.
    assert all(passed for _, passed in summary(e2e.stdout))
    assert not any(alive(pid) for pid in e2e.pids())


def test_a_detached_grandchild_of_a_stopped_consumer_is_killed(e2e: E2E) -> None:
    dev = e2e.start(FAKE_PLAYWRIGHT="grandchild")
    wait_for(e2e.root / "playwright-ready")
    grandchild = int((e2e.root / "grandchild.pid").read_text())
    dev.send_signal(signal.SIGINT)
    assert e2e.finished(dev) == 130
    assert not alive(grandchild)


def test_a_later_instance_that_fails_to_start_still_tears_down_the_earlier(
    e2e: E2E,
) -> None:
    result = e2e.run(JAUNDER_E2E_WORKERS="2", FAKE_SERVER="instance-1:exit")
    assert result.returncode == 1
    assert summary(result.stdout) == lifecycle(
        "collector",
        "instance-0-server",
        "instance-0-seed",
        "!instance-1-server",
        "collector-shutdown",
        "capture",
        "instance-0-panic-gate",
        "instance-1-panic-gate",
    )
    assert details(result.stdout)[f"{CHROMIUM}instance-1-server"] == (
        "server runtime file not ready within 15s: server exited"
    )
    assert e2e.notes("playwright") == []
    assert not any(alive(pid) for pid in e2e.pids())


def test_one_failing_panic_gate_does_not_skip_the_next(e2e: E2E) -> None:
    result = e2e.run(JAUNDER_E2E_WORKERS="2", FAKE_PANIC="instance-0")
    assert result.returncode == 1
    assert summary(result.stdout)[-2:] == lifecycle(
        "!instance-0-panic-gate", "instance-1-panic-gate"
    )


def test_the_capture_is_not_copied_while_a_writer_is_alive(e2e: E2E) -> None:
    try:
        result = e2e.run(FAKE_SERVER="orphan")
    finally:
        orphan = e2e.root / "orphan.pid"
        if orphan.exists():
            os.kill(int(orphan.read_text()), signal.SIGKILL)
    assert result.returncode == 1
    assert summary(result.stdout)[-3:] == lifecycle(
        "!capture", "playwright", "!instance-0-panic-gate"
    )
    detail = details(result.stdout)[f"{CHROMIUM}capture"]
    assert detail.startswith("capture writers could not be stopped")
    (run,) = (e2e.root / ".xtask/e2e-local").glob("run-*")
    assert not (run / "chromium" / "capture").exists()
    assert e2e.notes("verified") == []


def test_a_collector_bind_collision_retries_with_new_ports(e2e: E2E) -> None:
    result = e2e.run(FAKE_COLLECTOR="collide-once")
    assert result.returncode == 0
    assert len((e2e.root / "collector-attempts").read_text().splitlines()) == 2


def test_a_collector_that_keeps_colliding_fails_after_eight_attempts(
    e2e: E2E,
) -> None:
    result = e2e.run(FAKE_COLLECTOR="collide")
    assert result.returncode == 1
    assert len((e2e.root / "collector-attempts").read_text().splitlines()) == 8
    assert summary(result.stdout) == lifecycle("!collector", "!capture")
    found = details(result.stdout)
    assert "address already in use" in found[f"{CHROMIUM}collector"]
    assert found[f"{CHROMIUM}capture"].startswith("collector produced no trace file")
    (run,) = (e2e.root / ".xtask/e2e-local").glob("run-*")
    assert (run / "chromium/capture/otelcol-contrib.stderr.log").is_file()


def test_a_collector_that_cannot_start_fails_its_step(e2e: E2E) -> None:
    result = e2e.run(PATH=e2e.without("otelcol-contrib"))
    assert result.returncode == 1
    assert summary(result.stdout) == lifecycle("!collector", "!capture")
    assert "starting otelcol-contrib" in details(result.stdout)[f"{CHROMIUM}collector"]


@pytest.mark.parametrize(
    ("behaviour", "failure"),
    [
        ("premature", "otelcol-contrib exited prematurely before shutdown"),
        ("fail-shutdown", "otelcol-contrib exited unsuccessfully during shutdown"),
    ],
)
def test_a_collector_must_run_until_shutdown_and_exit_cleanly(
    e2e: E2E, behaviour: str, failure: str
) -> None:
    result = e2e.run(FAKE_COLLECTOR=behaviour)
    assert result.returncode == 1
    assert ("e2e-local-chromium-collector-shutdown", False) in summary(result.stdout)
    detail = details(result.stdout)[f"{CHROMIUM}collector-shutdown"]
    assert detail.startswith(failure)


@pytest.mark.parametrize(
    ("variable", "behaviour", "step", "failure"),
    [
        (
            "FAKE_CAPTURE",
            "fail",
            "capture",
            "cannot allocate retained capture: test-support capture-path otel failed",
        ),
        (
            "FAKE_CAPTURE",
            "outside",
            "capture",
            (
                "cannot allocate retained capture: test-support returned OTel "
                "capture path outside"
            ),
        ),
        (
            "FAKE_COLLECTOR",
            "instance-file",
            "instance-0-server-log",
            "failed to create server stderr capture",
        ),
        (
            "FAKE_COLLECTOR",
            "readonly-tmp",
            "instance-0-tmpdir",
            "cannot create temp storage dir",
        ),
        ("FAKE_SEED", "fail", "instance-0-seed", "test-support seed-e2e failed"),
        (
            "FAKE_PLAYWRIGHT",
            "fail",
            "playwright",
            "browser Playwright invocation reported failures",
        ),
        (
            "FAKE_CAPTURE",
            "file",
            "capture",
            "failed to retain collector capture",
        ),
        (
            "FAKE_SERVER",
            "exit-after-runtime",
            "instance-0-server",
            "server exited before becoming HTTP-ready",
        ),
    ],
)
def test_a_failing_step_is_reported_and_fails_the_run(
    e2e: E2E, variable: str, behaviour: str, step: str, failure: str
) -> None:
    result = e2e.run(**{variable: behaviour})
    assert result.returncode == 1
    assert (f"{CHROMIUM}{step}", False) in summary(result.stdout)
    assert details(result.stdout)[f"{CHROMIUM}{step}"].startswith(failure)
    assert not any(alive(pid) for pid in e2e.pids())


def test_a_copy_failure_panic_gates_the_kept_source(e2e: E2E) -> None:
    result = e2e.run(FAKE_CAPTURE="file")
    detail = details(result.stdout)[f"{CHROMIUM}capture"]
    source = Path(detail.rpartition("source retained at ")[2])
    assert (source / "otel-traces.jsonl").is_file()
    ((_, _, capture, _, _),) = e2e.notes("verified")
    assert capture == str(source / "instance-0")


def test_an_exited_child_stays_unreaped_until_reap() -> None:
    child = Child(["true"])
    while child.running:
        time.sleep(0.01)
    # Exited but unreaped: its pid, and so its group id, is still its own.
    assert psutil.Process(child.popen.pid).status() == psutil.STATUS_ZOMBIE
    assert child.reap()
    assert child.status == 0
    assert not child.running


@pytest.mark.parametrize("missing", ["jaunder", "playwright"])
def test_a_child_that_cannot_start_fails_its_step(e2e: E2E, missing: str) -> None:
    if missing == "jaunder":
        # Found, but its interpreter is not.
        executable(e2e.bundle / "jaunder", "#!/nonexistent/interpreter\n")
        result = e2e.run()
    else:
        result = e2e.run(PATH=e2e.without("playwright"))
    assert result.returncode == 1
    found = details(result.stdout)
    if missing == "jaunder":
        assert found[f"{CHROMIUM}instance-0-server"].startswith(
            "failed to spawn jaunder serve"
        )
    else:
        assert found[f"{CHROMIUM}playwright"] == (
            "browser Playwright invocation reported failures"
        )


def test_an_instance_path_json_cannot_carry_fails_the_instance_map(
    e2e: E2E, tmp_path: Path
) -> None:
    e2e.tmp = tmp_path / os.fsdecode(b"tmp-\xff")
    e2e.tmp.mkdir()
    result = e2e.run()
    assert result.returncode == 1
    assert ("e2e-local-chromium-instance-map", False) in summary(result.stdout)
    assert e2e.notes("playwright") == []


def test_a_failed_build_ends_the_gate_without_a_summary(e2e: E2E) -> None:
    result = e2e.run(FAKE_BUILD="fail")
    assert result.returncode == 9
    assert result.stdout == b""
    assert e2e.gcroots() == []
    assert record(e2e.root)["build_succeeded"] is False


def test_artifacts_without_an_executable_test_support_are_refused(e2e: E2E) -> None:
    (e2e.bundle / "test-support").unlink()
    result = e2e.run()
    assert result.returncode == 1
    assert result.stdout == b""
    assert b"not an executable: " in result.stderr
    assert e2e.gcroots() == []
    build = e2e.checkout.merged()["e2e-local-build"]["args"]
    assert build["jaunder.status"] == "failed"
