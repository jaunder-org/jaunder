"""The gates, CI lanes and host check groups, each a fixed sequence of steps.

Every function runs its steps in order on the host, from the checkout root,
and stops at the first failure (`steps.StepFailed`). Host tools that print one
JSON report are shown through `report.render`; their exit status stays the
verdict.
"""

import os
import platform
from enum import StrEnum
from pathlib import Path

from jaunder_dev import e2e, report, static, steps
from jaunder_dev.checkout import clean_tree
from jaunder_dev.commands import (
    COVERAGE_CHECK,
    DOCTEST_CHECK,
    HOST_PROOF,
    NIX_LOCK_FLAGS,
    REPOSITORY_POLICY,
    SOURCE_CONTRACT,
)
from jaunder_dev.static import Mode


class Docs(StrEnum):
    """Where the Markdown verdict comes from.

    `PREPUSH` takes the shared clean-index verdict on x86_64-linux, and only
    verifies; `HOST` runs host Prettier.
    """

    HOST = "host"
    PREPUSH = "prepush"


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


# The host check surface: its named groups in fail-fast order.


def host(mode: Mode, docs: Docs) -> None:
    format_checks(mode, docs)
    docs_shape()
    source_contracts()
    compile_checks(mode)
    elisp_checks(mode)


def format_checks(mode: Mode, docs: Docs) -> None:
    static.run("fmt", mode)
    leptosfmt(mode)
    markdown(mode, docs)
    static.run("prettier-css", mode)
    static.run("elisp-fmt", mode)
    static.run("tools-fmt", mode)


def _build_tools(name: str, *packages: str) -> steps.Environment:
    """Builds tool binaries from this checkout; returns a PATH that runs them first.

    `packages` alternates `-p <package>` and `--bin <binary>` selections.
    """
    steps.run(
        name,
        [
            "cargo",
            "build",
            "--locked",
            "--quiet",
            "--manifest-path",
            "tools/Cargo.toml",
            *packages,
            "--target-dir",
            "tools/target",
        ],
    )
    built = Path.cwd() / "tools" / "target" / "debug"
    return {"PATH": f"{built}:{os.environ.get('PATH', '')}"}


def leptosfmt(mode: Mode) -> None:
    """Runs the scoped adapter as built from this checkout, not a stale one on PATH."""
    path = _build_tools(
        "scoped-leptosfmt-build",
        "-p",
        "repository-policy",
        "--bin",
        "scoped-leptosfmt",
    )
    steps.run("leptosfmt", static.argv("leptosfmt", mode), environment=path)


def static_all(mode: Mode) -> None:
    """Every host static leaf in order, with the adapter built from this checkout."""
    path = _build_tools(
        "static-all-build",
        "-p",
        "repository-policy",
        "--bin",
        "scoped-leptosfmt",
    )
    static.run("all", mode, environment=path)


def markdown(mode: Mode, docs: Docs) -> None:
    """Markdown and end2end Prettier, the Markdown verdict taken as `docs` says."""
    if docs is Docs.PREPUSH and (platform.system(), platform.machine()) == (
        "Linux",
        "x86_64",
    ):
        steps.run(
            "prepush-docs",
            [*REPOSITORY_POLICY, "git-index-safety", "--", "prepush-docs"],
        )
    else:
        static.run("prettier-markdown", mode)
    static.run("prettier-end2end", mode)


def repository_docs(check: str) -> None:
    steps.run(check, [*REPOSITORY_POLICY, "repository-docs", "--", check])


def docs_shape() -> None:
    for check in DOCS_SHAPE:
        repository_docs(check)


def source_contract(name: str) -> None:
    steps.run(name, [*SOURCE_CONTRACT, name])


def source_contracts() -> None:
    static.run("ast-grep-tests", Mode.VERIFY)
    static.run("no-full-reload", Mode.VERIFY)
    for name in SOURCE_CONTRACTS:
        source_contract(name)
    steps.run("doctest-fences", [*DOCTEST_CHECK, "fences"], report.render)


def tsc(mode: Mode) -> None:
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
    static.run("tsc", mode)


def compile_checks(mode: Mode) -> None:
    tsc(mode)
    static.run("cargo-deny", mode)
    for name in static.CLIPPY:
        static.run(name, mode)


def elisp_checks(mode: Mode) -> None:
    static.run("byte-compile", mode)
    static.run("ert", mode)


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
    steps.nix_check(check, NIX_LOCK_FLAGS)
    steps.nix_check(f"{check}-gate", NIX_LOCK_FLAGS, keep=False)


def coverage_workflow() -> None:
    _producer_and_gate("coverage")
    steps.run(
        "coverage-policy",
        [*COVERAGE_CHECK, "policy", "--out", f"{steps.GCROOTS}/coverage"],
        report.render,
    )


def doctests_workflow() -> None:
    _producer_and_gate("doctests")


def elisp_coverage() -> None:
    steps.nix_check("elisp-coverage-producer")
    steps.run("elisp-consume", [*COVERAGE_CHECK, "elisp-consume"], report.render)


# Gates.


def check_no_test() -> None:
    host(Mode.FIX, Docs.HOST)
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
    host(Mode.VERIFY, Docs.PREPUSH)
    host_tests()
    test_local()
    workspace_doctests()


def validate_no_e2e() -> None:
    clean_tree()
    host(Mode.VERIFY, Docs.HOST)
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
    e2e.run_all("")


def ci_validate_host() -> None:
    clean_tree()
    host(Mode.VERIFY, Docs.HOST)
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
    markdown(Mode.FIX, Docs.HOST)
    docs_shape()
