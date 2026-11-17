"""The checkout a workflow runs in: its root, the compile cache its worktrees
share, and its Git state.

Every Git call here runs with inherited repository overrides removed, as every
step does, so it always answers for the checkout `dev` was started in.
"""

import os
import subprocess
from collections.abc import Callable
from pathlib import Path

import click

from jaunder_dev import steps


def untraced_git(*args: str) -> subprocess.CompletedProcess[str]:
    """A Git query that is not a step: it records no span; its streams are kept."""
    return subprocess.run(
        ["git", *args],
        env=steps.changed(os.environ, steps.WITHOUT_GIT_OVERRIDES),
        capture_output=True,
        text=True,
        check=False,
    )


def traced_git(
    name: str, *args: str, refusal: Callable[[str], str | None] | None = None
) -> str:
    """A Git query run as the step `name`, its output as `steps.text` gives it."""
    return steps.text(name, ["git", *args], refusal=refusal)


def root() -> Path:
    found = untraced_git("rev-parse", "--show-toplevel")
    if found.returncode != 0:
        raise click.ClickException(
            f"cannot find the checkout root: {found.stderr.strip()}"
        )
    return Path(found.stdout.strip())


def compile_cache_environment() -> dict[str, str]:
    """Host Cargo compilation shares rustc artifacts across every worktree.

    A failed worktree listing keeps the current root alone.
    """
    listed = untraced_git("worktree", "list", "--porcelain").stdout.splitlines()
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


def enter(sandbox: bool) -> Path:
    """Moves to where a workflow runs and returns it.

    The host runs from the checkout root with the shared compile cache; a Nix
    sandbox runs in place with its own Cargo environment.
    """
    where = Path.cwd() if sandbox else root()
    os.chdir(where)
    if not sandbox:
        os.environ.update(compile_cache_environment())
    return where


def clean_tree() -> None:
    """Verify workflows refuse dirty or unavailable Git state before work."""
    traced_git(
        "clean-tree",
        "status",
        "--porcelain",
        "--untracked-files=all",
        refusal=lambda status: (
            f"verify workflows require a clean checkout\n{status}" if status else None
        ),
    )
