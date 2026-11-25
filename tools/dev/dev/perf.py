"""Performance measurement: Nix selects the producers, and each is built,
retained and checked before the next can run.

A failure stops the run and keeps every finished producer's evidence.
Comparison with the committed baseline is advisory; only an explicit
authenticated import replaces that baseline.

Each producer's VM has already validated its manifest and fragment in Rust
(the seeder and `perf-producer`), so this host side reads that data rather
than validating it again. It checks only what no single producer can: that a
producer succeeded, that its fragment fills the slot Nix selected, and that
every fragment describes the same dataset. A result or baseline read here
may come from another commit, so its `schema_version` is still checked.
"""

import dataclasses
import json
import os
import secrets
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, NamedTuple, NoReturn

from dev import steps

SELECT_PRODUCERS = (
    'let flake = builtins.getFlake ("git+file://" + toString ./.); '
    'parameters = builtins.fromJSON (builtins.getEnv "JAUNDER_PERF_PARAMETERS"); '
    "in flake.performanceLib.${builtins.currentSystem}.selectPerformanceProducers "
    "parameters"
)
SCHEMA_VERSION = 1
RESULT = "performance-result-v1.json"
PERFORMANCE_ROOT = Path(".xtask/performance")
BASELINE = Path("tools/performance/baseline-v1.json")
# The only run whose artifact may become the baseline.
TRUSTED_REPOSITORY = "jaunder-org/jaunder"
TRUSTED_WORKFLOW = "Performance"
TRUSTED_WORKFLOW_PATH = ".github/workflows/performance.yml"
TRUSTED_JOB = "performance"
ARTIFACT = "performance-result"


@dataclasses.dataclass(frozen=True)
class Request:
    """The producers to measure, as Nix's selector reads them; empty is unset."""

    profile: str
    backends: str
    browsers: str
    workload: str
    posts: str
    authors: str
    revisions: str


class Retained(NamedTuple):
    """A built producer: its name, its Nix output and its checked fragment."""

    name: str
    identity: str
    envelope: dict[str, Any]


def _refuse(message: str) -> NoReturn:
    print(message, file=sys.stderr)
    raise steps.StepFailed(1, signalled=False)


def _compact(value: object) -> str:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def _read(path: Path) -> Any:
    return json.loads(path.read_bytes())


def _publish(path: Path, value: object) -> None:
    """Replaces `path` only with complete JSON; a failure keeps the old bytes."""
    data = (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, staged = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    try:
        with os.fdopen(descriptor, "wb") as file:
            file.write(data)
            file.flush()
            os.fsync(file.fileno())
        os.replace(staged, path)
    except BaseException:
        os.unlink(staged)
        raise


def _fragment_name(selected: dict[str, Any]) -> str:
    if selected["producer"] == "storage":
        return f"storage-{selected['backend']}-v1.json"
    return f"browser-{selected['backend']}-{selected['browser']}-v1.json"


def _check(name: str, selected: dict[str, Any], retained: Path) -> dict[str, Any]:
    """The producer's fragment, once its status and slot are known good."""
    status = _read(retained / "producer-status-v1.json")
    if status["schema_version"] != SCHEMA_VERSION:
        _refuse(f"{name}: producer status schema is unsupported")
    if not status["ok"]:
        _refuse(f"{name}: producer reported failure: {status['detail']}")
    path = retained / "fragments" / _fragment_name(selected)
    if not path.is_file():
        _refuse(f"{name}: producer fragment is missing: {path}")
    envelope: dict[str, Any] = _read(path)
    # The VM validated the fragment against what it declares; nothing there
    # ties that declaration to the slot Nix selected.
    fragment = envelope["fragment"]
    browser = selected.get("browser") or None
    if (
        fragment["producer"] != selected["producer"]
        or fragment["result"]["setup"]["backend"] != selected["backend"]
        or any(
            item["key"]["browser"] != browser
            for item in fragment["result"]["workloads"]
        )
    ):
        _refuse(f"{name}: producer fragment does not fill its selected slot")
    return envelope


def _retain(run_root: Path, selected: dict[str, Any]) -> Retained:
    """Builds one selected producer, keeps a writable copy of its evidence,
    and checks its fragment."""
    name = str(selected["name"])
    identity = str(selected["identity"])

    def refusal(output: str) -> str | None:
        if output != identity:
            return (
                f"Nix output identity mismatches selection for {name}: "
                f"{output} != {identity}"
            )
        if not Path(output, "performance").is_dir():
            return f"Nix output lacks performance evidence for {name}: {output}"
        return None

    # Without the flake's configuration, so its substituters stay out of a
    # measurement run.
    output = steps.nix_build(
        name,
        str(selected["installable"]),
        flags=(),
        keep=False,
        refusal=refusal,
    )
    producer_root = run_root / "producers" / name
    producer_root.mkdir(parents=True, exist_ok=True)
    retained = producer_root / "performance"
    steps.run(
        f"{name}-retain", ["cp", "-a", "--", f"{output}/performance", str(retained)]
    )
    # Retained evidence is disposable; store referents and symlinks stay untouched.
    steps.run(
        f"{name}-writable",
        ["find", "-P", str(retained), "(", "-type", "f", "-o", "-type", "d", ")"]
        + ["-exec", "chmod", "u+w", "--", "{}", "+"],
    )
    with steps.phase(f"{name}-check"):
        return Retained(name, output, _check(name, selected, retained))


def _canonical(selection: dict[str, Any]) -> bool:
    return (
        selection["backends"] == ["sqlite", "postgres"]
        and selection["browsers"] == ["chromium"]
        and selection["storage"]
        and selection["browser"]
        and all(value is None for value in selection["count_overrides"].values())
    )


def _required(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        _refuse(f"CI performance provenance requires {name}")
    return value


def _provenance() -> dict[str, Any]:
    if "GITHUB_REPOSITORY" in os.environ:
        return {
            "source": "github",
            "details": {
                "repository": _required("GITHUB_REPOSITORY"),
                "workflow": _required("GITHUB_WORKFLOW"),
                "job": _required("GITHUB_JOB"),
                "reference": _required("GITHUB_REF"),
                "head_sha": _required("GITHUB_SHA"),
                "run_id": int(_required("GITHUB_RUN_ID")),
                "attempt": int(_required("GITHUB_RUN_ATTEMPT")),
            },
        }
    commit = steps.text("perf-provenance", ["git", "rev-parse", "HEAD"])
    return {"source": "local", "details": {"git_commit": commit}}


def _assemble(
    run_root: Path,
    nonce: str,
    selection: dict[str, Any],
    built: list[Retained],
) -> None:
    """Joins the checked fragments, in selection order, into one result."""
    manifest = built[0].envelope["manifest"]
    if any(producer.envelope["manifest"] != manifest for producer in built):
        _refuse("producer fragments describe different datasets")
    canonical = _canonical(selection)
    results = [producer.envelope["fragment"]["result"] for producer in built]
    result = {
        "schema_version": SCHEMA_VERSION,
        "manifest": manifest,
        "selection": selection,
        "canonical_selection": canonical,
        "provenance": _provenance(),
        "evidence": {
            "measured_at_unix_ms": time.time_ns() // 1_000_000,
            "freshness_nonce": nonce,
            "producer_derivation_identities": sorted(
                (
                    {"name": producer.name, "identity": producer.identity}
                    for producer in built
                ),
                key=lambda identity: identity["name"],
            ),
        },
        "setup": [item["setup"] for item in results],
        "workloads": [workload for item in results for workload in item["workloads"]],
    }
    _publish(run_root / RESULT, result)
    _publish(PERFORMANCE_ROOT / RESULT, result)
    print(
        f"assembled {len(result['workloads'])} workload(s) at {run_root / RESULT} "
        f"({'canonical' if canonical else 'noncanonical'} selection)"
    )


def measure(request: Request) -> None:
    # A new nonce changes every producer derivation identity, so no earlier
    # measurement can be reused.
    nonce = secrets.token_hex(32)
    run_root = PERFORMANCE_ROOT / nonce
    (run_root / "producers").mkdir(parents=True, exist_ok=True)
    parameters = {"freshnessNonce": nonce, **dataclasses.asdict(request)}
    selection_path = run_root / "selection.json"
    selection_path.write_bytes(
        steps.output(
            "perf-select",
            ["nix", "eval", "--impure", "--json", "--expr", SELECT_PRODUCERS],
            {"JAUNDER_PERF_PARAMETERS": _compact(parameters)},
        )
    )
    selection = _read(selection_path)
    built = [_retain(run_root, selected) for selected in selection["producers"]]
    with steps.phase("perf-assemble"):
        _assemble(run_root, nonce, selection["selection"], built)


def _versioned(path: Path) -> dict[str, Any]:
    document: dict[str, Any] = _read(path)
    if document.get("schema_version") != SCHEMA_VERSION:
        _refuse(f"{path}: performance schema is unsupported")
    return document


def _key(item: dict[str, Any]) -> str:
    """A workload's compatibility key, comparable across runs."""
    return json.dumps(item["key"], sort_keys=True)


def _keys(run: dict[str, Any]) -> list[str]:
    return sorted(_key(item) for item in run["workloads"])


def _classify(baseline: int, candidate: int) -> str:
    if baseline == 0:
        return "zero-baseline-increase" if candidate else "none"
    # 20% of the baseline, rounded up, in integer microseconds.
    if candidate - baseline >= -(-baseline // 5):
        return "at-least-20-percent"
    return "none"


def _timing(summary: dict[str, Any]) -> str:
    return (
        f"median={summary['median_us']}us p95={summary['p95_us']}us "
        f"spread={summary['minimum_us']}..{summary['maximum_us']}us"
    )


def compare(result: str) -> None:
    with steps.phase("perf-compare"):
        candidate = _versioned(Path(result))
        baseline = _versioned(BASELINE)["run"]
        if candidate["manifest"]["plan"] != baseline["manifest"]["plan"]:
            _refuse("candidate dataset plan differs from baseline")
        if _keys(candidate) != _keys(baseline):
            _refuse("candidate compatibility keys differ from baseline")
        reference = {_key(item): item["summary"] for item in baseline["workloads"]}
        print("advisory only")
        for item in candidate["workloads"]:
            key = item["key"]
            before = reference[_key(item)]
            after = item["summary"]
            print(
                f"{key['workload']}/{key['backend']}/{key['browser'] or '-'}/"
                f"{key['measurement_frame']}/{key['measurement_position']}: "
                f"baseline {_timing(before)}; candidate {_timing(after)}; "
                f"advisory median={_classify(before['median_us'], after['median_us'])} "
                f"p95={_classify(before['p95_us'], after['p95_us'])}"
            )


def _github(name: str, path: str) -> Any:
    body = json.loads(steps.output(name, ["gh", "api", path]))
    if body.get("errors"):
        _refuse(f"GitHub API response contains errors: {path}")
    return body


def _positive(value: object) -> bool:
    """A positive integer, as GitHub's numeric identifiers are."""
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def _admit(run_id: int, metadata: Any, jobs: Any, artifacts: Any) -> tuple[str, int]:
    """The run's head SHA and attempt, once the run, its job and its artifact
    are the trusted ones; nothing is downloaded before this."""
    if metadata.get("id") != run_id:
        _refuse("Actions run ID mismatch")
    if (metadata.get("repository") or {}).get("full_name") != TRUSTED_REPOSITORY or (
        metadata.get("head_repository") or {}
    ).get("full_name") != TRUSTED_REPOSITORY:
        _refuse("Actions run belongs to another repository")
    if (
        metadata.get("path") != TRUSTED_WORKFLOW_PATH
        or metadata.get("name") != TRUSTED_WORKFLOW
    ):
        _refuse("Actions run did not use the approved performance workflow")
    if metadata.get("event") not in ("workflow_dispatch", "schedule"):
        _refuse("pull-request and unapproved workflow events cannot become a baseline")
    if (metadata.get("status"), metadata.get("conclusion")) != ("completed", "success"):
        _refuse("baseline import requires a completed successful run")
    if metadata.get("head_branch") != "main":
        _refuse("baseline import requires main")
    head_sha = metadata.get("head_sha")
    attempt = metadata.get("run_attempt")
    if not isinstance(head_sha, str) or not head_sha.strip() or not _positive(attempt):
        _refuse("Actions run has no head SHA or attempt")
    if not any(
        job.get("name") == TRUSTED_JOB
        and job.get("status") == "completed"
        and job.get("conclusion") == "success"
        and job.get("head_sha") == head_sha
        and job.get("run_attempt") == attempt
        for job in jobs.get("jobs", [])
    ):
        _refuse("approved performance job is absent or unsuccessful")
    if not any(
        artifact.get("name") == ARTIFACT
        and artifact.get("expired") is False
        and _positive(artifact.get("id"))
        and _positive(artifact.get("size_in_bytes"))
        and (artifact.get("workflow_run") or {}).get("id") == run_id
        and (artifact.get("workflow_run") or {}).get("head_branch") == "main"
        and (artifact.get("workflow_run") or {}).get("head_sha") == head_sha
        for artifact in artifacts.get("artifacts", [])
    ):
        _refuse(
            "successful canonical run lacks the complete performance-result artifact"
        )
    return head_sha, attempt


def import_baseline(run_id: int) -> None:
    run_path = f"/repos/{TRUSTED_REPOSITORY}/actions/runs/{run_id}"
    metadata = _github("perf-run", run_path)
    jobs = _github("perf-run-jobs", f"{run_path}/jobs")
    artifacts = _github("perf-run-artifacts", f"{run_path}/artifacts")
    with steps.phase("perf-admit"):
        head_sha, attempt = _admit(run_id, metadata, jobs, artifacts)
    with tempfile.TemporaryDirectory() as staging:
        steps.run(
            "perf-download",
            ["gh", "run", "download", str(run_id), "--repo", TRUSTED_REPOSITORY]
            + ["--name", ARTIFACT, "--dir", staging],
        )
        with steps.phase("perf-import"):
            path = Path(staging, RESULT)
            if not path.is_file():
                _refuse(f"run artifact lacks {RESULT}")
            run = _versioned(path)
            if run["manifest"]["plan"]["profile"] != "medium":
                _refuse("canonical baseline requires the medium profile")
            if not run["canonical_selection"]:
                _refuse("baseline import rejects noncanonical selection")
            provenance = run["provenance"]
            expected = {
                "repository": TRUSTED_REPOSITORY,
                "workflow": TRUSTED_WORKFLOW,
                "job": TRUSTED_JOB,
                "reference": "refs/heads/main",
                "head_sha": head_sha,
                "run_id": run_id,
                "attempt": attempt,
            }
            if provenance["source"] != "github" or provenance["details"] != expected:
                _refuse("artifact provenance does not match the approved Actions run")
            if BASELINE.is_file() and _keys(run) != _keys(_versioned(BASELINE)["run"]):
                _refuse(
                    "imported run compatibility keys differ from the committed baseline"
                )
            _publish(BASELINE, {"schema_version": SCHEMA_VERSION, "run": run})
    print(f"imported canonical Actions run {run_id}")
