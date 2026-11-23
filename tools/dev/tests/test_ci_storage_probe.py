"""`scripts/ci-storage-probe`, the one source of CI storage measurements.

Each call appends a labelled section to a report that CI uploads; the probe is
diagnostic only, so a failed measurement is recorded and never fails the step.
"""

import os
import subprocess
from pathlib import Path

import pytest
from conftest import REPOSITORY, require

SCRIPT = REPOSITORY / "scripts" / "ci-storage-probe"


def probe(
    report: Path, label: str, workspace: Path
) -> subprocess.CompletedProcess[str]:
    require(SCRIPT.is_file(), "the repository's scripts/ci-storage-probe")
    return subprocess.run(
        ["bash", str(SCRIPT), str(report), label],
        env={**os.environ, "GITHUB_WORKSPACE": str(workspace)},
        capture_output=True,
        text=True,
        check=False,
    )


def sections(report: Path) -> dict[str, str]:
    """The report's sections by label, in order."""
    found: dict[str, str] = {}
    label = None
    for line in report.read_text().splitlines():
        if line.startswith("=== ") and line.endswith(" ==="):
            label = line[4:-4]
            found[label] = ""
        elif label is not None:
            found[label] += line + "\n"
    return found


def measurements(section: str) -> list[str]:
    """The section's measurement titles: each is followed, after its output,
    by exactly one `capture_exit=` marker."""
    blocks = section.strip("\n").split("\n\n")
    for block in blocks:
        assert block.splitlines()[-1].startswith("capture_exit="), block
    return [block.splitlines()[0] for block in blocks]


def test_baseline_then_final_append_labelled_sections_with_a_marker_each(
    tmp_path: Path,
) -> None:
    report = tmp_path / ".xtask" / "storage.txt"
    assert probe(report, "baseline", tmp_path).returncode == 0
    assert probe(report, "final", tmp_path).returncode == 0
    found = sections(report)
    assert list(found) == ["baseline", "final"]
    baseline = measurements(found["baseline"])
    final = measurements(found["final"])
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
    result = probe(report, "final", missing)
    assert result.returncode == 0, result
    text = report.read_text()
    assert f"filesystem bytes: {missing}\n" in text
    block = text.split(f"filesystem bytes: {missing}\n", 1)[1].split("\n\n", 1)[0]
    assert block.splitlines()[-1].endswith(" missing_evidence=true"), block


@pytest.mark.parametrize("arguments", [[], ["only-a-report"]])
def test_a_wrong_invocation_is_a_usage_error(
    tmp_path: Path, arguments: list[str]
) -> None:
    require(SCRIPT.is_file(), "the repository's scripts/ci-storage-probe")
    result = subprocess.run(
        ["bash", str(SCRIPT), *arguments],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2
    assert "usage:" in result.stderr
