"""The gates, CI lanes and host check groups, each a fixed sequence of steps.

Every function runs its steps in order on the host, from the checkout root,
and stops at the first failure (`steps.StepFailed`). Host tools that print one
JSON report are shown through `report.render`; their exit status stays the
verdict.
"""

import fcntl
import json
import os
from collections.abc import Callable
from pathlib import Path

from opentelemetry.trace import Span

from dev import e2e, index, report, static, steps, trace, wasm_audit
from dev.checkout import clean_tree
from dev.commands import (
    ADMIT_INPUTS,
    COVERAGE_CHECK,
    SOURCE_CONTRACT,
    repository_policy,
)
from dev.static import Mode

# The Nix checks prepush takes on x86_64-linux in place of host leaves, each
# with the leaves it covers. Their filtered sources let a verdict built for
# another tree, here or in CI, be reused. A check is requested at its first
# leaf's slot; its other leaves then run nothing.
SHARED_VERDICTS = {
    "rust-format": ("fmt", "leptosfmt", "tools-fmt"),
    "static-docs": ("prettier-markdown",),
    "static-elisp": ("elisp-fmt", "byte-compile"),
    "static-typescript": ("tsc",),
    "static-ert": ("ert",),
}

DOCS_SHAPE = (
    "sequence-check",
    "adr-format",
    "adr-readme-parity",
    "adr-view-parity",
    "doc-links",
    "flow-docs",
)
SOURCE_CONTRACTS = (
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


def leaf(name: str, mode: Mode, shared: bool) -> None:
    """The host static leaf `name`, or with `shared` the Nix check covering it.

    `shared` is only ever prepush's: Verify mode, inside its index custody.
    """
    for check, leaves in SHARED_VERDICTS.items():
        if shared and name in leaves:
            if name == leaves[0]:
                steps.nix_check(check, index.SHARED_FLAGS)
            return
    static.run(name, mode)


def host(mode: Mode, shared: bool = False) -> None:
    format_checks(mode, shared)
    docs_shape()
    source_contracts()
    compile_checks(mode, shared)
    elisp_checks(mode, shared)


def format_checks(mode: Mode, shared: bool = False) -> None:
    leaf("fmt", mode, shared)
    leaf("leptosfmt", mode, shared)
    markdown(mode, shared)
    static.run("prettier-css", mode)
    leaf("elisp-fmt", mode, shared)
    leaf("tools-fmt", mode, shared)


def markdown(mode: Mode, shared: bool = False) -> None:
    """Markdown and end2end Prettier."""
    leaf("prettier-markdown", mode, shared)
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


def tsc_and_cargo_deny(mode: Mode, shared: bool = False) -> None:
    leaf("tsc", mode, shared)
    static.run("cargo-deny", mode)


def compile_checks(mode: Mode, shared: bool = False) -> None:
    tsc_and_cargo_deny(mode, shared)
    for name in static.CLIPPY:
        static.run(name, mode)


def elisp_checks(mode: Mode, shared: bool = False) -> None:
    leaf("byte-compile", mode, shared)
    leaf("ert", mode, shared)


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
            steps.run(
                "csr-admission", [*steps.tool(ADMIT_INPUTS), ".#csrBundle.sourceAdmits"]
            )
            bundle = csr_bundle()
        # server's build script validates the bundle and fails closed. It runs
        # in server/ and tracks the variable's value, so the root is absolute
        # and resolved past the GC-root symlink: a new bundle is a new value.
        root = os.path.realpath(bundle)
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
    wasm_audit.budget(csr_bundle())


def wasm_tests() -> None:
    steps.nix_check("wasm-tests")


def _producer_and_gate(
    check: str,
    finished: Callable[[Span, int], None] | None = None,
    verdict: bool = False,
) -> None:
    """Realizes a hermetic producer as a GC root, then its hard status gate.

    `finished` is as for the producer's `build` step and `verdict` as for both
    builds (`steps.nix_build`).
    """
    steps.nix_check(check, steps.NIX_LOCK_FLAGS, finished=finished, verdict=verdict)
    steps.nix_check(f"{check}-gate", steps.NIX_LOCK_FLAGS, keep=False, verdict=verdict)


def _non_negative_int(value: object) -> bool:
    # bool is an int to Python, never to the producer.
    return type(value) is int and value >= 0


def _coverage_stages(output: str) -> list[trace.Phase]:
    """The stages the coverage producer timed, from its output's `status.json`.

    Only what places a stage is read: a stage that never started has neither
    start nor duration and is skipped. A file that cannot be read whole yields
    nothing rather than a partial picture.
    """
    try:
        stages = json.loads((Path(output) / "status.json").read_text("utf-8"))["stages"]
        phases = []
        for stage in stages:
            start, duration = stage.get("started_unix_ms"), stage.get("duration_ms")
            if start is None and duration is None:
                continue
            name = stage["stage"]
            if not (
                isinstance(name, str)
                and _non_negative_int(start)
                and _non_negative_int(duration)
            ):
                return []
            phases.append(
                trace.Phase(
                    name,
                    start * 1_000_000,
                    (start + duration) * 1_000_000,
                    stage["outcome"] == "success",
                )
            )
    except (
        OSError,
        ValueError,
        LookupError,
        TypeError,
        AttributeError,
        RecursionError,
    ):
        return []
    return phases


# The coverage producer's output, as its step holds it.
COVERAGE_ROOT = f"{steps.GCROOTS}/coverage"


def _coverage_stage_spans(build: Span, started_ns: int) -> None:
    """Traces the stages the producer's run timed under its `build` step.

    They are read from the GC root, which a failed build leaves on an earlier
    output whose stages fall outside the step, as a reused output's do.
    """
    trace.record_phases(build, started_ns, _coverage_stages(COVERAGE_ROOT))


def coverage_workflow() -> None:
    # Rust coverage must execute for every tested commit (ADR-0192); static
    # doctests may be substituted.
    _producer_and_gate("coverage", _coverage_stage_spans, verdict=True)
    steps.run(
        "coverage-policy",
        [*steps.tool(COVERAGE_CHECK), "policy", "--out", COVERAGE_ROOT],
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
    host(Mode.FIX)
    host_tests()


def check() -> None:
    # Fix-mode check keeps a dirty tree: it runs the doctest producer without
    # the public `doctests` admission.
    check_no_test()
    test_local()
    wasm_tests()
    doctests_workflow()


def prepush() -> None:
    """On x86_64-linux the shared verdicts replace their host leaves, and every
    check runs inside index custody; elsewhere the host leaves run."""
    clean_tree()
    shared = index.shares_verdicts()

    def checks() -> None:
        host(Mode.VERIFY, shared)
        host_tests()
        test_local()
        doctests_workflow()

    if shared:
        index.prepush(checks)
    else:
        checks()


def validate_no_e2e() -> None:
    clean_tree()
    host(Mode.VERIFY)
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


def ci_validate_host(advisories: bool) -> None:
    """The host check groups without Clippy, whose CI verdict is the hermetic
    lane's `static-code` (docs/adr/0255-ci-clippy-verdict-from-nix.md).

    cargo-deny runs only with `advisories`, which CI passes for a change to the
    product dependencies (`dev.scope`): the hermetic lane checks bans, licenses
    and sources on every run, so only the advisory check is the host's
    (docs/adr/0275-ci-checks-advisories-on-dependency-change.md).
    """
    clean_tree()
    format_checks(Mode.VERIFY)
    docs_shape()
    source_contracts()
    static.run("tsc", Mode.VERIFY)
    if advisories:
        static.run("cargo-deny", Mode.VERIFY)
    elisp_checks(Mode.VERIFY)
    platform_contract()
    host_tests()


def docs_checks(mode: Mode) -> None:
    """The checks a Markdown-only change needs (`rawdiff.markdown_only`)."""
    markdown(mode)
    docs_shape()


def ci_validate_docs() -> None:
    """CI's whole verdict for a Markdown-only change (`dev.scope`)."""
    clean_tree()
    docs_checks(Mode.VERIFY)


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
        docs_checks(Mode.FIX)
    else:
        check_no_test()


def precommit() -> None:
    """Index custody selects the host checks and reconciles the index."""
    index.precommit(_precommit_checks)
