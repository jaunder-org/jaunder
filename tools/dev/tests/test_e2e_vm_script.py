"""The e2e VM test script fails a lane that panicked or whose Playwright failed.

`nix-instantiate` evaluates one lane's test script from the real
`nix/checks.nix`, with only NixOS realization and store inputs replaced. The
script's imports and function definitions then run in this process against a
fake machine. It needs Nix and the repository, so CI's host lane runs it, not
`checks.dev-python`.
"""

import ast
import json
import runpy
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
from conftest import REPOSITORY, require

pytestmark = pytest.mark.repository

CHECKS = REPOSITORY / "nix" / "checks.nix"
LANE = "sqlite-firefox-1-of-5"
PRELUDE = """
let
  lib = rec {
    inherit (builtins) listToAttrs genList concatLists concatStringsSep;
    concatStrings = concatStringsSep "";
    concatMap = f: xs: concatLists (map f xs);
    concatMapStringsSep = sep: f: xs: concatStringsSep sep (map f xs);
    unique = xs: builtins.foldl' (acc: x: if builtins.elem x acc then acc else acc ++ [ x ]) [] xs;
    optionalString = condition: value: if condition then value else "";
    optionalAttrs = condition: value: if condition then value else {};
    removeSuffix = suffix: value: builtins.substring 0 (builtins.stringLength value - builtins.stringLength suffix) value;
    escapeShellArg = arg: "'${builtins.replaceStrings [ "'" ] [ "'\\\\''" ] arg}'";
  };
  outputs = import (builtins.toPath CHECKS_PATH) {
    self = {};
    system = "x86_64-linux";
    nixpkgs = {};
    nixosInternals = {};
    packageInternals = {
      visualFontConfig = "/primitive/fonts";
      jaunderBin = "/primitive/jaunder";
      e2ePackage = "/primitive/e2e";
    };
    pkgs = {
      inherit lib;
      stdenv.hostPlatform.isLinux = true;
      testers.nixosTest = args: args // { overrideTestDerivation = _: args; };
      writeText = _: _: "/primitive/input";
      nodejs = "/primitive/node";
      playwright-driver.browsers = "/primitive/browsers";
    };
  };
in
"""


@pytest.fixture(scope="module")
def script() -> str:
    """The lane's generated test script. Read-write mode materializes the
    interpolated source files (not derivation outputs), so the helpers import
    the same capture module as the VM driver."""
    require(shutil.which("nix-instantiate") is not None, "Nix")
    require(CHECKS.is_file(), "the repository's nix/checks.nix")
    expression = (
        PRELUDE.replace("CHECKS_PATH", json.dumps(str(CHECKS)))
        + f'outputs.checks."e2e-{LANE}".testScript'
    )
    result = subprocess.run(
        ["nix-instantiate", "--eval", "--read-write-mode", "--strict", "--json"]
        + ["--expr", expression],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    evaluated: str = json.loads(result.stdout)
    return evaluated


def helpers(script: str, directory: Path, **names: Any) -> dict[str, Any]:
    """A fresh namespace holding `names`, the script's top-level imports and
    its function definitions, as the VM driver would see them; nothing else of
    the script runs."""
    body: list[ast.stmt] = [
        node
        for node in ast.parse(script).body
        if isinstance(node, (ast.Import, ast.ImportFrom, ast.FunctionDef))
    ]
    path = directory / "vm_helpers.py"
    path.write_text(ast.unparse(ast.Module(body=body, type_ignores=[])))
    return runpy.run_path(str(path), init_globals=names)


def test_e2e_capture_host_transport_behavior() -> None:
    capture_tests = REPOSITORY / "nix" / "test_e2e_capture.py"
    require(capture_tests.is_file(), "the repository's nix/test_e2e_capture.py")
    result = subprocess.run(
        [sys.executable, "-B", "-m", "unittest", "discover", "-s", "nix"]
        + ["-p", "test_e2e_capture.py"],
        cwd=REPOSITORY,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, f"capture transport tests failed: {result.stderr}"


class PanicMachine:
    def __init__(self, panicking: str) -> None:
        self.panicking = panicking
        self.verified: list[str] = []

    def succeed(self, command: str) -> str:
        return ""

    def execute(self, command: str) -> tuple[int, str]:
        if command.startswith("test-support verify-no-panics"):
            self.verified.append(command)
            return (1 if self.panicking in command else 0, "")
        return (0, "")

    def copy_from_machine(self, path: str, target: str) -> None:
        pass


@pytest.mark.parametrize(
    ("panicking", "status"),
    [("capture/instance-0 ", 1), ("capture/instance-1 ", 1), ("no instance", 0)],
)
def test_e2e_panic_gate_verifies_every_instance(
    script: str, tmp_path: Path, panicking: str, status: int
) -> None:
    """The zero-panic gate (ADR-0032) covers every server instance (#2076):
    each instance's own journal and capture subdirectory reach the verifier
    under a lane-qualified name, and a panic in any one instance fails the
    lane."""
    machine = PanicMachine(panicking)
    namespace = helpers(script, tmp_path, machine=machine, retention_errors=[])
    assert namespace["verify_instance_panics"]() == status
    assert machine.verified == [
        f"test-support verify-no-panics --capture-dir /var/lib/jaunder/capture/instance-{index} "
        f"--server-log /tmp/jaunder-journal-instance-{index}-{LANE}.log"
        for index in range(2)
    ]


def test_a_playwright_or_capture_failure_fails_the_lane(
    script: str, tmp_path: Path
) -> None:
    outcome = helpers(script, tmp_path)["assert_capture_outcome"]
    # A Playwright failure is reported as itself, not as the capture failure.
    with pytest.raises(AssertionError) as raised:
        outcome(9, ["ancillary lift failed"], LANE)
    assert "Playwright failed (exit 9)" in str(raised.value)
    assert "capture retention failed" not in str(raised.value)
    # A successful Playwright run cannot hide a required capture failure.
    with pytest.raises(AssertionError, match="capture retention failed"):
        outcome(0, ["required lift failed"], LANE)
