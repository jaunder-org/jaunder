"""The gates, CI lanes and host check groups, each a fixed sequence of steps.

Every function runs its steps in order on the host, from the checkout root,
and stops at the first failure (`steps.StepFailed`). Host tools that print one
JSON report are shown through `report.render`; their exit status stays the
verdict.
"""

import fcntl
import os
from enum import StrEnum
from pathlib import Path

from dev import e2e, index, report, static, steps
from dev.checkout import clean_tree
from dev.commands import (
    COVERAGE_CHECK,
    SOURCE_CONTRACT,
    host_proof,
    repository_policy,
)
from dev.static import Mode


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
    "tools-doctests",
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
    static.run("leptosfmt", mode)
    markdown(mode, docs)
    static.run("prettier-css", mode)
    static.run("elisp-fmt", mode)
    static.run("tools-fmt", mode)


def markdown(mode: Mode, docs: Docs) -> None:
    """Markdown and end2end Prettier, the Markdown verdict taken as `docs` says."""
    if docs is Docs.PREPUSH and index.shares_docs():
        index.prepush_docs()
    else:
        static.run("prettier-markdown", mode)
    static.run("prettier-end2end", mode)


def repository_docs(check: str) -> None:
    steps.run(check, [*steps.tool(repository_policy("repository-docs")), check])


def docs_shape() -> None:
    for check in DOCS_SHAPE:
        repository_docs(check)


def source_contract(name: str) -> None:
    steps.run(name, [*steps.tool(SOURCE_CONTRACT), name])


def source_contracts() -> None:
    static.run("ast-grep-tests", Mode.VERIFY)
    static.run("no-full-reload", Mode.VERIFY)
    for name in SOURCE_CONTRACTS:
        source_contract(name)


def tsc_and_cargo_deny(mode: Mode) -> None:
    static.run("tsc", mode)
    static.run("cargo-deny", mode)


def compile_checks(mode: Mode) -> None:
    tsc_and_cargo_deny(mode)
    for name in static.CLIPPY:
        static.run(name, mode)


def elisp_checks(mode: Mode) -> None:
    static.run("byte-compile", mode)
    static.run("ert", mode)


# Shared steps of the gates.


TOOLS_TESTS = (
    "--manifest-path",
    "tools/Cargo.toml",
    "--lib",
    "--bins",
    "--tests",
    "--features",
    "coverage/host",
)


def host_tests() -> None:
    # Compiled first with the run's own arguments, so `tools-tests` times the
    # tests alone.
    steps.run("tools-tests-build", ["cargo", "test", "--no-run", *TOOLS_TESTS])
    # The build rebuilt the tools' binaries with the tests' features.
    steps.forget_tools()
    steps.run("tools-tests", ["cargo", "test", *TOOLS_TESTS])
    steps.run("ruff-format", ["ruff", "format", "--check", "tools/dev"])
    steps.run("ruff-check", ["ruff", "check", "tools/dev"])
    steps.run("mypy", ["mypy", "--strict", "tools/dev"])
    steps.run("pytest", ["pytest", "tools/dev"])


# Held for the whole of test-local: a concurrent test-local in this checkout
# would otherwise replace the CSR bundle root or the image worker while a run's
# tests still use them.
HOST_ARTIFACTS_LOCK = ".xtask/host-artifacts.lock"
# The step that builds `.#csrBundle`, and the name of its GC root.
CSR_BUNDLE = "csr-bundle"


def csr_bundle() -> str:
    """Builds `.#csrBundle` as its own step; returns the GC root that holds it."""
    steps.nix_build(CSR_BUNDLE, ".#csrBundle")
    return f"{steps.GCROOTS}/{CSR_BUNDLE}"


def test_local(nextest_args: tuple[str, ...] = ()) -> None:
    """Focused host-native tests against a verified CSR bundle and image worker.

    JAUNDER_CSR_BUNDLE_DIR names a bundle to use instead of building one.
    `nextest_args` select tests; none run the whole workspace.
    """
    Path(HOST_ARTIFACTS_LOCK).parent.mkdir(parents=True, exist_ok=True)
    # Python opens files non-inheritable, so no step's child (or a daemon it
    # leaves behind, such as the kache daemon) holds the lock past this run.
    with open(HOST_ARTIFACTS_LOCK, "a") as lock:
        with steps.phase("host-artifacts-lock"):
            fcntl.flock(lock, fcntl.LOCK_EX)
        bundle = os.environ.get("JAUNDER_CSR_BUNDLE_DIR")
        if bundle is None:
            admit = steps.tool(host_proof("admit-inputs"))
            steps.run("csr-admission", [*admit, ".#csrBundle.sourceAdmits"])
            bundle = csr_bundle()
        prerequisite = steps.tool(host_proof("csr-prerequisite"))
        root = steps.text("csr-prerequisite", [*prerequisite, bundle])
        # One environment for every Cargo step below: build scripts track
        # these (server's JAUNDER_CSR_BUNDLE_DIR, aws-lc-sys's CFLAGS), so any
        # difference would rebuild between them. aws-lc-sys keys its
        # FORTIFY_SOURCE handling on CFLAGS being present: set it empty when
        # absent and keep an inherited value unchanged.
        environment = {
            "CFLAGS": os.environ.get("CFLAGS", ""),
            "JAUNDER_CSR_BUNDLE_DIR": root,
            "JAUNDER_TEST_EXECUTABLE": str(Path.cwd() / "target/debug/jaunder"),
        }
        # Storage and seeding tests start the application worker, so it is
        # built even when nextest selects only storage.
        steps.run(
            "image-worker",
            ["cargo", "build", "--locked", "-p", "jaunder", "--bin", "jaunder"],
            environment=environment,
        )
        if not nextest_args:
            # The tests compile apart from their run when `dev` chooses the
            # selection. Hand-typed arguments may include run options nextest
            # refuses with --no-run (--no-fail-fast), so they compile in the run.
            steps.run(
                "nextest-build",
                ["cargo", "nextest", "run", "--no-run", "--workspace"],
                environment=environment,
            )
        steps.run(
            "nextest",
            ["pg-run", "cargo", "nextest", "run", *(nextest_args or ("--workspace",))],
            environment=environment,
        )


def platform_contract() -> None:
    steps.run(
        "platform",
        [
            "nix",
            "eval",
            "--impure",
            "--expr",
            'import ./nix/platform-contract.nix { flake = builtins.getFlake ("git+file://" + toString ./.); }',
        ],
    )


def static_checks() -> None:
    for check in STATIC_CHECKS:
        steps.nix_check(check)


def wasm_budget() -> None:
    """Builds the CSR bundle as its own step, then measures that output."""
    bundle = Path(csr_bundle()).resolve()
    steps.run(
        "wasm-budget",
        [*steps.tool(host_proof("wasm-audit")), "budget", "--site-path", str(bundle)],
        report.render,
    )


def wasm_tests() -> None:
    steps.nix_check("wasm-tests")


def _producer_and_gate(check: str) -> None:
    """Realizes a hermetic producer as a GC root, then its hard status gate."""
    steps.nix_check(check, steps.NIX_LOCK_FLAGS)
    steps.nix_check(f"{check}-gate", steps.NIX_LOCK_FLAGS, keep=False)


def coverage_workflow() -> None:
    _producer_and_gate("coverage")
    steps.run(
        "coverage-policy",
        [*steps.tool(COVERAGE_CHECK), "policy", "--out", f"{steps.GCROOTS}/coverage"],
        report.render,
    )


def doctests_workflow() -> None:
    _producer_and_gate("doctests")


def elisp_coverage() -> None:
    steps.nix_check("elisp-coverage-producer")
    consume = [*steps.tool(COVERAGE_CHECK), "elisp-consume"]
    steps.run("elisp-consume", consume, report.render)


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
    doctests_workflow()


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
    """The host check groups without Clippy, whose CI verdict is the hermetic
    lane's `static-code` (docs/adr/0255-ci-clippy-verdict-from-nix.md)."""
    clean_tree()
    format_checks(Mode.VERIFY, Docs.HOST)
    docs_shape()
    source_contracts()
    tsc_and_cargo_deny(Mode.VERIFY)
    elisp_checks(Mode.VERIFY)
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


def _precommit_checks(markdown_only: bool) -> None:
    """A Markdown-only change takes the Markdown-sensitive Fix checks, in order;
    any other change takes check-no-test."""
    if markdown_only:
        markdown(Mode.FIX, Docs.HOST)
        docs_shape()
    else:
        check_no_test()


def precommit() -> None:
    """Index custody selects the host checks and reconciles the index."""
    index.precommit(_precommit_checks)
