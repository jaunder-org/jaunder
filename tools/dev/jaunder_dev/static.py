"""Pinned static checks: one named check (a leaf), or a fixed group of them.

The host runs them from the checkout root; Nix checks run them in place as the
`sandbox` lane. A Cargo leaf's sandbox lane works offline against the Cargo
home its derivation provides, and a sandbox run admits every home it may need
before any check starts.
"""

import os
from collections.abc import Callable
from dataclasses import dataclass
from enum import StrEnum

from jaunder_dev import steps


class Mode(StrEnum):
    """Whether a check repairs what it finds or only reports it."""

    FIX = "fix"
    VERIFY = "verify"


PRODUCT_HOME = "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME"
TOOLS_HOME = "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME"


@dataclass(frozen=True)
class Leaf:
    summary: str
    # The check's argv for (fix, sandbox).
    argv: Callable[[bool, bool], list[str]]
    # A Cargo leaf: the variable naming its sandbox Cargo home.
    home: str | None = None


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
    "tsc": _fixed(
        "Type-check the end-to-end suite; its node_modules must already be linked.",
        "tsc",
        "--noEmit",
        "-p",
        "end2end/tsconfig.json",
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
        "Format Leptos view! macros through scoped-leptosfmt.",
        lambda fix, _sandbox: ["scoped-leptosfmt", "fix" if fix else "verify"],
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
        "doctests/host,coverage/host",
    ),
    "prettier-markdown": _prettier("**/*.md"),
    "prettier-end2end": _prettier("end2end"),
    "prettier-css": _prettier("**/*.css"),
}

# Only the `all` group requests ERT through devtool, which times the request.
_ERT_REQUEST = _fixed("", "devtool", "check", "ert")

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
    "code": (
        "Elisp, ast-grep, cargo-deny and Clippy checks.",
        [
            "elisp-fmt",
            "ast-grep-tests",
            "no-full-reload",
            "byte-compile",
            "cargo-deny",
            *CLIPPY,
        ],
    ),
    "all": (
        "Every static check, ERT through devtool last.",
        [
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
            "ert-request",
        ],
    ),
}


def _members(name: str) -> list[tuple[str, Leaf]]:
    if name in GROUPS:
        return [
            (member, _ERT_REQUEST if member == "ert-request" else LEAVES[member])
            for member in GROUPS[name][1]
        ]
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


def argv(name: str, mode: Mode) -> list[str]:
    """Leaf `name`'s host argv."""
    return LEAVES[name].argv(mode is Mode.FIX, False)


def run(
    name: str,
    mode: Mode,
    sandbox: bool = False,
    environment: steps.Environment | None = None,
) -> None:
    """Runs a leaf or group as steps; the caller has admitted sandbox homes.

    `environment` changes every member's environment.
    """
    for member, leaf in _members(name):
        changes = dict(environment or {})
        if sandbox and leaf.home:
            changes |= {
                "CARGO_NET_OFFLINE": "true",
                "CARGO_HOME": os.environ[leaf.home],
            }
        steps.run(member, leaf.argv(mode is Mode.FIX, sandbox), environment=changes)
