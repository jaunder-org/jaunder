"""The gates, CI lanes and host check groups, each a fixed sequence of steps.

Every function runs its steps in order on the host, from the checkout root,
and stops at the first failure (`steps.StepFailed`). Host tools that print one
JSON report are shown through `report.render`; their exit status stays the
verdict.
"""

import os
import platform
import sys
from pathlib import Path

from jaunder_dev import report, static, steps

CARGO_RUN = (
    "cargo",
    "run",
    "--locked",
    "--quiet",
    "--manifest-path",
    "tools/Cargo.toml",
)
DEVTOOL = (*CARGO_RUN, "-p", "devtool", "--bin", "devtool", "--")
REPOSITORY_POLICY = (*CARGO_RUN, "-p", "repository-policy", "--bin")
SOURCE_CONTRACT = (*REPOSITORY_POLICY, "source-contracts", "--")
HOST_PROOF = (*CARGO_RUN, "-p", "host-proof", "--bin")
DOCTEST_CHECK = (
    *CARGO_RUN,
    "-p",
    "doctests",
    "--features",
    "host",
    "--bin",
    "doctest-check",
    "--",
)
COVERAGE_CHECK = (
    *CARGO_RUN,
    "-p",
    "coverage",
    "--features",
    "host",
    "--bin",
    "coverage-check",
    "--",
)

# Inherited Git overrides (as set inside hooks) would redirect discovery.
GIT_OVERRIDES = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
)

DOCS_SHAPE = (
    "sequence-check",
    "adr-format",
    "adr-readme-parity",
    "adr-view-parity",
    "doc-links",
    "flow-docs",
)
SOURCE_CONTRACTS = (
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
)
CLIPPY = (
    "product-core-clippy",
    "web-server-clippy",
    "web-no-server-clippy",
    "client-host-clippy",
    "wasm-clippy",
    "tools-clippy",
)
STATIC_CHECKS = (
    "static-docs",
    "static-typescript",
    "rust-format",
    "static-code",
    "static-elisp",
    "static-e2e-census",
    "static-ert",
    "dev-python",
)


def clean_tree() -> None:
    """Verify workflows refuse dirty or unavailable Git state before work."""
    status = steps.output(
        "clean-tree",
        ["git", "status", "--porcelain", "--untracked-files=all"],
        dict.fromkeys(GIT_OVERRIDES),
    ).decode(errors="replace")
    if status:
        print(
            f"verify workflows require a clean checkout\n{status.rstrip('\n')}",
            file=sys.stderr,
            flush=True,
        )
        raise steps.StepFailed(1, signalled=False)


# The host check surface: its named groups in fail-fast order.


def host(fix: bool, docs: str) -> None:
    format_checks(fix, docs)
    docs_shape()
    source_contracts()
    compile_checks(fix)
    elisp_checks(fix)


def format_checks(fix: bool, docs: str) -> None:
    """Formatters; docs `prepush` takes the shared Markdown verdict (Verify only)."""
    static.run("fmt", fix)
    leptosfmt(fix)
    markdown(fix, docs)
    static.run("prettier-css", fix)
    static.run("elisp-fmt", fix)
    static.run("tools-fmt", fix)


def leptosfmt(fix: bool) -> None:
    """Runs the scoped adapter as built from this checkout, not a stale one on PATH."""
    steps.run(
        "scoped-leptosfmt-build",
        [
            "cargo",
            "build",
            "--locked",
            "--quiet",
            "--manifest-path",
            "tools/Cargo.toml",
            "-p",
            "repository-policy",
            "--bin",
            "scoped-leptosfmt",
            "--target-dir",
            "tools/target",
        ],
    )
    built = Path.cwd() / "tools" / "target" / "debug"
    steps.run(
        "leptosfmt",
        static.argv("leptosfmt", fix),
        environment={"PATH": f"{built}:{os.environ.get('PATH', '')}"},
    )


def markdown(fix: bool, docs: str) -> None:
    """Markdown and end2end Prettier.

    On x86_64-linux, prepush replaces host Prettier Markdown with the
    clean-index static-docs verdict.
    """
    if docs == "prepush" and (platform.system(), platform.machine()) == (
        "Linux",
        "x86_64",
    ):
        steps.run(
            "prepush-docs",
            [*REPOSITORY_POLICY, "git-index-safety", "--", "prepush-docs"],
        )
    else:
        static.run("prettier-markdown", fix)
    static.run("prettier-end2end", fix)


def docs_shape() -> None:
    for check in DOCS_SHAPE:
        steps.run(check, [*REPOSITORY_POLICY, "repository-docs", "--", check])


def source_contract(name: str) -> None:
    steps.run(name, [*SOURCE_CONTRACT, name])


def source_contracts() -> None:
    static.run("ast-grep-tests", False)
    static.run("no-full-reload", False)
    for name in SOURCE_CONTRACTS:
        source_contract(name)
    steps.run("doctest-fences", [*DOCTEST_CHECK, "fences"], report.render)


def tsc(fix: bool) -> None:
    """Type-checks the end-to-end suite after linking its pinned node_modules.

    Provisioning follows the current checkout, so it also works after `cd`
    into another worktree from an existing devShell.
    """
    steps.run(
        "setup-e2e-node-modules",
        [
            "setup-e2e-node-modules",
            os.environ.get("E2E_TYPES_NODE_MODULES", ""),
            os.environ.get("E2E_PLAYWRIGHT_TEST", ""),
        ],
    )
    static.run("tsc", fix)


def compile_checks(fix: bool) -> None:
    tsc(fix)
    static.run("cargo-deny", fix)
    for name in CLIPPY:
        static.run(name, fix)


def elisp_checks(fix: bool) -> None:
    static.run("byte-compile", fix)
    # devtool times the ERT request; `cargo run` builds it from current source.
    steps.run("ert", [*DEVTOOL, "check", "ert", *(["--fix"] if fix else [])])


# Shared steps of the gates.


def host_tests() -> None:
    steps.run(
        "tools-tests",
        [
            "cargo",
            "test",
            "--manifest-path",
            "tools/Cargo.toml",
            "--lib",
            "--bins",
            "--tests",
            "--features",
            "doctests/host,coverage/host",
        ],
    )
    steps.run("ruff-format", ["ruff", "format", "--check", "tools/dev"])
    steps.run("ruff-check", ["ruff", "check", "tools/dev"])
    steps.run("mypy", ["mypy", "--strict", "tools/dev"])
    steps.run("pytest", ["pytest", "tools/dev"])


def test_local(nextest_args: tuple[str, ...] = ()) -> None:
    steps.run(
        "test-local",
        [*HOST_PROOF, "test-local", "--", *nextest_args],
        report.render,
    )


def workspace_doctests() -> None:
    steps.run("workspace-doctests", [*DOCTEST_CHECK, "workspace"], report.render)


def platform_contract() -> None:
    steps.run(
        "platform",
        [
            "nix",
            "eval",
            "--impure",
            "--expr",
            "import ./nix/platform-contract.nix { flake = builtins.getFlake (toString ./.); }",
        ],
    )


def static_checks() -> None:
    for check in STATIC_CHECKS:
        steps.nix_check(check)


def wasm_budget() -> None:
    steps.run("wasm-budget", [*HOST_PROOF, "wasm-audit", "--", "budget"], report.render)


def wasm_tests() -> None:
    steps.nix_check("wasm-tests")


def _producer_and_gate(check: str) -> None:
    """Realizes a hermetic producer as a GC root, then its hard status gate."""
    flags = ["nix", "build", "--no-update-lock-file", "--no-write-lock-file", "-L"]
    Path(".xtask/gcroots").mkdir(parents=True, exist_ok=True)
    steps.run(
        check,
        [
            *flags,
            f".#checks.x86_64-linux.{check}",
            "--out-link",
            f".xtask/gcroots/{check}",
        ],
    )
    steps.run(
        f"{check}-gate", [*flags, f".#checks.x86_64-linux.{check}-gate", "--no-link"]
    )


def coverage_workflow() -> None:
    _producer_and_gate("coverage")
    steps.run(
        "coverage-policy",
        [*COVERAGE_CHECK, "policy", "--out", ".xtask/gcroots/coverage"],
        report.render,
    )


def doctests_workflow() -> None:
    _producer_and_gate("doctests")


def elisp_coverage() -> None:
    steps.nix_check("elisp-coverage-producer")
    steps.run("elisp-consume", [*COVERAGE_CHECK, "elisp-consume"], report.render)


def e2e() -> None:
    """The full E2E workflow: every producer, then reconciliation of both backends."""
    steps.run("e2e", ["bash", "scripts/e2e-workflow", "all", ""])


# Gates.


def check_no_test() -> None:
    host(True, "host")
    host_tests()


def check() -> None:
    # Fix-mode check keeps a dirty tree: it runs the doctest producer without
    # the public `doctests` admission.
    check_no_test()
    test_local()
    wasm_tests()
    doctests_workflow()


def prepush() -> None:
    clean_tree()
    host(False, "prepush")
    host_tests()
    test_local()
    workspace_doctests()


def validate_no_e2e() -> None:
    clean_tree()
    host(False, "host")
    platform_contract()
    static_checks()
    wasm_budget()
    host_tests()
    wasm_tests()
    coverage_workflow()
    doctests_workflow()
    elisp_coverage()


def validate() -> None:
    validate_no_e2e()
    e2e()


def ci_validate_host() -> None:
    clean_tree()
    host(False, "host")
    platform_contract()
    host_tests()


def ci_validate_hermetic() -> None:
    clean_tree()
    static_checks()
    wasm_budget()


def ci_validate_test_checks() -> None:
    clean_tree()
    wasm_tests()
    doctests_workflow()
    elisp_coverage()


def coverage() -> None:
    clean_tree()
    coverage_workflow()


def doctests() -> None:
    clean_tree()
    doctests_workflow()


def precommit() -> None:
    """The Git/index safety owner selects the host checks and reconciles the index."""
    steps.run(
        "git-index-safety", [*REPOSITORY_POLICY, "git-index-safety", "--", "precommit"]
    )


def precommit_markdown_host() -> None:
    markdown(True, "host")
    docs_shape()
