"""A throwaway Git checkout with fake tools on PATH, driven through the real CLI."""

import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
import yaml

from dev.e2e_evidence import LaneEvidence, lane_evidence
from dev.traces import TRACE_MEMBER, instance_diag_member

PACKAGE = Path(__file__).resolve().parents[1]
# The repository this package lives in; absent when the tests run from the
# filtered `tools/dev/` source, as `checks.dev-python` does.
REPOSITORY = PACKAGE.parents[1]
# The commit the fake Git resolves `earlier` to, for the upgrade commands.
EARLIER = "b" * 40
# Fake tools share this prelude; each records its argv in `order` at its cwd.
PRELUDE = """\
import os, pathlib, signal, subprocess, sys, time
with open("order", "a") as order:
    order.write(" ".join([pathlib.Path(sys.argv[0]).name, *sys.argv[1:]]) + "\\n")
"""


# The fixtures of the e2e verdicts and trace core, ported with them from Rust.
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def write_capture(
    path: Path, members: list[tuple[str, bytes]], directories: tuple[str, ...] = ()
) -> None:
    """A `capture-*.tar.gz` holding `members` in order, as regular files, and
    `directories` as directory entries."""
    with tarfile.open(path, "w:gz") as archive:
        for name in directories:
            entry = tarfile.TarInfo(name)
            entry.type = tarfile.DIRTYPE
            archive.addfile(entry)
        for name, content in members:
            entry = tarfile.TarInfo(name)
            entry.size = len(content)
            archive.addfile(entry, io.BytesIO(content))


def write_lane_capture(path: Path, trace: str, diag_log: bytes | None = b"") -> None:
    """A VM-shaped capture: the trace beside one server instance's `diag.log`,
    which every real capture carries (#2029, #2076); None omits the log."""
    members = [(TRACE_MEMBER, trace.encode())]
    if diag_log is not None:
        members.append((instance_diag_member(0), diag_log))
    write_capture(path, members)


def lift_lanes(root: Path, browser: str = "firefox") -> list[LaneEvidence]:
    """The SQLite `browser` pair's passing lane evidence, lifted as producers
    lift it: `root/e2e-<lane>/<lane-qualified files>`.

    Firefox's five lanes are the recorded fixtures; any other browser's three
    lanes are the first three relabelled, with the census narrowed to them.
    """
    count = 5 if browser == "firefox" else 3

    def fixture(directory: str, name: str) -> str:
        text = (FIXTURES / directory / name).read_text()
        return (
            text.replace("firefox", browser)
            .replace("-of-5", f"-of-{count}")
            .replace('"shard_count": 5', f'"shard_count": {count}')
        )

    census = json.loads(fixture("e2e-ownership", "expected-census.json"))
    selected = {
        test["test_id"]
        for index in range(1, count + 1)
        for test in json.loads(
            fixture("e2e-ownership", f"shard-{index}-manifest.json")
        )["tests"]
    }
    census["tests"] = [test for test in census["tests"] if test["test_id"] in selected]
    lanes = []
    for index in range(1, count + 1):
        identity = f"sqlite-{browser}-{index}-of-{count}"
        lane = lane_evidence(identity, root / f"e2e-{identity}")
        lane.census.parent.mkdir(parents=True)
        lane.census.write_text(json.dumps(census))
        lane.report.write_text(fixture("e2e-ownership", f"shard-{index}-report.json"))
        lane.lane_manifest.write_text(
            fixture("e2e-ownership", f"shard-{index}-manifest.json")
        )
        lane.duration_manifest.write_text(
            fixture("e2e-evidence", f"shard-{index}-duration.json")
        )
        write_lane_capture(
            lane.capture, fixture("e2e-evidence", f"shard-{index}-trace.jsonl")
        )
        lane.phase.write_text(fixture("e2e-evidence", f"shard-{index}-phase.json"))
        lanes.append(lane)
    return lanes


def nests(events: list[dict[str, Any]]) -> bool:
    """Whether every pair of merged slices sharing a Perfetto track is nested
    or disjoint, as the track layout promises."""
    for track in {event["tid"] for event in events}:
        on = [event for event in events if event["tid"] == track]
        for a in on:
            for b in on:
                a_end, b_end = a["ts"] + a["dur"], b["ts"] + b["dur"]
                if a["ts"] < b["ts"] < a_end < b_end:
                    return False
    return True


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


def cells(out: str) -> list[tuple[str, ...]]:
    """Every row of the box-drawn tables in `out`, headings included, as its
    stripped cells."""
    return [
        tuple(cell.strip() for cell in line.strip("│").split("│"))
        for line in out.splitlines()
        if line.startswith("│")
    ]


def load_workflow(name: str) -> Any:
    """A repository GitHub workflow, parsed (see `require`). PyYAML is YAML
    1.1, so the bare `on` key loads as `True`."""
    path = REPOSITORY / ".github" / "workflows" / name
    require(path.is_file(), f"the repository's .github/workflows/{name}")
    return yaml.safe_load(path.read_text(encoding="utf-8"))


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


# The flake packages that build host tools, with the binaries each holds. A
# fake's copy of `nix/packages.nix` and `nix/repository-policy.nix`: it cannot
# catch a binary those packages lack, which building each package proves.
TOOL_PACKAGES = {
    "host-tool": ("admit-inputs",),
    "repository-policy": (
        "source-contracts",
        "repository-docs",
        "scoped-leptosfmt",
    ),
    "coverage-check": ("coverage-check",),
    "system-artifacts": ("system-artifacts",),
}


# A fake for every tool a workflow runs. Each appends its argv to $FAKE_LOG
# (unit and record separators keep empty and whitespace-bearing arguments
# exact) and exits 17 when its command line equals $FAIL_COMMAND; otherwise a
# host tool that reports prints a minimal report with no steps. Fake Git
# answers index custody's queries (the only Git that honours a hook's
# overrides) for a clean index, refuses inherited repository overrides
# anywhere else with 23, answers checkout discovery from the real Git, and
# reports a dirty tree while `dirty-status` exists.
# An evaluation prints a fake derivation named after the installable's last
# component, and a check's build prints `$FAKE_STORE/fake-<check>`. A store
# query reports a path built here (`ultimate`) with no registration time, so Nix
# steps record no cache source; FAKE_SUBSTITUTED and FAKE_UNREADABLE name the
# checks whose outputs it reports substituted, or fails to read. A build's
# out-link points at a path named after the link in $FAKE_STORE, where
# `fake-csr-bundle` is a small valid CSR bundle (`write_bundle`). A tool
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
  'nix path-info '*)
    for path; do :; done
    case " ${FAKE_UNREADABLE-} " in *" ${path##*/fake-} "*) exit 1 ;; esac
    ultimate=true
    case " ${FAKE_SUBSTITUTED-} " in *" ${path##*/fake-} "*) ultimate=false ;; esac
    printf '{"%s":{"ultimate":%s}}\n' "$path" "$ultimate"
    exit 0 ;;
esac
if [ "$name" = cargo ]; then
    printf '%s\n' "${RUSTC_WRAPPER-unset}" "${CARGO_INCREMENTAL-unset}" "${KACHE_CACHE_EXECUTABLES-unset}" > cache-env
    printf '%s\n' "${CARGO_HOME-unset}" "${CARGO_NET_OFFLINE-unset}" > cargo-env
fi
if [ "$name $1" = "nix build" ]; then
    previous=
    for argument in "$@"; do
        if [ "$previous" = --out-link ]; then
            ln -sfn "$FAKE_STORE/fake-${argument##*/}" "$argument"
        fi
        previous=$argument
    done
fi
printf '%s\037' "$name" "$@" >> "$FAKE_LOG"
printf '\036' >> "$FAKE_LOG"
if [ "$name" = nix ] && [ -n "${UPGRADE_OUTPUT-}" ]; then
    case "$*" in *').outPath') printf '%s\n' "$UPGRADE_OUTPUT" ;; esac
fi
if [ "$name" = gh ] && [ -n "${GH_LAST_GREEN-}" ]; then
    printf '%s\n' "$GH_LAST_GREEN"
fi
if [ "$name" = git ] && [ -f dirty-status ]; then
    printf ' M tracked.rs\n'
fi
if [ "$name" = git ]; then
    case "$*" in
      'rev-parse --verify --end-of-options earlier^{commit}') printf '%s\n' "$EARLIER" ;;
      'rev-parse --verify --end-of-options '*'^{commit}')
        revision=${4%^\{commit\}}; printf '%s\n' "$revision" ;;
      'tag --list v* --sort=-version:refname --merged v2^') printf 'v1\n' ;;
      'branch --remotes --list origin/* --contains '*)
        [ -f not-on-origin ] || printf '  origin/main\n' ;;
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
  'nix build '*'.#checks.'*)
    for installable; do :; done
    printf '%s/fake-%s\n' "$FAKE_STORE" "${installable##*.}" ;;
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
  'admit-inputs '*|'coverage-check '*)
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
# is that output, the derivations and CLOSURE's dependencies. A build writes
# NIX_BUILD_STATUS, if set, as the output's `status.json`. The other
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
    status = env("NIX_BUILD_STATUS")
    if status is not None:
        # A producer's own timings, from inside this build: "@T0_MS@" is a
        # millisecond after it started, and the build lasts 20 ms more.
        import time
        started_ms = time.time_ns() // 1_000_000 + 1
        time.sleep(0.02)
        pathlib.Path("build-out", "status.json").write_text(
            status.replace("@T0_MS@", str(started_ms))
        )
    print("build-out")
    if env("NIX_TWO_OUTPUTS"):
        print("build-out-dev")
    print("build-err", file=sys.stderr)
    # A comma-separated status per build attempt; the last one repeats.
    attempt = len(pathlib.Path("attempts").read_text()) if os.path.exists("attempts") else 0
    with open("attempts", "a") as attempts:
        attempts.write(".")
    codes = env("NIX_BUILD_CODE", "0").split(",")
    code = int(codes[min(attempt, len(codes) - 1)])
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
    "gh",
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


def leb128(value: int) -> bytes:
    encoded = bytearray()
    while True:
        byte, value = value & 0x7F, value >> 7
        encoded.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(encoded)


def _name(text: str) -> bytes:
    return leb128(len(text.encode())) + text.encode()


def section(identity: int, payload: bytes) -> bytes:
    return bytes([identity]) + leb128(len(payload)) + payload


def module(names: list[str] | None = None, functions: int = 1) -> bytes:
    """A valid wasm module of `functions` functions returning 7, each named
    from `names` in a name section when given."""
    body = b"\x00\x41\x07\x0b"  # no locals; i32.const 7; end
    sections = [
        section(1, leb128(1) + b"\x60\x00\x01\x7f"),  # one type: () -> i32
        section(3, leb128(functions) + b"\x00" * functions),
        section(10, leb128(functions) + (leb128(len(body)) + body) * functions),
    ]
    if names is not None:
        entries = b"".join(leb128(i) + _name(name) for i, name in enumerate(names))
        functions_map = leb128(len(names)) + entries
        sections.append(
            section(
                0, _name("name") + b"\x01" + leb128(len(functions_map)) + functions_map
            )
        )
    return b"\0asm\x01\0\0\0" + b"".join(sections)


def write_bundle(root: Path, wasm: bytes) -> dict[str, Any]:
    """A CSR bundle at `root` as Vite emits it: the entry JavaScript and the
    wasm, each with `.gz` and `.br` copies, and the manifest naming them."""
    files = {"assets/index-a1b2.js": b"export {}", "assets/csr_bg-e5f6.wasm": wasm}
    for name, content in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        path.with_name(path.name + ".gz").write_bytes(b"g" * 3)
        path.with_name(path.name + ".br").write_bytes(b"b" * 2)
    manifest = {
        "index.html": {
            "file": "assets/index-a1b2.js",
            "src": "index.html",
            "isEntry": True,
            "assets": ["assets/csr_bg-e5f6.wasm"],
        },
        "csr_bg.wasm": {"file": "assets/csr_bg-e5f6.wasm", "src": "csr_bg.wasm"},
    }
    (root / ".vite").mkdir(parents=True, exist_ok=True)
    (root / ".vite/manifest.json").write_text(json.dumps(manifest))
    return manifest


@dataclass
class Workflow:
    """A checkout whose every workflow tool is a recording fake."""

    checkout: Checkout

    @property
    def root(self) -> Path:
        return self.checkout.root

    @property
    def store(self) -> Path:
        return self.root.parent / "store"

    def dev(
        self, *args: str, failure: str = "", cwd: Path | None = None, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        log = self.root / "commands"
        log.write_bytes(b"")
        environment = {
            "FAKE_LOG": str(log),
            "FAKE_STORE": str(self.store),
            "FAIL_COMMAND": failure,
            "REAL_GIT": REAL_GIT,
            "EARLIER": EARLIER,
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
    workflow = Workflow(checkout)
    write_bundle(workflow.store / "fake-csr-bundle", module())
    return workflow


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
