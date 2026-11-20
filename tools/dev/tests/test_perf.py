"""`dev perf`: Nix selects the producers; each is built, retained and validated
in order before the next, and a failure keeps every finished producer's output.
"""

import errno
import json
import os
import shutil
import subprocess
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path

import pytest
from conftest import Checkout

NONCE = "0123456789abcdef" * 4
PRODUCERS = [
    ("performance-storage-sqlite", "storage", "sqlite", None),
    ("performance-browser-sqlite-chromium", "browser", "sqlite", "chromium"),
    ("performance-storage-postgres", "storage", "postgres", None),
    ("performance-browser-postgres-chromium", "browser", "postgres", "chromium"),
]
SELECTION = {
    "backends": ["sqlite", "postgres"],
    "browsers": ["chromium"],
    "storage": True,
    "browser": True,
    "count_overrides": {"posts": None, "authors": None, "revisions": None},
}
# The selector, the store and the build of each producer; MODE injects one
# failure. A producer's output is valid once it has been built, built here and
# registered far in the future, so after the build step started.
NIX = """\
log = pathlib.Path(os.environ["PERF_LOG"])
if sys.argv[1] == "path-info":
    path = sys.argv[-1]
    built = log.exists() and f"build:{pathlib.Path(path).name}" in log.read_text()
    entry = {"ultimate": True, "registrationTime": 4102444800}
    print(json.dumps({path: entry if built else None}))
    sys.exit(0)
if sys.argv[1] == "eval":
    # The checkout is a git+file: flake, never a path: copy of .git and target/.
    assert 'builtins.getFlake ("git+file://" + toString ./.)' in sys.argv[-1], sys.argv
    parameters = json.loads(os.environ["JAUNDER_PERF_PARAMETERS"])
    pathlib.Path(os.environ["PERF_PARAMETERS"]).write_text(json.dumps(parameters))
    selection = json.loads(pathlib.Path(os.environ["PERF_SELECTION"]).read_text())
    print(json.dumps({**selection, "profile": parameters["profile"]}))
    sys.exit(0)
assert sys.argv[1:5] == ["build", "-L", "--print-out-paths", "--no-link"], sys.argv
name = pathlib.Path(sys.argv[-1].removesuffix("^out")).stem
with log.open("a") as out:
    out.write(f"build:{name}\\n")
if os.environ["PERF_MODE"] == "build-failure" and name == "performance-browser-sqlite-chromium":
    sys.exit(73)
print(os.environ.get("PERF_IDENTITY", pathlib.Path(os.environ["PERF_OUTPUTS"], name)))
"""
CARGO = f"""\
log = pathlib.Path(os.environ["PERF_LOG"])
command = sys.argv[sys.argv.index("--") + 1]
if command == "nonce":
    print("{NONCE}")
elif command == "validate-selection":
    with log.open("a") as out:
        out.write("preflight\\n")
    sys.exit(os.environ["PERF_MODE"] == "preflight-failure")
elif command == "validate-producer":
    root = pathlib.Path(sys.argv[-1])
    with log.open("a") as out:
        out.write(f"validate:{{root.parent.name}}\\n")
    status = json.loads((root / "producer-status-v1.json").read_text())
    sys.exit(not status["ok"])
elif command == "assemble":
    with log.open("a") as out:
        out.write(" ".join(["assemble", *sys.argv[sys.argv.index("--") + 2:]]) + "\\n")
else:
    sys.exit(91)
"""


@dataclass
class Perf:
    checkout: Checkout
    outputs: Path

    @property
    def run_root(self) -> Path:
        return self.checkout.root / ".xtask" / "performance" / NONCE

    def run(
        self, mode: str, *args: str, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        result = self.checkout.run(
            self.checkout.command("perf", *args),
            self.checkout.environment(
                PERF_LOG=str(self.checkout.root.parent / "calls.log"),
                PERF_OUTPUTS=str(self.outputs),
                PERF_SELECTION=str(self.checkout.root.parent / "selection.json"),
                PERF_PARAMETERS=str(self.checkout.root.parent / "parameters.json"),
                PERF_MODE=mode,
                **extra,
            ),
        )
        assert self.run_root.is_dir(), result
        return result

    def parameters(self) -> dict[str, str]:
        parameters: dict[str, str] = json.loads(
            (self.checkout.root.parent / "parameters.json").read_text()
        )
        return parameters

    def calls(self) -> list[str]:
        return (self.checkout.root.parent / "calls.log").read_text().splitlines()

    def retained(self, name: str) -> Path:
        return self.run_root / "producers" / name / "performance"

    def source(self, name: str) -> Path:
        return self.outputs / name / "performance"


def fixture(checkout: Checkout, failed: str | None = None) -> Perf:
    outputs = checkout.root.parent / "outputs"
    (checkout.root.parent / "selection.json").write_text(
        json.dumps(
            {
                "profile": "small",
                "selection": SELECTION,
                "producers": [
                    {
                        "name": name,
                        "installable": f"/fake/{name}.drv^out",
                        "identity": str(outputs / name),
                        "producer": producer,
                        "backend": backend,
                        "browser": browser,
                    }
                    for name, producer, backend, browser in PRODUCERS
                ],
            }
        )
    )
    for name, *_ in PRODUCERS:
        root = outputs / name / "performance"
        (root / "fragments").mkdir(parents=True)
        (root / "producer-status-v1.json").write_text(
            json.dumps(
                {"schema_version": 1, "ok": failed != name, "detail": "injected"}
            )
        )
        (root / "fragments" / "placeholder.json").write_text("{}")
    checkout.tool("nix", "import json\n" + NIX)
    checkout.tool("cargo", "import json\n" + CARGO)
    return Perf(checkout, outputs)


@pytest.fixture
def perf(checkout: Checkout) -> Iterator[Perf]:
    perf = fixture(checkout)
    yield perf
    # Tests leave producer outputs read-only, as a store would; whatever the
    # outcome, make their directories removable again.
    for path in [perf.outputs, *perf.outputs.rglob("*")]:
        if path.is_dir() and not path.is_symlink():
            path.chmod(0o755)


def test_defaults_forward_the_medium_profile_and_unset_selectors(perf: Perf) -> None:
    assert perf.run("build-failure").returncode == 73
    assert perf.parameters() == {
        "freshnessNonce": NONCE,
        "profile": "medium",
        "backends": "sqlite,postgres",
        "browsers": "",
        "workload": "all",
        "posts": "",
        "authors": "",
        "revisions": "",
    }


def test_storage_parameters_and_count_overrides_reach_the_nix_selection(
    perf: Perf,
) -> None:
    args = ["small", "sqlite,postgres", "", "storage", "120", "12", "777"]
    assert perf.run("build-failure", *args).returncode == 73
    assert perf.parameters() == {
        "freshnessNonce": NONCE,
        "profile": "small",
        "backends": "sqlite,postgres",
        "browsers": "",
        "workload": "storage",
        "posts": "120",
        "authors": "12",
        "revisions": "777",
    }


def test_a_rejected_selection_stops_before_any_producer_build(perf: Perf) -> None:
    args = ["small", "sqlite", "", "storage", "100", "12", "777"]
    assert perf.run("preflight-failure", *args).returncode == 1
    assert perf.calls() == ["preflight"]
    assert not (perf.run_root / "producers" / PRODUCERS[0][0]).exists()


def test_a_later_build_failure_keeps_the_finished_producer_and_stops(
    perf: Perf,
) -> None:
    assert (
        perf.run("build-failure", "small", "sqlite,postgres", "", "all").returncode
        == 73
    )
    assert (perf.retained(PRODUCERS[0][0]) / "producer-status-v1.json").is_file()
    assert not perf.retained(PRODUCERS[1][0]).exists()
    assert perf.calls() == [
        "preflight",
        "build:performance-storage-sqlite",
        "validate:performance-storage-sqlite",
        "build:performance-browser-sqlite-chromium",
    ]


def test_a_later_producer_status_failure_keeps_evidence_and_stops(
    checkout: Checkout,
) -> None:
    perf = fixture(checkout, failed="performance-browser-sqlite-chromium")
    assert (
        perf.run("status-failure", "small", "sqlite,postgres", "", "all").returncode
        == 1
    )
    for name in PRODUCERS[0][0], PRODUCERS[1][0]:
        assert (perf.retained(name) / "producer-status-v1.json").is_file()
    assert not perf.retained(PRODUCERS[2][0]).exists()
    assert perf.calls() == [
        "preflight",
        "build:performance-storage-sqlite",
        "validate:performance-storage-sqlite",
        "build:performance-browser-sqlite-chromium",
        "validate:performance-browser-sqlite-chromium",
    ]


def test_each_producer_build_is_a_step_with_its_cache_state(perf: Perf) -> None:
    assert perf.run("success", "small").returncode == 0
    slices = perf.checkout.merged()
    for name, *_ in PRODUCERS:
        step = slices[name]["args"]
        assert step["process.command_line"] == (
            f"nix build -L --print-out-paths --no-link '/fake/{name}.drv^out'"
        )
        assert (step["jaunder.status"], step["jaunder.cache"]) == ("ok", "built")


def test_every_retained_producer_is_assembled_into_one_invocation(
    perf: Perf,
) -> None:
    assert perf.run("success", "small").returncode == 0
    invocation = perf.run_root / "invocation.json"
    assert (
        perf.calls()[-1]
        == f"assemble --invocation {invocation.relative_to(perf.checkout.root)}"
    )
    document = json.loads(invocation.read_text())
    assert (document["profile"], document["freshness_nonce"]) == ("small", NONCE)
    assert document["selection"] == SELECTION
    assert [producer["name"] for producer in document["producers"]] == sorted(
        name for name, *_ in PRODUCERS
    )
    assert document["producers"][0] == {
        "name": "performance-browser-postgres-chromium",
        "identity": str(perf.outputs / "performance-browser-postgres-chromium"),
        "producer": "browser",
        "backend": "postgres",
        "browser": "chromium",
        "retained_root": f".xtask/performance/{NONCE}/producers/"
        "performance-browser-postgres-chromium/performance",
    }
    assert document["producers"][2]["browser"] is None


@pytest.mark.parametrize(
    ("identity", "message"),
    [
        ("/elsewhere", "Nix output identity mismatches selection"),
        (None, "Nix output lacks performance evidence"),
    ],
)
def test_an_output_that_is_not_the_selected_evidence_is_refused(
    perf: Perf, identity: str | None, message: str
) -> None:
    extra = {} if identity is None else {"PERF_IDENTITY": identity}
    if identity is None:
        shutil.rmtree(perf.source(PRODUCERS[0][0]))
    result = perf.run("success", **extra)
    assert result.returncode == 1
    assert message.encode() in result.stderr
    assert not perf.retained(PRODUCERS[0][0]).exists()
    # The refusal belongs to the build step whose output it judged.
    step = perf.checkout.merged()[PRODUCERS[0][0]]["args"]
    assert (step["jaunder.status"], step["process.exit.code"]) == ("failed", 1)


def set_mode(path: Path, mode: int) -> int:
    """Sets `mode`, or its ordinary bits where the builder forbids set-ID modes."""
    ordinary = mode & ~0o6000
    os.chmod(path, ordinary)
    try:
        os.chmod(path, mode)
    except PermissionError as error:
        assert error.errno == errno.EPERM and mode != ordinary
        return ordinary
    return mode


def mode_of(path: Path) -> int:
    return path.stat().st_mode & 0o7777


def test_an_unreadable_selection_fails_the_gate_with_its_traceback(
    perf: Perf,
) -> None:
    (perf.checkout.root.parent / "selection.json").write_text("{}")
    result = perf.run("success")
    assert result.returncode == 1
    assert b"Traceback" in result.stderr
    assert b"KeyError: 'producers'" in result.stderr
    gate = perf.checkout.merged()["dev perf"]["args"]
    assert (gate["jaunder.status"], gate["process.exit.code"]) == ("failed", 1)


def test_retained_evidence_is_owner_writable_without_changing_sources_or_links(
    perf: Perf,
) -> None:
    source = perf.source(PRODUCERS[0][0])
    files = [
        ("fragments/placeholder.json", 0o444, b"{}"),
        ("executable", 0o4551, b"executable evidence"),
        ("hardlink-a", 0o440, b"linked evidence"),
    ]
    applied = []
    for name, mode, data in files:
        (source / name).write_bytes(data)
        applied.append((name, set_mode(source / name, mode), data))
    os.link(source / "hardlink-a", source / "hardlink-b")
    referent = perf.checkout.root.parent / "store-like"
    referent.mkdir()
    (referent / "evidence").write_bytes(b"immutable referent")
    os.chmod(referent / "evidence", 0o444)
    os.symlink(referent / "evidence", source / "file-link")
    os.symlink(referent, source / "directory-link")
    os.symlink("missing", source / "dangling-link")
    directories = [
        (name, set_mode(source / name, mode))
        for name, mode in [("", 0o1555), ("fragments", 0o2555)]
    ]

    assert perf.run("build-failure", "small").returncode == 73
    assert perf.calls()[2] == "validate:performance-storage-sqlite"
    destination = perf.retained(PRODUCERS[0][0])
    for name, mode, data in applied:
        assert mode_of(source / name) == mode
        assert (source / name).read_bytes() == data
        assert (destination / name).read_bytes() == data
        assert not (source / name).samefile(destination / name)
        assert mode_of(destination / name) == mode | 0o200
    for name, mode in directories:
        assert mode_of(source / name) == mode
        assert mode_of(destination / name) == mode | 0o200
        assert not (source / name).samefile(destination / name)
    assert (destination / "hardlink-a").samefile(destination / "hardlink-b")
    assert mode_of(source / "hardlink-b") == 0o440
    for name in ["file-link", "directory-link", "dangling-link"]:
        assert (destination / name).is_symlink()
        assert os.readlink(destination / name) == os.readlink(source / name)
    assert mode_of(referent / "evidence") == 0o444
    assert (referent / "evidence").read_bytes() == b"immutable referent"


def test_a_retained_starting_symlink_does_not_normalize_its_referent(
    perf: Perf,
) -> None:
    source = perf.source(PRODUCERS[0][0])
    referent = source.parent / "store-like"
    source.rename(referent)
    os.chmod(referent / "fragments" / "placeholder.json", 0o444)
    os.chmod(referent, 0o555)
    os.symlink(referent, source)

    assert perf.run("build-failure", "small").returncode == 73
    assert perf.calls()[2] == "validate:performance-storage-sqlite"
    destination = perf.retained(PRODUCERS[0][0])
    assert destination.is_symlink()
    assert Path(os.readlink(destination)) == referent
    assert mode_of(referent) == 0o555
    assert mode_of(referent / "fragments" / "placeholder.json") == 0o444
    assert (referent / "fragments" / "placeholder.json").read_bytes() == b"{}"
