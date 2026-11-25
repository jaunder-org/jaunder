"""The remaining entry points: each one's commands, argv and admission.

Expected commands are literal, not derived from `dev`'s own tables.
"""

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from conftest import (
    FIXTURES,
    PACKAGE,
    REGISTERED_LONG_AGO,
    TOOL_PACKAGES,
    Checkout,
    Workflow,
    lift_lanes,
    write_lane_capture,
)

from dev.e2e_evidence import LaneEvidence
from dev.traces import read_trace_member

SHELL_SYNTAX = "a b ; touch pwned $(touch pwned)"

Commands = list[list[str]]


def words(line: str, *tail: str) -> list[str]:
    return [*line.split(" "), *tail]


def build(package: str) -> Commands:
    """A tool package's build step; the fake output holds the fake as each of
    its binaries, which then records itself under the binary's name."""
    return [
        words(f"nix path-info --derivation --accept-flake-config .#{package}"),
        words(
            "nix build -L --accept-flake-config --print-out-paths --no-link "
            f".#{package}"
        ),
    ]


FORWARDED: list[tuple[list[str], Commands]] = [
    (
        ["sandbox", "--port", "0", SHELL_SYNTAX],
        [
            words(
                "nix run --no-update-lock-file --no-write-lock-file .#ux-sandbox --",
                "--port",
                "0",
                SHELL_SYNTAX,
            )
        ],
    ),
    (
        ["sandbox"],
        [words("nix run --no-update-lock-file --no-write-lock-file .#ux-sandbox --")],
    ),
    (
        ["pg-run", "psql", "-c", "select 1"],
        [["pg-run", "psql", "-c", "select 1"]],
    ),
    # A leading `--` is optional and dropped; any later one is forwarded.
    (
        ["pg-run", "--", "psql", "--", "-c"],
        [["pg-run", "psql", "--", "-c"]],
    ),
    (
        ["coverage-check", "probe-source", "--help"],
        [
            *build("coverage-check"),
            ["coverage-check", "probe-source", "--help"],
        ],
    ),
    (
        ["coverage-check", "--flag", SHELL_SYNTAX],
        [*build("coverage-check"), ["coverage-check", "--flag", SHELL_SYNTAX]],
    ),
    (
        ["adr-format"],
        [
            *build("repository-policy"),
            words("repository-docs adr-format"),
        ],
    ),
    (["e2e"], [["bash", "scripts/e2e-workflow", "all", ""]]),
    (["e2e", "3"], [["bash", "scripts/e2e-workflow", "all", "3"]]),
    (
        ["pr-land", "731", "f" * 40],
        [["bash", "scripts/pr-land.sh", "731", "f" * 40, "ordinary"]],
    ),
    (
        ["pr-land", "731", "f" * 40, "--reenqueue"],
        [["bash", "scripts/pr-land.sh", "731", "f" * 40, "--reenqueue"]],
    ),
    (
        ["pr-cleanup", "731", "f" * 40, "unmanaged"],
        [["bash", "scripts/pr-cleanup.sh", "731", "f" * 40, "unmanaged"]],
    ),
    (["upgrade-dispatch"], [words("gh workflow run upgrade.yml --ref main")]),
    (
        ["upgrade-dispatch", "--from", "earlier", "--ref", "topic"],
        [
            words("git rev-parse --verify --end-of-options earlier^{commit}"),
            words("git branch --remotes --list origin/* --contains", "b" * 40),
            words("gh workflow run upgrade.yml --ref topic -f", f"from={'b' * 40}"),
        ],
    ),
]


# Answers `nix path-info` for the selected lane output: built here unless
# ULTIMATE says otherwise, with no registration time, so no cache source is
# recorded.
BUILT_NIX = r"""#!/bin/sh
case "$1 $2" in
  'path-info --derivation') ;;
  path-info*)
    for path; do :; done
    printf '{"%s":{"ultimate":%s}}\n' "$path" "${ULTIMATE:-true}"
    exit 0 ;;
  eval*) exit 1 ;;
esac
printf '%s\037' nix "$@" >> "$FAKE_LOG"
printf '\036' >> "$FAKE_LOG"
"""


def test_a_lane_this_machine_did_not_build_fails_before_verification(
    workflow: Workflow,
) -> None:
    lane = "sqlite-chromium-1-of-3"
    record = workflow.root / ".xtask" / "diagnostics" / f"e2e-{lane}" / "build.json"
    record.parent.mkdir(parents=True)
    record.write_text(json.dumps([{"outputs": {"out": "/nix/store/0000-selected"}}]))
    workflow.checkout.executable("nix", BUILT_NIX)
    result = workflow.dev("e2e-producer", lane, ULTIMATE="false")
    assert result.returncode == 1, result
    assert b"dev: e2e-producer: verdict not executed here" in result.stderr
    assert workflow.commands() == [["bash", "scripts/e2e-workflow", "producer", lane]]
    assert b"e2e-evidence" not in result.stdout


def catalog(root: Path) -> None:
    """The lane catalog the verdicts read, at its checkout path."""
    path = root / "end2end" / "e2e-lanes.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(FIXTURES / "e2e-lanes.json", path)


@pytest.mark.parametrize("passing", [True, False])
def test_a_producer_then_verifies_the_output_its_build_selected_in_process(
    workflow: Workflow, passing: bool
) -> None:
    lane = "sqlite-firefox-1-of-5"
    root = workflow.root
    catalog(root)
    selected = root.parent / "selected"
    if passing:
        lift_lanes(selected)
    record = root / ".xtask" / "diagnostics" / f"e2e-{lane}" / "build.json"
    record.parent.mkdir(parents=True)
    output = selected / f"e2e-{lane}"
    record.write_text(json.dumps([{"outputs": {"out": str(output)}}]))
    workflow.checkout.executable("nix", BUILT_NIX)
    result = workflow.dev("e2e-producer", lane)
    assert result.returncode == (0 if passing else 1), result
    # Verification builds and runs no tool.
    assert workflow.commands() == [["bash", "scripts/e2e-workflow", "producer", lane]]
    shown = result.stdout.decode()
    if passing:
        assert shown.startswith("[ ok ] e2e-evidence"), shown
        assert shown.endswith(
            f" — {lane}: population, retry-duration and boot evidence verified\n"
        )
    else:
        assert shown.startswith("[FAIL] e2e-evidence ("), shown
        assert f"reading report {output}/playwright-report-{lane}.json" in shown


# The longest measured phase; the fake producer sleeps this long before
# reporting, so every phase it reports has ended.
LONGEST_PHASE_MS = 3

# A failed producer that leaves this run's phase manifest, unless PHASES is
# empty. "@NOW@" is the producer's clock when its phases start. Like a real
# producer, it reports them only once they have ended, so `dev` reading the
# manifest straight away still finds them inside the step.
PHASE_PRODUCER = f"""#!{sys.executable}
import os, pathlib, sys, time
lane = sys.argv[-1]
directory = pathlib.Path(".xtask", "diagnostics", f"e2e-{{lane}}")
directory.mkdir(parents=True, exist_ok=True)
if os.environ["PHASES"]:
    started = time.time_ns()
    time.sleep({LONGEST_PHASE_MS} / 1000)
    text = os.environ["PHASES"].replace("@NOW@", str(started))
    (directory / f"e2e-phase-{{lane}}.json").write_text(text)
sys.exit(1)
"""
NOW = "@NOW@"
MEASURED = json.dumps(
    {
        "phases": [
            {
                "name": "vm-startup-readiness",
                "started_at_unix_ns": NOW,
                "duration_ms": LONGEST_PHASE_MS,
                "outcome": "success",
            },
            {
                "name": "gate-execution",
                "started_at_unix_ns": NOW,
                "duration_ms": LONGEST_PHASE_MS - 1,
                "outcome": "failed",
            },
            {"name": "nix-evaluation", "duration_ms": None, "outcome": "unavailable"},
            {
                "name": "earlier-run",
                "started_at_unix_ns": 1,
                "duration_ms": 1,
                "outcome": "success",
            },
            "not a phase",
        ]
    }
).replace(f'"{NOW}"', NOW)


@pytest.mark.parametrize(
    ("phases", "children", "warned"),
    [
        (
            MEASURED,
            {
                "vm-startup-readiness": (LONGEST_PHASE_MS * 1000, "ok"),
                "gate-execution": ((LONGEST_PHASE_MS - 1) * 1000, "failed"),
            },
            False,
        ),
        ("", {}, False),
        ('{"phases": 3}', {}, False),
        ("{", {}, True),
    ],
    ids=["measured", "missing", "not-a-list", "malformed"],
)
def test_a_failed_producer_shows_only_this_runs_measured_phases(
    workflow: Workflow,
    phases: str,
    children: dict[str, tuple[int, str]],
    warned: bool,
) -> None:
    workflow.checkout.executable("bash", PHASE_PRODUCER)
    lane = "sqlite-chromium-1-of-3"
    result = workflow.dev("e2e-producer", lane, PHASES=phases)
    assert result.returncode == 1, result
    assert (b"no phase spans" in result.stderr) == warned, result.stderr
    events = workflow.checkout.events()
    (step,) = [event for event in events if event["name"] == "e2e-producer"]
    assert step["args"]["jaunder.status"] == "failed"
    shown = {
        event["name"]: (event["dur"], event["args"]["jaunder.status"])
        for event in events
        if event["args"]["parent_id"] == step["args"]["span_id"]
    }
    assert shown == children


@pytest.mark.parametrize(("args", "expected"), FORWARDED)
def test_each_entry_point_runs_its_command_with_arguments_kept_as_argv(
    workflow: Workflow, args: list[str], expected: Commands
) -> None:
    result = workflow.dev(*args)
    assert result.returncode == 0, result
    assert workflow.commands() == expected
    assert not (workflow.root / "pwned").exists()
    assert f"dev {args[0]}" in workflow.checkout.merged()


LAST_GREEN = words(
    "gh run list --workflow upgrade.yml --branch main --status success --limit 1"
    " --json headSha --jq",
    '.[0].headSha // ""',
)


@pytest.mark.parametrize(
    ("args", "last_green", "resolution"),
    [
        (
            ["--from", "earlier"],
            "",
            [words("git rev-parse --verify --end-of-options earlier^{commit}")],
        ),
        (
            ["--tag", "v2"],
            "",
            [
                words("git tag --list v* --sort=-version:refname --merged v2^"),
                words("git rev-parse --verify --end-of-options v1^{commit}"),
            ],
        ),
        (
            ["--tag", "v1"],
            "",
            [
                words("git tag --list v* --sort=-version:refname --merged v1^"),
                words("git rev-parse --verify --end-of-options v1^{commit}"),
            ],
        ),
        (
            [],
            "c" * 40,
            [
                LAST_GREEN,
                words(f"git rev-parse --verify --end-of-options {'c' * 40}^{{commit}}"),
            ],
        ),
        (
            [],
            "",
            [
                LAST_GREEN,
                words("git rev-parse --verify --end-of-options HEAD^{commit}"),
            ],
        ),
    ],
)
def test_upgrade_test_resolves_its_earlier_build_first(
    workflow: Workflow,
    args: list[str],
    last_green: str,
    resolution: Commands,
) -> None:
    result = workflow.dev("upgrade-test", *args, "sqlite", GH_LAST_GREEN=last_green)
    assert result.returncode == 0, result
    assert workflow.commands()[: len(resolution)] == resolution


def test_upgrade_test_lifts_a_failed_checks_artifacts(workflow: Workflow) -> None:
    output = workflow.root / "kept-output"
    output.mkdir()
    (output / "upgrade-baseline-journal-upgrade-sqlite.log").write_text("journal")
    (output / "nested").mkdir()
    result = workflow.dev(
        "upgrade-test",
        "--from",
        "earlier",
        "sqlite",
        UPGRADE_OUTPUT=str(output),
        failure="nix build -L --accept-flake-config --keep-failed --print-out-paths"
        " --out-link .xtask/gcroots/upgrade-sqlite ^out",
    )
    assert result.returncode != 0, result
    lifted = workflow.root / ".xtask/diagnostics/upgrade-sqlite"
    assert sorted(path.name for path in lifted.iterdir()) == [
        "upgrade-baseline-journal-upgrade-sqlite.log"
    ]


def test_upgrade_test_refuses_both_an_explicit_build_and_a_tag(
    workflow: Workflow,
) -> None:
    result = workflow.dev("upgrade-test", "--from", "earlier", "--tag", "v2", "sqlite")
    assert result.returncode == 2, result
    assert workflow.commands() == []


def test_upgrade_dispatch_refuses_an_earlier_build_absent_from_origin(
    workflow: Workflow,
) -> None:
    (workflow.root / "not-on-origin").touch()
    result = workflow.dev("upgrade-dispatch", "--from", "earlier")
    assert result.returncode != 0, result
    assert ["gh", "workflow", "run"] not in [
        command[:3] for command in workflow.commands()
    ]
    refusal = f"{'b' * 40} is on no origin branch".encode()
    assert refusal in result.stdout + result.stderr


@pytest.mark.parametrize("code", [0, 3])
def test_a_forwarded_tool_keeps_its_streams_and_status(
    checkout: Checkout, code: int
) -> None:
    checkout.tool(
        "nix",
        f'print("sandbox out"); print("sandbox err", file=sys.stderr); sys.exit({code})',
    )
    result = checkout.dev("sandbox", "--help")
    assert result.returncode == code
    assert (result.stdout, result.stderr) == (b"sandbox out\n", b"sandbox err\n")


@pytest.mark.parametrize("output", [b"not a report\n", b"", b" \n"])
@pytest.mark.parametrize("command", ["trace-analysis", "coverage-check"])
def test_report_tools_fail_closed_without_a_report(
    checkout: Checkout, command: str, output: bytes
) -> None:
    checkout.tool("nix", store_nix())
    checkout.built(command, f"sys.stdout.buffer.write({output!r})")
    result = checkout.dev(command, "analyze")
    assert result.returncode == 1
    assert result.stdout == output
    assert result.stderr == f"dev: {command}: output is not a step report\n".encode()


@pytest.mark.parametrize("help_args", [["--help"], ["-h"], ["report", "--help"]])
@pytest.mark.parametrize(
    "command",
    ["coverage-check"],
)
def test_report_tools_show_their_help_unchanged(
    checkout: Checkout, command: str, help_args: list[str]
) -> None:
    checkout.tool("nix", store_nix())
    checkout.built(command, 'print("Usage: tool [OPTIONS]")')
    result = checkout.dev(command, *help_args)
    assert (result.returncode, result.stdout, result.stderr) == (
        0,
        b"Usage: tool [OPTIONS]\n",
        b"",
    )


def store_nix(output: str = "/nix/store/fake-out") -> str:
    """A fake `nix` whose tool packages build to FAKE_TOOL_OUT and every other
    installable to `output`, all registered in its store long ago."""
    return (
        "import json\n"
        'if "--derivation" in sys.argv:\n'
        '    print("/nix/store/fake.drv")\n'
        'elif sys.argv[1] == "path-info":\n'
        f"    entry = {{'ultimate': False, 'registrationTime': {REGISTERED_LONG_AGO}}}\n"
        "    print(json.dumps({sys.argv[-1]: entry}))\n"
        f"elif sys.argv[-1].removeprefix('.#') in {list(TOOL_PACKAGES)!r}:\n"
        '    print(os.environ["FAKE_TOOL_OUT"])\n'
        "else:\n"
        f"    print({output!r})\n"
    )


# This checkout's server-fn inventory, as `source-contracts` lists it: one fn
# the fixture lanes never drive.
INVENTORY = """\
import json
print(json.dumps([{"qualified": "posts::create", "module": "posts::api",
                   "ident": "create", "endpoint": "posts/create"}]))
"""
# The snapshot coverage of those lanes renders.
UNCOVERED = '{\n  "covered": [],\n  "orphans": {}\n}\n'


def verdict_lines(result: subprocess.CompletedProcess[bytes]) -> list[str]:
    """The shown verdicts' lines, each step's timing dropped."""
    return re.sub(r" \(\d+ ms\) —", " —", result.stdout.decode()).splitlines()


def test_e2e_reconcile_verifies_artifacts_in_process(checkout: Checkout) -> None:
    checkout.built("source-contracts", INVENTORY)
    checkout.tool("nix", store_nix())
    catalog(checkout.root)
    lift_lanes(checkout.root / ".xtask" / "diagnostics")
    lift_lanes(checkout.root / "downloaded artifacts")
    lift_lanes(checkout.root / ".xtask" / "diagnostics", "chromium")
    snapshot = checkout.root / "docs" / "coverage" / "server-fns.json"
    snapshot.parent.mkdir(parents=True)
    snapshot.write_text(UNCOVERED)
    for browser, args, lanes in [
        ("firefox", [], 5),
        ("firefox", ["downloaded artifacts"], 5),
        ("chromium", [], 3),
    ]:
        result = checkout.dev("e2e-reconcile", "sqlite", browser, *args)
        assert result.returncode == 0, result
        expected = [
            (
                f"[ ok ] e2e-reconcile — sqlite/{browser}: reconciled {lanes} "
                "lane-qualified evidence sets"
            )
        ]
        # The SQLite/Chromium verdict includes coverage over the same artifacts.
        if browser == "chromium":
            expected.append(
                "[ ok ] server-fn-coverage-verify — 0 covered; snapshot current"
            )
            step = checkout.merged()["repository-policy-build"]["args"]
            assert step["jaunder.cache"] == "reused"
        assert verdict_lines(result) == expected
        shutil.rmtree(checkout.root / ".xtask" / "run")
    # Only coverage runs a tool: the inventory, from this checkout's own build.
    assert [line for line in checkout.order().splitlines() if "nix" not in line] == [
        "source-contracts server-fn-inventory"
    ]
    assert [line for line in checkout.order().splitlines() if " build " in line] == [
        " ".join(build("repository-policy")[-1])
    ]


def test_e2e_reconcile_refuses_drift_and_unreconciled_lanes(checkout: Checkout) -> None:
    checkout.built("source-contracts", INVENTORY)
    checkout.tool("nix", store_nix())
    catalog(checkout.root)
    result = checkout.dev("e2e-reconcile", "sqlite", "chromium", "missing")
    assert result.returncode == 1, result
    shown = verdict_lines(result)
    assert shown[0].startswith(
        "[FAIL] e2e-reconcile — reading expected census missing/"
    )
    assert "server-fn-coverage" not in "\n".join(shown)
    assert checkout.order() == "", "an unreconciled pair never reaches coverage"
    lift_lanes(checkout.root / "lifted", "chromium")
    result = checkout.dev("e2e-reconcile", "sqlite", "chromium", "lifted")
    assert result.returncode == 1, result
    assert verdict_lines(result)[1] == (
        "[FAIL] server-fn-coverage-verify — docs/coverage/server-fns.json is out of "
        "date with this run's traces — regenerate it with "
        "`dev e2e-regenerate-coverage` and commit the result"
    )


def test_e2e_regenerate_coverage_produces_the_pair_then_regenerates(
    checkout: Checkout,
) -> None:
    checkout.built("source-contracts", INVENTORY)
    checkout.tool("nix", store_nix())
    catalog(checkout.root)
    # The pair's producers are recorded, not run; their lanes are already lifted.
    checkout.tool("bash", "")
    lift_lanes(checkout.root / ".xtask" / "diagnostics", "chromium")
    result = checkout.dev("e2e-regenerate-coverage")
    assert result.returncode == 0, result
    order = checkout.order().splitlines()
    pair = order.index("bash scripts/e2e-workflow pair sqlite chromium")
    build = next(i for i, line in enumerate(order) if ".#repository-policy" in line)
    assert pair < build, order
    snapshot = checkout.root / "docs" / "coverage" / "server-fns.json"
    assert snapshot.read_text() == UNCOVERED
    assert verdict_lines(result) == [
        (
            "[ ok ] server-fn-coverage-regenerate — 0 covered, 0 fn(s) with "
            "unattributed hits → docs/coverage/server-fns.json"
        )
    ]


def test_e2e_regenerate_coverage_refuses_unreconciled_lanes(checkout: Checkout) -> None:
    checkout.tool("nix", store_nix())
    catalog(checkout.root)
    checkout.tool("bash", "")
    result = checkout.dev("e2e-regenerate-coverage")
    assert result.returncode == 1, result
    assert verdict_lines(result)[0].startswith(
        "[FAIL] server-fn-coverage-regenerate — the SQLite/Chromium lanes must "
        "reconcile before flow coverage: reading expected census"
    )
    assert not (checkout.root / "docs" / "coverage").exists()


# Each coverage verdict: its command, and where its line falls among those shown.
COVERAGE_MODES = {
    "verify": (["e2e-reconcile", "sqlite", "chromium", ".xtask/diagnostics"], 1),
    "regenerate": (["e2e-regenerate-coverage"], 0),
}
# The OpenTelemetry SDK's own record of losing spans, in a server's diag.log.
SPAN_LOSS = json.dumps(
    {
        "level": "WARN",
        "target": "opentelemetry_sdk",
        "fields": {"name": "BatchSpanProcessor.SpanDroppingStarted"},
    }
)


def coverage_lanes(checkout: Checkout, inventory: str) -> list[LaneEvidence]:
    """The coverage pair's lanes, lifted where both coverage verdicts read
    them, with `inventory` as this checkout's `source-contracts` body."""
    checkout.built("source-contracts", inventory)
    checkout.tool("nix", store_nix())
    catalog(checkout.root)
    # The pair's producers are recorded, not run; their lanes are already lifted.
    checkout.tool("bash", "")
    return lift_lanes(checkout.root / ".xtask" / "diagnostics", "chromium")


@pytest.mark.parametrize("mode", list(COVERAGE_MODES))
def test_coverage_refuses_a_lane_whose_server_lost_spans(
    checkout: Checkout, mode: str
) -> None:
    lanes = coverage_lanes(checkout, INVENTORY)
    capture = lanes[1].capture
    write_lane_capture(capture, read_trace_member(capture), SPAN_LOSS.encode())
    args, line = COVERAGE_MODES[mode]
    result = checkout.dev(*args)
    assert result.returncode == 1, result
    assert verdict_lines(result)[line].startswith(
        f"[FAIL] server-fn-coverage-{mode} — refusing "
        f"{capture.relative_to(checkout.root)}: the capture is incomplete: the "
        "server's OpenTelemetry SDK lost an unknown number of span(s)"
    )
    assert not (checkout.root / "docs" / "coverage").exists()


@pytest.mark.parametrize("mode", list(COVERAGE_MODES))
@pytest.mark.parametrize(
    ("inventory", "detail"),
    [
        ("sys.exit(3)", "listing the server-fn inventory failed with status 3"),
        ("print('not a listing')", "the server-fn inventory listing is malformed: "),
    ],
)
def test_coverage_without_an_inventory_is_a_failed_verdict(
    checkout: Checkout, mode: str, inventory: str, detail: str
) -> None:
    coverage_lanes(checkout, inventory)
    args, line = COVERAGE_MODES[mode]
    result = checkout.dev(*args)
    assert result.returncode == 1, result
    assert verdict_lines(result)[line].startswith(
        f"[FAIL] server-fn-coverage-{mode} — {detail}"
    )


def test_a_signalled_inventory_ends_the_gate_without_a_verdict(
    checkout: Checkout,
) -> None:
    coverage_lanes(checkout, "os.kill(os.getpid(), signal.SIGTERM)")
    result = checkout.dev("e2e-regenerate-coverage")
    assert result.returncode == 128 + 15, result
    assert b"server-fn-coverage-regenerate" not in result.stdout


def test_a_snapshot_regeneration_cannot_write_is_a_failed_verdict(
    checkout: Checkout,
) -> None:
    coverage_lanes(checkout, INVENTORY)
    (checkout.root / "docs").mkdir()
    (checkout.root / "docs" / "coverage").write_text("not a directory")
    result = checkout.dev("e2e-regenerate-coverage")
    assert result.returncode == 1, result
    assert verdict_lines(result) == [
        (
            "[FAIL] server-fn-coverage-regenerate — writing "
            "docs/coverage/server-fns.json: [Errno 17] File exists: 'docs/coverage'"
        )
    ]


STATIC_ALL = [
    words("cargo fmt --check"),
    *build("repository-policy"),
    words("scoped-leptosfmt verify"),
    words("emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check"),
    words("cargo fmt --manifest-path tools/Cargo.toml --all --check"),
    words("ast-grep test --config sgconfig.yml"),
    words(
        "ast-grep scan --config sgconfig.yml --filter ^no-full-reload$ web/src client/src"
    ),
    words("emacs --batch -Q -l elisp/scripts/byte-compile.el"),
    ["setup-e2e-node-modules", "", ""],
    words("tsc --noEmit -p end2end/tsconfig.json"),
    words("cargo deny check"),
    words(
        "cargo clippy --workspace --exclude web --exclude client --exclude csr "
        "--all-targets -- -D warnings"
    ),
    words("cargo clippy -p web --features server --all-targets -- -D warnings"),
    words("cargo clippy -p web --no-default-features --all-targets -- -D warnings"),
    words("cargo clippy -p client --features csr --all-targets -- -D warnings"),
    words(
        "cargo clippy -p web -p client -p csr --features csr "
        "--target wasm32-unknown-unknown -- -D warnings"
    ),
    words(
        "cargo clippy --manifest-path tools/Cargo.toml --workspace --all-targets "
        "--features coverage/host -- -D warnings"
    ),
    words("emacs --batch -Q -l elisp/scripts/run-tests.el"),
]


def test_static_all_runs_every_static_check_each_prepared_here(
    workflow: Workflow,
) -> None:
    result = workflow.dev("static-all")
    assert result.returncode == 0, result
    assert workflow.commands() == STATIC_ALL
    result = workflow.dev("static-all", "fix", failure="cargo fmt")
    assert result.returncode == 17
    assert workflow.commands() == [words("cargo fmt")]


CSR_ASSETS = """\
script, wasm, project, _input, artifacts, dist = sys.argv[1:]
pathlib.Path(dist).mkdir()
pathlib.Path(dist, "index.html").write_text(
    " ".join([wasm, os.environ["CSR_VITE_PUBLIC_DIR"],
              os.readlink(pathlib.Path(project, "node_modules"))])
)
"""


@pytest.mark.parametrize(
    ("profile", "release"), [("debug", []), ("release", ["--release"])]
)
def test_build_csr_assembles_the_bundle_into_target_site(
    checkout: Checkout, profile: str, release: list[str]
) -> None:
    for name in [
        "package.json",
        "package-lock.json",
        "vite.config.js",
        "csr/index.html",
        "csr/entry.js",
    ]:
        (checkout.root / name).parent.mkdir(exist_ok=True)
        (checkout.root / name).write_text(name)
    stale = checkout.root / "target" / "site" / "stale"
    stale.parent.mkdir(parents=True)
    stale.write_text("old")
    checkout.tool("nix", store_nix("/vite-tools"))
    checkout.tool("cargo", "")
    checkout.built("system-artifacts", "")
    checkout.tool("bash", CSR_ASSETS)
    args = [profile] if profile == "release" else []
    result = checkout.dev("build-csr", *args)
    assert result.returncode == 0, result
    site = checkout.root / "target" / "site"
    assert [path.name for path in site.iterdir()] == ["index.html"]
    assert (site / "index.html").read_text() == (
        f"target/wasm32-unknown-unknown/{profile}/csr.wasm "
        f"{checkout.root}/public /vite-tools/node_modules"
    )
    order = [
        line
        for line in checkout.order().splitlines()
        if not line.startswith(("nix eval ", "nix path-info "))
    ]
    assert checkout.merged()["csr-vite-node-modules"]["args"]["jaunder.cache"] == (
        "reused"
    )
    assert order[:2] == [
        (
            "nix build -L --accept-flake-config --print-out-paths --no-link "
            ".#csr-vite-node-modules"
        ),
        " ".join(
            [
                "cargo",
                "build",
                "--locked",
                "-p",
                "csr",
                "--target",
                "wasm32-unknown-unknown",
                *release,
            ]
        ),
    ]
    assert order[2] == " ".join(build("system-artifacts")[-1])
    assert order[3].startswith("system-artifacts --out ")
    assert order[4].startswith(f"bash {checkout.root}/scripts/build-csr-assets.sh ")


@pytest.mark.parametrize(
    "args",
    [
        ["perf", *"12345678"],
        ["perf-compare"],
        ["perf-import-baseline", "1", "2"],
        ["e2e", "1", "2"],
        ["e2e-producer"],
        ["e2e-reconcile"],
        ["e2e-reconcile", "sqlite"],
        ["issue-discover"],
        ["issue-create", "title", "Task", "3", "P2", "tooling"],
        ["issue-edit", "731", "title", "Task", "3", "P2", "tooling"],
        ["pr-land", "731"],
        ["pr-cleanup", "731", "f" * 40],
        ["upgrade-test"],
        ["upgrade-test", "--from", "earlier"],
        ["upgrade-test", "mysql"],
        ["upgrade-dispatch", "extra"],
        ["build-csr", "bogus"],
        ["static-all", "bogus"],
        ["adr-format", "extra"],
    ],
)
def test_missing_or_extra_arguments_are_refused_before_any_work(
    workflow: Workflow, args: list[str]
) -> None:
    result = workflow.dev(*args)
    assert result.returncode == 2, result
    assert workflow.commands() == []
    assert workflow.checkout.runs() == []


def test_forwarded_help_belongs_to_the_tool(workflow: Workflow) -> None:
    result = workflow.dev("pg-run", "--help")
    assert result.returncode == 0, result
    assert workflow.commands() == [["pg-run", "--help"]]


def test_the_package_runs_as_a_module(checkout: Checkout) -> None:
    result = subprocess.run(
        [sys.executable, "-m", "dev", "--help"],
        cwd=PACKAGE,
        env=checkout.environment(),
        capture_output=True,
        check=False,
        timeout=60,
    )
    assert result.returncode == 0, result
    assert result.stdout.startswith(b"Usage: dev ")
