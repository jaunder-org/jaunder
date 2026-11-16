"""The remaining recipes: each entry point's commands, argv and admission.

Expected commands are literal, not derived from `dev`'s own tables.
"""

import json

import pytest
from conftest import Checkout, Workflow

CARGO_RUN = "cargo run --locked --quiet --manifest-path tools/Cargo.toml"
HOST_PROOF = f"{CARGO_RUN} -p host-proof --bin"
PERF_DATA = f"{CARGO_RUN} -p performance --bin perf-data --"
SHELL_SYNTAX = "a b ; touch pwned $(touch pwned)"

Commands = list[list[str]]


def words(line: str, *tail: str) -> list[str]:
    return [*line.split(" "), *tail]


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
        ["census", "--json", SHELL_SYNTAX],
        [
            words(
                f"{CARGO_RUN} -p repository-policy --bin source-contracts -- census",
                "--json",
                SHELL_SYNTAX,
            )
        ],
    ),
    (
        ["pg-run", "psql", "-c", "select 1"],
        [words(f"{HOST_PROOF} pg-run --", "psql", "-c", "select 1")],
    ),
    # A leading `--` is optional and dropped; any later one is forwarded.
    (
        ["pg-run", "--", "psql", "--", "-c"],
        [words(f"{HOST_PROOF} pg-run --", "psql", "--", "-c")],
    ),
    (
        ["census", "--json", "--", "--"],
        [
            words(
                f"{CARGO_RUN} -p repository-policy --bin source-contracts -- census",
                "--json",
                "--",
                "--",
            )
        ],
    ),
    (
        ["org-timezone-manifest", "--", "-x.org"],
        [
            words(
                f"{CARGO_RUN} -p repository-policy --bin org-timezone-manifest -- --",
                "-x.org",
            )
        ],
    ),
    (
        ["coverage-check", "probe-source", "--help"],
        [
            words(
                f"{CARGO_RUN} -p coverage --features host --bin coverage-check --",
                "probe-source",
                "--help",
            )
        ],
    ),
    *(
        (
            [binary, "--flag", SHELL_SYNTAX],
            [words(f"{HOST_PROOF} {binary} --", "--flag", SHELL_SYNTAX)],
        )
        for binary in [
            "wasm-audit",
            "source-diagnostics",
            "trace-analysis",
            "e2e-local",
        ]
    ),
    (
        ["e2e-local", "--browser", "firefox", "end2end/tests/auth.spec.ts:12"],
        [
            words(
                f"{HOST_PROOF} e2e-local --",
                "--browser",
                "firefox",
                "end2end/tests/auth.spec.ts:12",
            )
        ],
    ),
    (
        ["pr-watch", "731", "--once", "--expected-head", "f" * 40],
        [
            words(
                f"{CARGO_RUN} -p pr-observer -- watch",
                "731",
                "--once",
                "--expected-head",
                "f" * 40,
            )
        ],
    ),
    (
        ["org-timezone-manifest", "a.org", SHELL_SYNTAX],
        [
            words(
                f"{CARGO_RUN} -p repository-policy --bin org-timezone-manifest -- --",
                "a.org",
                SHELL_SYNTAX,
            )
        ],
    ),
    (
        ["adr-format"],
        [
            words(
                f"{CARGO_RUN} -p repository-policy --bin repository-docs -- adr-format"
            )
        ],
    ),
    (
        ["perf-compare", "candidate evidence.json"],
        [words(f"{PERF_DATA} compare --result", "candidate evidence.json")],
    ),
    (["perf-import-baseline", "42"], [words(f"{PERF_DATA} import-baseline 42")]),
    (["e2e"], [["bash", "scripts/e2e-workflow", "all", ""]]),
    (["e2e", "3"], [["bash", "scripts/e2e-workflow", "all", "3"]]),
    (
        ["e2e-producer", "sqlite-chromium-unsplit"],
        [["bash", "scripts/e2e-workflow", "producer", "sqlite-chromium-unsplit"]],
    ),
    (
        ["e2e-regenerate-coverage"],
        [
            words(
                "bash scripts/e2e-workflow producer sqlite-chromium-unsplit regenerate"
            )
        ],
    ),
    (
        ["issue-discover", SHELL_SYNTAX],
        [["bash", "scripts/issue-discover.sh", SHELL_SYNTAX]],
    ),
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
    (["production-baseline-smoke"], [words("nix run .#qualification-host")]),
]


@pytest.mark.parametrize(("args", "expected"), FORWARDED)
def test_each_entry_point_runs_its_command_with_arguments_kept_as_argv(
    workflow: Workflow, args: list[str], expected: Commands
) -> None:
    result = workflow.dev(*args)
    assert result.returncode == 0, result
    assert workflow.commands() == expected
    assert not (workflow.root / "pwned").exists()
    assert f"dev {args[0]}" in workflow.checkout.merged()


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


@pytest.mark.parametrize("command", ["wasm-audit", "coverage-check", "e2e-local"])
def test_report_tools_fail_closed_without_a_report(
    checkout: Checkout, command: str
) -> None:
    checkout.tool("cargo", 'print("not a report")')
    result = checkout.dev(command, "budget")
    assert result.returncode == 5
    assert result.stdout == b"not a report\n"
    assert result.stderr.endswith(b"output is not a step report\n")


# Ported from repository-policy's org_frontdoor: the front door forwards
# explicit local originals only. Nothing is bootstrapped and nothing but the
# trace run directory (`.xtask/run`, which `dev` writes for every run) appears.
ORG_TOOL = """\
files = sys.argv[sys.argv.index("--") + 2:]
reports = [{"file": name} for name in files if pathlib.Path(name).is_file()]
print(json.dumps({"mappings": reports}))
sys.exit(0 if len(reports) == len(files) else 1)
"""


def test_org_timezone_manifest_forwards_originals_without_bootstrap(
    checkout: Checkout,
) -> None:
    checkout.tool("cargo", "import json\n" + ORG_TOOL)
    name = "local ; touch unexpected.org"
    text = "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n"
    (checkout.root / name).write_text(text)
    for file, status in [(name, 0), ("missing.org", 1)]:
        result = checkout.dev(
            "org-timezone-manifest",
            file,
            GITHUB_STEP_SUMMARY=str(checkout.root / "summary.md"),
        )
        assert result.returncode == status, result
        report = json.loads(result.stdout)
        assert len(report["mappings"]) == (status == 0)
        assert not (checkout.root / "summary.md").exists()
        assert not (checkout.root / "unexpected.org").exists()
        assert [path.name for path in (checkout.root / ".xtask").iterdir()] == ["run"]
    assert (checkout.root / name).read_text() == text


def test_e2e_reconcile_runs_the_built_verifier_on_artifacts_only(
    checkout: Checkout,
) -> None:
    verifier = checkout.root.parent / "host-proof"
    (verifier / "bin").mkdir(parents=True)
    evidence = verifier / "bin" / "e2e-evidence"
    evidence.write_text(
        '#!/bin/sh\nprintf \'%s\\n\' "$@" > "$EVENTS"\n'
        'printf \'{"steps":[{"name":"reconcile","ok":true,"duration_ms":1}]}\'\n'
    )
    evidence.chmod(0o755)
    checkout.tool("nix", f'print("{verifier}")')
    for args, artifacts in [
        ([], ".xtask/diagnostics"),
        (["downloaded artifacts"], "downloaded artifacts"),
    ]:
        events = checkout.root / "events"
        result = checkout.dev("e2e-reconcile", "sqlite", *args, EVENTS=str(events))
        assert result.returncode == 0, result
        assert result.stdout == b"[ ok ] reconcile\n"
        assert events.read_text().splitlines() == [
            "reconcile",
            "sqlite",
            "--catalog",
            "end2end/e2e-lanes.json",
            "--diagnostics-root",
            artifacts,
        ]
    assert (
        checkout.order().splitlines()
        == ["nix build --no-link --print-out-paths .#host-proof"] * 2
    )


STATIC_ALL = [
    words(
        "cargo build --locked --quiet --manifest-path tools/Cargo.toml -p devtool "
        "-p repository-policy --bin devtool --bin scoped-leptosfmt --target-dir "
        "tools/target"
    ),
    words("cargo fmt --check"),
    words("scoped-leptosfmt verify"),
    words("emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check"),
    words("cargo fmt --manifest-path tools/Cargo.toml --all --check"),
    words("ast-grep test --config sgconfig.yml"),
    words(
        "ast-grep scan --config sgconfig.yml --filter ^no-full-reload$ web/src client/src"
    ),
    words("emacs --batch -Q -l elisp/scripts/byte-compile.el"),
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
        "--features doctests/host,coverage/host -- -D warnings"
    ),
    words("devtool check ert"),
]


def test_static_all_runs_every_static_check_with_tools_built_here(
    workflow: Workflow,
) -> None:
    built = workflow.root / "tools" / "target" / "debug"
    built.mkdir(parents=True)
    devtool = built / "devtool"
    devtool.write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" > built-devtool\n")
    devtool.chmod(0o755)
    result = workflow.dev("static-all")
    assert result.returncode == 0, result
    assert workflow.commands() == STATIC_ALL[:-1]
    assert (workflow.root / "built-devtool").read_text() == "check ert\n"
    result = workflow.dev("static-all", "fix", failure="cargo fmt")
    assert result.returncode == 17
    assert workflow.commands() == [STATIC_ALL[0], words("cargo fmt")]


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
    checkout.tool("nix", 'print("/vite-tools")')
    checkout.tool("cargo", "")
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
    order = checkout.order().splitlines()
    assert order[:2] == [
        "nix build .#csr-vite-node-modules --no-link --print-out-paths",
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
    assert order[2].startswith(
        f"cargo run --locked --manifest-path {checkout.root}/tools/Cargo.toml "
        "-p devtool -- system-artifacts --out "
    )
    assert order[3].startswith(f"bash {checkout.root}/scripts/build-csr-assets.sh ")


@pytest.mark.parametrize(
    "args",
    [
        ["perf", *"12345678"],
        ["perf-compare"],
        ["perf-import-baseline", "1", "2"],
        ["e2e", "1", "2"],
        ["e2e-producer"],
        ["e2e-reconcile"],
        ["issue-discover"],
        ["issue-create", "title", "Task", "3", "P2", "tooling"],
        ["issue-edit", "731", "title", "Task", "3", "P2", "tooling"],
        ["pr-land", "731"],
        ["pr-cleanup", "731", "f" * 40],
        ["production-baseline-discover"],
        ["production-baseline-accept", "source"],
        ["production-baseline-qualify-styling", "extra"],
        ["build-csr", "bogus"],
        ["static-all", "bogus"],
        ["adr-format", "extra"],
        ["org-timezone-manifest"],
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
    assert workflow.commands() == [words(f"{HOST_PROOF} pg-run -- --help")]
