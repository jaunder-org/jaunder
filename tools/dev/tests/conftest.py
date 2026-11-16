"""A throwaway Git checkout with fake tools on PATH, driven through the real CLI."""

import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

PACKAGE = Path(__file__).resolve().parents[1]
# The repository this package lives in; absent when the tests run from the
# filtered `tools/dev/` source, as `checks.dev-python` does.
REPOSITORY = PACKAGE.parents[1]
# Fixed commits the fake Git resolves for the qualification preflight.
HARNESS = "a" * 40
SOURCE = "b" * 40
TARGET = "c" * 40
# Fake tools share this prelude; each records its argv in `order` at its cwd.
PRELUDE = """\
import os, pathlib, signal, subprocess, sys, time
with open("order", "a") as order:
    order.write(" ".join([pathlib.Path(sys.argv[0]).name, *sys.argv[1:]]) + "\\n")
"""
# How the devShim (nix/packages.nix) starts `dev`: the checkout's tools/dev
# joins this interpreter's sys.path alone, never PYTHONPATH, so every step's
# child inherits the caller's environment unchanged.
LAUNCH = "import sys; sys.path.insert(0, sys.argv.pop(1)); "
DEV = "from jaunder_dev.__main__ import main; main()"
# No public command runs a Nix check yet, so a gate is driven exactly as a
# `dev` subcommand would drive it.
NIX_GATE = (
    "from pathlib import Path; from jaunder_dev import steps; "
    'sys.exit(steps.gate(Path.cwd(), "dev nix", lambda: steps.nix_check("demo")))'
)


def launch(code: str, *args: str) -> list[str]:
    return [sys.executable, "-P", "-c", LAUNCH + code, str(PACKAGE), *args]


@dataclass
class Checkout:
    root: Path
    bin: Path

    def tool(self, name: str, body: str) -> None:
        path = self.bin / name
        path.write_text(f"#!{sys.executable}\n{PRELUDE}{body}\n", encoding="utf-8")
        path.chmod(0o755)

    def environment(self, **extra: str) -> dict[str, str]:
        environment = {
            key: value
            for key, value in os.environ.items()
            if key
            not in (
                "JAUNDER_TRACE_DIR",
                "TRACEPARENT",
                "TRACESTATE",
                "E2E_TYPES_NODE_MODULES",
                "E2E_PLAYWRIGHT_TEST",
                "PYTHONPATH",
            )
        }
        environment["PATH"] = f"{self.bin}{os.pathsep}{environment['PATH']}"
        environment.update(extra)
        return environment

    def command(self, *args: str) -> list[str]:
        return launch(DEV, *args)

    def run(
        self, argv: list[str], environment: dict[str, str], cwd: Path | None = None
    ) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            argv,
            cwd=cwd or self.root,
            env=environment,
            capture_output=True,
            check=False,
            timeout=60,
        )

    def dev(
        self, *args: str, cwd: Path | None = None, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        return self.run(self.command(*args), self.environment(**extra), cwd)

    def spawn(self, argv: list[str], **extra: str) -> subprocess.Popen[bytes]:
        return subprocess.Popen(
            argv,
            cwd=self.root,
            env=self.environment(**extra),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def popen(self, *args: str, **extra: str) -> subprocess.Popen[bytes]:
        return self.spawn(self.command(*args), **extra)

    def executable(self, name: str, text: str) -> Path:
        """A tool written verbatim, recording nothing (no `order` prelude)."""
        path = self.bin / name
        path.write_text(text, encoding="utf-8")
        path.chmod(0o755)
        return path

    def forward_just_to_dev(self) -> None:
        """Repository scripts still call `just X`; here that runs `dev X` instead.

        This stands in for the caller switch to `dev`, so a script's nested
        recipe is this package's port, joined to the same trace.
        """
        self.executable(
            "just",
            f"#!{sys.executable}\nimport os, sys\n"
            f"argv = {launch(DEV)!r} + sys.argv[1:]\n"
            "os.execv(argv[0], argv)\n",
        )

    def from_repository(self, *paths: str) -> None:
        """Copies repository files the workflow runs; skips outside the repository."""
        for path in paths:
            source = REPOSITORY / path
            if not source.is_file():
                pytest.skip(f"needs the repository's {path}")
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(source, target)

    def order(self) -> str:
        path = self.root / "order"
        return path.read_text(encoding="utf-8") if path.exists() else ""

    def runs(self) -> list[Path]:
        return sorted((self.root / ".xtask" / "run").glob("*.trace.d"))

    def merged(self) -> dict[str, dict[str, Any]]:
        """The single run's merged slices, keyed by span name."""
        (run,) = self.runs()
        trace = json.loads(run.with_name(run.name[: -len(".d")] + ".json").read_text())
        return {
            event["name"]: event for event in trace["traceEvents"] if event["ph"] == "X"
        }


# A fake for every tool a workflow runs. Each appends its argv to $FAKE_LOG
# (unit and record separators keep empty and whitespace-bearing arguments
# exact) and exits 17 when its command line equals $FAIL_COMMAND. Fake Git
# refuses inherited repository overrides with 23, answers checkout discovery
# from the real Git, and reports a dirty tree while `dirty-status` exists.
# Nix cache probes fail, so Nix steps record no cache source.
FAKE = r"""#!/bin/sh
name=${0##*/}
if [ "$name" = git ]; then
    if [ -n "${GIT_DIR+x}${GIT_WORK_TREE+x}${GIT_INDEX_FILE+x}${GIT_OBJECT_DIRECTORY+x}${GIT_COMMON_DIR+x}${GIT_NAMESPACE+x}" ]; then
        printf 'Git inherited a repository override\n' >&2
        exit 23
    fi
    case "$*" in
      'rev-parse --show-toplevel') exec "$REAL_GIT" "$@" ;;
      'worktree list --porcelain')
        printf 'worktree %s\n\nworktree %s/.\n\nworktree %s/cache-link\n' "$PWD" "$PWD" "$PWD"
        exit 0 ;;
    esac
fi
case "$name $*" in
  'nix eval --raw '*|'nix path-info '*) exit 1 ;;
esac
if [ "$name" = cargo ]; then
    printf '%s\n' "${RUSTC_WRAPPER-unset}" "${CARGO_INCREMENTAL-unset}" "${SCCACHE_BASEDIRS-unset}" > cache-env
    printf '%s\n' "${CARGO_HOME-unset}" "${CARGO_NET_OFFLINE-unset}" > cargo-env
fi
printf '%s\037' "$name" "$@" >> "$FAKE_LOG"
printf '\036' >> "$FAKE_LOG"
if [ "$name" = git ] && [ -f dirty-status ]; then
    printf ' M tracked.rs\n'
fi
if [ "$name" = git ]; then
    case "$*" in
      'rev-parse --verify HEAD^{commit}'|'rev-parse --verify --end-of-options HEAD^{commit}')
        printf '%s\n' "$HARNESS" ;;
      'rev-parse --verify --end-of-options source^{commit}') printf '%s\n' "$SOURCE" ;;
      'rev-parse --verify --end-of-options target^{commit}') printf '%s\n' "$TARGET" ;;
      'remote get-url origin')
        if [ -f wrong-origin ]; then
            printf 'https://example.invalid/foreign\n'
        else
            printf 'https://github.com/jaunder-org/jaunder.git\n'
        fi ;;
    esac
fi
if [ "$name $*" = "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p host-proof --bin test-local --" ]; then
    touch producer-resource
    trap 'sleep 0.2; rm producer-resource; printf "cleaned\n" >> producer-cleanup' EXIT
fi
if [ "$name $*" = "${FAIL_COMMAND-}" ]; then
    printf '%s failed\n' "$name" >&2
    exit 17
fi
"""
FAKE_TOOLS = (
    "bash",
    "cargo",
    "nix",
    "devtool",
    "prettier",
    "git",
    "scoped-leptosfmt",
    "leptosfmt",
    "emacs",
    "ast-grep",
    "tsc",
    "setup-e2e-node-modules",
    "ruff",
    "mypy",
    "pytest",
)
# Hook-style inherited overrides that would redirect any Git they reach.
GIT_OVERRIDES = {
    "GIT_DIR": "foreign-git-dir",
    "GIT_WORK_TREE": "foreign-work-tree",
    "GIT_INDEX_FILE": "foreign-index",
    "GIT_OBJECT_DIRECTORY": "foreign-objects",
    "GIT_COMMON_DIR": "foreign-common-dir",
    "GIT_NAMESPACE": "foreign-namespace",
}


@dataclass
class Workflow:
    """A checkout whose every workflow tool is a recording fake."""

    checkout: Checkout

    @property
    def root(self) -> Path:
        return self.checkout.root

    def dev(
        self, *args: str, failure: str = "", cwd: Path | None = None, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        log = self.root / "commands"
        log.write_bytes(b"")
        environment = {
            "FAKE_LOG": str(log),
            "FAIL_COMMAND": failure,
            "REAL_GIT": REAL_GIT,
            "HARNESS": HARNESS,
            "SOURCE": SOURCE,
            "TARGET": TARGET,
            **GIT_OVERRIDES,
        }
        return self.checkout.dev(*args, cwd=cwd, **(environment | extra))

    def commands(self) -> list[list[str]]:
        records = (self.root / "commands").read_text(encoding="utf-8").split("\x1e")
        return [record.split("\x1f")[:-1] for record in records[:-1]]


REAL_GIT = shutil.which("git") or "git"


@pytest.fixture
def workflow(checkout: Checkout) -> Workflow:
    for name in FAKE_TOOLS:
        path = checkout.bin / name
        path.write_text(FAKE, encoding="utf-8")
        path.chmod(0o755)
    return Workflow(checkout)


@pytest.fixture(autouse=True)
def isolated_from_the_callers_git(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Keeps every git the tests run, directly or through `dev`, to the fixture.

    A Git hook exports GIT_DIR and friends. Left in place, `git init` on a
    throwaway checkout would reinitialise the caller's repository instead,
    and `dev` would resolve its checkout root there. The developer's own
    configuration (templates, hooks, identity) stays out as well.
    """
    for key in [key for key in os.environ if key.startswith("GIT_")]:
        monkeypatch.delenv(key)
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(home / ".config"))
    monkeypatch.setenv("GIT_CONFIG_NOSYSTEM", "1")
    monkeypatch.setenv("GIT_CONFIG_GLOBAL", os.devnull)


@pytest.fixture
def checkout(tmp_path: Path) -> Checkout:
    root = tmp_path / "checkout"
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    (root / "sub").mkdir()
    tools = tmp_path / "bin"
    tools.mkdir()
    return Checkout(root, tools)
