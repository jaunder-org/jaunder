"""The `dev` command line: one subcommand per workflow."""

import os
import subprocess
from collections.abc import Callable
from pathlib import Path

import click

from jaunder_dev import report, steps, summary

HOST_PROOF = (
    "cargo",
    "run",
    "--locked",
    "--quiet",
    "--manifest-path",
    "tools/Cargo.toml",
    "-p",
    "host-proof",
    "--bin",
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


def _git(*args: str) -> subprocess.CompletedProcess[str]:
    environment = {
        key: value for key, value in os.environ.items() if key not in GIT_OVERRIDES
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


def static_tsc() -> None:
    steps.run("tsc", ["tsc", "--noEmit", "-p", "end2end/tsconfig.json"])


@cli.command()
@click.argument("mode", type=click.Choice(["verify", "fix"]), default="verify")
def tsc(mode: str) -> None:
    """Type-check the end-to-end suite after linking its pinned node_modules.

    Provisioning follows the current checkout, so it also works after `cd` into
    another worktree from an existing devShell. TypeScript has no autofix, so
    MODE changes nothing.
    """

    def body() -> None:
        steps.run(
            "setup-e2e-node-modules",
            [
                "setup-e2e-node-modules",
                os.environ.get("E2E_TYPES_NODE_MODULES", ""),
                os.environ.get("E2E_PLAYWRIGHT_TEST", ""),
            ],
        )
        static_tsc()

    _gate(body)


@cli.command("test-local")
@click.argument("nextest_args", nargs=-1, type=click.UNPROCESSED)
def test_local(nextest_args: tuple[str, ...]) -> None:
    """Focused Rust proof with isolated PostgreSQL.

    NEXTEST_ARGS, given after `--`, are nextest selectors (none: the workspace).
    """

    def body() -> None:
        steps.run(
            "test-local",
            [*HOST_PROOF, "test-local", "--", *nextest_args],
            report.render,
        )

    _gate(body)


@cli.group()
def static() -> None:
    """Pinned static checks, one named check at a time."""


sandbox_option = click.option(
    "--sandbox", is_flag=True, help="Run in place, as a Nix check does."
)
fix_option = click.option("--fix", is_flag=True, help="Fix instead of verifying.")


@static.command("tsc")
@fix_option
@sandbox_option
def static_tsc_command(fix: bool, sandbox: bool) -> None:
    """Type-check the end-to-end suite; its node_modules must already be linked.

    TypeScript has no autofix, so --fix changes nothing.
    """
    _gate(static_tsc, sandbox)


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
    click.echo(summary.render(summary.rows(path), markdown), nl=False)
