"""The internal Nix-check step: its build, status and `jaunder.cache` value."""

import subprocess
import sys

import pytest
from conftest import Checkout

# No public command runs a Nix check yet, so a gate is driven from a separate
# interpreter exactly as a `dev` subcommand would drive it.
GATE = (
    "import sys\n"
    "from pathlib import Path\n"
    "from jaunder_dev import steps\n"
    'sys.exit(steps.gate(Path.cwd(), "dev nix", lambda: steps.nix_check("demo")))\n'
)
OUT = "/nix/store/0000-demo"
EVAL = "nix eval --raw --accept-flake-config .#checks.x86_64-linux.demo.outPath\n"
PATH_INFO = f"nix path-info --json --json-format 1 -- {OUT}\n"
BUILD = (
    "nix build -L --accept-flake-config --out-link .xtask/gcroots/demo"
    " .#checks.x86_64-linux.demo\n"
)

# A fake `nix`: the output is valid once built, or from the start with
# NIX_VALID.
FAKE_NIX = """\
command = sys.argv[1]
env = os.environ.get
if command == "eval":
    if env("NIX_EVAL_FAIL"):
        print("eval-err", file=sys.stderr)
        sys.exit(1)
    print("/nix/store/0000-demo", end="")
elif command == "path-info":
    if env("NIX_INFO_FAIL"):
        print("info-err", file=sys.stderr)
        sys.exit(1)
    if env("NIX_INFO_GARBLED"):
        print("not json")
        sys.exit()
    path = sys.argv[-1]
    valid = bool(env("NIX_VALID")) or os.path.exists("built")
    info = {"ultimate": bool(env("NIX_ULTIMATE")), "version": 1}
    print(json.dumps({path: info if valid else None}))
    print("info-warning", file=sys.stderr)
else:
    print("build-out"); print("build-err", file=sys.stderr)
    code = int(env("NIX_BUILD_CODE", "0"))
    if code == 0 and not env("NIX_LOST"):
        open("built", "w").close()
    sys.exit(code)
"""


def nix_check(checkout: Checkout, **extra: str) -> subprocess.CompletedProcess[bytes]:
    checkout.tool("nix", "import json\n" + FAKE_NIX)
    return subprocess.run(
        [sys.executable, "-P", "-c", GATE],
        cwd=checkout.root,
        env=checkout.environment(**extra),
        capture_output=True,
        check=False,
        timeout=60,
    )


@pytest.mark.parametrize(
    ("extra", "cache", "order"),
    [
        ({"NIX_VALID": "1"}, "reused", EVAL + PATH_INFO + BUILD),
        ({"NIX_ULTIMATE": "1"}, "built", EVAL + PATH_INFO + BUILD + PATH_INFO),
        ({}, "substituted", EVAL + PATH_INFO + BUILD + PATH_INFO),
    ],
)
def test_nix_check_records_where_its_output_came_from(
    checkout: Checkout, extra: dict[str, str], cache: str, order: str
) -> None:
    result = nix_check(checkout, **extra)

    assert result.returncode == 0, result
    assert checkout.order() == order
    assert (checkout.root / ".xtask" / "gcroots").is_dir()
    # Only the build's own streams reach the user; the probes' stay captured.
    assert result.stdout == b"build-out\n"
    assert result.stderr == b"build-err\n"
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.cache"] == cache
    assert step["jaunder.status"] == "ok"
    assert step["process.command_line"] == (
        "nix build -L --accept-flake-config --out-link .xtask/gcroots/demo"
        " '.#checks.x86_64-linux.demo'"
    )


def test_failed_build_records_no_cache_and_stops_the_gate(checkout: Checkout) -> None:
    result = nix_check(checkout, NIX_BUILD_CODE="3", NIX_ULTIMATE="1")

    assert result.returncode == 3
    assert checkout.order() == EVAL + PATH_INFO + BUILD
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "failed"
    assert step["process.exit.code"] == 3
    assert "jaunder.cache" not in step


@pytest.mark.parametrize(
    ("failure", "build_code"),
    [
        ("NIX_EVAL_FAIL", 0),
        ("NIX_INFO_FAIL", 0),
        ("NIX_INFO_GARBLED", 0),
        # A successful build whose output is still not valid afterwards.
        ("NIX_LOST", 0),
        ("NIX_EVAL_FAIL", 5),
    ],
)
def test_failed_probe_records_no_cache_and_changes_nothing_else(
    checkout: Checkout, failure: str, build_code: int
) -> None:
    result = nix_check(
        checkout, NIX_BUILD_CODE=str(build_code), NIX_ULTIMATE="1", **{failure: "1"}
    )

    assert result.returncode == build_code
    assert result.stdout == b"build-out\n"
    assert result.stderr == b"build-err\n"
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == ("ok" if build_code == 0 else "failed")
    assert "jaunder.cache" not in step


def test_cache_shows_in_the_summary(checkout: Checkout) -> None:
    assert nix_check(checkout, NIX_VALID="1").returncode == 0
    (run,) = checkout.runs()
    summary = checkout.dev("trace", "summary", str(run))
    assert summary.returncode == 0, summary
    (line,) = [line for line in summary.stdout.decode().splitlines() if "demo" in line]
    assert line.split()[-2:] == ["reused", "demo"]


def test_steps_that_cannot_use_a_cache_record_none(checkout: Checkout) -> None:
    checkout.tool("tsc", "")
    assert checkout.dev("static", "tsc", "--sandbox").returncode == 0
    assert "jaunder.cache" not in checkout.merged()["tsc"]["args"]
