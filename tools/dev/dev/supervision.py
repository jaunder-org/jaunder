"""Long-lived children that one `dev` step holds at once, as `dev e2e-local` does.

A gate's ordinary step runs one child to exit (`steps`). Here several run
together, each in its own process group, so a terminal's Ctrl-C reaches only
`dev`; `dev` decides what each child gets and when. Their output is drained as
it arrives, line by line to stderr, so a child that writes more than a pipe
holds can never block.
"""

import contextlib
import os
import signal
import socket
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Sequence
from pathlib import Path
from types import FrameType
from typing import IO

import psutil

from dev import steps, trace

# How often a wait looks again at a child, a port or a termination request.
TICK = 0.05
# How long a child asked to stop may take before it and its descendants are
# killed.
GRACE = 10.0
# How long a reaped child's output may still drain: only a descendant that
# escaped its stop can hold the pipes open longer.
DRAIN = 2.0


class Signals:
    """Termination requests for the rest of a run, recorded rather than acted on.

    The first request's 128+n is the run's exit code. Every request counts, so a
    stop can tell whether another has arrived since it began.
    """

    HANDLED = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)

    def __init__(self) -> None:
        self.code: int | None = None
        self.count = 0

    def handle(self, signum: int, _frame: FrameType | None) -> None:
        self.count += 1
        if self.code is None:
            self.code = 128 + signum

    def install(self, stack: contextlib.ExitStack) -> None:
        """Handles HANDLED from now until `stack` closes, then restores the gate's."""
        previous = {
            number: signal.signal(number, self.handle) for number in self.HANDLED
        }

        def restore() -> None:
            for number, handler in previous.items():
                signal.signal(number, handler)

        stack.callback(restore)


def _echo(line: bytes) -> None:
    sys.stderr.buffer.write(line)
    sys.stderr.buffer.flush()


def _drain(stream: IO[bytes], sinks: Sequence[Callable[[bytes], object]]) -> None:
    for line in iter(stream.readline, b""):
        for sink in sinks:
            sink(line)


class Child:
    """One started child: its own process group, its streams drained as it runs.

    stdout goes to stderr, or with `capture` into `output`; stderr goes to
    stderr and, with `tee`, into that file as well. `changes` alter the
    environment a step's child would get. Raises OSError if it cannot start.
    """

    def __init__(
        self,
        argv: Sequence[str],
        changes: steps.Environment | None = None,
        cwd: Path | None = None,
        tee: Path | None = None,
        capture: bool = False,
    ) -> None:
        environment = steps.changed(
            steps.changed(trace.child_environment(), steps.WITHOUT_GIT_OVERRIDES),
            changes,
        )
        with contextlib.ExitStack() as opened:
            # Closed here only if the child cannot start; else its drain owns it.
            log = opened.enter_context(open(tee or os.devnull, "wb", buffering=0))
            self.popen = subprocess.Popen(
                argv,
                cwd=cwd,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
            opened.pop_all()
        self.output = bytearray()
        stdout = self.popen.stdout
        stderr = self.popen.stderr
        assert stdout is not None and stderr is not None

        def tee_stderr() -> None:
            with log:
                _drain(stderr, [_echo, log.write])

        self._drains = [
            threading.Thread(
                target=_drain,
                args=(stdout, [self.output.extend if capture else _echo]),
                daemon=True,
            ),
            threading.Thread(target=tee_stderr, daemon=True),
        ]
        for drain in self._drains:
            drain.start()

    @property
    def running(self) -> bool:
        """Whether it has not exited, judged without reaping it.

        Only `reap` reaps, so until then its pid, which is also its process
        group's id, cannot be reused: signalling the group never reaches a
        stranger's, and a `runtime.json` naming that pid can only be its own.
        """
        if self.popen.returncode is not None:
            return False
        status = psutil.Process(self.popen.pid).status()
        return status != psutil.STATUS_ZOMBIE

    @property
    def status(self) -> int | None:
        return self.popen.returncode

    def reap(self) -> bool:
        """Waits for exit, then its output; whether the output has all drained."""
        self.popen.wait()
        deadline = time.monotonic() + DRAIN
        for drain in self._drains:
            drain.join(max(0.0, deadline - time.monotonic()))
        return not any(drain.is_alive() for drain in self._drains)

    def _signal_group(self, signum: int) -> None:
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.killpg(self.popen.pid, signum)

    def stop(self, grace: float, escalation: Signals | None = None) -> bool:
        """TERM to its group, then KILL to the group and its descendants.

        Descendants are listed first, as Playwright's browsers run in groups of
        their own. With `escalation`, a further request ends the grace at once.
        Returns `reap`'s answer: whether nothing it ran still writes.
        """
        descendants: list[psutil.Process] = []
        with contextlib.suppress(psutil.Error):
            descendants = psutil.Process(self.popen.pid).children(recursive=True)
        baseline = escalation.count if escalation else 0
        self._signal_group(signal.SIGTERM)
        deadline = time.monotonic() + grace
        while (
            self.running
            and time.monotonic() < deadline
            and not (escalation is not None and escalation.count > baseline)
        ):
            time.sleep(TICK)
        self._signal_group(signal.SIGKILL)
        for descendant in descendants:
            with contextlib.suppress(psutil.Error):
                descendant.kill()
        return self.reap()


def run(
    argv: Sequence[str],
    changes: steps.Environment | None = None,
    signals: Signals | None = None,
    cwd: Path | None = None,
    capture: bool = False,
) -> tuple[int | None, bytes]:
    """Runs a child to exit: its status and, with `capture`, its stdout.

    With `signals`, a termination request, pending or arriving, stops it
    instead and its status is None. A child that cannot start has a shell's
    127. A failure is the caller's to judge; nothing here raises.
    """
    try:
        child = Child(argv, changes, cwd=cwd, capture=capture)
    except OSError as error:
        print(f"dev: {argv[0]}: {error.strerror}", file=sys.stderr, flush=True)
        return 127, b""
    # A request already pending stops the child before its first tick.
    while signals is not None:
        if signals.code is not None:
            child.stop(GRACE, signals)
            return None, bytes(child.output)
        if not child.running:
            break
        time.sleep(TICK)
    child.reap()
    return child.status, bytes(child.output)


def accepting(address: tuple[str, int], within: float, child: Child) -> bool:
    """Whether `address` accepts a connection within `within` seconds while
    `child` runs."""
    deadline = time.monotonic() + within
    while True:
        with contextlib.suppress(OSError):
            socket.create_connection(address, timeout=TICK).close()
            return True
        if not child.running or time.monotonic() >= deadline:
            return False
        time.sleep(TICK)
