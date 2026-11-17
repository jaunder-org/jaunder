"""Gates, CI lanes and host check groups: step order, admission and fail-fast.

Expected populations are literal lists of the commands each gate must run,
not derived from `dev`'s own tables.
"""

import os
import platform

import pytest
from conftest import Workflow

PREPUSH_SHARES_DOCS = (platform.system(), platform.machine()) == ("Linux", "x86_64")
CARGO_RUN = "cargo run --locked --quiet --manifest-path tools/Cargo.toml"
POLICY = f"{CARGO_RUN} -p repository-policy --bin"

DOCS_SHAPE = [
    "sequence-check",
    "adr-format",
    "adr-readme-parity",
    "adr-view-parity",
    "doc-links",
    "flow-docs",
]
SOURCE_CONTRACTS = [
    "test-patterns",
    "server-fn-registrar",
    "server-fn-tracing",
    "server-fn-coverage",
    "server-fn-wire-arg-error",
    "traced-context",
    "e2e-telemetry-boundary",
    "e2e-collector-config",
    "proffered-secret",
    "e2e-goto-wrapper",
    "e2e-server-fn-endpoints",
    "target-arch-placement",
    "write-transaction-contract",
    "common-host-target-closure",
    "lint-suppression",
    "thin-components",
    "sqlx-newtype-bind",
    "sqlx-newtype-decode",
    "raw-html-door",
    "html-sink",
    "e2e-scaffold",
    "xlang-literal",
]
CLIPPY = [
    (
        "cargo clippy --workspace --exclude web --exclude client --exclude csr "
        "--all-targets -- -D warnings"
    ),
    "cargo clippy -p web --features server --all-targets -- -D warnings",
    "cargo clippy -p web --no-default-features --all-targets -- -D warnings",
    "cargo clippy -p client --features csr --all-targets -- -D warnings",
    (
        "cargo clippy -p web -p client -p csr --features csr "
        "--target wasm32-unknown-unknown -- -D warnings"
    ),
    (
        "cargo clippy --manifest-path tools/Cargo.toml --workspace --all-targets "
        "--features coverage/host -- -D warnings"
    ),
]

Commands = list[list[str]]


def commands(*lines: str) -> Commands:
    return [line.split(" ") for line in lines]


def format_checks(mode: str, shared_docs: bool = False) -> Commands:
    fix = mode == "fix"
    check = "" if fix else " --check"
    write = "-w" if fix else "--check"
    expected = commands(
        f"cargo fmt{check}",
        "cargo build --locked --quiet --manifest-path tools/Cargo.toml "
        "-p repository-policy --bin scoped-leptosfmt --target-dir tools/target",
        f"scoped-leptosfmt {mode}",
    )
    if shared_docs:
        expected += commands(f"{POLICY} git-index-safety -- prepush-docs")
    else:
        expected += [["prettier", write, "**/*.md"]]
    return expected + [
        ["prettier", write, "end2end"],
        ["prettier", write, "**/*.css"],
        *commands(
            "emacs --batch -Q -l elisp/scripts/format.el -f "
            + ("jaunder-fmt-fix" if fix else "jaunder-fmt-check"),
            f"cargo fmt --manifest-path tools/Cargo.toml --all{check}",
        ),
    ]


def docs_shape() -> Commands:
    return commands(*(f"{POLICY} repository-docs -- {name}" for name in DOCS_SHAPE))


def source_contract(name: str) -> list[str]:
    return [*f"{POLICY} source-contracts --".split(" "), name]


def source_contracts() -> Commands:
    return [
        *commands(
            "ast-grep test --config sgconfig.yml",
            "ast-grep scan --config sgconfig.yml --filter ^no-full-reload$ "
            "web/src client/src",
        ),
        *map(source_contract, SOURCE_CONTRACTS),
    ]


def compile_checks() -> Commands:
    return [
        ["setup-e2e-node-modules", "", ""],
        *commands("tsc --noEmit -p end2end/tsconfig.json", "cargo deny check", *CLIPPY),
    ]


def elisp_checks() -> Commands:
    return commands(
        "emacs --batch -Q -l elisp/scripts/byte-compile.el",
        "emacs --batch -Q -l elisp/scripts/run-tests.el",
    )


def host(mode: str, shared_docs: bool = False) -> Commands:
    return [
        *format_checks(mode, shared_docs),
        *docs_shape(),
        *source_contracts(),
        *compile_checks(),
        *elisp_checks(),
    ]


def host_tests() -> Commands:
    return commands(
        "cargo test --manifest-path tools/Cargo.toml --lib --bins --tests "
        "--features coverage/host",
        "ruff format --check tools/dev",
        "ruff check tools/dev",
        "mypy --strict tools/dev",
        "pytest tools/dev",
    )


def nix(check: str) -> list[str]:
    return commands(
        f"nix build -L --accept-flake-config --out-link .xtask/gcroots/{check} "
        f".#checks.x86_64-linux.{check}"
    )[0]


PLATFORM = [
    "nix",
    "eval",
    "--impure",
    "--expr",
    "import ./nix/platform-contract.nix { flake = builtins.getFlake (toString ./.); }",
]
STATIC = [
    nix(check)
    for check in [
        "static-docs",
        "static-typescript",
        "rust-format",
        "static-code",
        "static-elisp",
        "static-e2e-census",
        "static-ert",
        "dev-python",
        "tools-doctests",
    ]
]
CLEAN = commands("git status --porcelain --untracked-files=all")
TEST_LOCAL = commands(
    "git config core.hooksPath .githooks",
    f"{CARGO_RUN} -p host-proof --bin admit-inputs -- .#csrBundle.sourceAdmits",
    "nix build -L --accept-flake-config --out-link .xtask/gcroots/csr-bundle "
    ".#csrBundle",
    f"{CARGO_RUN} -p host-proof --bin csr-prerequisite -- .xtask/gcroots/csr-bundle",
    "cargo build --locked -p jaunder --bin jaunder",
    "pg-run cargo nextest run --workspace",
)

# The bundle is its own Nix step; the budget measures the output it linked.
WASM_BUDGET = commands(
    "nix build -L --accept-flake-config --out-link .xtask/gcroots/csrBundle "
    ".#csrBundle",
    f"{CARGO_RUN} -p host-proof --bin wasm-audit -- budget "
    "--site-path /nix/store/fake-csrBundle",
)
E2E = [["bash", "scripts/e2e-workflow", "all", ""]]


def producer_and_gate(check: str) -> Commands:
    flags = "nix build -L --no-update-lock-file --no-write-lock-file"
    return commands(
        f"{flags} --out-link .xtask/gcroots/{check} .#checks.x86_64-linux.{check}",
        f"{flags} --no-link .#checks.x86_64-linux.{check}-gate",
    )


COVERAGE = [
    *producer_and_gate("coverage"),
    *commands(
        f"{CARGO_RUN} -p coverage --features host --bin coverage-check -- "
        "policy --out .xtask/gcroots/coverage"
    ),
]
DOCTESTS = producer_and_gate("doctests")
ELISP = [
    nix("elisp-coverage-producer"),
    *commands(
        f"{CARGO_RUN} -p coverage --features host --bin coverage-check -- elisp-consume"
    ),
]
CHECK_NO_TEST = [*host("fix"), *host_tests()]
CHECK = [*CHECK_NO_TEST, *TEST_LOCAL, nix("wasm-tests"), *DOCTESTS]
PREPUSH = [
    *CLEAN,
    *host("verify", PREPUSH_SHARES_DOCS),
    *host_tests(),
    *TEST_LOCAL,
    *DOCTESTS,
]
HOST_LANE = [*host("verify"), PLATFORM]
HERMETIC = [*STATIC, *WASM_BUDGET]
VALIDATE_NO_E2E = [
    *CLEAN,
    *HOST_LANE,
    *HERMETIC,
    *host_tests(),
    nix("wasm-tests"),
    *COVERAGE,
    *DOCTESTS,
    *ELISP,
]
PRECOMMIT = commands(f"{POLICY} git-index-safety -- precommit")
GATES: dict[str, Commands] = {
    "check-no-test": CHECK_NO_TEST,
    "precommit-broad-host": CHECK_NO_TEST,
    "check": CHECK,
    "prepush": PREPUSH,
    "validate-no-e2e": VALIDATE_NO_E2E,
    "validate": [*VALIDATE_NO_E2E, *E2E],
    "ci-validate-host": [*CLEAN, *HOST_LANE, *host_tests()],
    "ci-validate-hermetic": [*CLEAN, *HERMETIC],
    "ci-validate-test-checks": [*CLEAN, nix("wasm-tests"), *DOCTESTS, *ELISP],
    "ci-validate-coverage": [*CLEAN, *COVERAGE],
    "coverage": [*CLEAN, *COVERAGE],
    "doctests": [*CLEAN, *DOCTESTS],
    "precommit": PRECOMMIT,
    "precommit-markdown-host": [
        ["prettier", "-w", "**/*.md"],
        ["prettier", "-w", "end2end"],
        *docs_shape(),
    ],
}


@pytest.mark.parametrize(("gate", "expected"), GATES.items())
def test_each_gate_runs_its_complete_population_in_order(
    workflow: Workflow, gate: str, expected: Commands
) -> None:
    result = workflow.dev(gate, cwd=workflow.root / "sub")
    assert result.returncode == 0, result
    assert workflow.commands() == expected
    assert (workflow.root / ".xtask" / "gcroots").is_dir() == any(
        argv[0] == "nix" and argv[1] == "build" for argv in expected
    )
    assert not (workflow.root / ".xtask" / "last-result.json").exists()
    assert f"dev {gate}" in workflow.checkout.merged()


GROUPS: list[tuple[list[str], Commands]] = [
    (["format"], format_checks("verify")),
    (["format", "verify", "prepush"], format_checks("verify", PREPUSH_SHARES_DOCS)),
    (["docs-shape"], docs_shape()),
    (["source-contracts"], source_contracts()),
    (["compile-checks"], compile_checks()),
    (["elisp-checks"], elisp_checks()),
    (["source-contract", "html-sink"], [source_contract("html-sink")]),
    *(
        (args, expected)
        for mode in ["fix", "verify"]
        for args, expected in [
            (["format", mode], format_checks(mode)),
            (["compile-checks", mode], compile_checks()),
            (["elisp-checks", mode], elisp_checks()),
        ]
    ),
]


@pytest.mark.parametrize(("args", "expected"), GROUPS)
def test_each_check_group_and_one_source_contract_run_alone(
    workflow: Workflow, args: list[str], expected: Commands
) -> None:
    result = workflow.dev(*args)
    assert result.returncode == 0, result
    assert workflow.commands() == expected


def test_a_source_contract_name_stays_one_argument(workflow: Workflow) -> None:
    name = "html-sink ; touch pwned $(touch pwned)"
    result = workflow.dev("source-contract", name)
    assert result.returncode == 0, result
    assert workflow.commands() == [source_contract(name)]
    assert not (workflow.root / "pwned").exists()


@pytest.mark.parametrize(
    "args",
    [
        ["format", "verify", "bogus"],
        ["format", "fix", "prepush"],
        ["format", "bogus"],
        ["compile-checks", "--fix"],
        ["source-contract"],
        ["check", "--no-test"],
        ["validate", "--no-e2e"],
        ["ci-validate", "host"],
        ["prepush", "extra"],
    ],
)
def test_unknown_or_contradictory_arguments_are_refused_before_any_work(
    workflow: Workflow, args: list[str]
) -> None:
    result = workflow.dev(*args)
    assert result.returncode == 2, result
    assert workflow.commands() == []
    assert workflow.checkout.runs() == []


ADMITTED = [
    "prepush",
    "validate-no-e2e",
    "validate",
    "ci-validate-host",
    "ci-validate-hermetic",
    "ci-validate-test-checks",
    "ci-validate-coverage",
    "coverage",
    "doctests",
]


@pytest.mark.parametrize("gate", ADMITTED)
def test_clean_admission_refuses_a_dirty_tree_before_work(
    workflow: Workflow, gate: str
) -> None:
    (workflow.root / "dirty-status").write_text("dirty fixture")
    result = workflow.dev(gate)
    assert result.returncode == 1
    assert workflow.commands() == CLEAN
    assert result.stderr == (
        b"verify workflows require a clean checkout\n M tracked.rs\n"
    )
    # The refusal belongs to the step whose output it judged.
    step = workflow.checkout.merged()["clean-tree"]["args"]
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 1)


@pytest.mark.parametrize("gate", ADMITTED)
def test_clean_admission_refuses_when_git_fails_before_work(
    workflow: Workflow, gate: str
) -> None:
    result = workflow.dev(gate, failure=" ".join(CLEAN[0]))
    assert result.returncode == 17
    assert workflow.commands() == CLEAN


def test_dirty_fix_check_reaches_its_doctests_without_admission(
    workflow: Workflow,
) -> None:
    (workflow.root / "dirty-status").write_text("dirty fixture")
    result = workflow.dev("check")
    assert result.returncode == 0, result
    assert workflow.commands() == CHECK
    assert CLEAN[0] not in workflow.commands()


# Every gate and lane whose failures the hooks and CI depend on.
FAIL_FAST = [
    "precommit-markdown-host",
    "check",
    "prepush",
    "validate",
    "ci-validate-host",
    "ci-validate-hermetic",
    "ci-validate-test-checks",
    "ci-validate-coverage",
    "precommit",
]


def test_every_step_failure_is_its_status_and_stops_subsequent_work(
    workflow: Workflow,
) -> None:
    # Shared steps need one fault injection, plus each gate's first and final
    # steps; repeating the whole host surface per gate adds nothing.
    tested: set[tuple[str, ...]] = set()
    for gate in FAIL_FAST:
        success = GATES[gate]
        for index, argv in enumerate(success):
            first_time = tuple(argv) not in tested
            tested.add(tuple(argv))
            if not first_time and index not in (0, len(success) - 1):
                continue
            result = workflow.dev(gate, failure=" ".join(argv))
            assert result.returncode == 17, (gate, argv, result)
            assert workflow.commands() == success[: index + 1], (gate, argv)
            assert result.stderr.endswith(f"{argv[0]} failed\n".encode())


def test_a_failing_gate_records_the_failed_step_and_status(
    workflow: Workflow,
) -> None:
    result = workflow.dev("ci-validate-hermetic", failure=" ".join(nix("rust-format")))
    assert result.returncode == 17
    slices = workflow.checkout.merged()
    gate = slices["dev ci-validate-hermetic"]["args"]
    step = slices["rust-format"]["args"]
    assert (gate["jaunder.status"], gate["process.exit.code"]) == ("failed", 17)
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 17)
    assert step["parent_id"] == gate["span_id"]
    assert "static-code" not in slices


def test_a_report_step_without_a_report_fails_closed(workflow: Workflow) -> None:
    # Every Cargo tool prints the same non-report; in the hermetic lane only
    # the wasm budget is a Cargo step, so it alone fails, with jq's status 5.
    workflow.checkout.tool("cargo", 'print("not a report")')
    result = workflow.dev("ci-validate-hermetic")
    assert result.returncode == 5, result
    assert result.stdout == b"not a report\n"
    assert result.stderr == b"dev: wasm-budget: output is not a step report\n"
    order = (workflow.root / "order").read_text().splitlines()
    assert order == [" ".join(WASM_BUDGET[-1])]


def test_the_wasm_budget_never_runs_without_its_bundle(workflow: Workflow) -> None:
    result = workflow.dev("ci-validate-hermetic", failure=" ".join(WASM_BUDGET[0]))
    assert result.returncode == 17
    assert workflow.commands() == [*CLEAN, *STATIC, WASM_BUDGET[0]]
    assert "wasm-budget" not in workflow.checkout.merged()


@pytest.mark.parametrize("failure", ["", " ".join(TEST_LOCAL[-1])])
def test_producer_cleanup_finishes_before_dev_returns(
    workflow: Workflow, failure: str
) -> None:
    result = workflow.dev("check", failure=failure)
    assert result.returncode == (17 if failure else 0)
    assert not (workflow.root / "producer-resource").exists()
    assert (workflow.root / "producer-cleanup").read_text() == "cleaned\n"


@pytest.mark.parametrize("gate", ["check-no-test", "prepush", "ci-validate-host"])
def test_cargo_steps_share_the_compile_cache_across_worktree_roots(
    workflow: Workflow, gate: str
) -> None:
    os.symlink(workflow.root, workflow.root / "cache-link")
    result = workflow.dev(gate, cwd=workflow.root / "sub")
    assert result.returncode == 0, result
    assert (workflow.root / "cache-env").read_text() == (
        f"sccache\n0\n{workflow.root.resolve()}\n"
    )


def test_leptosfmt_runs_the_adapter_this_checkout_built(workflow: Workflow) -> None:
    built = workflow.root / "tools" / "target" / "debug"
    built.mkdir(parents=True)
    adapter = built / "scoped-leptosfmt"
    adapter.write_text('#!/bin/sh\nprintf "%s\\n" "$*" > built-adapter\n')
    adapter.chmod(0o755)
    for mode in ["verify", "fix"]:
        result = workflow.dev("leptosfmt", mode)
        assert result.returncode == 0, result
        assert workflow.commands() == format_checks(mode)[1:2]
        assert (workflow.root / "built-adapter").read_text() == f"{mode}\n"


@pytest.mark.parametrize("code", [0, 1, 23])
def test_precommit_delegates_to_the_safety_owner_and_keeps_its_status(
    workflow: Workflow, code: int
) -> None:
    workflow.checkout.tool(
        "cargo",
        'print("safety stdout"); print("safety stderr", file=sys.stderr)\n'
        "print(os.environ.get('GIT_INDEX_FILE'), file=sys.stderr)\n"
        f"sys.exit({code})",
    )
    result = workflow.dev("precommit")
    assert result.returncode == code
    assert result.stdout == b"safety stdout\n"
    # The hook's index stays visible to the owner that reconciles it.
    assert result.stderr == b"safety stderr\nforeign-index\n"
    assert (workflow.root / "order").read_text() == " ".join(PRECOMMIT[0]) + "\n"
