"""Every devShell binds TMPDIR to its checkout, or refuses to start.

Without this, host-tool temporary files would quietly land in `/tmp` again.
Each shell's `shellHook` comes from `nix eval` of the repository's flake and
runs in a child Bash, without realizing the shell's tools. Provisioning is
intercepted only to observe its startup environment; Git, Bash, Node and the
filesystem run for real. It needs Nix and the repository, so only the
filtered `checks.dev-python` source may skip it.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from conftest import REPOSITORY, require

SHELLS = ["ci", "ci-nix", "default", "mutants"]
# ci-nix is Linux-only.
HOOKS = """all: let
  shells = all.${builtins.currentSystem};
  selected = { inherit (shells) default ci mutants; }
    // (if builtins.hasAttr "ci-nix" shells then { inherit (shells) ci-nix; } else {});
in builtins.mapAttrs (_: shell: shell.shellHook) selected"""


# Module-scoped, so the evaluation runs once per worker and before conftest's
# autouse isolation replaces HOME and with it the Nix configuration. The
# Git-backed flake keeps ignored build/test outputs out of Nix's source
# snapshot.
@pytest.fixture(scope="module")
def hooks() -> dict[str, str]:
    require(shutil.which("nix") is not None, "Nix")
    require((REPOSITORY / "flake.nix").is_file(), "the repository's flake.nix")
    result = subprocess.run(
        ["nix", "eval", "--impure", "--json", ".#devShells", "--apply", HOOKS],
        cwd=REPOSITORY,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    evaluated: dict[str, str] = json.loads(result.stdout)
    return evaluated


@pytest.fixture(params=SHELLS)
def shell(request: pytest.FixtureRequest, hooks: dict[str, str]) -> tuple[str, str]:
    name: str = request.param
    if name not in hooks:
        pytest.skip(f"{name} is not a devShell on {sys.platform}")
    return name, hooks[name]


def repository(parent: Path, name: str) -> Path:
    root = parent / name
    subprocess.run(["git", "init", "--quiet", str(root)], check=True)
    return root


def enter(
    hook: str, cwd: Path, parent: Path, record: Path, body: str
) -> subprocess.CompletedProcess[str]:
    """Enters `hook` in a child Bash at `cwd`, then runs `body`.

    The child inherits TMPDIR=`parent`, so replacing it is observable and the
    result never depends on the test runner's own shell. Fixture Git
    discovery stops at `parent`, where every fixture repository sits.
    """
    environment = os.environ | {
        "GIT_CEILING_DIRECTORIES": str(parent),
        "TMPDIR": str(parent),
        "JAUNDER_TEST_SHELL_HOOK": hook,
        "JAUNDER_TEST_PROVISION_RECORD": str(record),
        "JAUNDER_TEST_PARENT": str(parent),
    }
    script = (
        "setup-e2e-node-modules() { printf '%s\\n' \"$TMPDIR\" "
        '> "$JAUNDER_TEST_PROVISION_RECORD"; }\n'
        'eval "$JAUNDER_TEST_SHELL_HOOK"\n' + body
    )
    return subprocess.run(
        ["bash", "--noprofile", "--norc", "-c", script],
        cwd=cwd,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )


def assert_provisioned(name: str, record: Path, tmpdir: Path) -> None:
    if name == "ci-nix":
        assert not record.exists(), "ci-nix must not provision IDE node modules"
    else:
        assert record.read_text() == f"{tmpdir}\n"


def test_shared_shell_entry_replaces_inherited_tmpdir_before_provisioning(
    tmp_path: Path, shell: tuple[str, str]
) -> None:
    name, hook = shell
    root = repository(tmp_path, f"{name} checkout")
    nested = root / "nested directory"
    nested.mkdir()
    record = tmp_path / "provision"
    output = enter(
        hook,
        nested,
        tmp_path,
        record,
        "printf 'shell=%s\\n' \"$TMPDIR\"\n"
        "node -p 'require(\"os\").tmpdir()'\n"
        "bash -c 'printenv TMPDIR; bash -c \"printenv TMPDIR\"'",
    )
    assert output.returncode == 0, f"{name}: {output.stderr}"
    expected = root / ".xtask" / "tmp"
    assert expected.is_dir(), f"{name}: shell must create TMPDIR"
    assert list(expected.iterdir()) == [], (
        "usability probe must be removed by its startup owner"
    )
    # Direct, child and grandchild all inherit it.
    assert output.stdout == f"shell={expected}\n{expected}\n{expected}\n{expected}\n"
    assert_provisioned(name, record, expected)


def test_shell_entry_binding_survives_cd_and_allows_later_command_override(
    tmp_path: Path, shell: tuple[str, str]
) -> None:
    name, hook = shell
    a = repository(tmp_path, "checkout A")
    b = repository(tmp_path, "checkout B")
    override = tmp_path / "override"
    override.mkdir()
    record = tmp_path / "provision"
    output = enter(
        hook,
        a,
        tmp_path,
        record,
        """
TMPDIR="$JAUNDER_TEST_PARENT/override" node -p 'require("os").tmpdir()'
printenv TMPDIR
cd "$JAUNDER_TEST_PARENT/checkout B"
printenv TMPDIR
""",
    )
    assert output.returncode == 0, f"{name}: {output.stderr}"
    a_tmp = a / ".xtask" / "tmp"
    assert output.stdout == f"{override}\n{a_tmp}\n{a_tmp}\n"
    assert not (b / ".xtask" / "tmp").exists(), "cd must not create a new temp root"
    output = enter(hook, b, tmp_path, record, "printenv TMPDIR")
    assert output.returncode == 0, f"{name}: {output.stderr}"
    b_tmp = b / ".xtask" / "tmp"
    assert output.stdout == f"{b_tmp}\n"
    assert_provisioned(name, record, b_tmp)


def test_direnv_rebinds_tmpdir_after_nix_direnv_restores_it(tmp_path: Path) -> None:
    """nix-direnv runs the shellHook, then restores the pre-entry TMPDIR."""
    envrc = REPOSITORY / ".envrc"
    shared = REPOSITORY / "nix" / "checkout-tmpdir.sh"
    require(envrc.is_file() and shared.is_file(), "the repository's .envrc")
    root = repository(tmp_path, "checkout")
    (root / "nix").mkdir()
    shutil.copy(envrc, root / ".envrc")
    shutil.copy(shared, root / "nix" / "checkout-tmpdir.sh")
    script = (
        "watch_file() { :; }\n"
        'use() { export TMPDIR="$JAUNDER_TEST_PARENT"; }\n'
        "source .envrc\n"
        "printenv TMPDIR"
    )
    output = subprocess.run(
        ["bash", "--noprofile", "--norc", "-c", script],
        cwd=root,
        env=os.environ
        | {
            "GIT_CEILING_DIRECTORIES": str(tmp_path),
            "JAUNDER_TEST_PARENT": str(tmp_path),
        },
        capture_output=True,
        text=True,
        check=False,
    )
    assert output.returncode == 0, output.stderr
    assert output.stdout == f"{root / '.xtask' / 'tmp'}\n"


@pytest.mark.parametrize("failure", ["no-checkout", "blocked", "unusable"])
def test_invalid_checkout_or_temp_directory_stops_startup_and_command(
    tmp_path: Path, shell: tuple[str, str], failure: str
) -> None:
    name, hook = shell
    if failure == "no-checkout":
        root = tmp_path / "not a repository"
        root.mkdir()
    else:
        root = repository(tmp_path, "checkout")
    tmpdir = root / ".xtask" / "tmp"
    if failure == "blocked":
        tmpdir.parent.mkdir()
        tmpdir.write_text("caller-owned obstruction")
    elif failure == "unusable":
        tmpdir.mkdir(parents=True)
        tmpdir.chmod(0o500)
    record = tmp_path / "provision"
    # No errexit: the hook itself must stop the shell, not merely return a
    # failure that the caller could overwrite by running its command.
    try:
        output = enter(
            hook,
            root,
            tmp_path,
            record,
            "printf 'command ran\\n' > \"$JAUNDER_TEST_PARENT/command-ran\"",
        )
    finally:
        if failure == "unusable":
            tmpdir.chmod(0o700)
    assert output.returncode == 1, f"{name}/{failure}: startup must fail with exit 1"
    diagnostic = (
        "jaunder: cannot resolve an absolute Git checkout root for TMPDIR"
        if failure == "no-checkout"
        else "jaunder: cannot create or use TMPDIR"
    )
    assert diagnostic in output.stderr, f"{name}/{failure}: {output.stderr}"
    assert not record.exists(), f"{name}/{failure}: provisioning must not run"
    assert not (tmp_path / "command-ran").exists(), (
        f"{name}/{failure}: command must not run or fall back"
    )
    if failure == "blocked":
        assert tmpdir.read_text() == "caller-owned obstruction"
