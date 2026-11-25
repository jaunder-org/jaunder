"""`dev perf`: Nix selects the producers; each is built, retained and checked in
order before the next, and a failure keeps every finished producer's output.
`dev perf-compare` is advisory; `dev perf-import-baseline` admits only the
trusted CI run.
"""

import copy
import errno
import json
import os
import shutil
import subprocess
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from conftest import REAL_GIT, REGISTERED_LATER, Checkout

COMMIT = "c" * 40
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
# Opaque to the host, which compares it whole and reads only the profile.
MANIFEST = {
    "schema_version": 1,
    "plan": {"profile": "small", "posts": 100},
    "subjects": {"username": "author"},
    "cursors": [],
}
# The selector, the store and the build of each producer; MODE injects one
# failure. A producer's output is valid once it has been built, built here
# and registered after the build step started.
NIX = (
    f"REGISTERED = {REGISTERED_LATER}\n"
    + """\
log = pathlib.Path(os.environ["PERF_LOG"])
if sys.argv[1] == "path-info":
    path = sys.argv[-1]
    built = log.exists() and f"build:{pathlib.Path(path).name}" in log.read_text()
    entry = {"ultimate": True, "registrationTime": REGISTERED}
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
)
# Git as the checkout has it, except that the uncommitted fixture has a HEAD.
GIT = f"""\
if sys.argv[1:] == ["rev-parse", "HEAD"]:
    print("{COMMIT}")
    sys.exit(0)
os.execv({REAL_GIT!r}, ["git", *sys.argv[1:]])
"""


def key(workload: str, backend: str, browser: str | None) -> dict[str, Any]:
    return {"workload": workload, "backend": backend, "browser": browser}


def summary(median: int, p95: int, minimum: int = 1, maximum: int = 9) -> Any:
    return {
        "sample_count": 1,
        "minimum_us": minimum,
        "maximum_us": maximum,
        "mean_us": median,
        "median_us": median,
        "p95_us": p95,
    }


def fragment_name(producer: str, backend: str, browser: str | None) -> str:
    if browser is None:
        return f"{producer}-{backend}-v1.json"
    return f"{producer}-{backend}-{browser}-v1.json"


def fragment(producer: str, backend: str, browser: str | None) -> dict[str, Any]:
    workload = "home" if producer == "browser" else "public_timeline"
    result: dict[str, Any] = {
        "setup": {
            "producer": producer,
            "backend": backend,
            "provisioning_us": 1,
            "seeding_us": 2,
        },
        "workloads": [
            {
                "key": key(workload, backend, browser),
                "samples": [{"duration_us": 5}],
                "summary": summary(5, 5),
                "rows_returned": 1,
            }
        ],
    }
    if producer == "browser":
        result["diagnostics"] = {"otel_trace_artifact": "diagnostics/otel.jsonl"}
    return {
        "schema_version": 1,
        "manifest": MANIFEST,
        "fragment": {"producer": producer, "result": result},
    }


@dataclass
class Perf:
    checkout: Checkout
    outputs: Path

    @property
    def run_root(self) -> Path:
        (root,) = [
            path
            for path in (self.checkout.root / ".xtask" / "performance").iterdir()
            if path.is_dir()
        ]
        return root

    def run(
        self, mode: str, *args: str, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        environment = self.checkout.environment(
            PERF_LOG=str(self.checkout.root.parent / "calls.log"),
            PERF_OUTPUTS=str(self.outputs),
            PERF_SELECTION=str(self.checkout.root.parent / "selection.json"),
            PERF_PARAMETERS=str(self.checkout.root.parent / "parameters.json"),
            PERF_MODE=mode,
        )
        # CI sets these for the tests themselves; a test opts in to them.
        for name in [name for name in environment if name.startswith("GITHUB_")]:
            del environment[name]
        environment.update(extra)
        result = self.checkout.run(self.checkout.command("perf", *args), environment)
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

    def fragment(self, name: str) -> Path:
        (_, producer, backend, browser) = next(
            producer for producer in PRODUCERS if producer[0] == name
        )
        return (
            self.source(name) / "fragments" / fragment_name(producer, backend, browser)
        )

    def status(self, name: str, status: dict[str, Any]) -> None:
        (self.source(name) / "producer-status-v1.json").write_text(json.dumps(status))

    def change(self, name: str, edit: Any) -> None:
        envelope = json.loads(self.fragment(name).read_text())
        edit(envelope)
        self.fragment(name).write_text(json.dumps(envelope))

    def result(self) -> dict[str, Any]:
        result: dict[str, Any] = json.loads(
            (self.run_root / "performance-result-v1.json").read_text()
        )
        return result


def fixture(checkout: Checkout, selection: dict[str, Any] = SELECTION) -> Perf:
    outputs = checkout.root.parent / "outputs"
    (checkout.root.parent / "selection.json").write_text(
        json.dumps(
            {
                "profile": "small",
                "selection": selection,
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
    perf = Perf(checkout, outputs)
    for name, producer, backend, browser in PRODUCERS:
        perf.fragment(name).parent.mkdir(parents=True)
        perf.status(name, {"schema_version": 1, "ok": True, "detail": "success"})
        perf.fragment(name).write_text(json.dumps(fragment(producer, backend, browser)))
    checkout.tool("nix", "import json\n" + NIX)
    # The closure query fails: these producers' fetched bytes are not tested here.
    checkout.tool("nix-store", "sys.exit(1)")
    checkout.tool("git", GIT)
    return perf


@pytest.fixture
def perf(checkout: Checkout) -> Iterator[Perf]:
    perf = fixture(checkout)
    yield perf
    # Tests leave producer outputs read-only, as a store would; whatever the
    # outcome, make their directories removable again.
    for path in [perf.outputs, *perf.outputs.rglob("*")]:
        if path.is_dir() and not path.is_symlink():
            path.chmod(0o755)


def test_defaults_forward_the_medium_profile_and_a_fresh_nonce(perf: Perf) -> None:
    assert perf.run("build-failure").returncode == 73
    parameters = perf.parameters()
    nonce = parameters.pop("freshnessNonce")
    # The nonce names the run's evidence and is fresh each time.
    assert len(nonce) == 64 and int(nonce, 16) >= 0
    assert perf.run_root.name == nonce
    assert parameters == {
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
    parameters = perf.parameters()
    del parameters["freshnessNonce"]
    assert parameters == {
        "profile": "small",
        "backends": "sqlite,postgres",
        "browsers": "",
        "workload": "storage",
        "posts": "120",
        "authors": "12",
        "revisions": "777",
    }


def test_a_later_build_failure_keeps_the_finished_producer_and_stops(
    perf: Perf,
) -> None:
    assert perf.run("build-failure", "small").returncode == 73
    assert (perf.retained(PRODUCERS[0][0]) / "producer-status-v1.json").is_file()
    assert not perf.retained(PRODUCERS[1][0]).exists()
    assert perf.calls() == [
        "build:performance-storage-sqlite",
        "build:performance-browser-sqlite-chromium",
    ]


def unsupported_status(perf: Perf, name: str) -> None:
    perf.status(name, {"schema_version": 2, "ok": True, "detail": "success"})


def failed_status(perf: Perf, name: str) -> None:
    perf.status(name, {"schema_version": 1, "ok": False, "detail": "injected"})


def missing_fragment(perf: Perf, name: str) -> None:
    perf.fragment(name).unlink()


def wrong_backend(perf: Perf, name: str) -> None:
    def edit(envelope: dict[str, Any]) -> None:
        envelope["fragment"]["result"]["setup"]["backend"] = "postgres"

    perf.change(name, edit)


def wrong_browser(perf: Perf, name: str) -> None:
    def edit(envelope: dict[str, Any]) -> None:
        envelope["fragment"]["result"]["workloads"][0]["key"]["browser"] = "firefox"

    perf.change(name, edit)


def wrong_producer(perf: Perf, name: str) -> None:
    def edit(envelope: dict[str, Any]) -> None:
        envelope["fragment"]["producer"] = "storage"

    perf.change(name, edit)


@pytest.mark.parametrize(
    ("injure", "message"),
    [
        (failed_status, "producer reported failure: injected"),
        (unsupported_status, "producer status schema is unsupported"),
        (missing_fragment, "producer fragment is missing"),
        (wrong_backend, "producer fragment does not fill its selected slot"),
        (wrong_browser, "producer fragment does not fill its selected slot"),
        (wrong_producer, "producer fragment does not fill its selected slot"),
    ],
)
def test_a_producer_that_cannot_fill_its_slot_keeps_evidence_and_stops(
    perf: Perf, injure: Any, message: str
) -> None:
    name = PRODUCERS[1][0]
    injure(perf, name)
    result = perf.run("success", "small")
    assert result.returncode == 1
    assert f"{name}: {message}".encode() in result.stderr
    for retained in PRODUCERS[0][0], name:
        assert (perf.retained(retained) / "producer-status-v1.json").is_file()
    assert not perf.retained(PRODUCERS[2][0]).exists()
    assert perf.calls() == [
        "build:performance-storage-sqlite",
        "build:performance-browser-sqlite-chromium",
    ]
    assert not (perf.run_root / "performance-result-v1.json").exists()
    check = perf.checkout.merged()[f"{name}-check"]["args"]
    assert (check["jaunder.status"], check["process.exit.code"]) == ("failed", 1)


def test_each_producer_build_is_a_step_with_its_cache_state(perf: Perf) -> None:
    assert perf.run("success", "small").returncode == 0
    events = perf.checkout.events()
    order = perf.checkout.order().splitlines()
    for name, *_ in PRODUCERS:
        (step,) = [event["args"] for event in events if event["name"] == name]
        assert (step["jaunder.status"], step["jaunder.cache"]) == ("ok", "built")
        # A selected derivation needs no evaluation: the step's one child is
        # its build.
        (child,) = [
            event["args"]
            for event in events
            if event["args"].get("parent_id") == step["span_id"]
        ]
        assert child["process.command_line"] == (
            f"nix build -L --print-out-paths --no-link '/fake/{name}.drv^out'"
        )
        # What the build fetched is the derivation's closure, not its output's.
        closure = f"nix-store --query --requisites --include-outputs /fake/{name}.drv"
        assert closure in order


def test_the_retained_fragments_assemble_into_one_published_result(
    perf: Perf,
) -> None:
    result = perf.run("success", "small")
    assert result.returncode == 0, result.stderr
    document = perf.result()
    assert list(document) == [
        "schema_version",
        "manifest",
        "selection",
        "canonical_selection",
        "provenance",
        "evidence",
        "setup",
        "workloads",
    ]
    assert document["schema_version"] == 1
    assert document["manifest"] == MANIFEST
    assert document["selection"] == SELECTION
    assert document["canonical_selection"] is True
    assert document["provenance"] == {
        "source": "local",
        "details": {"git_commit": COMMIT},
    }
    evidence = document["evidence"]
    assert list(evidence) == [
        "measured_at_unix_ms",
        "freshness_nonce",
        "producer_derivation_identities",
    ]
    assert evidence["measured_at_unix_ms"] > 0
    assert evidence["freshness_nonce"] == perf.run_root.name
    assert evidence["producer_derivation_identities"] == [
        {"name": name, "identity": str(perf.outputs / name)}
        for name in sorted(name for name, *_ in PRODUCERS)
    ]
    # Setup and workloads follow the selection's producer order.
    assert document["setup"] == [
        fragment(producer, backend, browser)["fragment"]["result"]["setup"]
        for _, producer, backend, browser in PRODUCERS
    ]
    assert [item["key"] for item in document["workloads"]] == [
        key("public_timeline", "sqlite", None),
        key("home", "sqlite", "chromium"),
        key("public_timeline", "postgres", None),
        key("home", "postgres", "chromium"),
    ]
    stable = (
        perf.checkout.root / ".xtask" / "performance" / "performance-result-v1.json"
    )
    assert json.loads(stable.read_text()) == document
    location = (
        perf.run_root.relative_to(perf.checkout.root) / "performance-result-v1.json"
    )
    assert (
        f"assembled 4 workload(s) at {location} (canonical selection)".encode()
        in result.stdout
    )


@pytest.mark.parametrize(
    "change",
    [
        {"count_overrides": {"posts": 120, "authors": None, "revisions": None}},
        {"browsers": ["firefox"]},
        {"backends": ["sqlite"]},
        {"storage": False},
        {"browser": False, "browsers": []},
    ],
)
def test_a_nondefault_selection_is_assembled_as_noncanonical(
    checkout: Checkout, change: dict[str, Any]
) -> None:
    perf = fixture(checkout, {**SELECTION, **change})
    result = perf.run("success", "small")
    assert result.returncode == 0, result.stderr
    assert perf.result()["canonical_selection"] is False
    assert b"(noncanonical selection)" in result.stdout


def test_fragments_of_different_datasets_are_not_assembled(perf: Perf) -> None:
    def edit(envelope: dict[str, Any]) -> None:
        envelope["manifest"]["subjects"]["username"] = "someone-else"

    perf.change(PRODUCERS[3][0], edit)
    result = perf.run("success", "small")
    assert result.returncode == 1
    assert b"producer fragments describe different datasets" in result.stderr
    assert not (perf.run_root / "performance-result-v1.json").exists()


GITHUB = {
    "GITHUB_REPOSITORY": "jaunder-org/jaunder",
    "GITHUB_WORKFLOW": "Performance",
    "GITHUB_JOB": "performance",
    "GITHUB_REF": "refs/heads/main",
    "GITHUB_SHA": "d" * 40,
    "GITHUB_RUN_ID": "42",
    "GITHUB_RUN_ATTEMPT": "2",
}


def test_a_ci_run_records_its_github_provenance(perf: Perf) -> None:
    assert perf.run("success", "small", **GITHUB).returncode == 0
    assert perf.result()["provenance"] == {
        "source": "github",
        "details": {
            "repository": "jaunder-org/jaunder",
            "workflow": "Performance",
            "job": "performance",
            "reference": "refs/heads/main",
            "head_sha": "d" * 40,
            "run_id": 42,
            "attempt": 2,
        },
    }


def test_incomplete_github_provenance_never_falls_back_to_local(perf: Perf) -> None:
    result = perf.run("success", "small", **{**GITHUB, "GITHUB_JOB": ""})
    assert result.returncode == 1
    assert b"CI performance provenance requires GITHUB_JOB" in result.stderr
    assert not (perf.run_root / "performance-result-v1.json").exists()


def test_a_failed_publication_never_replaces_an_earlier_result(perf: Perf) -> None:
    performance = perf.checkout.root / ".xtask" / "performance"
    stable = performance / "performance-result-v1.json"
    stable.parent.mkdir(parents=True)
    stable.write_text("earlier result")
    # The run's own result is published first: a directory where it belongs
    # makes that publication fail. A pinned nonce puts the directory there.
    (performance / "pinned" / "performance-result-v1.json").mkdir(parents=True)
    hook = perf.checkout.root.parent / "hook"
    hook.mkdir()
    (hook / "sitecustomize.py").write_text(
        "import secrets\nsecrets.token_hex = lambda _size: 'pinned'\n"
    )
    result = perf.run("success", "small", PYTHONPATH=str(hook))
    assert result.returncode == 1
    assert b"IsADirectoryError" in result.stderr
    assert stable.read_text() == "earlier result"
    # No staged partial file is left for the next run.
    assert sorted(path.name for path in perf.run_root.iterdir()) == [
        "performance-result-v1.json",
        "producers",
        "selection.json",
    ]


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
    fragment_path = perf.fragment(PRODUCERS[0][0]).relative_to(source)
    files = [
        (str(fragment_path), 0o444, perf.fragment(PRODUCERS[0][0]).read_bytes()),
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
    fragment_path = perf.fragment(PRODUCERS[0][0]).relative_to(source)
    data = perf.fragment(PRODUCERS[0][0]).read_bytes()
    referent = source.parent / "store-like"
    source.rename(referent)
    os.chmod(referent / fragment_path, 0o444)
    os.chmod(referent, 0o555)
    os.symlink(referent, source)

    # The symlinked evidence is still read through: the first producer
    # passes its check and the next one is built.
    assert perf.run("build-failure", "small").returncode == 73
    assert perf.calls()[1] == "build:performance-browser-sqlite-chromium"
    destination = perf.retained(PRODUCERS[0][0])
    assert destination.is_symlink()
    assert Path(os.readlink(destination)) == referent
    assert mode_of(referent) == 0o555
    assert mode_of(referent / fragment_path) == 0o444
    assert (referent / fragment_path).read_bytes() == data


# --- perf-compare ---------------------------------------------------------


def run_document(
    workloads: list[tuple[dict[str, Any], dict[str, Any]]],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "manifest": MANIFEST,
        "selection": SELECTION,
        "canonical_selection": True,
        "provenance": {"source": "local", "details": {"git_commit": COMMIT}},
        "evidence": {},
        "setup": [],
        "workloads": [
            {"key": workload_key, "summary": workload_summary}
            for workload_key, workload_summary in workloads
        ],
    }


STORAGE = key("public_timeline", "sqlite", None)
BROWSER = key("home", "sqlite", "chromium")
DETAIL = key("revision_detail", "postgres", None)
ZERO = key("owner_history", "postgres", None)


@dataclass
class Compare:
    checkout: Checkout
    baseline: dict[str, Any]
    candidate: dict[str, Any]

    def run(self) -> subprocess.CompletedProcess[bytes]:
        path = self.checkout.root / "tools" / "performance" / "baseline-v1.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(self.baseline))
        (self.checkout.root / "candidate.json").write_text(json.dumps(self.candidate))
        return self.checkout.dev("perf-compare", "candidate.json")


@pytest.fixture
def comparison(checkout: Checkout) -> Compare:
    baseline = run_document(
        [
            (STORAGE, summary(100, 100)),
            (BROWSER, summary(100, 100)),
            (DETAIL, summary(0, 0)),
            (ZERO, summary(0, 0)),
        ]
    )
    candidate = run_document(
        [
            (BROWSER, summary(120, 119, 3, 300)),
            (STORAGE, summary(119, 121)),
            (DETAIL, summary(0, 1)),
            (ZERO, summary(0, 0)),
        ]
    )
    return Compare(checkout, {"schema_version": 1, "run": baseline}, candidate)


def test_comparison_reports_timing_and_advisory_regressions_without_failing(
    comparison: Compare,
) -> None:
    result = comparison.run()
    assert result.returncode == 0, result.stderr
    assert result.stdout.decode().splitlines() == [
        "advisory only",
        (
            "home/sqlite/chromium: baseline median=100us p95=100us spread=1..9us; "
            "candidate median=120us p95=119us spread=3..300us; "
            "advisory median=at-least-20-percent p95=none"
        ),
        (
            "public_timeline/sqlite/-: baseline median=100us p95=100us spread=1..9us; "
            "candidate median=119us p95=121us spread=1..9us; "
            "advisory median=none p95=at-least-20-percent"
        ),
        (
            "revision_detail/postgres/-: baseline median=0us p95=0us spread=1..9us; "
            "candidate median=0us p95=1us spread=1..9us; "
            "advisory median=none p95=zero-baseline-increase"
        ),
        (
            "owner_history/postgres/-: baseline median=0us p95=0us spread=1..9us; "
            "candidate median=0us p95=0us spread=1..9us; "
            "advisory median=none p95=none"
        ),
    ]


def unsupported_candidate(comparison: Compare) -> None:
    comparison.candidate["schema_version"] = 2


def unsupported_baseline(comparison: Compare) -> None:
    comparison.baseline["schema_version"] = 2


def different_plan(comparison: Compare) -> None:
    comparison.candidate["manifest"] = {**MANIFEST, "plan": {"profile": "medium"}}


def different_keys(comparison: Compare) -> None:
    comparison.candidate["workloads"][0]["key"] = key("app", "sqlite", "chromium")


@pytest.mark.parametrize(
    ("change", "message"),
    [
        (unsupported_candidate, "candidate.json: performance schema is unsupported"),
        (unsupported_baseline, "performance schema is unsupported"),
        (different_plan, "candidate dataset plan differs from baseline"),
        (different_keys, "candidate compatibility keys differ from baseline"),
    ],
)
def test_incomparable_results_are_refused(
    comparison: Compare, change: Any, message: str
) -> None:
    change(comparison)
    result = comparison.run()
    assert result.returncode == 1
    assert message.encode() in result.stderr
    assert b"advisory only" not in result.stdout


# --- perf-import-baseline -------------------------------------------------

RUN_ID = 4242
HEAD = "e" * 40
# Answers `gh api` from GH_RESPONSES, keyed by the path's last component, and
# `gh run download` by copying GH_ARTIFACT, if it exists, into --dir.
GH = """\
import json, shutil
if sys.argv[1] == "api":
    responses = json.loads(pathlib.Path(os.environ["GH_RESPONSES"]).read_text())
    print(json.dumps(responses[sys.argv[2].rsplit("/", 1)[-1]]))
    sys.exit(0)
assert sys.argv[1:3] == ["run", "download"], sys.argv
artifact = pathlib.Path(os.environ["GH_ARTIFACT"])
if artifact.exists():
    shutil.copy(artifact, pathlib.Path(sys.argv[sys.argv.index("--dir") + 1], artifact.name))
"""


def trusted_run() -> dict[str, Any]:
    return {
        "id": RUN_ID,
        "repository": {"full_name": "jaunder-org/jaunder"},
        "head_repository": {"full_name": "jaunder-org/jaunder"},
        "path": ".github/workflows/performance.yml",
        "name": "Performance",
        "event": "schedule",
        "status": "completed",
        "conclusion": "success",
        "head_branch": "main",
        "head_sha": HEAD,
        "run_attempt": 3,
    }


def trusted_jobs() -> dict[str, Any]:
    return {
        "jobs": [
            {"name": "lint", "status": "completed", "conclusion": "failure"},
            {
                "name": "performance",
                "status": "completed",
                "conclusion": "success",
                "head_sha": HEAD,
                "run_attempt": 3,
            },
        ]
    }


def trusted_artifacts() -> dict[str, Any]:
    return {
        "artifacts": [
            {
                "name": "performance-result",
                "expired": False,
                "id": 7,
                "size_in_bytes": 1024,
                "workflow_run": {"id": RUN_ID, "head_branch": "main", "head_sha": HEAD},
            }
        ]
    }


def trusted_result() -> dict[str, Any]:
    result = run_document([(STORAGE, summary(100, 100))])
    result["manifest"] = {**MANIFEST, "plan": {"profile": "medium"}}
    result["provenance"] = {
        "source": "github",
        "details": {
            "repository": "jaunder-org/jaunder",
            "workflow": "Performance",
            "job": "performance",
            "reference": "refs/heads/main",
            "head_sha": HEAD,
            "run_id": RUN_ID,
            "attempt": 3,
        },
    }
    return result


@dataclass
class Import:
    checkout: Checkout
    responses: dict[str, Any]
    artifact: dict[str, Any] | None

    @property
    def baseline(self) -> Path:
        return self.checkout.root / "tools" / "performance" / "baseline-v1.json"

    def run(self, run_id: str = str(RUN_ID)) -> subprocess.CompletedProcess[bytes]:
        staging = self.checkout.root.parent / "artifact"
        staging.mkdir(exist_ok=True)
        responses = self.checkout.root.parent / "responses.json"
        responses.write_text(json.dumps(self.responses))
        artifact = staging / "performance-result-v1.json"
        if self.artifact is not None:
            artifact.write_text(json.dumps(self.artifact))
        return self.checkout.dev(
            "perf-import-baseline",
            run_id,
            GH_RESPONSES=str(responses),
            GH_ARTIFACT=str(artifact),
        )

    def downloaded(self) -> bool:
        return "gh run download" in self.checkout.order()


@pytest.fixture
def importer(checkout: Checkout) -> Import:
    checkout.tool("gh", GH)
    return Import(
        checkout,
        {
            str(RUN_ID): trusted_run(),
            "jobs": trusted_jobs(),
            "artifacts": trusted_artifacts(),
        },
        trusted_result(),
    )


def test_a_trusted_canonical_run_becomes_the_baseline(importer: Import) -> None:
    result = importer.run()
    assert result.returncode == 0, result.stderr
    assert f"imported canonical Actions run {RUN_ID}".encode() in result.stdout
    assert json.loads(importer.baseline.read_text()) == {
        "schema_version": 1,
        "run": trusted_result(),
    }
    # Admission comes before the download.
    order = importer.checkout.order()
    assert order.index(f"/actions/runs/{RUN_ID}/artifacts") < order.index(
        "gh run download"
    )


def test_a_compatible_run_replaces_an_existing_baseline(importer: Import) -> None:
    importer.baseline.parent.mkdir(parents=True)
    earlier = trusted_result()
    earlier["workloads"][0]["summary"] = summary(80, 80)
    importer.baseline.write_text(json.dumps({"schema_version": 1, "run": earlier}))
    assert importer.run().returncode == 0
    assert json.loads(importer.baseline.read_text())["run"] == trusted_result()


@pytest.mark.parametrize("run_id", ["0", "-3", "forty-two"])
def test_a_run_id_must_be_a_positive_integer(importer: Import, run_id: str) -> None:
    result = importer.run(run_id)
    assert result.returncode == 2
    assert "gh" not in importer.checkout.order()


def edit(target: str, path: tuple[str | int, ...], value: Any) -> Any:
    def apply(importer: Import) -> None:
        document: Any = (
            importer.artifact
            if target == "artifact"
            else importer.responses[str(RUN_ID) if target == "run" else target]
        )
        for part in path[:-1]:
            document = document[part]
        document[path[-1]] = value

    return apply


ADMISSION = [
    (edit("run", ("id",), 1), "Actions run ID mismatch"),
    (
        edit("run", ("repository", "full_name"), "fork/jaunder"),
        "Actions run belongs to another repository",
    ),
    (
        edit("run", ("head_repository", "full_name"), "fork/jaunder"),
        "Actions run belongs to another repository",
    ),
    (
        edit("run", ("path",), ".github/workflows/other.yml"),
        "did not use the approved performance workflow",
    ),
    (edit("run", ("name",), "Other"), "did not use the approved performance workflow"),
    (edit("run", ("event",), "pull_request"), "unapproved workflow events"),
    (edit("run", ("status",), "in_progress"), "requires a completed successful run"),
    (edit("run", ("conclusion",), "failure"), "requires a completed successful run"),
    (edit("run", ("head_branch",), "feature"), "baseline import requires main"),
    (edit("run", ("head_sha",), ""), "has no head SHA or attempt"),
    (edit("run", ("run_attempt",), 0), "has no head SHA or attempt"),
    (edit("jobs", ("jobs", 1, "name"), "other"), "job is absent or unsuccessful"),
    (edit("jobs", ("jobs", 1, "conclusion"), "failure"), "job is absent"),
    (edit("jobs", ("jobs", 1, "head_sha"), "f" * 40), "job is absent"),
    (edit("jobs", ("jobs", 1, "run_attempt"), 2), "job is absent"),
    (edit("artifacts", ("artifacts", 0, "expired"), True), "lacks the complete"),
    (edit("artifacts", ("artifacts", 0, "size_in_bytes"), 0), "lacks the complete"),
    (
        edit("artifacts", ("artifacts", 0, "workflow_run", "head_sha"), "f" * 40),
        "lacks the complete",
    ),
    (edit("artifacts", ("errors",), ["rate limited"]), "contains errors"),
]


@pytest.mark.parametrize(("change", "message"), ADMISSION)
def test_an_untrusted_run_is_refused_before_download(
    importer: Import, change: Any, message: str
) -> None:
    change(importer)
    result = importer.run()
    assert result.returncode == 1
    assert message.encode() in result.stderr
    assert not importer.downloaded()
    assert not importer.baseline.exists()


def local_provenance(importer: Import) -> None:
    assert importer.artifact is not None
    importer.artifact["provenance"] = {"source": "local", "details": {}}


def missing_artifact(importer: Import) -> None:
    importer.artifact = None


EVIDENCE = [
    (missing_artifact, "run artifact lacks performance-result-v1.json"),
    (edit("artifact", ("schema_version",), 2), "performance schema is unsupported"),
    (
        edit("artifact", ("manifest", "plan", "profile"), "small"),
        "canonical baseline requires the medium profile",
    ),
    (
        edit("artifact", ("canonical_selection",), False),
        "baseline import rejects noncanonical selection",
    ),
    (local_provenance, "artifact provenance does not match"),
    (
        edit("artifact", ("provenance", "details", "head_sha"), "f" * 40),
        "artifact provenance does not match",
    ),
    (
        edit("artifact", ("provenance", "details", "attempt"), 2),
        "artifact provenance does not match",
    ),
    (
        edit("artifact", ("provenance", "details", "run_id"), 1),
        "artifact provenance does not match",
    ),
]


@pytest.mark.parametrize(("change", "message"), EVIDENCE)
def test_an_inadmissible_artifact_never_becomes_the_baseline(
    importer: Import, change: Any, message: str
) -> None:
    change(importer)
    result = importer.run()
    assert result.returncode == 1
    assert message.encode() in result.stderr
    assert not importer.baseline.exists()


def test_an_incompatible_run_never_replaces_the_baseline(importer: Import) -> None:
    importer.baseline.parent.mkdir(parents=True)
    earlier = copy.deepcopy(trusted_result())
    earlier["workloads"][0]["key"] = BROWSER
    importer.baseline.write_text(json.dumps({"schema_version": 1, "run": earlier}))
    result = importer.run()
    assert result.returncode == 1
    assert b"compatibility keys differ from the committed baseline" in result.stderr
    assert json.loads(importer.baseline.read_text())["run"] == earlier
