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
    # The child notes every signal it gets and leaves cleanly on SIGTERM.
    note_all = (
        "def leave(signum, _frame):\n"
        "    note(signal.Signals(signum).name)\n"
        "    if signum == signal.SIGTERM:\n"
        "        sys.exit(0)\n"
        "signal.signal(signal.SIGINT, leave)\n"
        "signal.signal(signal.SIGTERM, leave)\n"
    )
    checkout.tool("tsc", WAIT.format(handlers=note_all))
    dev = checkout.popen("static", "tsc")
    wait_for(checkout.root / "ready")
    # Pending signals are handled lowest number first, so `dev` has dealt with
    # SIGINT before it forwards SIGTERM: a forwarded SIGINT would reach the
    # child first. The forwarded SIGTERM is the handshake that proves it did not.
    dev.send_signal(signal.SIGINT)
    dev.send_signal(signal.SIGTERM)
    dev.communicate(timeout=30)

    assert signals(checkout) == "SIGTERM\n"
    assert dev.returncode == 128 + signal.SIGTERM


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


@pytest.mark.parametrize("sent", [signal.SIGTERM, signal.SIGHUP, signal.SIGINT])
def test_a_signal_while_no_step_runs_closes_the_gate_as_signalled(
    checkout: Checkout, sent: signal.Signals
) -> None:
    # The Nix check's output probe, not a step, is running when `dev` is signalled.
    checkout.tool(
        "nix",
        'if sys.argv[1] == "eval":\n'
        '    pathlib.Path("ready").touch()\n'
        "    time.sleep(30)\n",
    )
    dev = checkout.popen("nix-check", "demo")
    wait_for(checkout.root / "ready")
    dev.send_signal(sent)
    _, stderr = dev.communicate(timeout=30)

    assert dev.returncode == 128 + sent
    assert stderr == b""
    assert "nix build" not in checkout.order()
    slices = checkout.merged()
    for name in ("dev nix-check", "demo"):
        assert slices[name]["args"]["jaunder.status"] == "signalled"
        assert slices[name]["args"]["process.exit.code"] == 128 + sent


def test_a_child_killed_by_a_signal_exits_128_plus_n(checkout: Checkout) -> None:
    checkout.tool("tsc", "os.kill(os.getpid(), signal.SIGKILL)")
    result = checkout.dev("static", "tsc")
    assert result.returncode == 128 + signal.SIGKILL
    tsc = checkout.merged()["tsc"]["args"]
    assert (tsc["jaunder.status"], tsc["process.exit.code"]) == ("signalled", 137)
