"""`dev e2e-local`: focused browser proof against this checkout's Nix artifacts.

Each planned browser gets one lifecycle: an OTLP collector, one seeded
`jaunder serve` per Playwright worker (#2076), Playwright against them, then
teardown in a fixed order that keeps the collector's capture. Every lifecycle
step prints in one summary on stdout and is a span of its own; children's
output goes to stderr.
"""

import contextlib
import functools
import json
import os
import re
import secrets
import shutil
import socket
import sys
import tempfile
import time
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    import urllib.request

from dev import checkout, report, steps, supervision, trace, traces
from dev.commands import ADMIT_INPUTS
from dev.spans import STATUS, Status
from dev.supervision import Child, Signals

BROWSERS = ("chromium", "firefox", "webkit")
LANE = "host-local"
DIAGNOSTICS = Path(".xtask/diagnostics/e2e-host-local")
PRODUCER_INPUT = DIAGNOSTICS / f"producer-checkout-{LANE}.json"
MARKER = DIAGNOSTICS / f"producer-input-{LANE}.json"
RETAINED = Path(".xtask/e2e-local")
COLLECTOR = (
    "otelcol-contrib",
    "--config",
    "end2end/otel-collector.yaml",
    "--config",
    "nix/otel-processors.yaml",
    # The local harness does not read the collector's own metrics; turning
    # them off avoids its fixed Prometheus port and keeps the VM's
    # configuration shared.
    "--set=service.telemetry.metrics.level=none",
)
COLLECTOR_ATTEMPTS = 8
COLLECTOR_READY = 5.0
RUNTIME_READY = 15.0
HTTP_ATTEMPTS = 30
HTTP_INTERVAL = 0.5
INTERRUPTED = "E2E server startup interrupted"
GITHUB_ANNOTATIONS = (
    "GITHUB_RUN_ID",
    "GITHUB_RUN_ATTEMPT",
    "GITHUB_JOB",
    "GITHUB_EVENT_NAME",
    "GITHUB_REPOSITORY",
    "GITHUB_REF",
    "GITHUB_SHA",
)


@dataclass(frozen=True)
class Lifecycle:
    """One browser's fresh servers, and the Playwright arguments run against them."""

    browser: str
    label: str
    args: tuple[str, ...]


def plan(browser: str | None, test_filter: str | None, update: bool) -> list[Lifecycle]:
    """The lifecycles a run executes, in order.

    A run selects one browser's project, Chromium by default, so a filter finds
    a visual or a non-visual test alike. Visual update mode refreshes the
    Chromium and then the Firefox baselines, each in its own lifecycle.
    """
    if update:
        return [
            Lifecycle(
                name,
                "visual",
                ("test", "--project", name, "--grep", "@visual", "--update-snapshots")
                + ("--reporter=html,line",),
            )
            for name in ("chromium", "firefox")
        ]
    name = browser or "chromium"
    selected = () if test_filter is None else (test_filter,)
    return [
        Lifecycle(
            name,
            "browser",
            ("test", "--project", name, "--reporter=html,line", *selected),
        )
    ]


def refusal(
    update: bool, platform: str, environment: Mapping[str, str]
) -> tuple[str, str] | None:
    """The setup step that refuses this run, and why; None to go ahead."""
    if update and platform != "linux":
        return (
            "e2e-local-platform",
            (
                "canonical visual snapshot updates are Linux-only; run without "
                "--update-visual-snapshots for host browser investigation"
            ),
        )
    if "JAUNDER_CSR_BUNDLE_DIR" in environment:
        return (
            "e2e-local-env",
            (
                "e2e-local selects embedded Nix artifacts; unset "
                "JAUNDER_CSR_BUNDLE_DIR before running e2e-local"
            ),
        )
    workers = environment.get("JAUNDER_E2E_WORKERS", "1")
    # One server starts per worker, so only a real positive count will do.
    if not re.fullmatch("[0-9]+", workers) or not int(workers):
        return (
            "e2e-local-env",
            (
                "cannot configure Playwright environment: JAUNDER_E2E_WORKERS "
                f"must be a positive integer, got {workers!r}"
            ),
        )
    return None


def base_url(runtime: bytes, pid: int) -> str | None:
    """The server's URL from its `runtime.json`, if the spawned child wrote it.

    dev holds the child unreaped, so its pid stays its own, and the file is in a
    fresh directory, so it cannot be stale: the pid alone identifies it.
    """
    try:
        info = json.loads(runtime)
    except ValueError:
        return None
    match info:
        case {"pid": int(written), "ip": str(ip), "port": int(port)} if written == pid:
            return f"http://{ip}:{port}"
    return None


@dataclass
class Instance:
    """One Playwright worker's server: its storage, database and capture."""

    child: Child
    storage: Path
    capture: Path
    url: str = ""
    stopped: bool = False

    @property
    def db(self) -> str:
        return f"sqlite:{self.storage}/jaunder.db"


def instance_map(instances: list[Instance]) -> str:
    """`JAUNDER_E2E_INSTANCES`: each worker's instance, by parallel index.

    The browser reaches the server as `localhost`, whose registrable domain
    WebAuthn accepts as an RP ID; the server stays bound to 127.0.0.1. A path
    that is not UTF-8 cannot travel in it: UnicodeEncodeError.
    """
    entries = [
        {
            "baseUrl": instance.url.replace("://127.0.0.1:", "://localhost:"),
            "db": instance.db,
            "storagePath": str(instance.storage),
            "captureDir": str(instance.capture),
        }
        for instance in instances
    ]
    text = json.dumps(entries, separators=(",", ":"), ensure_ascii=False)
    # Refuse here, not in Popen: a lone surrogate from a non-UTF-8 path
    # survives json.dumps but cannot become an environment value.
    text.encode()
    return text


def attestation(
    before: dict[str, Any] | None, after: dict[str, Any] | None, cleared: bool
) -> str:
    """What the local run's checkout attests about its inputs.

    A host run selects no hermetic producer output, so even a clean, unchanged
    checkout ends at `selection-unavailable`.
    """
    if not cleared:
        return "marker-clearing-failed"
    if before is None or after is None:
        return "checkout-unavailable"
    if not (before["clean"] and after["clean"]):
        return "dirty-committed-base-only"
    if before != after:
        return "checkout-changed"
    return "selection-unavailable"


class _Unavailable(Exception):
    """A Git query failed; its message names the query and nothing it printed."""


def _git(*args: str) -> str:
    done = checkout.untraced_git(*args)
    if done.returncode:
        raise _Unavailable(f"git {args[0]}: exit {done.returncode}")
    return done.stdout.strip()


def checkout_identity() -> dict[str, Any]:
    """The checkout's HEAD commit, tree, parents, branch and cleanliness.

    HEAD is read once, so its tree and parents describe that same commit.
    """
    head = _git("rev-parse", "HEAD")
    header = _git("cat-file", "-p", head).partition("\n\n")[0].splitlines()
    reference = checkout.untraced_git("symbolic-ref", "-q", "HEAD")
    return {
        "head": head,
        "tree": header[0].removeprefix("tree "),
        "parents": [
            line.removeprefix("parent ")
            for line in header
            if line.startswith("parent ")
        ],
        # A detached checkout, ordinary in CI, has none.
        "reference": reference.stdout.strip() if reference.returncode == 0 else None,
        "clean": not _git("status", "--porcelain", "--untracked-files=all"),
    }


def _warn(detail: str) -> None:
    print(f"e2e-local: warning: e2e-local.e2e.provenance: {detail}", file=sys.stderr)


class Provenance:
    """The local lane's checkout attribution, which visual diagnostics read.

    Ancillary to the verdict: a failure to acquire or write it only warns.
    Starting clears the previous marker and record and writes a fresh record;
    `close` rewrites it with the checkout as it is then.
    """

    def __init__(self) -> None:
        self.errors: list[str] = []
        self.started_ms = time.time_ns() // 1_000_000
        self.before = self._acquire("checkout-before")
        self.cleared = True
        for operation, path in (
            ("clear-marker", MARKER),
            ("clear-checkout-sidecar", PRODUCER_INPUT),
        ):
            try:
                path.unlink(missing_ok=True)
            except OSError as error:
                self.cleared = False
                self._error(operation, f"remove {type(error).__name__}")
        self._write(None, None)

    def _error(self, operation: str, detail: str) -> None:
        text = f"{operation}: {detail}"
        _warn(text)
        if len(self.errors) < 16:
            self.errors.append(text[:256])

    def _acquire(self, operation: str) -> dict[str, Any] | None:
        try:
            return checkout_identity()
        except _Unavailable as error:
            self._error(operation, str(error))
            return None

    def _write(self, after: dict[str, Any] | None, succeeded: bool | None) -> None:
        record = {
            "schema_version": 1,
            "lane": LANE,
            "request_started_unix_ms": self.started_ms,
            "recorded_unix_ms": time.time_ns() // 1_000_000,
            "requesting_checkout": self.before,
            "checkout_after": after,
            "ci_request_annotations": {
                key: os.environ[key] for key in GITHUB_ANNOTATIONS if key in os.environ
            },
            "acquisition_errors": self.errors,
            "build_succeeded": succeeded,
            "marker_destination_cleared": self.cleared,
            "input_attestation": attestation(self.before, after, self.cleared),
        }
        try:
            PRODUCER_INPUT.parent.mkdir(parents=True, exist_ok=True)
            PRODUCER_INPUT.write_text(json.dumps(record, indent=2))
        except OSError as error:
            _warn(f"persist-sidecar: write {type(error).__name__}")

    def close(self, succeeded: bool) -> None:
        """Rewrites the record; `succeeded` means the run passed without a signal."""
        self._write(self._acquire("checkout-after"), succeeded)


class Report:
    """The run's steps, in report order; each is also recorded as a span."""

    def __init__(self) -> None:
        self.steps: list[dict[str, Any]] = []

    @property
    def ok(self) -> bool:
        return all(step["ok"] for step in self.steps)

    def record(
        self,
        name: str,
        started_ns: int,
        ok: bool,
        detail: str | None = None,
        ended_ns: int | None = None,
    ) -> None:
        ended_ns = ended_ns or time.time_ns()
        span = trace.tracer().start_span(name, start_time=started_ns)
        span.set_attribute(STATUS, Status.OK if ok else Status.FAILED)
        span.end(end_time=ended_ns)
        self.steps.append(
            {
                "name": name,
                "ok": ok,
                "duration_ms": (ended_ns - started_ns) // 1_000_000,
                "detail": detail,
            }
        )

    def print(self) -> None:
        sys.stdout.write(report.render_report({"steps": self.steps}))
        sys.stdout.flush()


@dataclass
class _Run:
    """What every lifecycle of one run shares."""

    root: Path
    jaunder: Path
    test_support: Path
    path: str
    workers: int
    signals: Signals
    report: Report


@dataclass(frozen=True)
class _Retained:
    """The kept capture directory a lifecycle's collector capture is copied to."""

    run_dir: Path
    capture: Path
    trace: Path


class _Refused(Exception):
    """The retained capture cannot be allocated, and why."""


def _retained(run: _Run, browser: str) -> _Retained:
    retained = run.root / RETAINED
    retained.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="run-", dir=retained))
    capture = run_dir / browser / "capture"
    code, output = supervision.run(
        [str(run.test_support), "capture-path", "otel"],
        {
            "JAUNDER_CAPTURE_DIR": str(capture),
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT": None,
            "OTEL_EXPORTER_OTLP_ENDPOINT": None,
        },
        capture=True,
    )
    if code:
        raise _Refused(f"test-support capture-path otel failed (exit {code})")
    trace_path = Path(output.decode(errors="replace").strip())
    if trace_path.parent != capture:
        raise _Refused(
            f"test-support returned OTel capture path outside {capture}: {trace_path}"
        )
    return _Retained(run_dir, capture, trace_path)


def _endpoints() -> tuple[tuple[str, int], tuple[str, int]]:
    """Two distinct free loopback ports, reserved together then released."""
    with socket.socket() as grpc, socket.socket() as http:
        grpc.bind(("127.0.0.1", 0))
        http.bind(("127.0.0.1", 0))
        return grpc.getsockname(), http.getsockname()


def _diagnostics(log: Path) -> str:
    return log.read_text(errors="replace") or "<no stderr output>"


@dataclass
class _Collector:
    child: Child
    grpc: tuple[str, int]
    http: tuple[str, int]
    log: Path

    @property
    def exporter(self) -> str:
        return f"http://{self.grpc[0]}:{self.grpc[1]}"

    @property
    def browser_traces(self) -> str:
        return f"http://{self.http[0]}:{self.http[1]}/v1/traces"

    def shutdown(self) -> tuple[str | None, bool]:
        """Why the shutdown failed, if it did, and whether it has stopped writing.

        It must still be running when its shutdown starts, and exit cleanly.
        A shutdown is never escalated by a further request.
        """
        running = supervision.accepting(self.grpc, 1.0, self.child)
        stopped = self.child.stop(supervision.GRACE)
        if not running:
            return "otelcol-contrib exited prematurely before shutdown", stopped
        if self.child.status:
            return (
                (
                    "otelcol-contrib exited unsuccessfully during shutdown "
                    f"(exit {self.child.status}); collector stderr: "
                    f"{_diagnostics(self.log)}"
                ),
                stopped,
            )
        return None, stopped


class _CollectorFailed(Exception):
    def __init__(self, detail: str, stopped: bool) -> None:
        super().__init__(detail)
        self.detail = detail
        self.stopped = stopped


def _start_collector(source: Path) -> _Collector:
    """A collector capturing into `source`, ready on two fresh loopback ports.

    Ports are reserved just before it binds them; if another process takes one
    first, the start is retried with new ones.
    """
    log = source / "otelcol-contrib.stderr.log"
    attempt = 1
    while True:
        grpc, http = _endpoints()
        try:
            child = Child(
                COLLECTOR,
                {
                    "OTELCOL_GRPC_ENDPOINT": f"{grpc[0]}:{grpc[1]}",
                    "OTELCOL_HTTP_ENDPOINT": f"{http[0]}:{http[1]}",
                    "JAUNDER_CAPTURE_DIR": str(source),
                    # Its Snowflake driver leaves a temp dir behind at every
                    # start; inside `source` it goes away with the capture.
                    "TMPDIR": str(source),
                },
                tee=log,
            )
        except OSError as error:
            raise _CollectorFailed(
                f"starting otelcol-contrib: {error}", True
            ) from error
        if supervision.accepting(
            grpc, COLLECTOR_READY, child
        ) and supervision.accepting(http, COLLECTOR_READY, child):
            return _Collector(child, grpc, http, log)
        stopped = child.stop(0)
        diagnostics = _diagnostics(log)
        if "address already in use" in diagnostics and attempt < COLLECTOR_ATTEMPTS:
            attempt += 1
            continue
        raise _CollectorFailed(
            "otelcol-contrib did not accept connections within "
            f"{COLLECTOR_READY:g}s; collector stderr: {diagnostics}",
            stopped,
        )


@functools.cache
def _opener() -> urllib.request.OpenerDirector:
    """Built on first use: building it creates a TLS context, which every
    other `dev` command would otherwise pay for at import."""
    import urllib.request

    return urllib.request.build_opener(urllib.request.ProxyHandler({}))


def _responds(url: str) -> bool:
    # Imported here with the opener, for the same reason.
    from http.client import HTTPException

    try:
        with _opener().open(f"{url}/", timeout=2):
            return True
    except OSError, HTTPException:
        return False


def ready(instance: Instance, signals: Signals) -> str | None:
    """Waits for the server to publish itself and answer; why it did not, if so.

    Both waits give way to a termination request at once.
    """
    runtime = instance.storage / "runtime.json"
    deadline = time.monotonic() + RUNTIME_READY
    while not runtime.exists():
        if signals.code is not None:
            return INTERRUPTED
        if not instance.child.running:
            return "server runtime file not ready within 15s: server exited"
        if time.monotonic() >= deadline:
            return "server runtime file not ready within 15s"
        time.sleep(supervision.TICK)
    for _ in range(HTTP_ATTEMPTS):
        if signals.code is not None:
            return INTERRUPTED
        if not instance.child.running:
            return "server exited before becoming HTTP-ready"
        url = base_url(runtime.read_bytes(), instance.child.popen.pid)
        if url is not None and _responds(url):
            instance.url = url
            return None
        pause = time.monotonic() + HTTP_INTERVAL
        while time.monotonic() < pause and signals.code is None:
            time.sleep(supervision.TICK)
    return f"server not reachable via runtime.json after {HTTP_ATTEMPTS} attempts"


def _step(browser: str, suffix: str) -> str:
    return f"e2e-local-{browser}-{suffix}"


@dataclass
class _BrowserRun:
    """One browser's lifecycle while it runs, and its teardown."""

    run: _Run
    browser: str
    retained: _Retained
    source: Path
    collector: _Collector
    instances: list[Instance] = field(default_factory=list)
    storages: list[Path] = field(default_factory=list)
    # The Playwright step, once it has run: start, end and any failure.
    playwright: tuple[int, int, str | None] | None = None

    def record(self, suffix: str, started_ns: int, failure: str | None) -> None:
        self.run.report.record(
            _step(self.browser, suffix), started_ns, failure is None, failure
        )

    def start_instance(self, index: int) -> bool:
        """Starts and seeds instance `index`; whether it is ready for Playwright.

        Every instance whose server started joins teardown, ready or not.
        """
        run = self.run
        prefix = traces.instance_directory(index)
        started = time.time_ns()
        capture = self.source / prefix
        log = capture / "server-stderr.log"
        try:
            capture.mkdir()
            log.touch()
        except OSError as error:
            self.record(
                f"{prefix}-server-log",
                started,
                f"failed to create server stderr capture: {error}",
            )
            return False
        try:
            storage = Path(tempfile.mkdtemp(prefix="jaunder-e2e-"))
        except OSError as error:
            self.record(
                f"{prefix}-tmpdir", started, f"cannot create temp storage dir: {error}"
            )
            return False
        self.storages.append(storage)
        started = time.time_ns()
        try:
            child = Child(
                [str(run.jaunder), "serve"],
                {
                    "JAUNDER_BIND": "127.0.0.1:0",
                    "JAUNDER_STORAGE_PATH": str(storage),
                    "JAUNDER_DB": f"sqlite:{storage}/jaunder.db",
                    "JAUNDER_RUNTIME_FILE": str(storage / "runtime.json"),
                    "JAUNDER_CAPTURE_DIR": str(capture),
                    "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT": self.collector.exporter,
                    "RUST_LOG": "info",
                },
                tee=log,
            )
        except OSError as error:
            self.record(
                f"{prefix}-server", started, f"failed to spawn jaunder serve: {error}"
            )
            return False
        instance = Instance(child, storage, capture)
        self.instances.append(instance)
        failure = ready(instance, run.signals)
        self.record(f"{prefix}-server", started, failure)
        if failure is not None:
            return False
        started = time.time_ns()
        # The selected test-support owns the seed list; its `jaunder` steps
        # use this instance's storage, not `./data` under some directory.
        code, _ = supervision.run(
            [
                str(run.test_support),
                "seed-e2e",
                "--db",
                instance.db,
                "--test-support-bin",
                str(run.test_support),
                "--jaunder-bin",
                str(run.jaunder),
            ],
            {
                "JAUNDER_STORAGE_PATH": str(storage),
                "JAUNDER_CAPTURE_DIR": str(capture),
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT": self.collector.exporter,
            },
            run.signals,
        )
        failure = None if code == 0 else "test-support seed-e2e failed"
        self.record(f"{prefix}-seed", started, failure)
        return failure is None

    def playwright_run(self, lifecycle: Lifecycle) -> None:
        run = self.run
        try:
            instances = instance_map(self.instances)
        except UnicodeEncodeError as error:
            self.record(
                "instance-map",
                time.time_ns(),
                f"cannot describe instances to Playwright: {error}",
            )
            return
        started = time.time_ns()
        code, _ = supervision.run(
            ["playwright", *lifecycle.args],
            {
                "JAUNDER_E2E_INSTANCES": instances,
                "JAUNDER_E2E_PRODUCER_INPUT": str(run.root / PRODUCER_INPUT),
                "JAUNDER_TEST_EXECUTABLE": str(run.jaunder),
                "JAUNDER_E2E_OTLP_HTTP_ENDPOINT": self.collector.browser_traces,
                # One fresh id joins this lifecycle's browser and server spans.
                # Unlike Rust's /dev/urandom read it cannot fail in practice,
                # so there is no `-trace-context` failure step.
                "JAUNDER_E2E_TRACE_ID": f"{secrets.randbits(128) or 1:032x}",
                "JAUNDER_E2E_WORKERS": str(run.workers),
                "PLAYWRIGHT_HTML_OPEN": "never",
                "PATH": run.path,
            },
            run.signals,
            cwd=run.root / "end2end",
        )
        failure = (
            None
            if code == 0
            else f"{lifecycle.label} Playwright invocation reported failures"
        )
        self.playwright = (started, time.time_ns(), failure)

    def finish(self) -> None:
        """Stops every server, then the collector, keeps the capture and
        panic-gates each instance; every step is attempted."""
        for instance in self.instances:
            instance.stopped = instance.child.stop(supervision.GRACE, self.run.signals)
        started = time.time_ns()
        failure, stopped = self.collector.shutdown()
        self.record("collector-shutdown", started, failure)
        stopped = stopped and all(instance.stopped for instance in self.instances)
        capture = finish_capture(
            self.run, self.browser, self.retained, self.source, stopped
        )
        if self.playwright is not None:
            started, ended, failure = self.playwright
            self.run.report.record(
                _step(self.browser, "playwright"),
                started,
                failure is None,
                failure,
                ended,
            )
        for index in range(len(self.instances)):
            started = time.time_ns()
            failure = (
                "zero-panic verification skipped because capture writers could "
                "not be stopped"
            )
            if capture is not None:
                directory = capture / traces.instance_directory(index)
                code, _ = supervision.run(
                    [
                        str(self.run.test_support),
                        "verify-no-panics",
                        "--capture-dir",
                        str(directory),
                        "--server-log",
                        str(directory / "server-stderr.log"),
                    ]
                )
                failure = None if code == 0 else "shared zero-panic verifier failed"
            self.record(f"instance-{index}-panic-gate", started, failure)
        for storage in self.storages:
            shutil.rmtree(storage, ignore_errors=True)


def finish_capture(
    run: _Run, browser: str, retained: _Retained, source: Path, stopped: bool
) -> Path | None:
    """Copies the collector capture to `retained` once nothing writes it.

    Returns where the capture now is, or None while a writer may still be
    running: then the live source is kept, uncopied.
    """
    started = time.time_ns()
    step = _step(browser, "capture")
    if not stopped:
        run.report.record(
            step,
            started,
            False,
            "capture writers could not be stopped; live capture source retained "
            f"without copying at {source}",
        )
        return None
    try:
        shutil.copytree(source, retained.capture, dirs_exist_ok=True)
    except OSError as error:
        run.report.record(
            step,
            started,
            False,
            f"failed to retain collector capture: {error}; source retained at {source}",
        )
        return source
    shutil.rmtree(source, ignore_errors=True)
    if retained.trace.is_file():
        print(f"e2e-local trace: {retained.trace}", file=sys.stderr)
        run.report.record(step, started, True, f"trace retained at {retained.trace}")
    else:
        print(
            f"e2e-local capture retained at {retained.run_dir}; expected trace "
            f"missing: {retained.trace}",
            file=sys.stderr,
        )
        run.report.record(
            step,
            started,
            False,
            f"collector produced no trace file; retained capture directory: "
            f"{retained.run_dir}; expected: {retained.trace}",
        )
    return retained.capture


def _run_lifecycle(run: _Run, lifecycle: Lifecycle) -> None:
    browser = lifecycle.browser
    started = time.time_ns()
    try:
        retained = _retained(run, browser)
    except (OSError, _Refused) as error:
        run.report.record(
            _step(browser, "capture"),
            started,
            False,
            f"cannot allocate retained capture: {error}",
        )
        return
    started = time.time_ns()
    # Unguarded: tempfile falls back from an unusable TMPDIR to /tmp and
    # beyond, so this fails only when no temporary directory is usable.
    source = Path(tempfile.mkdtemp(prefix="e2e-local-capture-"))
    try:
        collector = _start_collector(source)
    except _CollectorFailed as failure:
        run.report.record(
            _step(browser, "collector"),
            started,
            False,
            f"otelcol-contrib readiness failed: {failure.detail}",
        )
        finish_capture(run, browser, retained, source, failure.stopped)
        return
    run.report.record(_step(browser, "collector"), started, True)
    state = _BrowserRun(run, browser, retained, source, collector)
    try:
        if all(state.start_instance(index) for index in range(run.workers)):
            state.playwright_run(lifecycle)
    finally:
        state.finish()


def _unselectable(link: str) -> str | None:
    """Why the built artifact set cannot serve a run, if it cannot."""
    for name in ("jaunder", "test-support"):
        path = Path(link, "bin", name).resolve()
        if not (path.is_file() and os.access(path, os.X_OK)):
            return f"not an executable: {path}"
    return None


def run(browser: str | None, test_filter: str | None, update: bool) -> None:
    """Builds the artifacts once, then runs each planned browser's lifecycle.

    Raises StepFailed: 1 when a step failed, 128+n after a termination request.
    """
    lifecycles = plan(browser, test_filter, update)
    summary = Report()
    refused = refusal(update, sys.platform, os.environ)
    if refused is not None:
        summary.record(refused[0], time.time_ns(), False, refused[1])
        summary.print()
        raise steps.StepFailed(1, signalled=False)
    signals = Signals()
    root = Path.cwd()
    with contextlib.ExitStack() as stack:
        provenance = Provenance()
        succeeded = False
        # Reads `succeeded` when the stack closes, so it records the outcome
        # assigned below, not this initial False.
        stack.callback(lambda: provenance.close(succeeded))
        attribute = "sandboxArtifactsReleaseCsr" if update else "sandboxArtifacts"
        admit = steps.tool(ADMIT_INPUTS)
        steps.run("e2e-local-admission", [*admit, f".#{attribute}.sourceAdmits"])
        # A root of this run's own, so a concurrent run never repoints it;
        # removed on every exit, whether or not the build made it.
        link = f"{steps.GCROOTS}/e2e-local-{os.getpid()}-{secrets.token_hex(4)}"
        stack.callback(Path(link).unlink, missing_ok=True)
        steps.nix_build(
            "e2e-local-build",
            f".#{attribute}",
            (*steps.NIX_FLAGS, *steps.NIX_LOCK_FLAGS),
            link=link,
            refusal=lambda _: _unselectable(link),
        )
        selected = root / link / "bin"
        shared = _Run(
            root=root,
            jaunder=(selected / "jaunder").resolve(),
            test_support=(selected / "test-support").resolve(),
            path=f"{selected}{os.pathsep}{os.environ.get('PATH', os.defpath)}",
            workers=int(os.environ.get("JAUNDER_E2E_WORKERS", "1")),
            signals=signals,
            report=summary,
        )
        print(
            f"Selected {shared.jaunder} and {shared.test_support}; retained at {link}",
            file=sys.stderr,
        )
        signals.install(stack)
        for lifecycle in lifecycles:
            if signals.code is not None:
                break
            _run_lifecycle(shared, lifecycle)
        succeeded = summary.ok and signals.code is None
    summary.print()
    if signals.code is not None:
        raise steps.StepFailed(signals.code, signalled=True)
    if not summary.ok:
        raise steps.StepFailed(1, signalled=False)
