"""The `dev` command line: one subcommand per workflow."""

import os
import subprocess
from collections.abc import Callable
from pathlib import Path

import click

from jaunder_dev import steps, summary

# Inherited Git overrides (as set inside hooks) would redirect discovery.
GIT_OVERRIDES = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
)


def checkout_root() -> Path:
    environment = {
        key: value for key, value in os.environ.items() if key not in GIT_OVERRIDES
    }
    found = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    if found.returncode != 0:
        raise click.ClickException(
            f"cannot find the checkout root: {found.stderr.strip()}"
        )
    return Path(found.stdout.strip())


def _gate(body: Callable[[], None], sandbox: bool = False) -> None:
    """Runs a workflow from the checkout root, or in place inside a Nix sandbox."""
    context = click.get_current_context()
    root = Path.cwd() if sandbox else checkout_root()
    os.chdir(root)
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
