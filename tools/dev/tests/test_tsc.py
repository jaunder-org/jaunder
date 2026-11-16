"""`dev tsc` and `dev static tsc`: arguments, streams, status and their spans."""

import pytest
from conftest import Checkout

TSC = "tsc --noEmit -p end2end/tsconfig.json\n"


def fake_tools(checkout: Checkout) -> None:
    checkout.tool(
        "setup-e2e-node-modules",
        'print("setup-out"); print("setup-err", file=sys.stderr)\n'
        'sys.exit(int(os.environ["SETUP_CODE"]))',
    )
    checkout.tool(
        "tsc",
        'print("tsc-out"); print("tsc-err", file=sys.stderr)\n'
        'sys.exit(int(os.environ["TSC_CODE"]))',
    )


@pytest.mark.parametrize(
    ("mode", "setup", "tsc", "expected"),
    [("verify", 0, 0, 0), ("fix", 0, 17, 17), ("verify", 19, 0, 19)],
)
def test_tsc_provisions_then_type_checks_and_stops_at_the_first_failure(
    checkout: Checkout, mode: str, setup: int, tsc: int, expected: int
) -> None:
    fake_tools(checkout)
    result = checkout.dev(
        "tsc",
        mode,
        cwd=checkout.root / "sub",
        SETUP_CODE=str(setup),
        TSC_CODE=str(tsc),
        E2E_TYPES_NODE_MODULES="types-pin",
        E2E_PLAYWRIGHT_TEST="playwright-pin",
    )

    assert result.returncode == expected, result
    setup_line = "setup-e2e-node-modules types-pin playwright-pin\n"
    if setup == 0:
        assert checkout.order() == setup_line + TSC
        assert result.stdout == b"setup-out\ntsc-out\n"
        assert result.stderr == b"setup-err\ntsc-err\n"
    else:
        assert checkout.order() == setup_line
        assert result.stdout == b"setup-out\n"
        assert result.stderr == b"setup-err\n"

    slices = checkout.merged()
    gate = slices["dev tsc"]
    assert gate["args"]["jaunder.status"] == ("ok" if expected == 0 else "failed")
    assert gate["args"]["process.exit.code"] == expected
    provision = slices["setup-e2e-node-modules"]
    assert provision["args"]["parent_id"] == gate["args"]["span_id"]
    assert provision["args"]["process.command_line"] == (
        "setup-e2e-node-modules types-pin playwright-pin"
    )
    assert provision["args"]["process.exit.code"] == setup
    if setup == 0:
        check = slices["tsc"]
        assert check["args"]["parent_id"] == gate["args"]["span_id"]
        assert check["args"]["process.command_line"] == TSC.strip()
        assert check["args"]["jaunder.status"] == ("ok" if tsc == 0 else "failed")
    else:
        assert "tsc" not in slices


def test_tsc_passes_unset_pins_as_empty_arguments(checkout: Checkout) -> None:
    fake_tools(checkout)
    result = checkout.dev("tsc", SETUP_CODE="0", TSC_CODE="0")
    assert result.returncode == 0, result
    assert checkout.order() == "setup-e2e-node-modules  \n" + TSC


@pytest.mark.parametrize("args", [[], ["--fix"], ["--sandbox"], ["--fix", "--sandbox"]])
def test_static_tsc_only_type_checks(checkout: Checkout, args: list[str]) -> None:
    fake_tools(checkout)
    result = checkout.dev("static", "tsc", *args, TSC_CODE="3")
    assert result.returncode == 3, result
    assert checkout.order() == TSC
    assert result.stdout == b"tsc-out\n"
    assert set(checkout.merged()) == {"dev static tsc", "tsc"}


def test_sandbox_runs_where_it_is_started_without_git(checkout: Checkout) -> None:
    fake_tools(checkout)
    build = checkout.root / "sub"
    result = checkout.dev("static", "tsc", "--sandbox", cwd=build, TSC_CODE="0")
    assert result.returncode == 0, result
    assert (build / "order").read_text() == TSC
    assert len(list((build / ".xtask" / "run").glob("*.trace.json"))) == 1


def test_host_run_outside_a_checkout_is_refused(checkout: Checkout) -> None:
    fake_tools(checkout)
    outside = checkout.bin
    result = checkout.dev(
        "static",
        "tsc",
        cwd=outside,
        TSC_CODE="0",
        GIT_CEILING_DIRECTORIES=str(outside.parent),
    )
    assert result.returncode == 1
    assert b"cannot find the checkout root" in result.stderr
    assert not (outside / "order").exists()


@pytest.mark.parametrize(
    "args", [["tsc", "bogus"], ["tsc", "--bogus"], ["static", "tsc", "x"]]
)
def test_unknown_arguments_are_refused_before_any_work(
    checkout: Checkout, args: list[str]
) -> None:
    fake_tools(checkout)
    result = checkout.dev(*args, SETUP_CODE="0", TSC_CODE="0")
    assert result.returncode == 2
    assert checkout.order() == ""
    assert checkout.runs() == []


def test_missing_tool_fails_the_step_like_a_shell(checkout: Checkout) -> None:
    result = checkout.dev("static", "tsc", "--sandbox", PATH=str(checkout.bin))
    assert result.returncode == 127
    assert result.stderr.startswith(b"dev: tsc: ")
    assert checkout.merged()["tsc"]["args"]["process.exit.code"] == 127


@pytest.mark.parametrize("inherited", [None, "/caller/site-packages"])
def test_steps_inherit_the_callers_pythonpath_unchanged(
    checkout: Checkout, inherited: str | None
) -> None:
    checkout.tool("tsc", 'print(os.environ.get("PYTHONPATH"))')
    environment = checkout.environment()
    if inherited is not None:
        environment["PYTHONPATH"] = inherited
    result = checkout.run(checkout.command("static", "tsc"), environment)
    assert result.returncode == 0, result
    assert result.stdout == f"{inherited}\n".encode()


def test_tool_that_cannot_run_fails_the_step_like_a_shell(checkout: Checkout) -> None:
    fake_tools(checkout)
    (checkout.bin / "tsc").chmod(0o644)
    result = checkout.dev("static", "tsc", "--sandbox", PATH=str(checkout.bin))
    assert result.returncode == 126
    assert result.stderr.startswith(b"dev: tsc: ")
    assert b"Traceback" not in result.stderr
    tsc = checkout.merged()["tsc"]["args"]
    assert (tsc["jaunder.status"], tsc["process.exit.code"]) == ("failed", 126)
