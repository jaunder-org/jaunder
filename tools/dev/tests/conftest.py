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


def require(present: bool, prerequisite: str) -> None:
    """Skips inside `checks.dev-python` without `prerequisite`; fails elsewhere.

    That check sets JAUNDER_DEV_PYTHON_CHECK: its filtered source lacks the
    repository's scripts and its sandbox lacks jq and Nix. Anywhere else a
    missing prerequisite is a broken environment, not a reason to pass.
    """
    if present:
        return
    if os.environ.get("JAUNDER_DEV_PYTHON_CHECK") == "1":
        pytest.skip(f"needs {prerequisite}")
    pytest.fail(f"missing prerequisite: {prerequisite}")


# How the devShim (nix/packages.nix) starts `dev`: the checkout's tools/dev
# joins this interpreter's sys.path alone, never PYTHONPATH, so every step's
# child inherits the caller's environment unchanged.
LAUNCH = "import sys; sys.path.insert(0, sys.argv.pop(1)); "
DEV = "from dev.__main__ import main; main()"


def launch(code: str, *args: str) -> list[str]:
    return [sys.executable, "-P", "-c", LAUNCH + code, str(PACKAGE), *args]


def _script(path: Path, body: str) -> None:
    path.write_text(f"#!{sys.executable}\n{PRELUDE}{body}\n", encoding="utf-8")
    path.chmod(0o755)


@dataclass
class Checkout:
    root: Path
    bin: Path

    def tool(self, name: str, body: str) -> None:
        _script(self.bin / name, body)

    def nix_store(self) -> None:
        """`nix` and `nix-store` as FAKE_NIX_STORE's one fake store."""
        for name in ["nix", "nix-store"]:
            self.tool(name, FAKE_NIX_STORE)

    @property
    def tool_out(self) -> Path:
        """The output every fake tool package build prints (FAKE_TOOL_OUT)."""
        return self.bin.parent / "tool-out"

    def built(self, name: str, body: str) -> None:
        """A host tool binary in the fake tool package output."""
        path = self.tool_out / "bin" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        _script(path, body)

    def environment(self, **extra: str) -> dict[str, str]:
        environment = {
            key: value
            for key, value in os.environ.items()
            if key
            not in (
                "JAUNDER_TRACE_DIR",
                "TRACEPARENT",
                "TRACESTATE",
                # Set in CI jobs; a test opts in to it.
                "JAUNDER_CI_JOB_START_NS",
                "E2E_TYPES_NODE_MODULES",
                "E2E_PLAYWRIGHT_TEST",
                "PYTHONPATH",
            )
        }
        environment["PATH"] = f"{self.bin}{os.pathsep}{environment['PATH']}"
        environment["FAKE_TOOL_OUT"] = str(self.tool_out)
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

    def dev_on_path(self) -> None:
        """Repository scripts call `dev X`; here that runs this package's `dev`.

        It starts as the devShim does, so a script's nested workflow joins the
        same trace.
        """
        self.executable(
            "dev",
            f"#!{sys.executable}\nimport os, sys\n"
            f"argv = {launch(DEV)!r} + sys.argv[1:]\n"
            "os.execv(argv[0], argv)\n",
        )

    def from_repository(self, *paths: str) -> None:
        """Copies repository files the workflow runs (see `require`)."""
        for path in paths:
            source = REPOSITORY / path
            require(source.is_file(), f"the repository's {path}")
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(source, target)

    def order(self) -> str:
        path = self.root / "order"
        return path.read_text(encoding="utf-8") if path.exists() else ""

    def runs(self) -> list[Path]:
        return sorted((self.root / ".xtask" / "run").glob("*.trace.d"))

    def events(self) -> list[dict[str, Any]]:
        """The single run's merged slices, in trace order."""
        (run,) = self.runs()
        trace = json.loads(run.with_name(run.name[: -len(".d")] + ".json").read_text())
        return [event for event in trace["traceEvents"] if event["ph"] == "X"]

    def merged(self) -> dict[str, dict[str, Any]]:
        """The single run's merged slices, keyed by span name."""
        return {event["name"]: event for event in self.events()}


# The flake packages that build host tools, with the binaries each holds.
TOOL_PACKAGES = {
    "repository-policy": (
        "source-contracts",
        "repository-docs",
        "org-timezone-manifest",
        "scoped-leptosfmt",
    ),
    "host-proof": (
        "admit-inputs",
        "csr-prerequisite",
        "e2e-evidence",
        "e2e-local",
        "source-diagnostics",
        "trace-analysis",
        "wasm-audit",
    ),
    "coverage-check": ("coverage-check",),
    "perf-data": ("perf-data",),
    "pr-observer": ("pr-observer",),
    "system-artifacts": ("system-artifacts",),
}


# A fake for every tool a workflow runs. Each appends its argv to $FAKE_LOG
# (unit and record separators keep empty and whitespace-bearing arguments
# exact) and exits 17 when its command line equals $FAIL_COMMAND; otherwise a
# host tool that reports prints a minimal report with no steps. Fake Git
# answers index custody's queries (the only Git that honours a hook's
# overrides) for a clean index, refuses inherited repository overrides
# anywhere else with 23, answers checkout discovery from the real Git, and
# reports a dirty tree while `dirty-status` exists. The CSR prerequisite prints
# its root under `/verified`.
# An evaluation prints a fake derivation named after the installable's last
# component. Nix cache queries fail, so Nix steps record no cache source; a
# build's out-link points at a fake store path named after the link. A tool
# package's build prints $FAKE_TOOL_OUT, the one fake output of every tool
# package (`Checkout.built`).
FAKE = r"""#!/bin/sh
name=${0##*/}
if [ "$name" = git ]; then
    case "$*" in
      'status --porcelain=v1 -z '*|'diff --cached --raw -z '*|'ls-files --stage -z')
        printf '%s\037' "$name" "$@" >> "$FAKE_LOG"
        printf '\036' >> "$FAKE_LOG"
        exit 0 ;;
    esac
    if [ -n "${GIT_DIR+x}${GIT_WORK_TREE+x}${GIT_INDEX_FILE+x}${GIT_OBJECT_DIRECTORY+x}${GIT_COMMON_DIR+x}${GIT_NAMESPACE+x}" ]; then
        printf 'Git inherited a repository override\n' >&2
        exit 23
    fi
    case "$*" in
      'rev-parse --show-toplevel') exec "$REAL_GIT" "$@" ;;
    esac
fi
case "$name $*" in
  'nix path-info --derivation '*) ;;
  'nix path-info '*) exit 1 ;;
esac
if [ "$name" = cargo ]; then
    printf '%s\n' "${RUSTC_WRAPPER-unset}" "${CARGO_INCREMENTAL-unset}" "${KACHE_CACHE_EXECUTABLES-unset}" > cache-env
    printf '%s\n' "${CARGO_HOME-unset}" "${CARGO_NET_OFFLINE-unset}" > cargo-env
fi
if [ "$name $1" = "nix build" ]; then
    previous=
    for argument in "$@"; do
        if [ "$previous" = --out-link ]; then
            ln -sfn "/nix/store/fake-${argument##*/}" "$argument"
        fi
        previous=$argument
    done
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
if [ "$name $*" = "pg-run cargo nextest run --workspace" ]; then
    touch producer-resource
    trap 'sleep 0.2; rm producer-resource; printf "cleaned\n" >> producer-cleanup' EXIT
fi
if [ "$name $*" = "${FAIL_COMMAND-}" ]; then
    printf '%s failed\n' "$name" >&2
    exit 17
fi
case "$name $*" in
  'nix build '*' --no-link .#'*)
    for installable; do :; done
    case "$installable" in
      @TOOL_INSTALLABLES@)
        # Every binary a test has not written records itself, as this fake.
        mkdir -p "$FAKE_TOOL_OUT/bin"
        for binary in @TOOL_BINARIES@; do
            [ -e "$FAKE_TOOL_OUT/bin/$binary" ] || cp "$0" "$FAKE_TOOL_OUT/bin/$binary"
        done
        printf '%s\n' "$FAKE_TOOL_OUT" ;;
    esac ;;
  'nix path-info --derivation '*)
    for installable; do :; done
    printf '/nix/store/fake-%s.drv\n' "${installable##*[#/]}" ;;
  'csr-prerequisite '*)
    for root; do :; done
    printf '/verified%s\n' "$root" ;;
  'admit-inputs '*|'e2e-evidence '*|'e2e-local '*|'source-diagnostics '*|\
  'trace-analysis '*|'wasm-audit '*|'coverage-check '*)
    printf '{"steps":[]}\n' ;;
esac
""".replace(
    "@TOOL_INSTALLABLES@", "|".join(f".#{name}" for name in TOOL_PACKAGES)
).replace(
    "@TOOL_BINARIES@",
    " ".join(binary for binaries in TOOL_PACKAGES.values() for binary in binaries),
)
# Store registration times for fake `nix` path-info entries: before, and after,
# any real step's start.
REGISTERED_LONG_AGO = 0
REGISTERED_LATER = 4102444800
# A fake `nix` (and, under that name, `nix-store`) with a store holding one
# output, `build-out`, of OUTPUT_SIZE bytes. It is valid from the start with
# NIX_VALID, registered long ago; otherwise a build registers it later. A later
# build first moves that registration to long ago: the earlier step registered
# it before this one started. An evaluation prints DERIVATION, whose closure
# is that output, the derivations and CLOSURE's dependencies. The other
# variables inject evaluation, query and build failures.
OUTPUT_SIZE = 1000
DERIVATION = "/nix/store/fake-build.drv"
# Each dependency's entry: (registration, ultimate, narSize), or None while it
# is not valid. Only `fetched-dep` was substituted during the build.
CLOSURE: dict[str, tuple[int, bool, int] | None] = {
    "/nix/store/fake-input.drv": (REGISTERED_LATER, False, 50000),
    "fetched-dep": (REGISTERED_LATER, False, 200),
    "built-dep": (REGISTERED_LATER, True, 30),
    "old-dep": (REGISTERED_LONG_AGO, False, 4),
    "invalid-dep": None,
}
FETCHED_DEPENDENCIES = 200
FAKE_NIX_STORE = (
    f'LONG_AGO, FUTURE = "{REGISTERED_LONG_AGO}", "{REGISTERED_LATER}"\n'
    f"OUTPUT_SIZE, DERIVATION, CLOSURE = {OUTPUT_SIZE}, {DERIVATION!r}, {CLOSURE!r}\n"
    + """\
import json
env = os.environ.get
if pathlib.Path(sys.argv[0]).name == "nix-store":
    assert sys.argv[1:4] == ["--query", "--requisites", "--include-outputs"], sys.argv
    if env("NIX_CLOSURE_FAIL"):
        print("closure-err", file=sys.stderr)
        sys.exit(1)
    print("\\n".join([sys.argv[4], "build-out", *CLOSURE]))
    sys.exit()
command = sys.argv[1]


def entry(path):
    if path in CLOSURE:
        found = CLOSURE[path]
        if found is None:
            return None
        registered, ultimate, size = found
        return {"registrationTime": registered, "ultimate": ultimate, "narSize": size}
    if env("NIX_VALID"):
        registered = LONG_AGO
    elif os.path.exists("built"):
        registered = open("built").read()
    else:
        return None
    return {
        "ultimate": bool(env("NIX_ULTIMATE")),
        "version": 1,
        "registrationTime": int(registered),
        "narSize": OUTPUT_SIZE,
    }


if command == "path-info" and "--derivation" in sys.argv:
    code = int(env("NIX_EVAL_CODE", "0"))
    if code:
        print("eval-err", file=sys.stderr)
        sys.exit(code)
    print(DERIVATION)
elif command == "path-info":
    if env("NIX_INFO_FAIL"):
        print("info-err", file=sys.stderr)
        sys.exit(1)
    if env("NIX_INFO_GARBLED"):
        print("not json")
        sys.exit()
    if sys.argv[-1] == "--stdin":
        if env("NIX_STDIN_FAIL"):
            print("stdin-err", file=sys.stderr)
            sys.exit(1)
        paths = sys.stdin.read().split("\\n")
    else:
        paths = [sys.argv[-1]]
    print(json.dumps({path: entry(path) for path in paths}))
    print("info-warning", file=sys.stderr)
else:
    print("build-out")
    if env("NIX_TWO_OUTPUTS"):
        print("build-out-dev")
    print("build-err", file=sys.stderr)
    code = int(env("NIX_BUILD_CODE", "0"))
    if code == 0 and not env("NIX_LOST"):
        registered = LONG_AGO if os.path.exists("built") else FUTURE
        with open("built", "w") as store:
            store.write(registered)
    sys.exit(code)
"""
)
FAKE_TOOLS = (
    "bash",
    "cargo",
    "nix",
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
    "pg-run",
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
    configuration (templates, hooks, identity) stays out as well, and
    discovery stops at the test's directory: a fixture repository whose `.git`
    is missing must not resolve to a repository enclosing the temporary
    directory, as the checkout does when it holds pytest's base directory.
    """
    for key in [key for key in os.environ if key.startswith("GIT_")]:
        monkeypatch.delenv(key)
    monkeypatch.setenv("GIT_CEILING_DIRECTORIES", str(tmp_path))
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
