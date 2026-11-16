"""The `dev` command line: one subcommand per workflow."""

import os
import subprocess
from collections.abc import Callable
from pathlib import Path

import click

from jaunder_dev import gates, static, steps, summary


def _git(*args: str) -> subprocess.CompletedProcess[str]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if key not in gates.GIT_OVERRIDES
    }
    return subprocess.run(
        ["git", *args], env=environment, capture_output=True, text=True, check=False
    )


def checkout_root() -> Path:
    found = _git("rev-parse", "--show-toplevel")
    if found.returncode != 0:
        raise click.ClickException(
            f"cannot find the checkout root: {found.stderr.strip()}"
        )
    return Path(found.stdout.strip())


def compile_cache_environment() -> dict[str, str]:
    """Host Cargo compilation shares rustc artifacts across every worktree.

    A failed worktree listing keeps the current root alone.
    """
    listed = _git("worktree", "list", "--porcelain").stdout.splitlines()
    worktrees = [
        Path(line.removeprefix("worktree "))
        for line in listed
        if line.startswith("worktree ")
    ]
    roots = {
        str(path.resolve())
        for path in [*worktrees, Path.cwd()]
        if path.is_absolute() and path.is_dir()
    }
    return {
        "RUSTC_WRAPPER": "sccache",
        "CARGO_INCREMENTAL": "0",
        "SCCACHE_BASEDIRS": ":".join(sorted(roots)),
    }


def _gate(body: Callable[[], None], sandbox: bool = False) -> None:
    """Runs a workflow from the checkout root, or in place inside a Nix sandbox."""
    context = click.get_current_context()
    root = Path.cwd() if sandbox else checkout_root()
    os.chdir(root)
    if not sandbox:
        os.environ.update(compile_cache_environment())
    context.exit(steps.gate(root, context.command_path, body))


@click.group()
def cli() -> None:
    """Jaunder developer, hook, CI and sandbox workflows."""


mode_argument = click.argument(
    "mode", type=click.Choice(["verify", "fix"]), default="verify"
)


def _workflow(name: str, body: Callable[[], None], summary: str) -> None:
    @cli.command(name, help=summary)
    def command() -> None:
        _gate(body)


_workflow(
    "check-no-test",
    gates.check_no_test,
    "The host check groups in Fix mode, then the auxiliary host tests.",
)
_workflow(
    "check",
    gates.check,
    "check-no-test, then test-local, Nix wasm-tests and the doctests "
    "producer and gate.",
)
_workflow(
    "prepush",
    gates.prepush,
    "Clean tree; the host check groups in Verify mode, the auxiliary host "
    "tests, test-local and the workspace doctests.",
)
_workflow(
    "validate-no-e2e",
    gates.validate_no_e2e,
    "Clean tree; Verify host checks, the platform contract, Nix static "
    "proof, wasm budget and tests, coverage, doctests and Elisp coverage.",
)
_workflow("validate", gates.validate, "validate-no-e2e, then the full E2E workflow.")
_workflow(
    "ci-validate-host",
    gates.ci_validate_host,
    "CI host lane: clean tree, Verify host checks, the platform contract "
    "and the auxiliary host tests.",
)
_workflow(
    "ci-validate-hermetic",
    gates.ci_validate_hermetic,
    "CI hermetic lane: clean tree, Nix static proof and the wasm budget.",
)
_workflow(
    "ci-validate-test-checks",
    gates.ci_validate_test_checks,
    "CI test-checks lane: clean tree, wasm tests, doctests and Elisp coverage.",
)
_workflow(
    "ci-validate-coverage",
    gates.coverage,
    "CI coverage lane: clean tree, then Rust coverage and CRAP policy.",
)
_workflow("coverage", gates.coverage, "Clean tree, then Rust coverage and CRAP policy.")
_workflow(
    "doctests", gates.doctests, "Clean tree, then the doctests producer and gate."
)
_workflow(
    "precommit",
    gates.precommit,
    "The pre-commit gate: Git/index safety selects and runs the host checks.",
)
_workflow(
    "precommit-markdown-host",
    gates.precommit_markdown_host,
    "Fix Markdown and end2end formatting, then docs-shape. Called only by "
    "the Git/index safety owner.",
)
_workflow(
    "precommit-broad-host",
    gates.check_no_test,
    "check-no-test, as the Git/index safety owner calls it.",
)
_workflow(
    "docs-shape",
    gates.docs_shape,
    "Documentation sequence, ADR format and parity, doc links and flow docs.",
)
_workflow(
    "source-contracts",
    gates.source_contracts,
    "ast-grep rules, every repository source-contract policy and the "
    "doctest fence check.",
)


@cli.command("format")
@mode_argument
@click.argument("docs", type=click.Choice(["host", "prepush"]), default="host")
def format_command(mode: str, docs: str) -> None:
    """Formatters, in Fix or Verify MODE.

    DOCS `prepush` takes the shared clean-index Markdown verdict on
    x86_64-linux; it only verifies.
    """
    if docs == "prepush" and mode != "verify":
        raise click.UsageError("prepush docs require verify")
    _gate(lambda: gates.format_checks(mode == "fix", docs))


@cli.command("compile-checks")
@mode_argument
def compile_checks(mode: str) -> None:
    """TypeScript, cargo-deny and the Clippy units."""
    _gate(lambda: gates.compile_checks(mode == "fix"))


@cli.command("elisp-checks")
@mode_argument
def elisp_checks(mode: str) -> None:
    """Elisp byte-compilation and ERT."""
    _gate(lambda: gates.elisp_checks(mode == "fix"))


@cli.command("source-contract")
@click.argument("name")
def source_contract(name: str) -> None:
    """One repository source-contract policy by NAME, e.g. html-sink."""
    _gate(lambda: gates.source_contract(name))


@cli.command()
@mode_argument
def leptosfmt(mode: str) -> None:
    """Format Leptos view! macros with the adapter built from this checkout."""
    _gate(lambda: gates.leptosfmt(mode == "fix"))


@cli.command()
@mode_argument
def tsc(mode: str) -> None:
    """Type-check the end-to-end suite after linking its pinned node_modules.

    TypeScript has no autofix, so MODE changes nothing.
    """
    _gate(lambda: gates.tsc(mode == "fix"))


@cli.command("test-local")
@click.argument("nextest_args", nargs=-1, type=click.UNPROCESSED)
def test_local(nextest_args: tuple[str, ...]) -> None:
    """Focused Rust proof with isolated PostgreSQL.

    NEXTEST_ARGS, given after `--`, are nextest selectors (none: the workspace).
    """
    _gate(lambda: gates.test_local(nextest_args))


@cli.group("static")
def static_group() -> None:
    """Pinned static checks: one named check, or a fixed group of them.

    Checks without an autofix ignore --fix.
    """


def _static_command(name: str, summary: str) -> None:
    @static_group.command(name, help=summary)
    @click.option("--fix", is_flag=True, help="Fix instead of verifying.")
    @click.option(
        "--sandbox",
        is_flag=True,
        help="Run in place, offline against the provided Cargo homes, as a Nix "
        "check does.",
    )
    def command(fix: bool, sandbox: bool) -> None:
        if sandbox and static.missing_home(name):
            raise click.ClickException("sandbox Cargo home is missing")
        _gate(lambda: static.run(name, fix, sandbox), sandbox)


for _name, _leaf in static.LEAVES.items():
    _static_command(_name, _leaf.summary)
for _name, (_summary, _) in static.GROUPS.items():
    _static_command(_name, _summary)


@cli.group("trace")
def trace_group() -> None:
    """Read traced runs."""


@trace_group.command("summary")
@click.argument(
    "path",
    required=False,
    type=click.Path(exists=True, file_okay=False, path_type=Path),
)
@click.option("--markdown", is_flag=True, help="Print a Markdown table.")
def trace_summary(path: Path | None, markdown: bool) -> None:
    """Show a run's slowest spans; PATH is a run directory, default the latest."""
    if path is None:
        path = summary.latest(checkout_root() / ".xtask" / "run")
    # Any record content loads; a run that cannot be read is reported, never
    # a traceback.
    try:
        table = summary.rows(path)
    except OSError as error:
        raise click.ClickException(f"cannot summarise {path}: {error}") from error
    click.echo(summary.render(table, markdown), nl=False)
