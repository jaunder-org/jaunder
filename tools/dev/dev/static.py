"""Pinned static checks: one named check (a leaf), or a fixed group of them.

The host runs them from the checkout root, first preparing what a leaf needs
from this checkout; Nix checks run them in place as the `sandbox` lane, whose
derivation has prepared it. A Cargo leaf's sandbox lane works offline against
the Cargo home its derivation provides, and a sandbox run admits every home it
may need before any check starts.
"""

import os
from collections.abc import Callable
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

from dev import steps
from dev.commands import SCOPED_LEPTOSFMT


class Mode(StrEnum):
    """Whether a check repairs what it finds or only reports it."""

    FIX = "fix"
    VERIFY = "verify"


PRODUCT_HOME = "JAUNDER_PRODUCT_CARGO_HOME"
TOOLS_HOME = "JAUNDER_TOOLS_CARGO_HOME"


@dataclass(frozen=True)
class Leaf:
    summary: str
    # The check's argv for (fix, sandbox).
    argv: Callable[[bool, bool], list[str]]
    # A Cargo leaf: the variable naming its sandbox Cargo home.
    home: str | None = None
    # Host preparation, run as its own steps first: the environment changes
    # the check then needs.
    prepare: Callable[[], steps.Environment] | None = None


def _fixed(summary: str, *argv: str) -> Leaf:
    """A check with no autofix: both modes run the same command."""
    return Leaf(summary, lambda _fix, _sandbox: list(argv))


def _check(fix: bool) -> list[str]:
    return [] if fix else ["--check"]


def _cargo(summary: str, home: str, args: Callable[[bool, bool], list[str]]) -> Leaf:
    return Leaf(
        summary,
        lambda fix, sandbox: [
            "cargo",
            *(["--offline"] if sandbox else []),
            *args(fix, sandbox),
        ],
        home,
    )


def _clippy(summary: str, home: str, *args: str) -> Leaf:
    return _cargo(
        summary, home, lambda _fix, _sandbox: ["clippy", *args, "--", "-D", "warnings"]
    )


def _link_e2e_node_modules() -> steps.Environment:
    """Links the end-to-end suite's pinned node_modules into this checkout.

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
    return {}


def _build_scoped_leptosfmt() -> steps.Environment:
    """Builds the scoped adapter from this checkout, not a stale one on PATH;
    returns a PATH that runs it first, for leptosfmt to find."""
    steps.tool(SCOPED_LEPTOSFMT)
    directory = Path.cwd() / Path(SCOPED_LEPTOSFMT.path).parent
    return {"PATH": f"{directory}:{os.environ.get('PATH', '')}"}


def _prettier(population: str) -> Leaf:
    return Leaf(
        f"Prettier on {population}.",
        lambda fix, _sandbox: ["prettier", "-w" if fix else "--check", population],
    )


LEAVES = {
    "elisp-fmt": Leaf(
        "Format the Elisp sources.",
        lambda fix, _sandbox: [
            "emacs",
            "--batch",
            "-Q",
            "-l",
            "elisp/scripts/format.el",
            "-f",
            "jaunder-fmt-fix" if fix else "jaunder-fmt-check",
        ],
    ),
    "ast-grep-tests": _fixed(
        "Test the ast-grep rules.", "ast-grep", "test", "--config", "sgconfig.yml"
    ),
    "no-full-reload": _fixed(
        "Forbid full page reloads in web and client sources.",
        "ast-grep",
        "scan",
        "--config",
        "sgconfig.yml",
        "--filter",
        "^no-full-reload$",
        "web/src",
        "client/src",
    ),
    "byte-compile": _fixed(
        "Byte-compile the Elisp sources.",
        "emacs",
        "--batch",
        "-Q",
        "-l",
        "elisp/scripts/byte-compile.el",
    ),
    "tsc": Leaf(
        "Type-check the end-to-end suite, on the host after linking its pinned "
        "node_modules.",
        lambda _fix, _sandbox: ["tsc", "--noEmit", "-p", "end2end/tsconfig.json"],
        prepare=_link_e2e_node_modules,
    ),
    "ert": _fixed(
        "Run the Elisp ERT suite.",
        "emacs",
        "--batch",
        "-Q",
        "-l",
        "elisp/scripts/run-tests.el",
    ),
    # The adapter admits scoped UTF-8 inputs and promotes the pin's per-file
    # errors; it runs `leptosfmt-cli` on the population it admitted.
    "leptosfmt": Leaf(
        "Format Leptos view! macros through scoped-leptosfmt, on the host as "
        "built from this checkout.",
        lambda fix, _sandbox: ["scoped-leptosfmt", "fix" if fix else "verify"],
        prepare=_build_scoped_leptosfmt,
    ),
    "leptosfmt-cli": Leaf(
        "The pinned leptosfmt itself, as scoped-leptosfmt runs it.",
        lambda fix, _sandbox: [
            "leptosfmt",
            "-x",
            ".direnv",
            "-x",
            ".git",
            "-x",
            "target",
            *_check(fix),
            "web/src",
            "csr/src",
        ],
    ),
    "fmt": _cargo(
        "Format the product workspace.",
        PRODUCT_HOME,
        lambda fix, _sandbox: ["fmt", *_check(fix)],
    ),
    "tools-fmt": _cargo(
        "Format the tools workspace.",
        TOOLS_HOME,
        lambda fix, _sandbox: [
            "fmt",
            "--manifest-path",
            "tools/Cargo.toml",
            "--all",
            *_check(fix),
        ],
    ),
    # The sandbox has no advisory database, so it checks the offline policies.
    "cargo-deny": _cargo(
        "Check the dependency policy.",
        PRODUCT_HOME,
        lambda _fix, sandbox: [
            "deny",
            "check",
            *(["bans", "licenses", "sources"] if sandbox else []),
        ],
    ),
    "product-core-clippy": _clippy(
        "Clippy on the product workspace without web, client and csr.",
        PRODUCT_HOME,
        "--workspace",
        "--exclude",
        "web",
        "--exclude",
        "client",
        "--exclude",
        "csr",
        "--all-targets",
    ),
    "web-server-clippy": _clippy(
        "Clippy on web with its server feature.",
        PRODUCT_HOME,
        "-p",
        "web",
        "--features",
        "server",
        "--all-targets",
    ),
    "web-no-server-clippy": _clippy(
        "Clippy on web without default features.",
        PRODUCT_HOME,
        "-p",
        "web",
        "--no-default-features",
        "--all-targets",
    ),
    "client-host-clippy": _clippy(
        "Clippy on client for the host.",
        PRODUCT_HOME,
        "-p",
        "client",
        "--features",
        "csr",
        "--all-targets",
    ),
    "wasm-clippy": _clippy(
        "Clippy on web, client and csr for wasm.",
        PRODUCT_HOME,
        "-p",
        "web",
        "-p",
        "client",
        "-p",
        "csr",
        "--features",
        "csr",
        "--target",
        "wasm32-unknown-unknown",
    ),
    "tools-clippy": _clippy(
        "Clippy on the tools workspace.",
        TOOLS_HOME,
        "--manifest-path",
        "tools/Cargo.toml",
        "--workspace",
        "--all-targets",
        "--features",
        "coverage/host",
    ),
    "prettier-markdown": _prettier("**/*.md"),
    "prettier-end2end": _prettier("end2end"),
    "prettier-css": _prettier("**/*.css"),
}

# The Clippy units, in the order every caller runs them.
CLIPPY = [
    "product-core-clippy",
    "web-server-clippy",
    "web-no-server-clippy",
    "client-host-clippy",
    "wasm-clippy",
    "tools-clippy",
]
GROUPS = {
    "rust-format": (
        "Rust formatting: fmt, leptosfmt and tools-fmt.",
        ["fmt", "leptosfmt", "tools-fmt"],
    ),
}
# Every host static leaf, in the order `dev static-all` runs them, ERT last.
ALL = [
    "fmt",
    "leptosfmt",
    "elisp-fmt",
    "tools-fmt",
    "ast-grep-tests",
    "no-full-reload",
    "byte-compile",
    "tsc",
    "cargo-deny",
    *CLIPPY,
    "ert",
]


def _members(name: str) -> list[tuple[str, Leaf]]:
    if name in GROUPS:
        return [(member, LEAVES[member]) for member in GROUPS[name][1]]
    return [(name, LEAVES[name])]


def missing_home(name: str) -> bool:
    """Whether a sandbox run of `name` lacks a Cargo home it needs.

    A group needs both homes, whichever its members use.
    """
    homes = (
        [PRODUCT_HOME, TOOLS_HOME]
        if name in GROUPS
        else [home for home in [LEAVES[name].home] if home]
    )
    return any(not os.environ.get(home) for home in homes)


def run(name: str, mode: Mode, sandbox: bool = False) -> None:
    """Runs a leaf or group as steps; the caller has admitted sandbox homes.

    On the host each member's preparation runs just before it.
    """
    for member, leaf in _members(name):
        changes: steps.Environment = {}
        if sandbox:
            if leaf.home:
                changes = {
                    "CARGO_NET_OFFLINE": "true",
                    "CARGO_HOME": os.environ[leaf.home],
                }
        elif leaf.prepare is not None:
            changes = leaf.prepare()
        steps.run(member, leaf.argv(mode is Mode.FIX, sandbox), environment=changes)


def run_all(mode: Mode) -> None:
    """Every host static leaf in order, each prepared from this checkout."""
    for name in ALL:
        run(name, mode)
