"""`dev nix-check`: one flake check as a step of an evaluation and a build, its
status, output, `jaunder.cache` and fetched bytes."""

import subprocess
from typing import Any

import pytest
from conftest import (
    DERIVATION,
    FAKE_NIX_STORE,
    FETCHED_DEPENDENCIES,
    OUTPUT_SIZE,
    REGISTERED_LATER,
    Checkout,
    cells,
    launch,
)

from dev import steps
from dev.spans import Cache

EVAL = "nix path-info --derivation --accept-flake-config .#checks.x86_64-linux.demo\n"
PATH_INFO = "nix path-info --json --json-format 1 -- build-out\n"
BUILD = (
    "nix build -L --accept-flake-config --print-out-paths --out-link .xtask/gcroots/demo"
    " .#checks.x86_64-linux.demo\n"
)
STDIN_INFO = "nix path-info --json --json-format 1 --stdin\n"


def fetch(derivation: str = DERIVATION) -> str:
    """The queries for what the build of `derivation` fetched."""
    return (
        f"nix-store --query --requisites --include-outputs {derivation}\n" + STDIN_INFO
    )


def nix_check(checkout: Checkout, **extra: str) -> subprocess.CompletedProcess[bytes]:
    checkout.nix_store()
    return checkout.run(
        checkout.command("nix-check", "demo"), checkout.environment(**extra)
    )


def children(checkout: Checkout, step: str) -> dict[str, dict[str, Any]]:
    """`step`'s child spans, by name; each must be a child of `step`."""
    slices = checkout.merged()
    parent = slices[step]["args"]["span_id"]
    found = {
        name: event["args"]
        for name, event in slices.items()
        if event["args"].get("parent_id") == parent
    }
    return found


@pytest.mark.parametrize(
    ("extra", "cache", "fetched"),
    [
        ({"NIX_VALID": "1"}, "reused", None),
        ({"NIX_ULTIMATE": "1"}, "built", FETCHED_DEPENDENCIES),
        ({}, "substituted", OUTPUT_SIZE + FETCHED_DEPENDENCIES),
    ],
)
def test_nix_check_records_where_its_output_came_from(
    checkout: Checkout, extra: dict[str, str], cache: str, fetched: int | None
) -> None:
    result = nix_check(checkout, **extra)

    assert result.returncode == 0, result
    # The evaluation, the build, one query of its printed output and, unless
    # the output was reused, the queries for what the build fetched.
    assert checkout.order() == EVAL + BUILD + PATH_INFO + (
        "" if fetched is None else fetch()
    )
    assert (checkout.root / ".xtask" / "gcroots").is_dir()
    # Only the build's log reaches the user; the evaluation's derivation and
    # the build's printed output path are the caller's, and the queries'
    # streams stay captured.
    assert result.stdout == b""
    assert result.stderr == b"build-err\n"
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.cache"] == cache
    assert step["jaunder.status"] == "ok"
    assert step["jaunder.nix.output"] == "build-out"
    assert step.get("jaunder.nix.fetched_bytes") == fetched
    # The step is its children's: it runs no command of its own.
    assert "process.command_line" not in step
    child = children(checkout, "demo")
    assert sorted(child) == ["build", "eval"]
    assert child["eval"]["process.command_line"] == (
        "nix path-info --derivation --accept-flake-config '.#checks.x86_64-linux.demo'"
    )
    assert child["build"]["process.command_line"] == (
        "nix build -L --accept-flake-config --print-out-paths --out-link .xtask/gcroots/demo"
        " '.#checks.x86_64-linux.demo'"
    )
    for name in ["eval", "build"]:
        assert (child[name]["jaunder.status"], child[name]["process.exit.code"]) == (
            "ok",
            0,
        )
    slices = checkout.merged()
    assert slices["eval"]["ts"] < slices["build"]["ts"]


def test_failed_build_records_no_cache_and_stops_the_gate(checkout: Checkout) -> None:
    result = nix_check(checkout, NIX_BUILD_CODE="3", NIX_ULTIMATE="1")

    assert result.returncode == 3
    assert checkout.order() == EVAL + BUILD
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "failed"
    assert step["process.exit.code"] == 3
    assert "jaunder.cache" not in step
    assert "jaunder.nix.output" not in step
    build = children(checkout, "demo")["build"]
    assert (build["jaunder.status"], build["process.exit.code"]) == ("failed", 3)


def test_a_failed_evaluation_fails_the_step_before_any_build(
    checkout: Checkout,
) -> None:
    result = nix_check(checkout, NIX_EVAL_CODE="5")

    assert result.returncode == 5
    assert checkout.order() == EVAL
    assert result.stdout == b""
    # The evaluation's own diagnostics reach the user.
    assert result.stderr == b"eval-err\n"
    step = checkout.merged()["demo"]["args"]
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 5)
    child = children(checkout, "demo")
    assert list(child) == ["eval"]
    assert (child["eval"]["jaunder.status"], child["eval"]["process.exit.code"]) == (
        "failed",
        5,
    )
    assert not (checkout.root / "built").exists()


@pytest.mark.parametrize(
    "failure",
    [
        "NIX_INFO_FAIL",
        "NIX_INFO_GARBLED",
    ],
)
def test_failed_query_records_no_cache_and_changes_nothing_else(
    checkout: Checkout, failure: str
) -> None:
    result = nix_check(checkout, NIX_ULTIMATE="1", **{failure: "1"})

    assert result.returncode == 0
    assert result.stdout == b""
    assert result.stderr == b"build-err\n"
    assert checkout.order() == EVAL + BUILD + PATH_INFO
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "ok"
    assert step["jaunder.nix.output"] == "build-out"
    assert "jaunder.cache" not in step
    assert "jaunder.nix.fetched_bytes" not in step


def test_an_output_still_invalid_after_its_build_records_only_what_was_fetched(
    checkout: Checkout,
) -> None:
    # Its registration is undecidable, so the build may have fetched.
    result = nix_check(checkout, NIX_ULTIMATE="1", NIX_LOST="1")

    assert result.returncode == 0
    assert result.stderr == b"build-err\n"
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "ok"
    assert "jaunder.cache" not in step
    assert step["jaunder.nix.fetched_bytes"] == FETCHED_DEPENDENCIES


@pytest.mark.parametrize(
    ("failure", "order"),
    [
        (
            "NIX_CLOSURE_FAIL",
            f"nix-store --query --requisites --include-outputs {DERIVATION}\n",
        ),
        ("NIX_STDIN_FAIL", fetch()),
    ],
)
def test_a_failed_fetch_query_records_no_fetched_bytes_and_changes_nothing_else(
    checkout: Checkout, failure: str, order: str
) -> None:
    result = nix_check(checkout, **{failure: "1"})

    assert result.returncode == 0
    assert result.stdout == b""
    assert result.stderr == b"build-err\n"
    assert checkout.order() == EVAL + BUILD + PATH_INFO + order
    step = checkout.merged()["demo"]["args"]
    assert (step["jaunder.status"], step["jaunder.cache"]) == ("ok", "substituted")
    assert "jaunder.nix.fetched_bytes" not in step


# `steps.nix_check` as a gate's one step, without `dev`'s checkout discovery.
NIX_CHECK = (
    "from pathlib import Path; from dev import steps; "
    'sys.exit(steps.gate(Path.cwd(), "dev nix-check", '
    'lambda: steps.nix_check("demo")))'
)


def test_a_query_that_cannot_start_records_no_cache(checkout: Checkout) -> None:
    # The build succeeds, then `nix` is gone before its output is queried; no
    # other `nix` is on PATH.
    checkout.tool(
        "nix",
        'print("/nix/store/0000-demo")\n'
        'if sys.argv[1] == "build":\n'
        "    os.unlink(sys.argv[0])",
    )
    result = checkout.run(
        launch(NIX_CHECK), checkout.environment(PATH=str(checkout.bin))
    )

    assert result.returncode == 0, result
    assert result.stderr == b""
    step = checkout.merged()["demo"]["args"]
    assert step["jaunder.status"] == "ok"
    assert step["jaunder.nix.output"] == "/nix/store/0000-demo"
    assert "jaunder.cache" not in step
    assert "jaunder.nix.fetched_bytes" not in step


def test_cache_shows_in_the_summary(checkout: Checkout) -> None:
    assert nix_check(checkout, NIX_VALID="1").returncode == 0
    (run,) = checkout.runs()
    summary = checkout.dev("trace", "summary", str(run))
    assert summary.returncode == 0, summary
    heading, *rows = cells(summary.stdout.decode())
    (row,) = [row for row in rows if row[-1] == "demo"]
    assert row[heading.index("Cache")] == "reused"


def test_steps_that_cannot_use_a_cache_record_none(checkout: Checkout) -> None:
    checkout.tool("tsc", "")
    assert checkout.dev("static", "tsc", "--sandbox").returncode == 0
    assert "jaunder.cache" not in checkout.merged()["tsc"]["args"]


@pytest.mark.parametrize("args", [[], ["demo", "extra"]])
def test_nix_check_takes_exactly_one_check(checkout: Checkout, args: list[str]) -> None:
    checkout.nix_store()
    result = checkout.dev("nix-check", *args)
    assert result.returncode == 2, result
    assert checkout.order() == ""
    assert checkout.runs() == []


def build(
    checkout: Checkout, call: str, **extra: str
) -> subprocess.CompletedProcess[bytes]:
    """Runs `steps.nix_build(<call>)` as a gate's one step, printing its result."""
    checkout.nix_store()
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
        "nix path-info --derivation --accept-flake-config .#demo\n"
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


def test_a_derivation_installable_is_built_without_an_evaluation(
    checkout: Checkout,
) -> None:
    drv = "/nix/store/0000-demo.drv"
    result = build(checkout, f'"producer", "{drv}^out", flags=(), keep=False')
    assert result.returncode == 0, result
    # The closure is the derivation's own, without its output selector.
    assert checkout.order() == (
        f"nix build -L --print-out-paths --no-link {drv}^out\n" + PATH_INFO + fetch(drv)
    )
    step = checkout.merged()["producer"]["args"]
    assert step["jaunder.cache"] == "substituted"
    assert step["jaunder.nix.fetched_bytes"] == OUTPUT_SIZE + FETCHED_DEPENDENCIES
    assert list(children(checkout, "producer")) == ["build"]


def test_several_printed_outputs_are_returned_without_a_cache(
    checkout: Checkout,
) -> None:
    result = build(checkout, '"multi", ".#demo", keep=False', NIX_TWO_OUTPUTS="1")
    assert result.returncode == 0, result
    assert result.stdout == b"build-out\nbuild-out-dev\n"
    assert checkout.order() == (
        "nix path-info --derivation --accept-flake-config .#demo\n"
        "nix build -L --accept-flake-config --print-out-paths --no-link .#demo\n"
    )
    step = checkout.merged()["multi"]["args"]
    assert step["jaunder.status"] == "ok"
    for key in ["jaunder.cache", "jaunder.nix.output", "jaunder.nix.fetched_bytes"]:
        assert key not in step


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
    assert "jaunder.nix.output" not in step
    build_step = children(checkout, "producer")["build"]
    assert (build_step["jaunder.status"], build_step["process.exit.code"]) == (
        "failed",
        1,
    )


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
    result = checkout.run(
        launch(NIX_CHECK), checkout.environment(PATH=str(checkout.bin))
    )
    assert result.returncode == 127
    assert result.stderr == b"dev: nix: No such file or directory\n"
    step = checkout.merged()["demo"]["args"]
    assert step["process.exit.code"] == 127
    assert "jaunder.cache" not in step
    assert list(children(checkout, "demo")) == ["eval"]


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


@pytest.mark.parametrize(
    ("entry", "size"),
    [
        # Registered since the start and not built here: substituted.
        ({"registrationTime": START + 1, "ultimate": False, "narSize": 7}, 7),
        ({"registrationTime": START + 1, "narSize": 7}, 7),
        # Registered in the start second counts.
        ({"registrationTime": START, "ultimate": False, "narSize": 7}, 7),
        ({"registrationTime": START - 1, "ultimate": False, "narSize": 7}, 0),
        ({"registrationTime": START + 1, "ultimate": True, "narSize": 7}, 0),
        (None, 0),
        ({"ultimate": False, "narSize": 7}, 0),
        ({"registrationTime": START + 1, "ultimate": False}, 0),
        ({"registrationTime": str(START + 1), "narSize": 7}, 0),
        ({"registrationTime": True, "narSize": 7}, 0),
        ({"registrationTime": START + 1, "narSize": "7"}, 0),
        ({"registrationTime": START + 1, "narSize": True}, 0),
    ],
)
def test_a_substituted_size_counts_only_what_a_substituter_supplied_since_the_start(
    entry: dict[str, Any] | None, size: int
) -> None:
    assert steps.substituted_size(entry, START) == size
