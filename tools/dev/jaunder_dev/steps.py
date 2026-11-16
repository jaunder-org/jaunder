"""Gates and their steps: each step is one traced child process.

A gate stops at the first failing step and exits with that step's status. A
child killed by signal n reports 128+n, as a shell would; Python reports such a
child's return code as -n.
"""

import shlex
import signal
import subprocess
import sys
from collections.abc import Callable, Sequence
from pathlib import Path
from types import FrameType

from opentelemetry.trace import Span

from jaunder_dev import trace
from jaunder_dev.spans import COMMAND_LINE, EXIT_CODE, STATUS


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


def _supervise(argv: Sequence[str]) -> tuple[int, bool]:
    """Runs the child to completion, relaying a termination request to it.

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
            child = subprocess.Popen(argv, env=trace.child_environment())
        except FileNotFoundError as error:
            print(f"dev: {argv[0]}: {error.strerror}", file=sys.stderr, flush=True)
            return 127, False
        # A signal that arrived while the child was starting is relayed now.
        for signum in received:
            child.send_signal(signum)
        returncode = child.wait()
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)
    if returncode < 0:
        return 128 - returncode, True
    if returncode == 0 and received:
        return 128 + received[0], True
    return returncode, False


def run(name: str, argv: Sequence[str]) -> None:
    """Runs one step; raises StepFailed so the gate stops at the first failure."""
    with trace.tracer().start_as_current_span(name) as span:
        span.set_attribute(COMMAND_LINE, trace.cap_command_line(shlex.join(argv)))
        code, signalled = _supervise(argv)
        _record_outcome(span, code, signalled)
    if code:
        raise StepFailed(code, signalled)
