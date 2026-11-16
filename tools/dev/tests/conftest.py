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
            )
        }
        environment["PATH"] = f"{self.bin}{os.pathsep}{environment['PATH']}"
        environment["PYTHONPATH"] = str(PACKAGE)
        environment.update(extra)
        return environment

    def command(self, *args: str) -> list[str]:
        return [sys.executable, "-P", "-m", "jaunder_dev", *args]

    def dev(
        self, *args: str, cwd: Path | None = None, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            self.command(*args),
            cwd=cwd or self.root,
            env=self.environment(**extra),
            capture_output=True,
            check=False,
            timeout=60,
        )

    def popen(self, *args: str, **extra: str) -> subprocess.Popen[bytes]:
        return subprocess.Popen(
            self.command(*args),
            cwd=self.root,
            env=self.environment(**extra),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

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
def isolated_from_the_callers_repository(monkeypatch: pytest.MonkeyPatch) -> None:
    """Drops inherited Git overrides before any test runs git or `dev`.

    A Git hook exports GIT_DIR and friends. Left in place, `git init` on a
    throwaway checkout would reinitialise the caller's repository instead,
    and `dev` would resolve its checkout root there.
    """
    for key in [key for key in os.environ if key.startswith("GIT_")]:
        monkeypatch.delenv(key)


@pytest.fixture
def checkout(tmp_path: Path) -> Checkout:
    root = tmp_path / "checkout"
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    (root / "sub").mkdir()
    tools = tmp_path / "bin"
    tools.mkdir()
    return Checkout(root, tools)
