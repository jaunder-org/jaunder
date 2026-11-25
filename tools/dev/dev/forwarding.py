"""Front doors that forward their arguments to one tool or repository script.

Arguments always travel as argv elements, never through a shell string. The
tools own every decision: nothing here infers approval or ownership.
"""

from collections.abc import Callable, Sequence

from dev import report, steps
from dev.commands import COVERAGE_CHECK, host_proof


def sandbox(args: Sequence[str]) -> None:
    steps.run(
        "ux-sandbox",
        ["nix", "run", *steps.NIX_LOCK_FLAGS, ".#ux-sandbox", "--", *args],
    )


def pg_run(args: Sequence[str]) -> None:
    # The devShell's `pg-run` (scripts/pg-run.py) owns the throwaway cluster.
    steps.run("pg-run", ["pg-run", *args])


def _report(args: Sequence[str]) -> Callable[[bytes], str | None] | None:
    """How a report tool's output is shown: rendered, or as-is when it is help.

    Asked for help, the tool prints its usage instead of a report.
    """
    return None if {"--help", "-h"} & set(args) else report.render


def coverage_check(args: Sequence[str]) -> None:
    steps.run("coverage-check", [*steps.tool(COVERAGE_CHECK), *args], _report(args))


def host_proof_report(binary: str, args: Sequence[str]) -> None:
    """A host-proof diagnostic whose one JSON report is shown rendered."""
    steps.run(binary, [*steps.tool(host_proof(binary)), *args], _report(args))


def pr_land(number: str, expected_head: str, mode: str) -> None:
    steps.run("pr-land", ["bash", "scripts/pr-land.sh", number, expected_head, mode])


def pr_cleanup(number: str, expected_head: str, ownership: str) -> None:
    steps.run(
        "pr-cleanup",
        ["bash", "scripts/pr-cleanup.sh", number, expected_head, ownership],
    )
