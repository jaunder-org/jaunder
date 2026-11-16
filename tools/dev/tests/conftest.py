"""A throwaway Git checkout with fake tools on PATH, driven through the real CLI."""

import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

PACKAGE = Path(__file__).resolve().parents[1]
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
