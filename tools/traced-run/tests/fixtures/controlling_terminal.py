"""Real controlling terminal, not just a pipe marked as inherited stdin."""

import fcntl
import os
import pathlib
import pty
import signal
import subprocess
import sys
import termios
import time

runner = sys.argv[1]
master, slave = pty.openpty()
os.setsid()
fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
os.dup2(slave, 0)
original = os.tcgetpgrp(0)
assert original == os.getpgrp()
# The cgroup backend keeps all processes in this foreground group. A caught
# handler protects the test driver, while exec resets it in each supervised child.
signal.signal(signal.SIGINT, lambda *_: None)


def start(argv):
    return subprocess.Popen(
        [runner, "--launcher", "--name", "terminal-test", "--", *argv],
        stdin=0,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def abort(child):
    # Give the owner its normal bounded teardown path, including for a stopped
    # reader on the unfixed implementation, before resorting to killing it.
    child.terminate()
    try:
        child.communicate(timeout=4)
    except subprocess.TimeoutExpired:
        child.kill()
        child.communicate()


def finish(child, expected):
    try:
        out, err = child.communicate(timeout=8)
    except subprocess.TimeoutExpired:
        abort(child)
        raise
    assert child.returncode == expected, (child.returncode, out, err)
    assert os.tcgetpgrp(0) == original, "foreground ownership was not restored"
    return out, err


# The command reads immediately, even when data is queued before spawn. Repeat
# to exercise the first-read handoff race: foreground transfer must precede exec.
for _ in range(8):
    os.write(master, b"real terminal input\n\x04")
    out, err = finish(start(["cat"]), 0)
    assert out == b"real terminal input\n", (out, err)
    assert not err, err

# Helper exec failure must be honest and restore terminal ownership too.
out, err = finish(start(["/definitely/not/a/program"]), 64)
assert not out and err, (out, err)
if os.environ.get("TRACED_RUN_EXPECT_PROCESS_GROUP"):
    assert b"terminal exec" in err, err

for interruption, expected in [("terminal", 130), ("forwarded", 143)]:
    ready = pathlib.Path("ready")
    ready.unlink(missing_ok=True)
    program = (
        "import os,pathlib,time; "
        "assert os.tcgetpgrp(0)==os.getpgrp(); "
        "assert os.read(0,128)==b'interrupt input\\n'; "
        "pathlib.Path('ready').write_text(str(os.getpid())); "
        "time.sleep(60)"
    )
    child = start([sys.executable, "-c", program])
    os.write(master, b"interrupt input\n")
    deadline = time.monotonic() + 5
    while not ready.exists():
        if child.poll() is not None or time.monotonic() >= deadline:
            abort(child)
            raise AssertionError("terminal reader did not become ready")
        time.sleep(0.01)
    owned = int(ready.read_text())
    assert os.tcgetpgrp(0) in (original, owned)
    if os.environ.get("TRACED_RUN_EXPECT_PROCESS_GROUP"):
        assert os.tcgetpgrp(0) == owned
    if interruption == "terminal":
        os.write(master, b"\x03")  # Real terminal-generated SIGINT, not kill(2).
    else:
        os.kill(child.pid, signal.SIGTERM)  # Parent must forward while background.
    finish(child, expected)
    try:
        os.kill(owned, 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError(f"owned terminal child {owned} survived interruption")

signal.signal(signal.SIGHUP, signal.SIG_IGN)
os.close(master)
os.close(slave)
