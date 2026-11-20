"""`dev nix-check`: one flake check as a step, its status and `jaunder.cache`."""

import subprocess
from typing import Any

import pytest
from conftest import FAKE_NIX_STORE, REGISTERED_LATER, Checkout, launch

from dev import steps
from dev.spans import Cache

PATH_INFO = "nix path-info --json --json-format 1 -- build-out\n"
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
    ("extra", "cache"),
    [
        ({"NIX_VALID": "1"}, "reused"),
        ({"NIX_ULTIMATE": "1"}, "built"),
        ({}, "substituted"),
    ],
)
def test_nix_check_records_where_its_output_came_from(
    checkout: Checkout, extra: dict[str, str], cache: str
) -> None:
    result = nix_check(checkout, **extra)

    assert result.returncode == 0, result
    # The build is the step's only evaluation; one query of its printed output
    # follows it.
    assert checkout.order() == BUILD + PATH_INFO
    assert (checkout.root / ".xtask" / "gcroots").is_dir()
    # Only the build's log reaches the user; its printed output path is the
    # caller's, and the query's streams stay captured.
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
    assert checkout.order() == BUILD
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "failed"
    assert step["process.exit.code"] == 3
    assert "jaunder.cache" not in step


@pytest.mark.parametrize(
    "failure",
    [
        "NIX_INFO_FAIL",
        "NIX_INFO_GARBLED",
        # A successful build whose output is still not valid afterwards.
        "NIX_LOST",
    ],
)
def test_failed_query_records_no_cache_and_changes_nothing_else(
    checkout: Checkout, failure: str
) -> None:
    result = nix_check(checkout, NIX_ULTIMATE="1", **{failure: "1"})

    assert result.returncode == 0
    assert result.stdout == b""
    assert result.stderr == b"build-err\n"
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "ok"
    assert "jaunder.cache" not in step


def test_a_query_that_cannot_start_records_no_cache(checkout: Checkout) -> None:
    # The build succeeds, then `nix` is gone before its output is queried.
    checkout.tool("nix", 'print("/nix/store/0000-demo")\nos.unlink(sys.argv[0])')
    result = checkout.run(checkout.command("nix-check", "demo"), checkout.environment())

    assert result.returncode == 0, result
    assert result.stderr == b""
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "ok"
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


def test_a_named_package_build_records_its_step_and_cache(checkout: Checkout) -> None:
    result = build(checkout, '"bundle", ".#demo"', NIX_VALID="1")
    assert result.returncode == 0, result
    assert checkout.order() == (
        "nix build -L --accept-flake-config --print-out-paths --out-link .xtask/gcroots/bundle"
        " .#demo\n" + PATH_INFO
    )
    assert checkout.merged()["bundle"]["args"]["jaunder.cache"] == "reused"


def test_a_build_returns_the_output_path_it_printed(checkout: Checkout) -> None:
    result = build(checkout, '"vite", ".#vite", keep=False')
    assert result.returncode == 0, result
    assert result.stdout == b"build-out\n"
    assert (
        "nix build -L --accept-flake-config --print-out-paths --no-link .#vite"
        in checkout.order().splitlines()
    )
    assert not (checkout.root / ".xtask" / "gcroots").exists()


def test_a_derivation_installable_is_built_without_a_flake_config(
    checkout: Checkout,
) -> None:
    drv = "/nix/store/0000-demo.drv^out"
    result = build(checkout, f'"producer", "{drv}", flags=(), keep=False')
    assert result.returncode == 0, result
    assert checkout.order() == (
        f"nix build -L --print-out-paths --no-link {drv}\n" + PATH_INFO
    )
    assert checkout.merged()["producer"]["args"]["jaunder.cache"] == "substituted"


def test_several_printed_outputs_are_returned_without_a_cache(
    checkout: Checkout,
) -> None:
    result = build(checkout, '"multi", ".#demo", keep=False', NIX_TWO_OUTPUTS="1")
    assert result.returncode == 0, result
    assert result.stdout == b"build-out\nbuild-out-dev\n"
    assert checkout.order() == (
        "nix build -L --accept-flake-config --print-out-paths --no-link .#demo\n"
    )
    step = checkout.merged()["multi"]["args"]
    assert step["jaunder.status"] == "ok"
    assert "jaunder.cache" not in step


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


def realizing(
    checkout: Checkout, extra: dict[str, str], code: int = 0
) -> subprocess.CompletedProcess[bytes]:
    """A plain step that runs its own Nix build, as `run(realizes=...)` sees it.

    The builder registers a fresh output, as the fake store's own build does.
    """
    checkout.tool("nix", FAKE_NIX_STORE)
    checkout.tool(
        "builder",
        f'open("built", "w").write("{REGISTERED_LATER}"); sys.exit({code})',
    )
    gate = (
        "from pathlib import Path; from dev import steps; "
        'sys.exit(steps.gate(Path.cwd(), "dev demo", '
        'lambda: steps.run("producer", ["builder"], realizes=lambda: "build-out")))'
    )
    return checkout.run(launch(gate), checkout.environment(**extra))


@pytest.mark.parametrize(
    ("extra", "cache"),
    [
        ({"NIX_VALID": "1"}, "reused"),
        ({"NIX_ULTIMATE": "1"}, "built"),
        ({}, "substituted"),
    ],
)
def test_a_step_that_realizes_an_installable_records_its_cache(
    checkout: Checkout, extra: dict[str, str], cache: str
) -> None:
    result = realizing(checkout, extra)
    assert result.returncode == 0, result
    # No evaluation: one query of the selected output, after the step.
    assert checkout.order() == "builder\n" + PATH_INFO
    step = checkout.merged()["producer"]["args"]
    assert (step["jaunder.status"], step["jaunder.cache"]) == ("ok", cache)
    assert step["process.command_line"] == "builder"


@pytest.mark.parametrize(
    ("code", "extra"),
    [
        (4, {"NIX_ULTIMATE": "1"}),
        (0, {"NIX_INFO_FAIL": "1"}),
        (0, {"NIX_INFO_GARBLED": "1"}),
    ],
)
def test_a_failed_realizing_step_or_query_records_no_cache(
    checkout: Checkout, code: int, extra: dict[str, str]
) -> None:
    result = realizing(checkout, extra, code)
    assert result.returncode == code
    step = checkout.merged()["producer"]["args"]
    assert step["process.exit.code"] == code
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


START = 1_800_000_000


@pytest.mark.parametrize(
    ("entry", "cache"),
    [
        ({"registrationTime": START - 1, "ultimate": True}, Cache.REUSED),
        ({"registrationTime": START - 1, "ultimate": False}, Cache.REUSED),
        ({"registrationTime": START + 1, "ultimate": True}, Cache.BUILT),
        ({"registrationTime": START + 1, "ultimate": False}, Cache.SUBSTITUTED),
        ({"registrationTime": START + 1}, Cache.SUBSTITUTED),
        # Registered in the start second: before or after the start, unknown.
        ({"registrationTime": START, "ultimate": True}, None),
        (None, None),
        ({"ultimate": True}, None),
        ({"registrationTime": "1800000001", "ultimate": True}, None),
        ({"registrationTime": True, "ultimate": True}, None),
    ],
)
def test_an_output_source_is_judged_by_its_registration_against_the_start(
    entry: dict[str, Any] | None, cache: Cache | None
) -> None:
    assert steps.output_source(entry, START) == cache
