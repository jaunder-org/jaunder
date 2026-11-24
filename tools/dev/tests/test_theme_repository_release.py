"""The theme repository workflow's release job, run against a fake `gh`.

Each test runs a step's own `run:` block, found in the workflow by job and step
name, under bash. A theme release version is strict SemVer, and the release
step publishes the package once and never replaces it: a re-run uploads a
missing package, accepts identical bytes, refuses different or duplicate
packages, and corrects the draft and prerelease flags. It needs the
repository's `.github/`, so only the filtered `checks.dev-python` source may
skip it.
"""

import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml
from conftest import REPOSITORY, require

WORKFLOWS = REPOSITORY / ".github" / "workflows"

# Answers `gh release view` from the case's release state and logs every other
# call, one line of arguments each, for the test to compare exactly.
FAKE_GH = """\
import os, pathlib, shutil, sys
arguments = sys.argv[1:]
if arguments[:2] == ["release", "view"]:
    if os.environ["FAKE_RELEASE"] == "absent":
        sys.exit(1)
    field = arguments[arguments.index("--json") + 1] if "--json" in arguments else None
    answers = {
        "assets": "FAKE_ASSET_COUNT",
        "isDraft": "FAKE_DRAFT",
        "isPrerelease": "FAKE_PRERELEASE",
    }
    if field is not None:
        print(os.environ[answers[field]])
    sys.exit(0)
if arguments[:2] not in (
    ["release", "create"],
    ["release", "upload"],
    ["release", "download"],
    ["release", "edit"],
):
    sys.exit(f"unexpected gh invocation: {arguments}")
with open(os.environ["FAKE_GH_LOG"], "a") as log:
    log.write(" ".join(arguments) + "\\n")
if arguments[1] == "download":
    target = pathlib.Path(arguments[arguments.index("--dir") + 1]) / "theme-package.zip"
    if os.environ["FAKE_DOWNLOADED_BYTES"] == "identical":
        shutil.copyfile(os.environ["PACKAGE"], target)
    else:
        target.write_text("different bytes\\n")
"""


def load(name: str) -> Any:
    path = WORKFLOWS / name
    require(path.is_file(), f"the repository's .github/workflows/{name}")
    return yaml.safe_load(path.read_text())


def step(name: str) -> str:
    """The `run:` block of the release job's step called `name`."""
    steps = load("theme-repository.yml")["jobs"]["release"]["steps"]
    (block,) = [each["run"] for each in steps if each.get("name") == name]
    assert isinstance(block, str)
    return block


def run(block: str, **env: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", "-c", block],
        env={"PATH": os.environ["PATH"], **env},
        capture_output=True,
        text=True,
        check=False,
    )


def test_the_fixture_workflow_triggers_on_the_semver_fixture_namespace() -> None:
    # YAML 1.1 reads the bare `on` key as true.
    triggers = load("theme-repository-fixture.yml")[True]
    assert "v0.0.0-theme-repository-fixture.[1-9]*" in triggers["push"]["tags"]


@pytest.mark.parametrize(
    "tag",
    [
        "v0.0.0",
        "v1.2.3",
        "v12.34.56",
        "v0.0.0-theme-repository-fixture.1",
        "v1.2.3-rc.1",
        "v1.2.3-0",
        "v1.2.3-alpha.01beta",
        "v1.2.3+build.5",
        "v1.2.3-rc.1+build.5",
    ],
)
def test_a_semver_tag_is_accepted(tag: str) -> None:
    result = run(step("Validate release version"), TAG=tag)
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    "tag",
    [
        "theme-repository-fixture-1",
        "v01.2.3",
        "v1.02.3",
        "v1.2.03",
        "v1.2",
        "v1.2.3-",
        "v1.2.3-01",
        "v1.2.3-alpha.01",
        "v1.2.3-rc.01.1",
        "v1.2.3-rc..1",
        "v1.2.3+",
        "v1.2.3+build..5",
    ],
)
def test_a_non_semver_tag_is_rejected(tag: str) -> None:
    result = run(step("Validate release version"), TAG=tag)
    assert result.returncode != 0
    assert "release tag must be SemVer" in result.stderr


def release(tmp_path: Path, tag: str, **state: str) -> tuple[int, list[str]]:
    """Runs the release step for `tag` against a fake `gh` holding `state`;
    returns its exit status and the logged `gh` calls, with the temporary
    directory written as `$TMP`."""
    block = step("Create immutable release")
    bin_directory = tmp_path / "bin"
    bin_directory.mkdir()
    gh = bin_directory / "gh"
    gh.write_text(f"#!{sys.executable}\n{FAKE_GH}")
    gh.chmod(0o755)
    package = tmp_path / "theme-package.zip"
    package.write_text("package bytes\n")
    log = tmp_path / "gh.log"
    log.touch()
    result = run(
        block,
        PATH=f"{bin_directory}{os.pathsep}{os.environ['PATH']}",
        TAG=tag,
        PACKAGE=str(package),
        RUNNER_TEMP=str(tmp_path),
        FAKE_GH_LOG=str(log),
        **state,
    )
    calls = log.read_text().replace(str(tmp_path), "$TMP").splitlines()
    return result.returncode, calls


UPLOAD = "release upload {tag} $TMP/theme-package.zip#theme-package.zip"
DOWNLOAD = "release download {tag} --pattern theme-package.zip --dir $TMP/existing-theme-package"


def present(
    *, draft: bool, prerelease: bool, assets: int, downloaded: str = "identical"
) -> dict[str, str]:
    """The fake `gh`'s state for a release that already exists."""
    return {
        "FAKE_RELEASE": "present",
        "FAKE_DRAFT": str(draft).lower(),
        "FAKE_PRERELEASE": str(prerelease).lower(),
        "FAKE_ASSET_COUNT": str(assets),
        "FAKE_DOWNLOADED_BYTES": downloaded,
    }


@pytest.mark.parametrize(
    ("tag", "state", "status", "calls"),
    [
        pytest.param(
            "v1.2.3",
            present(draft=False, prerelease=False, assets=0),
            0,
            [UPLOAD],
            id="stable_matching",
        ),
        pytest.param(
            "v1.2.3",
            present(draft=True, prerelease=True, assets=0),
            0,
            [UPLOAD, "release edit {tag} --draft=false --prerelease=false"],
            id="stable_mismatched_draft",
        ),
        pytest.param(
            "v1.2.3-rc.1",
            present(draft=True, prerelease=True, assets=0),
            0,
            [UPLOAD, "release edit {tag} --draft=false"],
            id="prerelease_matching_draft",
        ),
        pytest.param(
            "v1.2.3-rc.1",
            present(draft=False, prerelease=False, assets=0),
            0,
            [UPLOAD, "release edit {tag} --prerelease=true"],
            id="prerelease_mismatched",
        ),
        pytest.param(
            "v1.2.3-rc.1",
            present(draft=True, prerelease=False, assets=1),
            0,
            [DOWNLOAD, "release edit {tag} --draft=false --prerelease=true"],
            id="identical_asset",
        ),
        pytest.param(
            "v1.2.3",
            present(draft=True, prerelease=True, assets=1, downloaded="different"),
            1,
            [DOWNLOAD],
            id="different_asset",
        ),
        pytest.param(
            "v1.2.3",
            present(draft=True, prerelease=True, assets=2),
            1,
            [],
            id="duplicate_assets",
        ),
    ],
)
def test_an_existing_release_keeps_its_package_and_corrects_its_flags(
    tmp_path: Path, tag: str, state: dict[str, str], status: int, calls: list[str]
) -> None:
    actual = release(tmp_path, tag, **state)
    assert actual == (status, [call.format(tag=tag) for call in calls])


@pytest.mark.parametrize(
    ("tag", "flags"),
    [
        pytest.param("v1.2.3", "", id="stable"),
        pytest.param("v1.2.3-rc.1", " --prerelease", id="prerelease"),
        pytest.param("v1.2.3+build.5", "", id="stable_build"),
        pytest.param("v1.2.3-rc.1+build.5", " --prerelease", id="prerelease_build"),
    ],
)
def test_an_absent_release_is_created_with_the_package_alone(
    tmp_path: Path, tag: str, flags: str
) -> None:
    create = (
        f"release create {tag} $TMP/theme-package.zip#theme-package.zip --title {tag}"
    )
    assert release(tmp_path, tag, FAKE_RELEASE="absent") == (0, [create + flags])
