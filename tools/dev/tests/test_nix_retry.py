"""`scripts/nix-retry`, and the workflow jobs that enter their shell with it.

Nix exits 1 when no builder failed, as when a substitute download fails
(#2135): the script runs such a command once more. Each job enters its dev
shell through it before any other `nix develop` step, so those steps find the
shell already realised; a direct `nix build` step goes through it too.
"""

import os
import re
import subprocess
from pathlib import Path
from typing import Any

import pytest
import yaml
from conftest import REPOSITORY, require

SCRIPT = REPOSITORY / "scripts" / "nix-retry"
WORKFLOWS = [
    REPOSITORY / ".github" / "workflows" / name
    for name in ("ci.yml", "mutants.yml", "upgrade.yml")
]
SHELL = re.compile(r"nix develop (\.#[\w-]+)")
# `nix build` as a command, not inside `scripts/nix-retry build`.
BUILD = re.compile(r"(?:^|[\s;&|(])nix build")

# Records its arguments, then exits with the next of NIX_BUILD_CODE, a
# comma-separated status per attempt (the last repeats), as conftest's fake does.
FAKE_NIX = """\
#!/usr/bin/env bash
echo "$*" >> "$NIX_CALLS"
echo out
echo err >&2
read -ra codes <<< "${NIX_BUILD_CODE//,/ }"
attempt=$(($(wc -l < "$NIX_CALLS") - 1))
last=$((${#codes[@]} - 1))
exit "${codes[$((attempt < last ? attempt : last))]}"
"""


def launch(tmp_path: Path, **extra: str) -> subprocess.Popen[str]:
    require(SCRIPT.is_file(), "the repository's scripts/nix-retry")
    nix = tmp_path / "bin" / "nix"
    nix.parent.mkdir()
    nix.write_text(FAKE_NIX)
    nix.chmod(0o755)
    (tmp_path / "calls").touch()
    environment = {
        **os.environ,
        "PATH": f"{nix.parent}:{os.environ['PATH']}",
        "NIX_CALLS": str(tmp_path / "calls"),
        **extra,
    }
    return subprocess.Popen(
        [str(SCRIPT), "build", "--no-link", ".#demo"],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def calls(tmp_path: Path) -> list[str]:
    return (tmp_path / "calls").read_text().splitlines()


@pytest.mark.parametrize(
    ("codes", "status", "attempts"),
    [("0", 0, 1), ("1,0", 0, 2), ("1", 1, 2), ("100,0", 100, 1), ("1,100", 100, 2)],
)
def test_nix_retry_runs_a_command_that_exits_1_once_more(
    tmp_path: Path, codes: str, status: int, attempts: int
) -> None:
    script = launch(tmp_path, NIX_BUILD_CODE=codes)
    stdout, stderr = script.communicate(timeout=30)

    assert script.returncode == status
    assert calls(tmp_path) == ["build --no-link .#demo"] * attempts
    assert stdout == "out\n" * attempts
    assert stderr.count("err\n") == attempts
    assert ("running it once more" in stderr) == (attempts == 2)


def steps(job: dict[str, Any]) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = job.get("steps", [])
    return found


def test_each_job_enters_its_dev_shell_through_nix_retry_first() -> None:
    entered = 0
    for workflow in WORKFLOWS:
        require(workflow.is_file(), f"the repository's {workflow.name}")
        for name, job in yaml.safe_load(workflow.read_text())["jobs"].items():
            where = f"{workflow.name} {name}"
            runs = [step.get("run", "") for step in steps(job)]
            shells = {shell for run in runs for shell in SHELL.findall(run)}
            assert len(shells) <= 1, where
            assert not [run for run in runs if BUILD.search(run)], where
            if not shells:
                continue
            (shell,) = shells
            first = min(index for index, run in enumerate(runs) if "nix develop" in run)
            entry = f"scripts/nix-retry develop {shell} --accept-flake-config -c true"
            assert entry in runs[:first], where
            entered += 1
    # host, hermetic, test checks, coverage, docs, e2e lane and reconcile;
    # the mutants shard; the upgrade test.
    assert entered == 9
