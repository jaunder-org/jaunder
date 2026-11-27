"""Gates and their steps: each step is one traced child process.

A gate stops at the first failing step and exits with that step's status. A
child killed by signal n reports 128+n, as a shell would; Python reports such a
child's return code as -n.

Termination requests sent to `dev` alone are handled for the whole gate. While
a step's child runs, SIGTERM and SIGHUP are forwarded to it and `dev` waits for
it; a terminal's SIGINT already reaches the whole foreground process group, so
`dev` only waits. At any other point (between steps, or while a Nix check
queries its output) the gate stops at once. Either way the open spans close as
`signalled`, the merged trace is written and `dev` exits with 128+n.
"""

import contextlib
import json
import os
import shlex
import signal
import subprocess
import sys
import time
from collections.abc import Callable, Iterator, Mapping, Sequence
from pathlib import Path
from types import FrameType
from typing import Any, NoReturn

from opentelemetry.trace import Span

from dev import trace
from dev.commands import Tool
from dev.evidence import EvidenceError
from dev.report import render_report
from dev.spans import (
    CACHE,
    COMMAND_LINE,
    EXIT_CODE,
    FAILED_REASON,
    FAILED_STEP,
    FETCHED_BYTES,
    KACHE_AVOIDED_COMPILE_MS,
    KACHE_HITS,
    KACHE_MISSES,
    NIX_OUTPUT,
    NIX_RETRIED,
    STATUS,
    Cache,
    Status,
)

_HANDLED = (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)

# Changes to one step's environment; None unsets a name.
Environment = Mapping[str, str | None]

# What a Git hook exports to point Git at the repository being committed. No
# step inherits them: they would redirect any Git a step runs, including a
# test's `git init` of a scratch repository, at that repository. Only the
# index custody of `dev precommit`/`dev prepush` (`index`) honours them.
GIT_OVERRIDES = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
)
WITHOUT_GIT_OVERRIDES: Environment = dict.fromkeys(GIT_OVERRIDES)

# The output-holding GC roots of Nix builds, under the checkout root.
GCROOTS = ".xtask/gcroots"
# The flags a Nix build takes unless its caller names others.
NIX_FLAGS = ("--accept-flake-config",)
# Flake commands that must leave flake.lock exactly as committed.
NIX_LOCK_FLAGS = ("--no-update-lock-file", "--no-write-lock-file")


def changed(base: Mapping[str, str], changes: Environment | None) -> dict[str, str]:
    """`base` with `changes` applied."""
    environment = dict(base)
    for key, value in (changes or {}).items():
        if value is None:
            environment.pop(key, None)
        else:
            environment[key] = value
    return environment


class StepFailed(Exception):
    """A step's failure; `reason`, when given, says why better than its status."""

    def __init__(self, code: int, signalled: bool, reason: str | None = None) -> None:
        super().__init__(code)
        self.code = code
        self.signalled = signalled
        self.reason = reason


def refuse(message: str) -> NoReturn:
    """Fails the current step or phase with status 1, saying why on stderr."""
    print(message, file=sys.stderr)
    raise StepFailed(1, signalled=False, reason=message)


class _Interrupted(StepFailed):
    """A termination request that arrived while no step's child was running."""

    def __init__(self, signum: int) -> None:
        super().__init__(128 + signum, signalled=True)


class _Signals:
    """Where the running gate is, for its signal handler.

    Outside the gate's body (opening the run, writing the merged trace) a
    request is remembered and ends the gate once the trace is written.
    """

    def __init__(self) -> None:
        self.armed = False
        self.deferred: list[int] = []
        # Set while a step's child is being started or waited for.
        self.supervising = False
        self.child: subprocess.Popen[bytes] | None = None
        self.forwarded: list[int] = []
        # Whether any request, SIGINT included, arrived while the child ran: its
        # exit is then no longer its own verdict, so it is never run again.
        self.interrupted = False
        # Requests that arrived while the child was starting, relayed once it has.
        self.pending: list[int] = []

    def handle(self, signum: int, _frame: FrameType | None) -> None:
        if self.supervising:
            self.interrupted = True
            if signum == signal.SIGINT:
                return
            self.forwarded.append(signum)
            if self.child is None:
                self.pending.append(signum)
            else:
                self.child.send_signal(signum)
        elif self.armed:
            raise _Interrupted(signum)
        else:
            self.deferred.append(signum)


_signals = _Signals()


REASON_LIMIT = 256


def describe(error: BaseException) -> str:
    """An exception as a failure reason: its message, or its type without one."""
    return str(error) or type(error).__name__


# The first span of this process's gate to end unsuccessfully, and why: the
# gate span names it, so a kept trace says where a failed gate stopped.
_failure: tuple[str | None, str] | None = None


def _note_failure(
    name: str | None, code: int, signalled: bool, reason: str | None
) -> tuple[str | None, str]:
    """Remembers the gate's first failure, if it has none yet; returns that one."""
    global _failure
    if _failure is None:
        if reason is None:
            reason = f"signalled {code - 128}" if signalled else f"exited {code}"
        _failure = (name, trace.cap(reason.strip().partition("\n")[0], REASON_LIMIT))
    return _failure


def _set_outcome(span: Span, code: int, signalled: bool) -> bool:
    """Records how `span` ended; returns whether it succeeded."""
    span.set_attribute(EXIT_CODE, code)
    status = Status.SIGNALLED if signalled else Status.FAILED if code else Status.OK
    span.set_attribute(STATUS, status)
    return status is Status.OK


def _record_outcome(
    span: Span, name: str, code: int, signalled: bool, reason: str | None = None
) -> None:
    if not _set_outcome(span, code, signalled):
        _note_failure(name, code, signalled, reason)


def _record_gate_outcome(
    span: Span, code: int, signalled: bool, reason: str | None
) -> None:
    if _set_outcome(span, code, signalled):
        return
    # A failure no span recorded, such as a refusal between steps, still
    # gives its reason.
    step, why = _note_failure(None, code, signalled, reason)
    if step is not None:
        span.set_attribute(FAILED_STEP, step)
    span.set_attribute(FAILED_REASON, why)


def gate(root: Path, name: str, body: Callable[[], None]) -> int:
    """Runs `body`'s steps under the gate span `name`; returns the exit status."""
    global _signals, _failure
    _signals = _Signals()
    _failure = None
    previous = {number: signal.signal(number, _signals.handle) for number in _HANDLED}
    try:
        with trace.run(root, name) as span:
            code, signalled, reason = 0, False, None
            try:
                if _signals.deferred:
                    raise _Interrupted(_signals.deferred[0])
                _signals.armed = True
                body()
            except StepFailed as failure:
                code, signalled, reason = (
                    failure.code,
                    failure.signalled,
                    failure.reason,
                )
            except Exception as defect:
                # A defect, not a verdict: the gate still ends failed in the
                # trace, and the traceback and Python's status 1 reach the
                # caller unchanged.
                _record_gate_outcome(span, 1, False, describe(defect))
                raise
            finally:
                _signals.armed = False
            _record_gate_outcome(span, code, signalled, reason)
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)
    if not code and _signals.deferred:
        return 128 + _signals.deferred[0]
    return code


def _supervise(
    argv: Sequence[str],
    capture: bool,
    environment: Environment | None = None,
    stdin: bytes | None = None,
    capture_stderr: bool = False,
) -> tuple[int, bool, bytes, bytes]:
    """Runs the child to completion, relaying termination requests to it.

    With `capture`, the child's stdout is returned instead of passed through,
    and with `capture_stderr` its stderr likewise; `stdin`, if given, is the child's
    whole standard input.
    """
    child_environment = changed(
        changed(trace.child_environment(), WITHOUT_GIT_OVERRIDES), environment
    )
    _signals.supervising = True
    _signals.forwarded = []
    _signals.interrupted = False
    _signals.pending = []
    try:
        try:
            child = subprocess.Popen(
                argv,
                env=child_environment,
                stdin=None if stdin is None else subprocess.PIPE,
                stdout=subprocess.PIPE if capture else None,
                stderr=subprocess.PIPE if capture_stderr else None,
            )
        except FileNotFoundError as error:
            print(f"dev: {argv[0]}: {error.strerror}", file=sys.stderr, flush=True)
            return 127, False, b"", b""
        except OSError as error:
            # Found but not runnable (not executable, bad interpreter): a
            # shell's 126.
            print(f"dev: {argv[0]}: {error.strerror}", file=sys.stderr, flush=True)
            return 126, False, b"", b""
        _signals.child = child
        for signum in _signals.pending:
            child.send_signal(signum)
        output, diagnostics = child.communicate(stdin)
    finally:
        _signals.supervising = False
        _signals.child = None
    output, diagnostics = output or b"", diagnostics or b""
    if child.returncode < 0:
        return 128 - child.returncode, True, output, diagnostics
    if child.returncode == 0 and _signals.forwarded:
        # The request was for the whole gate, not just this step: a child that
        # handles it by exiting cleanly must not let the next step start.
        return 128 + _signals.forwarded[0], True, output, diagnostics
    return child.returncode, False, output, diagnostics


@contextlib.contextmanager
def _step(name: str, argv: Sequence[str]) -> Iterator[Span]:
    with trace.tracer().start_as_current_span(name) as span:
        span.set_attribute(COMMAND_LINE, trace.cap(shlex.join(argv)))
        try:
            yield span
        except _Interrupted as interrupted:
            _record_outcome(span, name, interrupted.code, True)
            raise


@contextlib.contextmanager
def phase(name: str) -> Iterator[None]:
    """One span for gate work that is not itself a step's child process.

    It ends `ok`; a step failure or termination request inside it ends it as
    that step's outcome, and any other exception ends it `failed`.
    """
    with trace.tracer().start_as_current_span(name) as span:
        try:
            yield
        except StepFailed as failure:
            _record_outcome(span, name, failure.code, failure.signalled, failure.reason)
            raise
        except Exception as error:
            span.set_attribute(STATUS, Status.FAILED)
            _note_failure(name, 1, False, describe(error))
            raise
        span.set_attribute(STATUS, Status.OK)


def verdict(
    name: str,
    decide: Callable[[], str],
    refused: Callable[[EvidenceError], int] = lambda _error: 1,
) -> None:
    """Runs one in-process verdict as the step `name` and shows its result.

    `decide` returns the step's detail, having written any report of its own
    first. Evidence it refuses fails the step with the status `refused` gives
    that refusal, as a failed tool's status would.
    """
    started_ns = time.time_ns()
    with phase(name):
        try:
            detail, code = decide(), 0
        except EvidenceError as error:
            detail, code = str(error), refused(error)
        duration_ms = (time.time_ns() - started_ns) // 1_000_000
        step = {
            "name": name,
            "ok": not code,
            "duration_ms": duration_ms,
            "detail": detail,
        }
        sys.stdout.write(render_report({"steps": [step]}))
        sys.stdout.flush()
        if code:
            raise StepFailed(code, signalled=False, reason=detail)


# Judges a successful step's captured output: the reason to refuse it, or None.
Refusal = Callable[[bytes], str | None]


def _run(
    name: str,
    argv: Sequence[str],
    capture: bool,
    environment: Environment | None,
    render: Callable[[bytes], str | None] | None = None,
    stdin: bytes | None = None,
    refusal: Refusal | None = None,
    realizes: Callable[[], str] | None = None,
    finished: Callable[[Span, int], None] | None = None,
    verdict: bool = False,
    nix_retry: bool = False,
    compiles: bool = False,
) -> bytes:
    with _step(name, argv) as span:
        tree = _kache_tree(span, environment) if compiles else None
        if tree is not None:
            environment = {**(environment or {}), KACHE_EVENT_ROOT: tree}
        started_ns = time.time_ns()
        start = started_ns // 1_000_000_000
        code, signalled, output, _ = _supervise(argv, capture, environment, stdin)
        if nix_retry and code == 1 and not signalled and not _signals.interrupted:
            print(
                f"dev: {name}: Nix exited 1 without a builder failing, as when a"
                " substitute download fails; running it once more",
                file=sys.stderr,
                flush=True,
            )
            span.set_attribute(NIX_RETRIED, True)
            code, signalled, output, _ = _supervise(argv, capture, environment, stdin)
        why: str | None = None
        if render is not None:
            rendered = render(output)
            sys.stdout.buffer.write(output if rendered is None else rendered.encode())
            sys.stdout.flush()
            if rendered is None and not code:
                # A tool that succeeds without its report has not shown it
                # passed. A failed tool's own status stays the verdict.
                why = "output is not a step report"
                print(f"dev: {name}: {why}", file=sys.stderr)
                sys.stderr.flush()
                code = 1
        if refusal is not None and not code:
            # Judged inside the step's span, so the trace shows which step's
            # output stopped the gate.
            why = refusal(output)
            if why is not None:
                print(why, file=sys.stderr, flush=True)
                code = 1
        if (
            verdict
            and realizes is not None
            and not code
            and _reported_not_executed_here(name, realizes())
        ):
            why, code = "verdict not executed here", 1
        _record_outcome(span, name, code, signalled, why)
        if finished is not None:
            finished(span, started_ns)
        if realizes is not None and not code:
            with contextlib.suppress(_QueryFailed):
                _record_cache(span, realizes(), start)
        # A failed build's counts still say what it compiled; a signalled
        # one's are cut short, and querying would only delay the exit.
        if tree is not None and not signalled:
            with contextlib.suppress(_QueryFailed):
                _record_kache(span, tree)
    if code:
        raise StepFailed(code, signalled, why)
    return output


def run(
    name: str,
    argv: Sequence[str],
    render: Callable[[bytes], str | None] | None = None,
    environment: Environment | None = None,
    realizes: Callable[[], str] | None = None,
    finished: Callable[[Span, int], None] | None = None,
    verdict: bool = False,
    compiles: bool = False,
) -> None:
    """Runs one step; raises StepFailed so the gate stops at the first failure.

    With `render`, the step's stdout is shown as `render` presents it, whatever
    the step's status; output it cannot render is shown unchanged and fails a
    step that otherwise succeeded. `environment` changes only this step's
    environment. `realizes`, for a step that runs a Nix build itself, returns
    the output path that build selected once the step succeeds; the step then
    records its `jaunder.cache` as `nix_build` does. `finished`, whatever the
    step's outcome, is called once the outcome is recorded, with the still-open
    step span and the wall-clock nanosecond at which the step started, to add
    what the child left behind. `verdict`, with `realizes`, fails a successful
    step whose output this machine did not build, as for `nix_build`.
    `compiles` marks a Cargo step that compiles: under kache it records what
    kache served and compiled for it (`_kache_tree`).
    """
    _run(
        name,
        argv,
        render is not None,
        environment,
        render,
        realizes=realizes,
        finished=finished,
        verdict=verdict,
        compiles=compiles,
    )


def output(
    name: str,
    argv: Sequence[str],
    environment: Environment | None = None,
    stdin: bytes | None = None,
) -> bytes:
    """Runs one step for its stdout, which the user never sees; as `run` otherwise.

    `stdin`, if given, is the step's whole standard input.
    """
    return _run(name, argv, True, environment, stdin=stdin)


def captured(name: str, argv: Sequence[str]) -> tuple[int, bytes, bytes]:
    """Runs one step for its status, stdout and stderr, which the user never sees.

    Unlike `output`, a failing child is an answer for the caller to judge, not
    the end of the gate; only a termination request still stops it.
    """
    with _step(name, argv) as span:
        code, signalled, output, diagnostics = _supervise(
            argv, True, capture_stderr=True
        )
        # A failure the caller judges is not the gate's failure unless the
        # caller says so; a termination request still is.
        if not _set_outcome(span, code, signalled) and signalled:
            _note_failure(name, code, signalled, None)
    if signalled:
        raise StepFailed(code, signalled)
    return code, output, diagnostics


def _shell_text(output: bytes) -> str:
    return output.decode(errors="replace").rstrip("\n")


def text(
    name: str,
    argv: Sequence[str],
    environment: Environment | None = None,
    refusal: Callable[[str], str | None] | None = None,
    finished: Callable[[Span, int], None] | None = None,
    nix_retry: bool = False,
) -> str:
    """A step's stdout as a shell's `$(...)` gives it: trailing newlines removed.

    `refusal`, if given, judges that text: a reason it returns is printed and
    fails the step with status 1. `finished` is as for `run`.

    With `nix_retry`, a Nix child that exits 1 runs once more within the same
    step, unless a signal arrived while it ran. Nix exits 100 to 104 when a
    builder failed and 1 when none did: a substitute download that failed
    (#2135) is one such failure, and its second attempt starts the download
    afresh. A persistent failure fails both.
    """
    judge = None if refusal is None else lambda output: refusal(_shell_text(output))
    return _shell_text(
        _run(
            name,
            argv,
            True,
            environment,
            refusal=judge,
            finished=finished,
            nix_retry=nix_retry,
        )
    )


class _QueryFailed(Exception):
    """A Nix query failed, so the step's cache source stays unrecorded."""


def _query(argv: Sequence[str], stdin: bytes | None = None) -> bytes:
    """A Nix query's stdout. Its streams never reach the user."""
    try:
        done = subprocess.run(
            argv,
            env=changed(os.environ, WITHOUT_GIT_OVERRIDES),
            input=stdin,
            capture_output=True,
            check=False,
        )
    except OSError as error:
        raise _QueryFailed from error
    if done.returncode:
        raise _QueryFailed
    return done.stdout


# `nix path-info` with a JSON shape that stays stable across Nix releases.
_PATH_INFO = ("nix", "path-info", "--json", "--json-format", "1")


def _path_info(argv: Sequence[str], stdin: bytes | None = None) -> dict[str, Any]:
    """Store metadata keyed by store path; an invalid path maps to null."""
    try:
        listed: dict[str, Any] = json.loads(_query(argv, stdin))
    except ValueError as error:
        raise _QueryFailed from error
    return listed


def _store_entry(path: str) -> dict[str, Any] | None:
    """`path`'s store metadata, or None while the path is not valid."""
    entry: dict[str, Any] | None = _path_info([*_PATH_INFO, "--", path]).get(path)
    return entry


def substituted_size(entry: Mapping[str, Any] | None, start: int) -> int:
    """The bytes a substituter supplied for this path since the second `start`.

    A path registered before `start` was already valid; an `ultimate` one was
    built here, fixed-output downloads included. The size is the unpacked NAR.
    """
    registered = None if entry is None else entry.get("registrationTime")
    size = None if entry is None else entry.get("narSize")
    # bool is an int to Python, never to `nix path-info`.
    if entry is None or type(registered) is not int or type(size) is not int:
        return 0
    return 0 if registered < start or entry.get("ultimate") else size


def _fetched_bytes(derivation: str, start: int) -> int:
    """What the build of `derivation` fetched: its closure's substituted paths."""
    closure = _query(
        ["nix-store", "--query", "--requisites", "--include-outputs", derivation]
    )
    # The evaluation wrote the derivations; they were never fetched.
    paths = [path for path in closure.decode().split() if not path.endswith(".drv")]
    listed = _path_info([*_PATH_INFO, "--stdin"], "\n".join(paths).encode())
    return sum(substituted_size(entry, start) for entry in listed.values())


def output_source(entry: Mapping[str, Any] | None, start: int) -> Cache | None:
    """Where a built output came from, by when the store registered it.

    `entry` is the output's `nix path-info` entry after the build, `start` the
    wall-clock second at which the build started. A path registered before that
    second was already valid; one registered after it is new, `built` here if
    `ultimate` and otherwise substituted. In the start second itself, or with no
    usable registration time, it cannot tell: None.
    """
    registered = None if entry is None else entry.get("registrationTime")
    # bool is an int to Python, never to `nix path-info`.
    if entry is None or type(registered) is not int or registered == start:
        return None
    if registered < start:
        return Cache.REUSED
    return Cache.BUILT if entry.get("ultimate") else Cache.SUBSTITUTED


def _record_cache(span: Span, path: str, start: int) -> Cache | None:
    """Records `jaunder.cache` for a successful step's output, if it can tell.

    Returns what it recorded; a failed query raises `_QueryFailed`.
    """
    cache = output_source(_store_entry(path), start)
    if cache is not None:
        span.set_attribute(CACHE, cache)
    return cache


# kache labels each compile's event with a build tree, and one session covers a
# tree's Cargo commands until it is idle for five minutes. A compiling step's
# events carry a tree of their own, under the checkout and never created, so its
# session is exactly its compiles. kache's own tools then see each step run as
# its own tree (docs/adr/drafts/dev-steps-label-their-kache-events.md).
KACHE_EVENT_ROOT = "KACHE_EVENT_ROOT"
KACHE_TREES = ".xtask/kache"
# The `kache stats --json --last-build` summary fields a step records: three
# kinds of hit, the misses, and the compile time kache estimates its hits
# avoided (summed over parallel compiles, so not the step's wall time).
_KACHE_SUMMARY = (
    "local_hits",
    "prefetch_hits",
    "remote_hits",
    "misses",
    "time_saved_ms",
)


def _kache_tree(span: Span, environment: Environment | None) -> str | None:
    """The build tree for a compiling step's kache events, or None when kache
    does not compile for it, as in a sandbox."""
    if changed(os.environ, environment).get("RUSTC_WRAPPER") != "kache":
        return None
    context = span.get_span_context()
    return str(
        Path.cwd() / KACHE_TREES / f"{context.trace_id:032x}-{context.span_id:016x}"
    )


def kache_counts(stats: bytes) -> dict[str, int] | None:
    """A step's kache attributes from its `kache stats --json --last-build`
    report, or None for a report of any other shape."""
    try:
        summary = json.loads(stats)["report"]["summary"]
        values = [summary[field] for field in _KACHE_SUMMARY]
    except ValueError, LookupError, TypeError:
        return None
    # bool is an int to Python, never to kache.
    if any(type(value) is not int for value in values):
        return None
    local, prefetch, remote, misses, avoided = values
    return {
        KACHE_HITS: local + prefetch + remote,
        KACHE_MISSES: misses,
        KACHE_AVOIDED_COMPILE_MS: avoided,
    }


def _record_kache(span: Span, tree: str) -> None:
    """Records what kache served and compiled for the step whose events carry
    `tree`, if it can tell; a failed query raises `_QueryFailed`.

    A step that compiled nothing, or ran with KACHE_DISABLED, left no events:
    kache refuses the empty selection and nothing is recorded.
    """
    report = _query(["kache", "stats", "--json", "--last-build", "--root", tree])
    counts = kache_counts(report)
    if counts is not None:
        span.set_attributes(counts)


def _reported_not_executed_here(name: str, path: str) -> bool:
    """Whether the verdict `name`'s output `path` fails to show it ran here.

    ADR-0192 needs every tested commit to execute its Rust coverage and e2e
    verdicts. Only an `ultimate` output was built by this machine's store, in
    this run or an earlier one; a substituted, missing or unreadable one is
    reported on stderr, naming the verdict, and fails.
    """
    try:
        entry = _store_entry(path)
    except _QueryFailed:
        entry = None
    if entry is not None and entry.get("ultimate") is True:
        return False
    print(
        f"dev: {name}: verdict not executed here: this machine's store did not"
        f" build {path} (`nix path-info` does not report it ultimate)",
        file=sys.stderr,
        flush=True,
    )
    return True


def nix_build(
    name: str,
    installable: str,
    flags: Sequence[str] = NIX_FLAGS,
    keep: bool = True,
    refusal: Callable[[str], str | None] | None = None,
    finished: Callable[[Span, int], None] | None = None,
    link: str | None = None,
    verdict: bool = False,
) -> str:
    """Builds one installable as the step `name`; returns its output path.

    A flake installable is evaluated first, as the child step `eval`, which
    leaves the evaluation cached for the child step `build`: the flake is
    evaluated once, and the evaluation is timed. A derivation path (one that
    starts with `/`) has nothing to evaluate. `keep` holds the output as the GC
    root `link`, by default GCROOTS/`name`: a caller that may run concurrently
    with itself names its own. `refusal`, if given, judges the printed output as
    `text` judges its text.

    For the one output path a build printed, the step then records it, where it
    came from (`jaunder.cache`, by `output_source`) and, unless it was reused,
    the bytes the build fetched from substituters. Several printed paths, an
    undecidable registration or a failed query record less rather than guess,
    and never change the step.

    `finished` is as for `run`, for the `build` step: what the builder timed
    itself belongs under it. A build that exits 1, with no builder failed, runs
    once more as `text`'s `nix_retry` says; the `build` span then records
    `jaunder.nix.retried`.

    A `verdict` must execute here: the step fails, naming it, unless the build
    printed one output that this machine's store built (`ultimate`).
    """
    root = link or f"{GCROOTS}/{name}"
    if keep:
        Path(root).parent.mkdir(parents=True, exist_ok=True)
    out_link = ["--out-link", root] if keep else ["--no-link"]
    argv = ["nix", "build", "-L", *flags, "--print-out-paths", *out_link, installable]
    with trace.tracer().start_as_current_span(name) as span:
        try:
            if installable.startswith("/"):
                derivation = installable.partition("^")[0]
            else:
                evaluate = ["nix", "path-info", "--derivation", *flags, installable]
                derivation = text("eval", evaluate)
            # After the evaluation, which registers what it writes.
            start = int(time.time())
            paths = text(
                "build", argv, refusal=refusal, finished=finished, nix_retry=True
            )
            if verdict and _reported_not_executed_here(name, paths):
                raise StepFailed(1, False, "verdict not executed here")
            _record_outcome(span, name, 0, False)
            if len(paths.splitlines()) == 1:
                span.set_attribute(NIX_OUTPUT, paths)
                with contextlib.suppress(_QueryFailed):
                    if _record_cache(span, paths, start) is not Cache.REUSED:
                        fetched = _fetched_bytes(derivation, start)
                        span.set_attribute(FETCHED_BYTES, fetched)
        except StepFailed as failure:
            _record_outcome(span, name, failure.code, failure.signalled, failure.reason)
            raise
    return paths


# The output path of each tool package this process has built.
_packages: dict[str, str] = {}


def tool(wanted: Tool) -> list[str]:
    """The argv that runs `wanted`, its package built first as `<package>-build`.

    A process builds each package once, however many of its binaries it runs.
    """
    if wanted.package not in _packages:
        _packages[wanted.package] = nix_build(
            f"{wanted.package}-build", f".#{wanted.package}", keep=False
        )
    return [f"{_packages[wanted.package]}/bin/{wanted.binary}"]


def nix_check(
    check: str,
    flags: Sequence[str] = NIX_FLAGS,
    keep: bool = True,
    finished: Callable[[Span, int], None] | None = None,
    verdict: bool = False,
) -> None:
    """Builds one flake check as `nix_build` does, as the step `check`."""
    nix_build(
        check,
        f".#checks.x86_64-linux.{check}",
        flags,
        keep,
        finished=finished,
        verdict=verdict,
    )
