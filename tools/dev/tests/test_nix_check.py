"""`dev nix-check`: one flake check as a step, its status and `jaunder.cache`."""

import subprocess

import pytest
from conftest import FAKE_NIX_STORE, Checkout, launch

OUT = "/nix/store/0000-demo"
EVAL = "nix eval --raw --accept-flake-config .#checks.x86_64-linux.demo.outPath\n"
PATH_INFO = f"nix path-info --json --json-format 1 -- {OUT}\n"
BUILD = (
    "nix build -L --accept-flake-config --print-out-paths --out-link .xtask/gcroots/demo"
    " .#checks.x86_64-linux.demo\n"
)


def nix_check(checkout: Checkout, **extra: str) -> subprocess.CompletedProcess[bytes]:
    checkout.tool("nix", FAKE_NIX_STORE)
    return checkout.run(
        checkout.command("nix-check", "demo"), checkout.environment(**extra)
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
    # Only the build's log reaches the user; its printed output path is the
    # caller's, and the probes' streams stay captured.
    assert result.stdout == b""
    assert result.stderr == b"build-err\n"
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.cache"] == cache
    assert step["jaunder.status"] == "ok"
    assert step["process.command_line"] == (
        "nix build -L --accept-flake-config --print-out-paths --out-link .xtask/gcroots/demo"
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
    assert result.stdout == b""
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


@pytest.mark.parametrize("args", [[], ["demo", "extra"]])
def test_nix_check_takes_exactly_one_check(checkout: Checkout, args: list[str]) -> None:
    checkout.tool("nix", FAKE_NIX_STORE)
    result = checkout.dev("nix-check", *args)
    assert result.returncode == 2, result
    assert checkout.order() == ""
    assert checkout.runs() == []


def test_a_named_package_build_records_its_step_and_cache(checkout: Checkout) -> None:
    checkout.tool("nix", FAKE_NIX_STORE)
    gate = (
        "from pathlib import Path; from dev import steps; "
        'sys.exit(steps.gate(Path.cwd(), "dev demo", '
        'lambda: steps.nix_build("bundle", ".#demo")))'
    )
    result = checkout.run(launch(gate), checkout.environment(NIX_VALID="1"))
    assert result.returncode == 0, result
    assert checkout.order() == (
        "nix eval --raw --accept-flake-config .#demo.outPath\n"
        + PATH_INFO
        + "nix build -L --accept-flake-config --print-out-paths --out-link .xtask/gcroots/bundle"
        " .#demo\n"
    )
    assert checkout.merged()["bundle"]["args"]["jaunder.cache"] == "reused"


def build(
    checkout: Checkout, call: str, **extra: str
) -> subprocess.CompletedProcess[bytes]:
    """Runs `steps.nix_build(<call>)` as a gate's one step, printing its result."""
    checkout.tool("nix", FAKE_NIX_STORE)
    gate = (
        "from pathlib import Path; from dev import steps; "
        'sys.exit(steps.gate(Path.cwd(), "dev demo", '
        f"lambda: print(steps.nix_build({call}))))"
    )
    return checkout.run(launch(gate), checkout.environment(**extra))


def test_a_build_returns_the_output_path_it_printed(checkout: Checkout) -> None:
    result = build(checkout, '"vite", ".#vite", keep=False')
    assert result.returncode == 0, result
    assert result.stdout == b"build-out\n"
    assert (
        "nix build -L --accept-flake-config --print-out-paths --no-link .#vite"
        in checkout.order().splitlines()
    )
    assert not (checkout.root / ".xtask" / "gcroots").exists()


def test_a_known_output_is_probed_instead_of_evaluated(checkout: Checkout) -> None:
    drv = "/nix/store/0000-demo.drv^out"
    result = build(
        checkout, f'"producer", "{drv}", flags=(), keep=False, output="{OUT}"'
    )
    assert result.returncode == 0, result
    assert checkout.order() == (
        PATH_INFO + f"nix build -L --print-out-paths --no-link {drv}\n" + PATH_INFO
    )
    assert checkout.merged()["producer"]["args"]["jaunder.cache"] == "substituted"


def test_a_refused_output_fails_its_build_step_without_a_cache(
    checkout: Checkout,
) -> None:
    result = build(
        checkout,
        '"producer", ".#demo", refusal=lambda path: f"refused {path}"',
        NIX_VALID="1",
    )
    assert result.returncode == 1
    assert result.stdout == b""
    assert result.stderr == b"build-err\nrefused build-out\n"
    step = checkout.merged()["producer"]["args"]
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 1)
    assert "jaunder.cache" not in step


def test_without_nix_the_check_fails_127_and_records_no_cache(
    checkout: Checkout,
) -> None:
    gate = (
        "from pathlib import Path; from dev import steps; "
        'sys.exit(steps.gate(Path.cwd(), "dev nix-check", '
        'lambda: steps.nix_check("demo")))'
    )
    result = checkout.run(launch(gate), checkout.environment(PATH=str(checkout.bin)))
    assert result.returncode == 127
    assert result.stderr == b"dev: nix: No such file or directory\n"
    step = checkout.merged()["demo"]["args"]
    assert step["process.exit.code"] == 127
    assert "jaunder.cache" not in step
