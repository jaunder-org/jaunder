"""Gates and their steps: each step is one traced child process.

A gate stops at the first failing step and exits with that step's status. A
child killed by signal n reports 128+n, as a shell would; Python reports such a
child's return code as -n.
"""

import contextlib
import json
import shlex
import signal
import subprocess
import sys
from collections.abc import Callable, Iterator, Sequence
from pathlib import Path
from types import FrameType
from typing import Any

from opentelemetry.trace import Span

from jaunder_dev import trace
from jaunder_dev.spans import CACHE, COMMAND_LINE, EXIT_CODE, STATUS


class StepFailed(Exception):
    def __init__(self, code: int, signalled: bool) -> None:
        super().__init__(code)
        self.code = code
        self.signalled = signalled


def _record_outcome(span: Span, code: int, signalled: bool) -> None:
    span.set_attribute(EXIT_CODE, code)
    span.set_attribute(STATUS, "signalled" if signalled else "failed" if code else "ok")


def gate(root: Path, name: str, body: Callable[[], None]) -> int:
    """Runs `body`'s steps under the gate span `name`; returns the exit status."""
    with trace.run(root, name) as span:
        try:
            body()
        except StepFailed as failure:
            _record_outcome(span, failure.code, failure.signalled)
            return failure.code
        _record_outcome(span, 0, False)
        return 0


def _supervise(argv: Sequence[str], capture: bool) -> tuple[int, bool, bytes]:
    """Runs the child to completion, relaying a termination request to it.

    With `capture`, the child's stdout is returned instead of passed through.

    SIGTERM or SIGHUP sent to `dev` alone is forwarded. A terminal's SIGINT
    already reaches the whole foreground process group, so `dev` only keeps
    waiting for the child to finish. If the child still exits cleanly after a
    forwarded signal, the gate stops anyway with that signal's status.
    """
    child: subprocess.Popen[bytes] | None = None
    received: list[int] = []

    def forward(signum: int, _frame: FrameType | None) -> None:
        received.append(signum)
        if child is not None:
            child.send_signal(signum)

    def hold(_signum: int, _frame: FrameType | None) -> None:
        pass

    previous = {
        number: signal.signal(number, handler)
        for number, handler in (
            (signal.SIGTERM, forward),
            (signal.SIGHUP, forward),
            (signal.SIGINT, hold),
        )
    }
    try:
        try:
            child = subprocess.Popen(
                argv,
                env=trace.child_environment(),
                stdout=subprocess.PIPE if capture else None,
            )
        except FileNotFoundError as error:
            print(f"dev: {argv[0]}: {error.strerror}", file=sys.stderr, flush=True)
            return 127, False, b""
        # A signal that arrived while the child was starting is relayed now.
        for signum in received:
            child.send_signal(signum)
        output, _ = child.communicate()
        returncode = child.returncode
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)
    output = output or b""
    if returncode < 0:
        return 128 - returncode, True, output
    if returncode == 0 and received:
        return 128 + received[0], True, output
    return returncode, False, output


@contextlib.contextmanager
def _step(name: str, argv: Sequence[str]) -> Iterator[Span]:
    with trace.tracer().start_as_current_span(name) as span:
        span.set_attribute(COMMAND_LINE, trace.cap_command_line(shlex.join(argv)))
        yield span


def run(
    name: str, argv: Sequence[str], render: Callable[[bytes], str] | None = None
) -> None:
    """Runs one step; raises StepFailed so the gate stops at the first failure.

    With `render`, the step's stdout is shown as `render` presents it, whatever
    the step's status.
    """
    with _step(name, argv) as span:
        code, signalled, output = _supervise(argv, capture=render is not None)
        _record_outcome(span, code, signalled)
    if render is not None:
        sys.stdout.buffer.write(render(output).encode())
        sys.stdout.flush()
    if code:
        raise StepFailed(code, signalled)


class _QueryFailed(Exception):
    """A Nix query failed, so the step's cache source stays unrecorded."""


def _query(*argv: str) -> bytes:
    """A Nix query's stdout; its streams never reach the user."""
    try:
        done = subprocess.run(argv, capture_output=True, check=False)
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


def _built_source(output: str) -> str:
    """Where a freshly valid output came from: built here, or substituted."""
    entry = _store_entry(output)
    if entry is None:
        raise _QueryFailed
    return "built" if entry.get("ultimate") else "substituted"


def nix_check(check: str) -> None:
    """Builds one flake check, keeping its output as a GC root.

    The step records `jaunder.cache` for the check's top-level output only:
    `reused` if it was already valid, else `built` or `substituted`. A failed
    query records nothing rather than guess, and never changes the step.
    """
    installable = f".#checks.x86_64-linux.{check}"
    argv = [
        "nix",
        "build",
        "-L",
        "--accept-flake-config",
        "--out-link",
        f".xtask/gcroots/{check}",
        installable,
    ]
    Path(".xtask/gcroots").mkdir(parents=True, exist_ok=True)
    with _step(check, argv) as span:
        output: str | None
        try:
            output = _query(
                "nix",
                "eval",
                "--raw",
                "--accept-flake-config",
                f"{installable}.outPath",
            ).decode()
            reused = _store_entry(output) is not None
        except _QueryFailed:
            output, reused = None, False
        code, signalled, _ = _supervise(argv, capture=False)
        _record_outcome(span, code, signalled)
        if code == 0 and output is not None:
            with contextlib.suppress(_QueryFailed):
                span.set_attribute(CACHE, "reused" if reused else _built_source(output))
    if code:
        raise StepFailed(code, signalled)
