"""Signals sent to `dev` alone, and children that die from a signal."""

import signal
import time
from pathlib import Path

import pytest
from conftest import Checkout

# The fake check announces readiness, then waits to be signalled or released.
WAIT = """\
def note(text):
    with open("signals", "a") as signals:
        signals.write(text + "\\n")
{handlers}
pathlib.Path("ready").touch()
deadline = time.monotonic() + 30
while not pathlib.Path("release").exists() and time.monotonic() < deadline:
    time.sleep(0.01)
"""

DIE_FROM_SIGNAL = """\
def die(signum, _frame):
    note(signal.Signals(signum).name)
    signal.signal(signum, signal.SIG_DFL)
    os.kill(os.getpid(), signum)
signal.signal(signal.SIGTERM, die)
signal.signal(signal.SIGHUP, die)
signal.signal(signal.SIGINT, die)
"""


def wait_for(path: Path) -> None:
    deadline = time.monotonic() + 30
    while not path.exists():
        assert time.monotonic() < deadline, f"{path} never appeared"
        time.sleep(0.01)


def signals(checkout: Checkout) -> str:
    path = checkout.root / "signals"
    return path.read_text() if path.exists() else ""


@pytest.mark.parametrize("sent", [signal.SIGTERM, signal.SIGHUP])
def test_termination_sent_to_dev_alone_is_forwarded_and_exits_128_plus_n(
    checkout: Checkout, sent: signal.Signals
) -> None:
    checkout.tool("tsc", WAIT.format(handlers=DIE_FROM_SIGNAL))
    dev = checkout.popen("static", "tsc")
    wait_for(checkout.root / "ready")
    dev.send_signal(sent)
    dev.communicate(timeout=30)

    assert dev.returncode == 128 + sent
    assert signals(checkout) == f"{sent.name}\n"
    slices = checkout.merged()
    for name in ("dev static tsc", "tsc"):
        assert slices[name]["args"]["jaunder.status"] == "signalled"
        assert slices[name]["args"]["process.exit.code"] == 128 + sent


def test_interrupt_sent_to_dev_alone_is_not_forwarded(checkout: Checkout) -> None:
    checkout.tool("tsc", WAIT.format(handlers=DIE_FROM_SIGNAL))
    dev = checkout.popen("static", "tsc")
    wait_for(checkout.root / "ready")
    dev.send_signal(signal.SIGINT)
    # Give a wrongly forwarded interrupt time to arrive before releasing the child.
    time.sleep(0.5)
    (checkout.root / "release").touch()
    dev.communicate(timeout=30)

    assert dev.returncode == 0
    assert signals(checkout) == ""
    assert checkout.merged()["tsc"]["args"]["jaunder.status"] == "ok"


def test_a_child_that_exits_cleanly_after_termination_still_stops_the_gate(
    checkout: Checkout,
) -> None:
    clean_exit = (
        "def leave(signum, _frame):\n"
        "    note('TERM')\n"
        "    sys.exit(0)\n"
        "signal.signal(signal.SIGTERM, leave)\n"
    )
    checkout.tool("setup-e2e-node-modules", WAIT.format(handlers=clean_exit))
    checkout.tool("tsc", "")
    dev = checkout.popen("tsc")
    wait_for(checkout.root / "ready")
    dev.send_signal(signal.SIGTERM)
    dev.communicate(timeout=30)

    assert dev.returncode == 128 + signal.SIGTERM
    assert "tsc --noEmit" not in checkout.order()
    slices = checkout.merged()
    assert slices["setup-e2e-node-modules"]["args"]["jaunder.status"] == "signalled"


def test_a_child_killed_by_a_signal_exits_128_plus_n(checkout: Checkout) -> None:
    checkout.tool("tsc", "os.kill(os.getpid(), signal.SIGKILL)")
    result = checkout.dev("static", "tsc")
    assert result.returncode == 128 + signal.SIGKILL
    tsc = checkout.merged()["tsc"]["args"]
    assert (tsc["jaunder.status"], tsc["process.exit.code"]) == ("signalled", 137)
