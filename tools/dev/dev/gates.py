"""The gates, CI lanes and host check groups, each a declaration of its steps.

Each function returns the steps it runs, in order, on the host, from the
checkout root; `steps.execute` runs them and stops at the first failure
(`steps.StepFailed`). No function here runs a step itself. Host tools that
print one JSON report are shown through `report.render`; their exit status
stays the verdict.
"""

import fcntl
import json
import os
from collections.abc import Callable
from functools import partial
from pathlib import Path

from opentelemetry.trace import Span

from dev import e2e, index, report, static, steps, trace, wasm_audit
from dev.checkout import clean_tree
from dev.commands import (
    ADMIT_INPUTS,
    COVERAGE_CHECK,
    SOURCE_CONTRACT,
    Tool,
    repository_policy,
)
from dev.static import Mode
from dev.steps import Declaration, Step

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


def _static(name: str, mode: Mode) -> Step:
    return partial(static.run, name, mode)


def _run(name: str, *argv: str, compiles: bool = False) -> Step:
    return partial(steps.run, name, list(argv), compiles=compiles)


def _tool_step(
    name: str,
    wanted: Tool,
    *args: str,
    render: Callable[[bytes], str | None] | None = None,
) -> Step:
    """Runs a binary of a tool package, built on its first use in the run."""

    def step() -> None:
        steps.run(name, [*steps.tool(wanted), *args], render)

    return step


# The host check surface: its named groups in fail-fast order.


def leaf(name: str, mode: Mode, shared: bool) -> Declaration:
    """The host static leaf `name`, or with `shared` the Nix check covering it.

    `shared` is only ever prepush's: Verify mode, inside its index custody.
    """
    for check, leaves in SHARED_VERDICTS.items():
        if shared and name in leaves:
            if name != leaves[0]:
                return ()
            return (partial(steps.nix_check, check, index.SHARED_FLAGS),)
    return (_static(name, mode),)


def host(mode: Mode, shared: bool = False) -> Declaration:
    return (
        *format_checks(mode, shared),
        *docs_shape(),
        *source_contracts(),
        *compile_checks(mode, shared),
        *elisp_checks(mode, shared),
    )


def format_checks(mode: Mode, shared: bool = False) -> Declaration:
    return (
        *leaf("fmt", mode, shared),
        *leaf("leptosfmt", mode, shared),
        *markdown(mode, shared),
        _static("prettier-css", mode),
        *leaf("elisp-fmt", mode, shared),
        *leaf("tools-fmt", mode, shared),
    )


def markdown(mode: Mode, shared: bool = False) -> Declaration:
    """Markdown and end2end Prettier."""
    return (
        *leaf("prettier-markdown", mode, shared),
        _static("prettier-end2end", mode),
    )


def repository_docs(check: str) -> Declaration:
    return (_tool_step(check, repository_policy("repository-docs"), check),)


def docs_shape() -> Declaration:
    return tuple(step for check in DOCS_SHAPE for step in repository_docs(check))


def source_contract(name: str) -> Declaration:
    return (_tool_step(name, SOURCE_CONTRACT, name),)


def source_contract_checks() -> Declaration:
    return tuple(step for name in SOURCE_CONTRACTS for step in source_contract(name))


def source_contracts() -> Declaration:
    return (
        _static("ast-grep-tests", Mode.VERIFY),
        _static("no-full-reload", Mode.VERIFY),
        *source_contract_checks(),
    )


def cargo_deny(mode: Mode) -> Declaration:
    """Both Cargo workspaces' dependency policy
    (docs/adr/drafts/cargo-deny-checks-every-workspace.md).

    It runs after the other compile checks, and last in the CI host lane: a
    newly published advisory is one the change at hand may have no way to fix,
    so it should hide as few other verdicts as it can.
    """
    return (_static("cargo-deny", mode), _static("tools-cargo-deny", mode))


def compile_checks(mode: Mode, shared: bool = False) -> Declaration:
    return (
        *leaf("tsc", mode, shared),
        *(_static(name, mode) for name in static.CLIPPY),
        *cargo_deny(mode),
    )


def elisp_checks(mode: Mode, shared: bool = False) -> Declaration:
    return (*leaf("byte-compile", mode, shared), *leaf("ert", mode, shared))


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


def tools_tests() -> Declaration:
    return (
        # Compiled first with the run's own arguments, so `tools-tests` times
        # the tests alone.
        _run(
            "tools-tests-build",
            "cargo",
            "test",
            "--no-run",
            *TOOLS_TESTS,
            compiles=True,
        ),
        _run("tools-tests", "cargo", "test", *TOOLS_TESTS, compiles=True),
    )


def host_tests() -> Declaration:
    return (
        *tools_tests(),
        _run("ruff-format", "ruff", "format", "--check", "tools/dev"),
        _run("ruff-check", "ruff", "check", "tools/dev"),
        _run("mypy", "mypy", "--strict", "tools/dev"),
        _run("pytest", "pytest", "tools/dev"),
    )


# Held for the whole of test-local: a concurrent test-local in this checkout
# would otherwise replace the CSR bundle root or the image worker while a run's
# tests still use them.
HOST_ARTIFACTS_LOCK = ".xtask/host-artifacts.lock"
# The step that builds `.#csrBundle`, and the GC root that holds its output.
CSR_BUNDLE = "csr-bundle"
CSR_BUNDLE_ROOT = f"{steps.GCROOTS}/{CSR_BUNDLE}"


def csr_bundle() -> str:
    """Builds `.#csrBundle` as its own step; returns the GC root that holds it."""
    steps.nix_build(CSR_BUNDLE, ".#csrBundle")
    return CSR_BUNDLE_ROOT


def _build_csr_bundle() -> None:
    # As a step: the root it returns is the constant CSR_BUNDLE_ROOT.
    csr_bundle()


def _holding_host_artifacts_lock(inner: Declaration) -> None:
    """Runs `inner` holding the checkout's host-artifacts lock throughout."""
    Path(HOST_ARTIFACTS_LOCK).parent.mkdir(parents=True, exist_ok=True)
    # Python opens files non-inheritable, so no step's child (or a daemon it
    # leaves behind, such as the kache daemon) holds the lock past this run.
    with open(HOST_ARTIFACTS_LOCK, "a") as lock:
        with steps.phase("host-artifacts-lock"):
            fcntl.flock(lock, fcntl.LOCK_EX)
        steps.execute(inner)


def _cargo_step(name: str, argv: list[str], bundle: str) -> Step:
    """A Cargo step of test-local, against the CSR bundle at `bundle`."""

    def step() -> None:
        # server's build script validates the bundle and fails closed. It runs
        # in server/ and tracks the variable's value, so the root is absolute
        # and resolved past the GC-root symlink: a new bundle is a new value.
        # The lock keeps that root fixed, so every Cargo step resolves the
        # same one: build scripts track these (server's
        # JAUNDER_CSR_BUNDLE_DIR, aws-lc-sys's CFLAGS), so any difference
        # would rebuild between steps. aws-lc-sys keys its FORTIFY_SOURCE
        # handling on CFLAGS being present: set it empty when absent and keep
        # an inherited value unchanged.
        environment = {
            "CFLAGS": os.environ.get("CFLAGS", ""),
            "JAUNDER_CSR_BUNDLE_DIR": os.path.realpath(bundle),
            "JAUNDER_TEST_EXECUTABLE": str(Path.cwd() / "target/debug/jaunder"),
        }
        steps.run(name, argv, environment=environment, compiles=True)

    return step


def test_local(nextest_args: tuple[str, ...] = ()) -> Declaration:
    """Focused host-native tests against a verified CSR bundle and image worker.

    JAUNDER_CSR_BUNDLE_DIR names a bundle to use instead of building one.
    `nextest_args` select tests; none run the whole workspace.
    """
    root = os.environ.get("JAUNDER_CSR_BUNDLE_DIR")
    bundle: Declaration = ()
    if root is None:
        root = CSR_BUNDLE_ROOT
        bundle = (
            _tool_step("csr-admission", ADMIT_INPUTS, ".#csrBundle.sourceAdmits"),
            _build_csr_bundle,
        )
    # The tests compile apart from their run when `dev` chooses the selection.
    # Hand-typed arguments may include run options nextest refuses with
    # --no-run (--no-fail-fast), so they compile in the run.
    nextest_build: Declaration = (
        ()
        if nextest_args
        else (
            _cargo_step(
                "nextest-build",
                ["cargo", "nextest", "run", "--no-run", "--workspace"],
                root,
            ),
        )
    )
    nextest = ["pg-run", "cargo", "nextest", "run", *(nextest_args or ("--workspace",))]
    inner = (
        *bundle,
        # Storage and seeding tests start the application worker, so it is
        # built even when nextest selects only storage.
        _cargo_step(
            "image-worker",
            ["cargo", "build", "--locked", "-p", "jaunder", "--bin", "jaunder"],
            root,
        ),
        *nextest_build,
        _cargo_step("nextest", nextest, root),
    )
    return (partial(_holding_host_artifacts_lock, inner),)


def platform_contract() -> Declaration:
    return (
        _run(
            "platform",
            "nix",
            "eval",
            "--impure",
            "--expr",
            'import ./nix/platform-contract.nix { flake = builtins.getFlake ("git+file://" + toString ./.); }',
        ),
    )


def static_checks() -> Declaration:
    return tuple(partial(steps.nix_check, check) for check in STATIC_CHECKS)


def wasm_budget() -> Declaration:
    """Builds the CSR bundle as its own step, then measures that output."""
    return (_build_csr_bundle, partial(wasm_audit.budget, CSR_BUNDLE_ROOT))


def wasm_tests() -> Declaration:
    return (partial(steps.nix_check, "wasm-tests"),)


def _producer_and_gate(
    check: str,
    finished: Callable[[Span, int], None] | None = None,
    verdict: bool = False,
) -> Declaration:
    """Realizes a hermetic producer as a GC root, then its hard status gate.

    `finished` is as for the producer's `build` step and `verdict` as for both
    builds (`steps.nix_build`).
    """
    flags = steps.NIX_LOCK_FLAGS
    return (
        partial(steps.nix_check, check, flags, finished=finished, verdict=verdict),
        partial(steps.nix_check, f"{check}-gate", flags, keep=False, verdict=verdict),
    )


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


def coverage_workflow() -> Declaration:
    # Rust coverage must execute for every tested commit (ADR-0192); static
    # doctests may be substituted.
    return (
        *_producer_and_gate("coverage", _coverage_stage_spans, verdict=True),
        _tool_step(
            "coverage-policy",
            COVERAGE_CHECK,
            "policy",
            "--out",
            COVERAGE_ROOT,
            render=report.render,
        ),
    )


def doctests_workflow() -> Declaration:
    return _producer_and_gate("doctests")


def elisp_coverage() -> Declaration:
    return (
        partial(steps.nix_check, "elisp-coverage-producer"),
        _tool_step(
            "elisp-consume", COVERAGE_CHECK, "elisp-consume", render=report.render
        ),
    )


# Gates.


def check_no_test() -> Declaration:
    return (*host(Mode.FIX), *host_tests())


def check() -> Declaration:
    # Fix-mode check keeps a dirty tree: it runs the doctest producer without
    # the public `doctests` admission.
    return (*check_no_test(), *test_local(), *wasm_tests(), *doctests_workflow())


def prepush() -> Declaration:
    """On x86_64-linux the shared verdicts replace their host leaves, and every
    check runs inside index custody; elsewhere the host leaves run."""
    shared = index.shares_verdicts()
    checks = (
        *host(Mode.VERIFY, shared),
        *host_tests(),
        *test_local(),
        *doctests_workflow(),
    )
    if shared:
        return (clean_tree, partial(index.prepush, checks))
    return (clean_tree, *checks)


def validate_no_e2e() -> Declaration:
    return (
        clean_tree,
        *host(Mode.VERIFY),
        *platform_contract(),
        *static_checks(),
        *wasm_budget(),
        *host_tests(),
        *wasm_tests(),
        *coverage_workflow(),
        *doctests_workflow(),
        *elisp_coverage(),
    )


def validate() -> Declaration:
    return (*validate_no_e2e(), partial(e2e.run_all, ""))


def ci_validate_host(advisories: bool) -> Declaration:
    """The host checks whose CI verdict the hermetic lane does not own.

    Every other host check is a step of a `STATIC_CHECKS` derivation, whose
    verdict CI takes instead
    (docs/adr/drafts/ci-hermetic-verdicts-not-repeated-on-host.md); the
    `repository` tests are those `checks.dev-python` deselects.

    cargo-deny runs only with `advisories`, which CI passes for a change to
    either workspace's dependencies (`dev.scope`): the hermetic lane checks
    bans, licenses and sources on every run, so only the advisory check is the
    host's (docs/adr/drafts/ci-checks-advisories-on-dependency-change.md).
    """
    return (
        clean_tree,
        *docs_shape(),
        *source_contract_checks(),
        *platform_contract(),
        *tools_tests(),
        _run("pytest-repository", "pytest", "-m", "repository", "tools/dev"),
        *(cargo_deny(Mode.VERIFY) if advisories else ()),
    )


def docs_checks(mode: Mode) -> Declaration:
    """The checks a Markdown-only change needs (`rawdiff.markdown_only`)."""
    return (*markdown(mode), *docs_shape())


def ci_validate_docs() -> Declaration:
    """CI's whole verdict for a Markdown-only change (`dev.scope`)."""
    return (clean_tree, *docs_checks(Mode.VERIFY))


def ci_validate_hermetic() -> Declaration:
    return (clean_tree, *static_checks(), *wasm_budget())


def ci_validate_test_checks() -> Declaration:
    return (clean_tree, *wasm_tests(), *doctests_workflow(), *elisp_coverage())


def coverage() -> Declaration:
    return (clean_tree, *coverage_workflow())


def doctests() -> Declaration:
    return (clean_tree, *doctests_workflow())


def _precommit_checks(markdown_only: bool) -> Declaration:
    """A Markdown-only change takes the Markdown-sensitive Fix checks, in order;
    any other change takes check-no-test."""
    return docs_checks(Mode.FIX) if markdown_only else check_no_test()


def precommit() -> Declaration:
    """Index custody selects the host checks and reconciles the index."""
    return (partial(index.precommit, _precommit_checks),)
