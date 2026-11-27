"""`scripts/ci-storage-probe`, the one source of CI storage measurements.

Each call appends a labelled section to a report that CI uploads; the probe is
diagnostic only, so a failed measurement is recorded and never fails the step.
"""

import os
import subprocess
from pathlib import Path

import pytest
from conftest import REPOSITORY, require

pytestmark = pytest.mark.repository

SCRIPT = REPOSITORY / "scripts" / "ci-storage-probe"


def run(*arguments: str, workspace: Path) -> subprocess.CompletedProcess[str]:
    require(SCRIPT.is_file(), "the repository's scripts/ci-storage-probe")
    return subprocess.run(
        ["bash", str(SCRIPT), *arguments],
        env={**os.environ, "GITHUB_WORKSPACE": str(workspace)},
        capture_output=True,
        text=True,
        check=False,
    )


def sections(report: Path) -> dict[str, dict[str, str]]:
    """The report's sections by label, each mapping a measurement's title to
    its `capture_exit=` marker, which must close every measurement."""
    found: dict[str, dict[str, str]] = {}
    for section in report.read_text().split("\n=== ")[1:]:
        heading, body = section.split(" ===\n", 1)
        found[heading] = {}
        for block in body.strip("\n").split("\n\n"):
            lines = block.splitlines()
            assert lines[-1].startswith("capture_exit="), block
            found[heading][lines[0]] = lines[-1]
    return found


def test_baseline_then_final_append_labelled_sections_with_a_marker_each(
    tmp_path: Path,
) -> None:
    report = tmp_path / ".xtask" / "storage.txt"
    assert run(str(report), "baseline", workspace=tmp_path).returncode == 0
    assert run(str(report), "final", workspace=tmp_path).returncode == 0
    found = sections(report)
    assert list(found) == ["baseline", "final"]
    baseline, final = list(found["baseline"]), list(found["final"])
    assert "filesystem bytes: /nix/store" in baseline
    assert f"filesystem inodes: {tmp_path}" in baseline
    assert "memory-and-swap" in baseline
    # Only the final snapshot pays for the bounded allocation scans.
    assert not any(title.startswith("allocated bytes") for title in baseline)
    assert f"allocated bytes: {tmp_path}/.xtask/tmp" in final
    assert final[: len(baseline)] == baseline


def test_a_failed_measurement_is_recorded_and_never_fails_the_probe(
    tmp_path: Path,
) -> None:
    report = tmp_path / "storage.txt"
    missing = tmp_path / "missing-workspace"
    result = run(str(report), "final", workspace=missing)
    assert result.returncode == 0, result
    measured = sections(report)["final"]
    failed = {
        title: marker
        for title, marker in measured.items()
        if title.endswith((str(missing), f"{missing}/.xtask/tmp"))
    }
    assert len(failed) == 5, measured  # bytes, inodes, mount and two scans
    assert all(m.endswith(" missing_evidence=true") for m in failed.values())


@pytest.mark.parametrize("arguments", [[], ["only-a-report"]])
def test_a_wrong_invocation_is_a_usage_error(
    tmp_path: Path, arguments: list[str]
) -> None:
    result = run(*arguments, workspace=tmp_path)
    assert result.returncode == 2
    assert "usage:" in result.stderr
