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

from jaunder_dev import e2e, index, report, static, steps, trace
from jaunder_dev.checkout import clean_tree, traced_git
from jaunder_dev.commands import (
    COVERAGE_CHECK,
    HOST_PROOF,
    NIX_LOCK_FLAGS,
    REPOSITORY_POLICY,
    SOURCE_CONTRACT,
)
from jaunder_dev.spans import STATUS, Status
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
    if docs is Docs.PREPUSH and index.shares_docs():
        index.prepush_docs()
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
            "coverage/host",
        ],
    )
    steps.run("ruff-format", ["ruff", "format", "--check", "tools/dev"])
    steps.run("ruff-check", ["ruff", "check", "tools/dev"])
    steps.run("mypy", ["mypy", "--strict", "tools/dev"])
    steps.run("pytest", ["pytest", "tools/dev"])


# Held for the whole of test-local: a concurrent test-local in this checkout
# would otherwise replace the CSR bundle root or the image worker while a run's
# tests still use them.
HOST_ARTIFACTS_LOCK = ".xtask/host-artifacts.lock"


def test_local(nextest_args: tuple[str, ...] = ()) -> None:
    """Focused host-native tests against a verified CSR bundle and image worker.

    JAUNDER_CSR_BUNDLE_DIR names a bundle to use instead of building one.
    `nextest_args` select tests; none run the whole workspace.
    """
    traced_git("hooks-path", "config", "core.hooksPath", ".githooks")
    Path(HOST_ARTIFACTS_LOCK).parent.mkdir(parents=True, exist_ok=True)
    # Python opens files non-inheritable, so no step's child (or a daemon it
    # leaves behind, such as the sccache server) holds the lock past this run.
    with open(HOST_ARTIFACTS_LOCK, "a") as lock:
        with trace.tracer().start_as_current_span("host-artifacts-lock") as span:
            fcntl.flock(lock, fcntl.LOCK_EX)
            span.set_attribute(STATUS, Status.OK)
        bundle = os.environ.get("JAUNDER_CSR_BUNDLE_DIR")
        if bundle is None:
            steps.run(
                "csr-admission",
                [*HOST_PROOF, "admit-inputs", "--", ".#csrBundle.sourceAdmits"],
            )
            steps.nix_build("csr-bundle", ".#csrBundle")
            bundle = f"{steps.GCROOTS}/csr-bundle"
        root = steps.text(
            "csr-prerequisite", [*HOST_PROOF, "csr-prerequisite", "--", bundle]
        )
        # Storage and seeding tests start the application worker, so it is
        # built even when nextest selects only storage. aws-lc-sys keys its
        # FORTIFY_SOURCE handling on CFLAGS being present: set it empty when
        # absent and keep an inherited value unchanged.
        steps.run(
            "image-worker",
            ["cargo", "build", "--locked", "-p", "jaunder", "--bin", "jaunder"],
            environment={"CFLAGS": os.environ.get("CFLAGS", "")},
        )
        steps.run(
            "nextest",
            ["pg-run", "cargo", "nextest", "run", *(nextest_args or ("--workspace",))],
            environment={
                "JAUNDER_CSR_BUNDLE_DIR": root,
                "JAUNDER_TEST_EXECUTABLE": str(Path.cwd() / "target/debug/jaunder"),
            },
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
    steps.nix_build("csrBundle", ".#csrBundle")
    bundle = (Path(steps.GCROOTS) / "csrBundle").resolve()
    steps.run(
        "wasm-budget",
        [*HOST_PROOF, "wasm-audit", "--", "budget", "--site-path", str(bundle)],
        report.render,
    )


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
