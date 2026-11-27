"""Gates, CI lanes and host check groups: step order, admission and fail-fast.

Expected populations are literal lists of the commands each gate must run,
not derived from `dev`'s own tables.
"""

import platform
import re
from functools import partial
from pathlib import Path

import pytest
from conftest import REAL_GIT, REPOSITORY, Workflow, launch, require, write_bundle

from dev import static, steps
from dev.trace import TRACE_DIR

PREPUSH_SHARES = (platform.system(), platform.machine()) == ("Linux", "x86_64")


DOCS_SHAPE = [
    "sequence-check",
    "adr-format",
    "adr-readme-parity",
    "adr-view-parity",
    "doc-links",
    "flow-docs",
]
SOURCE_CONTRACTS = [
    "test-backend-pattern",
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


# Index custody around prepush's checks: the clean admission's queries before
# them, the post-check's queries after.
INDEX_QUERIES = commands(
    "git status --porcelain=v1 -z --untracked-files=all --no-renames",
    "git diff --cached --raw -z --no-abbrev --no-renames",
    "git ls-files --stage -z",
)


def nix_build(
    installable: str, link: str, flags: str = "--accept-flake-config"
) -> Commands:
    """A Nix build step's children: the evaluation, then the build."""
    return commands(
        f"nix path-info --derivation {flags} {installable}",
        f"nix build -L {flags} --print-out-paths {link} {installable}",
    )


def build(package: str) -> Commands:
    """A tool package's build step, run before the first use of any of its
    binaries; the fake output holds the fake as each binary, recording under
    its name."""
    return nix_build(f".#{package}", "--no-link")


# Built once per gate run, before the first of its binaries runs.
POLICY_BUILD = build("repository-policy")
HOST_TOOL_BUILD = build("host-tool")
COVERAGE_CHECK_BUILD = build("coverage-check")


def shared(check: str) -> Commands:
    """A shared verdict request, which leaves flake.lock as committed."""
    return nix_build(
        f".#checks.x86_64-linux.{check}",
        f"--out-link .xtask/gcroots/{check}",
        "--accept-flake-config --no-update-lock-file --no-write-lock-file",
    )


STATIC_DOCS = shared("static-docs")


def format_checks(mode: str) -> Commands:
    fix = mode == "fix"
    check = "" if fix else " --check"
    write = "-w" if fix else "--check"
    return [
        *commands(f"cargo fmt{check}"),
        *POLICY_BUILD,
        *commands(f"scoped-leptosfmt {mode}"),
        ["prettier", write, "**/*.md"],
        ["prettier", write, "end2end"],
        ["prettier", write, "**/*.css"],
        *commands(
            "emacs --batch -Q -l elisp/scripts/format.el -f "
            + ("jaunder-fmt-fix" if fix else "jaunder-fmt-check"),
            f"cargo fmt --manifest-path tools/Cargo.toml --all{check}",
        ),
    ]


def docs_shape(policy_built: bool = False) -> Commands:
    return [
        *([] if policy_built else POLICY_BUILD),
        *commands(*(f"repository-docs {name}" for name in DOCS_SHAPE)),
    ]


def source_contract(name: str) -> list[str]:
    return ["source-contracts", name]


def source_contracts(policy_built: bool = False) -> Commands:
    return [
        *commands(
            "ast-grep test --config sgconfig.yml",
            "ast-grep scan --config sgconfig.yml --filter ^no-full-reload$ "
            "web/src client/src",
        ),
        *([] if policy_built else POLICY_BUILD),
        *map(source_contract, SOURCE_CONTRACTS),
    ]


TSC = [
    ["setup-e2e-node-modules", "", ""],
    *commands("tsc --noEmit -p end2end/tsconfig.json"),
]


def tsc_and_cargo_deny() -> Commands:
    return [*TSC, *commands("cargo deny check")]


def compile_checks() -> Commands:
    return [*tsc_and_cargo_deny(), *commands(*CLIPPY)]


def elisp_checks() -> Commands:
    return commands(
        "emacs --batch -Q -l elisp/scripts/byte-compile.el",
        "emacs --batch -Q -l elisp/scripts/run-tests.el",
    )


def host(mode: str) -> Commands:
    return [
        # The format checks built repository-policy for the scoped adapter.
        *format_checks(mode),
        *docs_shape(policy_built=True),
        *source_contracts(policy_built=True),
        *compile_checks(),
        *elisp_checks(),
    ]


# Prepush's Verify host surface on x86_64-linux: each shared verdict at its
# first covered leaf's slot, the leaves it covers gone.
SHARED_HOST = [
    *shared("rust-format"),
    *STATIC_DOCS,
    ["prettier", "--check", "end2end"],
    ["prettier", "--check", "**/*.css"],
    *shared("static-elisp"),
    # No scoped leptosfmt ran, so docs-shape builds repository-policy.
    *docs_shape(),
    *source_contracts(policy_built=True),
    *shared("static-typescript"),
    *commands("cargo deny check", *CLIPPY),
    *shared("static-ert"),
]


TOOLS_TESTS = (
    "--manifest-path tools/Cargo.toml --lib --bins --tests --features coverage/host"
)


def tools_tests() -> Commands:
    return commands(
        # The tests compile with exactly their run's arguments.
        f"cargo test --no-run {TOOLS_TESTS}",
        f"cargo test {TOOLS_TESTS}",
    )


def host_tests() -> Commands:
    return [*tools_tests(), *python_checks()]


def python_checks() -> Commands:
    return commands(
        "ruff format --check tools/dev",
        "ruff check tools/dev",
        "mypy --strict tools/dev",
        "pytest tools/dev",
    )


def nix(check: str) -> Commands:
    return nix_build(
        f".#checks.x86_64-linux.{check}", f"--out-link .xtask/gcroots/{check}"
    )


PLATFORM = [
    "nix",
    "eval",
    "--impure",
    "--expr",
    'import ./nix/platform-contract.nix { flake = builtins.getFlake ("git+file://" + toString ./.); }',
]
STATIC = [
    argv
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
    for argv in nix(check)
]
CLEAN = commands("git status --porcelain --untracked-files=all")
CSR_BUNDLE = nix_build(".#csrBundle", "--out-link .xtask/gcroots/csr-bundle")
TEST_LOCAL = [
    *HOST_TOOL_BUILD,
    *commands("admit-inputs .#csrBundle.sourceAdmits"),
    *CSR_BUNDLE,
    *commands(
        "cargo build --locked -p jaunder --bin jaunder",
        "cargo nextest run --no-run --workspace",
        "pg-run cargo nextest run --workspace",
    ),
]

# The bundle is its own Nix step; the budget measures the output it linked
# in-process, running nothing.
WASM_BUDGET = CSR_BUNDLE
E2E = [["bash", "scripts/e2e-workflow", "all", ""]]


def producer_and_gate(check: str) -> Commands:
    flags = "--no-update-lock-file --no-write-lock-file"
    installable = f".#checks.x86_64-linux.{check}"
    return [
        *nix_build(installable, f"--out-link .xtask/gcroots/{check}", flags),
        *nix_build(f"{installable}-gate", "--no-link", flags),
    ]


COVERAGE = [
    *producer_and_gate("coverage"),
    *COVERAGE_CHECK_BUILD,
    *commands("coverage-check policy --out .xtask/gcroots/coverage"),
]
DOCTESTS = producer_and_gate("doctests")
# A lane that ran the coverage policy already built its tool.
ELISP_AFTER_COVERAGE = [
    *nix("elisp-coverage-producer"),
    *commands("coverage-check elisp-consume"),
]
ELISP = [
    *nix("elisp-coverage-producer"),
    *COVERAGE_CHECK_BUILD,
    *commands("coverage-check elisp-consume"),
]
CHECK_NO_TEST = [*host("fix"), *host_tests()]
CHECK = [*CHECK_NO_TEST, *TEST_LOCAL, *nix("wasm-tests"), *DOCTESTS]
PREPUSH_CHECKS = [*host_tests(), *TEST_LOCAL, *DOCTESTS]
PREPUSH = (
    [*CLEAN, *INDEX_QUERIES, *SHARED_HOST, *PREPUSH_CHECKS, *INDEX_QUERIES]
    if PREPUSH_SHARES
    else [*CLEAN, *host("verify"), *PREPUSH_CHECKS]
)
HOST_LANE = [*host("verify"), PLATFORM]


def ci_host_lane(advisories: bool) -> Commands:
    """CI's host lane: only the host checks the hermetic lane does not own
    (HERMETIC_OWNED), and cargo-deny only when asked to check advisories."""
    return [
        *CLEAN,
        *docs_shape(),
        *map(source_contract, SOURCE_CONTRACTS),
        *(commands("cargo deny check") if advisories else []),
        PLATFORM,
        *tools_tests(),
        *commands("pytest -m repository tools/dev"),
    ]


HERMETIC = [*STATIC, *WASM_BUDGET]
VALIDATE_NO_E2E = [
    *CLEAN,
    *HOST_LANE,
    *HERMETIC,
    *host_tests(),
    *nix("wasm-tests"),
    *COVERAGE,
    *DOCTESTS,
    *ELISP_AFTER_COVERAGE,
]
GATES: dict[str, Commands] = {
    "check-no-test": CHECK_NO_TEST,
    "check": CHECK,
    "prepush": PREPUSH,
    "validate-no-e2e": VALIDATE_NO_E2E,
    "validate": [*VALIDATE_NO_E2E, *E2E],
    "ci-validate-host": ci_host_lane(advisories=False),
    "ci-validate-docs": [
        *CLEAN,
        ["prettier", "--check", "**/*.md"],
        ["prettier", "--check", "end2end"],
        *docs_shape(),
    ],
    "ci-validate-hermetic": [*CLEAN, *HERMETIC],
    "ci-validate-test-checks": [*CLEAN, *nix("wasm-tests"), *DOCTESTS, *ELISP],
    "coverage": [*CLEAN, *COVERAGE],
    "doctests": [*CLEAN, *DOCTESTS],
}


@pytest.mark.parametrize(("gate", "expected"), GATES.items())
def test_each_gate_runs_its_complete_population_in_order(
    workflow: Workflow, gate: str, expected: Commands
) -> None:
    result = workflow.dev(gate, cwd=workflow.root / "sub")
    assert result.returncode == 0, result
    assert workflow.commands() == expected
    # Only a kept build holds a GC root; a tool package build keeps none.
    assert (workflow.root / ".xtask" / "gcroots").is_dir() == any(
        argv[:2] == ["nix", "build"] and "--out-link" in argv for argv in expected
    )
    assert not (workflow.root / ".xtask" / "last-result.json").exists()
    assert f"dev {gate}" in workflow.checkout.merged()


def test_the_ci_host_lane_runs_cargo_deny_only_to_check_advisories(
    workflow: Workflow,
) -> None:
    result = workflow.dev("ci-validate-host", "--advisories")
    assert result.returncode == 0, result
    assert workflow.commands() == ci_host_lane(advisories=True)


# Each host step whose CI verdict a hermetic-lane check owns: that check, and
# the command in its derivation's builder that runs the step.
HERMETIC_OWNED = {
    "fmt": ("rust-format", "dev static rust-format --sandbox"),
    "leptosfmt": ("rust-format", "dev static rust-format --sandbox"),
    "tools-fmt": ("rust-format", "dev static rust-format --sandbox"),
    "prettier-markdown": ("static-docs", "dev static prettier-markdown --sandbox"),
    "prettier-end2end": ("static-code", "dev static prettier-end2end --sandbox"),
    "prettier-css": ("static-code", "dev static prettier-css --sandbox"),
    "ast-grep-tests": ("static-code", "dev static ast-grep-tests --sandbox"),
    "no-full-reload": ("static-code", "dev static no-full-reload --sandbox"),
    "tsc": ("static-typescript", "dev static tsc --sandbox"),
    "elisp-fmt": ("static-elisp", "dev static elisp-fmt --sandbox"),
    "byte-compile": ("static-elisp", "dev static byte-compile --sandbox"),
    "ert": ("static-ert", "dev static ert --sandbox"),
    "ruff-format": ("dev-python", "ruff format --check ."),
    "ruff-check": ("dev-python", "ruff check ."),
    "mypy": ("dev-python", "mypy --strict ."),
    "pytest": ("dev-python", 'coverage run -m pytest -q -m "not repository"'),
}


def derivation(nix: str, check: str) -> str:
    """`check`'s definition in nix/checks.nix: from its top-level binding to
    the next one."""
    match = re.search(
        rf"^{re.escape(check)} =\n?.*?(?=^[\w-]+ =)", nix, re.MULTILINE | re.DOTALL
    )
    assert match, check
    return match[0]


@pytest.mark.repository
def test_the_host_lane_repeats_no_verdict_the_hermetic_lane_owns(
    workflow: Workflow,
) -> None:
    path = REPOSITORY / "nix" / "checks.nix"
    require(path.is_file(), "the repository's nix/checks.nix")
    nix = path.read_text(encoding="utf-8")
    hermetic = {argv[-1].rsplit(".", 1)[-1] for argv in STATIC}
    for step, (check, command) in HERMETIC_OWNED.items():
        assert check in hermetic, step
        assert command in derivation(nix, check), step
    # The Rust formatting group runs the three formatters.
    assert static.GROUPS["rust-format"][1] == ["fmt", "leptosfmt", "tools-fmt"]
    result = workflow.dev("ci-validate-host")
    assert result.returncode == 0, result
    assert not HERMETIC_OWNED.keys() & workflow.checkout.merged().keys()


GROUPS: list[tuple[list[str], Commands]] = [
    (["format"], format_checks("verify")),
    (["docs-shape"], docs_shape()),
    (["source-contracts"], source_contracts()),
    (["compile-checks"], compile_checks()),
    (["elisp-checks"], elisp_checks()),
    (
        ["source-contract", "html-sink"],
        [*POLICY_BUILD, source_contract("html-sink")],
    ),
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
    assert workflow.commands() == [*POLICY_BUILD, source_contract(name)]
    assert not (workflow.root / "pwned").exists()


@pytest.mark.parametrize(
    "args",
    [
        ["format", "verify", "bogus"],
        ["format", "verify", "prepush"],
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
    "ci-validate-docs",
    "ci-validate-hermetic",
    "ci-validate-test-checks",
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


# Fail-fast is the runner's alone: no gate runs a step itself
# (`steps.execute`), so this one test stands for every gate. The population
# tests above pin each gate's steps and their order.
@pytest.mark.parametrize(
    ("code", "signalled"), [(17, False), (143, True)], ids=["failed", "signalled"]
)
def test_the_runner_stops_at_the_first_failed_step_and_exits_with_its_status(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, code: int, signalled: bool
) -> None:
    monkeypatch.delenv(TRACE_DIR, raising=False)
    ran: list[str] = []

    def step(name: str, failure: steps.StepFailed | None = None) -> steps.Step:
        def run() -> None:
            ran.append(name)
            if failure is not None:
                raise failure

        return run

    declaration = (
        step("first"),
        step("failing", steps.StepFailed(code, signalled)),
        step("later"),
    )
    assert steps.gate(tmp_path, "dev demo", lambda: declaration) == code
    assert ran == ["first", "failing"]


def test_the_runner_runs_every_step_of_a_passing_declaration_in_order(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(TRACE_DIR, raising=False)
    ran: list[int] = []
    declaration = tuple(partial(ran.append, number) for number in range(3))
    assert steps.gate(tmp_path, "dev demo", lambda: declaration) == 0
    assert ran == [0, 1, 2]


def test_a_failing_gate_records_the_failed_step_and_status(
    workflow: Workflow,
) -> None:
    build_step = nix("rust-format")[-1]
    result = workflow.dev("ci-validate-hermetic", failure=" ".join(build_step))
    assert result.returncode == 17
    slices = workflow.checkout.merged()
    gate = slices["dev ci-validate-hermetic"]["args"]
    step = slices["rust-format"]["args"]
    assert (gate["jaunder.status"], gate["process.exit.code"]) == ("failed", 17)
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 17)
    assert step["parent_id"] == gate["span_id"]
    assert "static-code" not in slices


def test_the_hermetic_lane_fails_on_a_wasm_over_the_budget(workflow: Workflow) -> None:
    write_bundle(workflow.store / "fake-csr-bundle", b"\0" * 5_000_001)
    result = workflow.dev("ci-validate-hermetic")
    assert result.returncode == 1, result
    assert b"5000001 bytes, over the 5000000 byte ceiling by 1" in result.stderr
    assert workflow.commands() == [*CLEAN, *STATIC, *CSR_BUNDLE]
    step = workflow.checkout.merged()["wasm-budget"]["args"]
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 1)


def test_the_wasm_budget_never_runs_without_its_bundle(workflow: Workflow) -> None:
    failure = " ".join(CSR_BUNDLE[-1])
    result = workflow.dev("ci-validate-hermetic", failure=failure)
    assert result.returncode == 17
    assert workflow.commands() == [*CLEAN, *STATIC, *CSR_BUNDLE]
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
def test_cargo_steps_run_under_the_compile_cache(workflow: Workflow, gate: str) -> None:
    result = workflow.dev(gate, cwd=workflow.root / "sub")
    assert result.returncode == 0, result
    assert (workflow.root / "cache-env").read_text() == "kache\n0\n0\n"


def test_a_tool_builds_once_per_process_then_runs_directly(
    workflow: Workflow,
) -> None:
    result = workflow.dev("source-contracts")
    assert result.returncode == 0, result
    ran = workflow.commands()
    assert ran[2:] == [*POLICY_BUILD, *map(source_contract, SOURCE_CONTRACTS)]
    slices = workflow.checkout.merged()
    gate = slices["dev source-contracts"]["args"]["span_id"]
    step = slices["repository-policy-build"]["args"]
    assert step["parent_id"] == gate
    assert slices["eval"]["args"]["parent_id"] == step["span_id"]
    assert slices["build"]["args"]["parent_id"] == step["span_id"]
    # The contract's own step times the binary from the package output, never
    # the build.
    assert slices["html-sink"]["args"]["process.command_line"] == (
        f"{workflow.checkout.tool_out}/bin/source-contracts html-sink"
    )


def test_a_failed_tool_build_stops_the_gate_before_the_tool_runs(
    workflow: Workflow,
) -> None:
    failure = " ".join(POLICY_BUILD[-1])
    result = workflow.dev("source-contract", "html-sink", failure=failure)
    assert result.returncode == 17
    assert workflow.commands() == POLICY_BUILD
    slices = workflow.checkout.merged()
    step = slices["repository-policy-build"]["args"]
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 17)
    assert "html-sink" not in slices
    assert not (workflow.root / "order").exists()


# A gate that runs two binaries of one package around the host tests.
AROUND_HOST_TESTS = """\
from pathlib import Path
from dev import gates, steps
from dev.commands import SOURCE_CONTRACT, repository_policy
def body():
    print(steps.tool(SOURCE_CONTRACT))
    steps.execute(gates.host_tests())
    print(steps.tool(repository_policy("repository-docs")))
sys.exit(steps.gate(Path.cwd(), "dev demo", lambda: (body,)))
"""


def test_a_package_builds_once_per_gate_run_for_all_its_binaries(
    workflow: Workflow,
) -> None:
    log = workflow.root / "commands"
    log.write_bytes(b"")
    checkout = workflow.checkout
    environment = checkout.environment(FAKE_LOG=str(log), REAL_GIT=REAL_GIT)
    result = checkout.run(launch(AROUND_HOST_TESTS), environment)
    assert result.returncode == 0, result
    assert workflow.commands() == [*POLICY_BUILD, *host_tests()]
    out = checkout.tool_out / "bin"
    assert result.stdout.decode().splitlines() == [
        str([f"{out}/source-contracts"]),
        str([f"{out}/repository-docs"]),
    ]


def test_the_tools_tests_compile_with_exactly_their_run_arguments(
    workflow: Workflow,
) -> None:
    result = workflow.dev("check-no-test")
    assert result.returncode == 0, result
    ran = workflow.commands()
    build = ran.index(["cargo", "test", "--no-run", *TOOLS_TESTS.split(" ")])
    assert ran[build + 1] == ["cargo", "test", *ran[build][3:]]
    slices = workflow.checkout.merged()
    assert slices["tools-tests-build"]["ts"] < slices["tools-tests"]["ts"]


def test_leptosfmt_runs_the_adapter_this_checkout_built(workflow: Workflow) -> None:
    # The adapter on PATH is stale: running it fails. The package output
    # holds the recording fake, and leptosfmt's PATH finds it first.
    workflow.checkout.executable("scoped-leptosfmt", "#!/bin/sh\nexit 9\n")
    for mode, args in [("verify", []), ("fix", ["--fix"])]:
        result = workflow.dev("static", "leptosfmt", *args)
        assert result.returncode == 0, result
        assert workflow.commands() == format_checks(mode)[1:4]


def test_prepush_runs_the_host_leaves_without_custody_off_x86_64_linux(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from dev import gates, index, static

    ran: list[str] = []
    monkeypatch.setattr(index, "shares_verdicts", lambda: False)
    monkeypatch.setattr(index, "prepush", lambda _checks: pytest.fail("custody"))
    monkeypatch.setattr(gates, "clean_tree", lambda: ran.append("clean-tree"))
    monkeypatch.setattr(
        gates, "test_local", lambda: (partial(ran.append, "test-local"),)
    )
    monkeypatch.setattr(static, "run", lambda name, _mode: ran.append(name))
    monkeypatch.setattr(steps, "tool", lambda name: [name])
    monkeypatch.setattr(steps, "run", lambda name, *_, **__: ran.append(name))
    monkeypatch.setattr(steps, "nix_check", lambda name, *_, **__: ran.append(name))
    steps.execute(gates.prepush())
    assert ran[0] == "clean-tree"
    covered = [name for leaves in gates.SHARED_VERDICTS.values() for name in leaves]
    assert set(covered) <= set(ran)
    assert not set(gates.SHARED_VERDICTS) & set(ran)
    assert ran[-3:] == ["test-local", "doctests", "doctests-gate"]
