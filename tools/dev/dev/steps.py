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
from typing import Any

from opentelemetry.trace import Span

from dev import trace
from dev.spans import CACHE, COMMAND_LINE, EXIT_CODE, STATUS, Cache, Status

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
    def __init__(self, code: int, signalled: bool) -> None:
        super().__init__(code)
        self.code = code
        self.signalled = signalled


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
        # Requests that arrived while the child was starting, relayed once it has.
        self.pending: list[int] = []

    def handle(self, signum: int, _frame: FrameType | None) -> None:
        if self.supervising:
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


def _record_outcome(span: Span, code: int, signalled: bool) -> None:
    span.set_attribute(EXIT_CODE, code)
    status = Status.SIGNALLED if signalled else Status.FAILED if code else Status.OK
    span.set_attribute(STATUS, status)


def gate(root: Path, name: str, body: Callable[[], None]) -> int:
    """Runs `body`'s steps under the gate span `name`; returns the exit status."""
    global _signals
    _signals = _Signals()
    previous = {number: signal.signal(number, _signals.handle) for number in _HANDLED}
    try:
        with trace.run(root, name) as span:
            code, signalled = 0, False
            try:
                if _signals.deferred:
                    raise _Interrupted(_signals.deferred[0])
                _signals.armed = True
                body()
            except StepFailed as failure:
                code, signalled = failure.code, failure.signalled
            except Exception:
                # A defect, not a verdict: the gate still ends failed in the
                # trace, and the traceback and Python's status 1 reach the
                # caller unchanged.
                _record_outcome(span, 1, False)
                raise
            finally:
                _signals.armed = False
            _record_outcome(span, code, signalled)
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
) -> tuple[int, bool, bytes]:
    """Runs the child to completion, relaying termination requests to it.

    With `capture`, the child's stdout is returned instead of passed through;
    `stdin`, if given, is the child's whole standard input.
    """
    child_environment = changed(
        changed(trace.child_environment(), WITHOUT_GIT_OVERRIDES), environment
    )
    _signals.supervising = True
    _signals.forwarded = []
    _signals.pending = []
    try:
        try:
            child = subprocess.Popen(
                argv,
                env=child_environment,
                stdin=None if stdin is None else subprocess.PIPE,
                stdout=subprocess.PIPE if capture else None,
            )
        except FileNotFoundError as error:
            print(f"dev: {argv[0]}: {error.strerror}", file=sys.stderr, flush=True)
            return 127, False, b""
        except OSError as error:
            # Found but not runnable (not executable, bad interpreter): a
            # shell's 126.
            print(f"dev: {argv[0]}: {error.strerror}", file=sys.stderr, flush=True)
            return 126, False, b""
        _signals.child = child
        for signum in _signals.pending:
            child.send_signal(signum)
        output, _ = child.communicate(stdin)
    finally:
        _signals.supervising = False
        _signals.child = None
    output = output or b""
    if child.returncode < 0:
        return 128 - child.returncode, True, output
    if child.returncode == 0 and _signals.forwarded:
        # The request was for the whole gate, not just this step: a child that
        # handles it by exiting cleanly must not let the next step start.
        return 128 + _signals.forwarded[0], True, output
    return child.returncode, False, output


@contextlib.contextmanager
def _step(name: str, argv: Sequence[str]) -> Iterator[Span]:
    with trace.tracer().start_as_current_span(name) as span:
        span.set_attribute(COMMAND_LINE, trace.cap_command_line(shlex.join(argv)))
        try:
            yield span
        except _Interrupted as interrupted:
            _record_outcome(span, interrupted.code, True)
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
            _record_outcome(span, failure.code, failure.signalled)
            raise
        except Exception:
            span.set_attribute(STATUS, Status.FAILED)
            raise
        span.set_attribute(STATUS, Status.OK)


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
) -> bytes:
    with _step(name, argv) as span:
        code, signalled, output = _supervise(argv, capture, environment, stdin)
        if render is not None:
            rendered = render(output)
            sys.stdout.buffer.write(output if rendered is None else rendered.encode())
            sys.stdout.flush()
            if rendered is None and not code:
                # A tool that succeeds without its report has not shown it
                # passed. A failed tool's own status stays the verdict.
                print(f"dev: {name}: output is not a step report", file=sys.stderr)
                sys.stderr.flush()
                code = 1
        if refusal is not None and not code:
            # Judged inside the step's span, so the trace shows which step's
            # output stopped the gate.
            reason = refusal(output)
            if reason is not None:
                print(reason, file=sys.stderr, flush=True)
                code = 1
        _record_outcome(span, code, signalled)
    if code:
        raise StepFailed(code, signalled)
    return output


def run(
    name: str,
    argv: Sequence[str],
    render: Callable[[bytes], str | None] | None = None,
    environment: Environment | None = None,
) -> None:
    """Runs one step; raises StepFailed so the gate stops at the first failure.

    With `render`, the step's stdout is shown as `render` presents it, whatever
    the step's status; output it cannot render is shown unchanged and fails a
    step that otherwise succeeded. `environment` changes only this step's
    environment.
    """
    _run(name, argv, render is not None, environment, render)


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


def _shell_text(output: bytes) -> str:
    return output.decode(errors="replace").rstrip("\n")


def text(
    name: str,
    argv: Sequence[str],
    environment: Environment | None = None,
    refusal: Callable[[str], str | None] | None = None,
) -> str:
    """A step's stdout as a shell's `$(...)` gives it: trailing newlines removed.

    `refusal`, if given, judges that text: a reason it returns is printed and
    fails the step with status 1.
    """
    judge = None if refusal is None else lambda output: refusal(_shell_text(output))
    return _shell_text(_run(name, argv, True, environment, refusal=judge))


class _QueryFailed(Exception):
    """A Nix query failed, so the step's cache source stays unrecorded."""


def _query(*argv: str) -> bytes:
    """A Nix query's stdout; its streams never reach the user."""
    try:
        done = subprocess.run(
            argv,
            env=changed(os.environ, WITHOUT_GIT_OVERRIDES),
            capture_output=True,
            check=False,
        )
    except OSError as error:
        raise _QueryFailed from error
    if done.returncode:
        raise _QueryFailed
    return done.stdout


def _store_entry(path: str) -> dict[str, Any] | None:
    """`path`'s store metadata, or None while the path is not valid."""
    try:
        listed = json.loads(
            _query("nix", "path-info", "--json", "--json-format", "1", "--", path)
        )
    except ValueError as error:
        raise _QueryFailed from error
    # An object keyed by store path; an invalid path maps to null.
    entry: dict[str, Any] | None = listed.get(path)
    return entry


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


def nix_build(
    name: str,
    installable: str,
    flags: Sequence[str] = NIX_FLAGS,
    keep: bool = True,
    refusal: Callable[[str], str | None] | None = None,
) -> str:
    """Builds one flake installable as the step `name`; returns its output path.

    `keep` holds the output as the GC root GCROOTS/`name`. The build is the
    step's only evaluation. Afterwards the step records `jaunder.cache` for the
    one output path the build printed, judged by `output_source`. Several
    printed paths, an undecidable registration or a failed query record nothing
    rather than guess, and never change the step. `refusal`, if given, judges
    the printed output as `text` judges its text.
    """
    if keep:
        Path(GCROOTS).mkdir(parents=True, exist_ok=True)
    link = ["--out-link", f"{GCROOTS}/{name}"] if keep else ["--no-link"]
    argv = ["nix", "build", "-L", *flags, "--print-out-paths", *link, installable]
    with _step(name, argv) as span:
        start = int(time.time())
        code, signalled, printed = _supervise(argv, capture=True)
        built = _shell_text(printed)
        if refusal is not None and not code:
            reason = refusal(built)
            if reason is not None:
                print(reason, file=sys.stderr, flush=True)
                code = 1
        _record_outcome(span, code, signalled)
        if code == 0 and len(built.splitlines()) == 1:
            with contextlib.suppress(_QueryFailed):
                cache = output_source(_store_entry(built), start)
                if cache is not None:
                    span.set_attribute(CACHE, cache)
    if code:
        raise StepFailed(code, signalled)
    return built


def nix_check(
    check: str,
    flags: Sequence[str] = NIX_FLAGS,
    keep: bool = True,
) -> None:
    """Builds one flake check as `nix_build` does, as the step `check`."""
    nix_build(check, f".#checks.x86_64-linux.{check}", flags, keep)
