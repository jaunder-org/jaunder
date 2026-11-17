"""Production-baseline qualification with this checkout's HEAD as the harness.

This module owns the ordinary Git preflight: a clean tree, resolved commits and
the canonical origin, then one refetch of every commit involved. The
qualification host binds HEAD to its clean source pin before selecting a Nix
harness, and runs only as its own Nix output.
"""

import click

from jaunder_dev import steps
from jaunder_dev.checkout import clean_tree, traced_git

ORIGINS = (
    "https://github.com/jaunder-org/jaunder.git",
    "https://github.com/jaunder-org/jaunder",
    "git@github.com:jaunder-org/jaunder.git",
    "ssh://git@github.com/jaunder-org/jaunder.git",
)
QUALIFICATION_HOST = ["nix", "run", ".#qualification-host"]


def nonempty_target(
    _context: click.Context, _parameter: click.Parameter, target: str
) -> str:
    """Admits acceptance's TARGET before any work; only discovery has none."""
    if not target:
        raise click.ClickException(
            "production-baseline accept requires a nonempty target"
        )
    return target


def _commit(name: str, revision: str, source: str = "") -> str:
    """`revision` resolved to a commit; a commit equal to `source` is refused."""
    return traced_git(
        name,
        "rev-parse",
        "--verify",
        "--end-of-options",
        f"{revision}^{{commit}}",
        refusal=lambda commit: (
            "production-baseline accept requires distinct resolved source "
            "and target commits"
            if source and commit == source
            else None
        ),
    )


def _harness() -> str:
    return traced_git("harness", "rev-parse", "--verify", "HEAD^{commit}")


def _qualify(source: str, target: str) -> None:
    """Fetches the harness, source and target commits, then runs the host.

    Only discovery passes an empty `target`.
    """
    clean_tree()
    harness = _harness()
    source = _commit("source", source)
    if target:
        target = _commit("target", target, source)
    traced_git(
        "origin",
        "remote",
        "get-url",
        "origin",
        refusal=lambda origin: (
            None
            if origin in ORIGINS
            else "production-baseline requires origin to be jaunder-org/jaunder"
        ),
    )
    commits = [commit for commit in dict.fromkeys([harness, source, target]) if commit]
    # Re-request every SHA: an ordinary fetch can accept locally complete
    # objects without asking the server whether it has them.
    steps.run(
        "fetch",
        ["git", "fetch", "--quiet", "--no-write-fetch-head", "--refetch", "origin"]
        + commits,
    )
    steps.run(
        "qualification-host",
        [
            *QUALIFICATION_HOST,
            "--",
            "--expected-harness",
            harness,
            "--source",
            source,
            *(["--target", target] if target else []),
        ],
    )


def discover(revision: str) -> None:
    _qualify(revision, "")


def accept(source: str, target: str) -> None:
    _qualify(source, target)


def qualify_styling() -> None:
    clean_tree()
    harness = _harness()
    steps.run(
        "qualification-host",
        [*QUALIFICATION_HOST, "--", "--expected-harness", harness],
    )


def smoke() -> None:
    """The mutable local lifecycle diagnostic: no qualification or evidence."""
    steps.run("qualification-host", QUALIFICATION_HOST)
