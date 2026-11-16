"""The remaining developer workflows: E2E, performance, qualification, issue and
PR front doors, and forwarding entry points to the host tools.

Each runs its steps in order from the checkout root and stops at the first
failure, as the gates do. Forwarded arguments always travel as argv elements.
"""

import json
import shutil
import tempfile
from collections.abc import Sequence
from pathlib import Path

from jaunder_dev import report, steps
from jaunder_dev.gates import (
    CARGO_RUN,
    COVERAGE_CHECK,
    HOST_PROOF,
    REPOSITORY_POLICY,
    SOURCE_CONTRACT,
    WITHOUT_GIT_OVERRIDES,
    clean_tree,
)

PERF_DATA = (*CARGO_RUN, "-p", "performance", "--bin", "perf-data", "--")
NIX_FLAGS = ("--no-update-lock-file", "--no-write-lock-file")

# Forwarding entry points.


def sandbox(args: Sequence[str]) -> None:
    steps.run("ux-sandbox", ["nix", "run", *NIX_FLAGS, ".#ux-sandbox", "--", *args])


def census(args: Sequence[str]) -> None:
    steps.run("census", [*SOURCE_CONTRACT, "census", *args])


def org_timezone_manifest(files: Sequence[str]) -> None:
    steps.run(
        "org-timezone-manifest",
        [*REPOSITORY_POLICY, "org-timezone-manifest", "--", "--", *files],
    )


def pg_run(args: Sequence[str]) -> None:
    steps.run("pg-run", [*HOST_PROOF, "pg-run", "--", *args])


def coverage_check(args: Sequence[str]) -> None:
    steps.run("coverage-check", [*COVERAGE_CHECK, *args], report.render)


def host_proof_report(binary: str, args: Sequence[str]) -> None:
    """A host-proof diagnostic whose one JSON report is shown rendered."""
    steps.run(binary, [*HOST_PROOF, binary, "--", *args], report.render)


def pr_watch(args: Sequence[str]) -> None:
    steps.run(
        "pr-watch",
        [*CARGO_RUN, "-p", "pr-observer", "--", "watch", *args],
    )


# Qualification. Its public catalog owns ordinary Git preflight; the
# qualification host binds HEAD to its clean source pin before selecting a Nix
# harness, and runs only as its own Nix output.

ORIGINS = (
    "https://github.com/jaunder-org/jaunder.git",
    "https://github.com/jaunder-org/jaunder",
    "git@github.com:jaunder-org/jaunder.git",
    "ssh://git@github.com/jaunder-org/jaunder.git",
)
QUALIFICATION_HOST = ["nix", "run", ".#qualification-host"]


def _git(name: str, *args: str) -> str:
    return steps.text(name, ["git", *args], WITHOUT_GIT_OVERRIDES)


def _commit(name: str, revision: str) -> str:
    return _git(
        name, "rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}"
    )


def production_baseline(source: str, target: str) -> None:
    """Fetches the harness, source and target commits, then runs the host.

    Only discovery passes an empty `target`.
    """
    clean_tree()
    harness = _git("harness", "rev-parse", "--verify", "HEAD^{commit}")
    source = _commit("source", source)
    if target:
        target = _commit("target", target)
        if source == target:
            steps.refuse(
                "production-baseline accept requires distinct resolved source "
                "and target commits"
            )
    origin = _git("origin", "remote", "get-url", "origin")
    if origin not in ORIGINS:
        steps.refuse("production-baseline requires origin to be jaunder-org/jaunder")
    commits = [commit for commit in dict.fromkeys([harness, source, target]) if commit]
    # Re-request every SHA: an ordinary fetch can accept locally complete
    # objects without asking the server whether it has them.
    steps.run(
        "fetch",
        ["git", "fetch", "--quiet", "--no-write-fetch-head", "--refetch", "origin"]
        + commits,
        environment=WITHOUT_GIT_OVERRIDES,
    )
    steps.run(
        "qualification-host",
        [
            *QUALIFICATION_HOST,
            "--",
            "--expected-harness",
            harness,
            "--source",
            source,
            *(["--target", target] if target else []),
        ],
        environment=WITHOUT_GIT_OVERRIDES,
    )


def production_baseline_qualify_styling() -> None:
    clean_tree()
    harness = _git("harness", "rev-parse", "--verify", "HEAD^{commit}")
    steps.run(
        "qualification-host",
        [*QUALIFICATION_HOST, "--", "--expected-harness", harness],
        environment=WITHOUT_GIT_OVERRIDES,
    )


def production_baseline_smoke() -> None:
    """The mutable local lifecycle diagnostic: no qualification or evidence."""
    steps.run("qualification-host", QUALIFICATION_HOST)


# Performance. Nix selects the producer set; this ordered loop retains each
# output before the next measurement can run, and validates it before going on.

SELECT_PRODUCERS = (
    "let flake = builtins.getFlake (toString ./.); "
    'parameters = builtins.fromJSON (builtins.getEnv "JAUNDER_PERF_PARAMETERS"); '
    "in flake.performanceLib.${builtins.currentSystem}.selectPerformanceProducers "
    "parameters"
)


def _compact(value: object) -> str:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def _retain(run_root: Path, selected: dict[str, str | None]) -> None:
    """Builds one selected producer and keeps a writable copy of its evidence."""
    name = str(selected["name"])
    identity = selected["identity"]
    backend = str(selected["backend"])
    kind = str(selected["producer"])
    browser = selected.get("browser") or ""
    output = steps.text(
        name,
        [
            "nix",
            "build",
            "--no-link",
            "--print-out-paths",
            str(selected["installable"]),
        ],
    )
    if output != identity:
        steps.refuse(
            f"Nix output identity mismatches selection for {name}: "
            f"{output} != {identity}"
        )
    if not Path(output, "performance").is_dir():
        steps.refuse(f"Nix output lacks performance evidence for {name}: {output}")
    producer_root = run_root / "producers" / name
    producer_root.mkdir(parents=True, exist_ok=True)
    retained = f"{producer_root}/performance"
    steps.run(f"{name}-retain", ["cp", "-a", "--", f"{output}/performance", retained])
    # Retained evidence is disposable; store referents and symlinks stay untouched.
    steps.run(
        f"{name}-writable",
        ["find", "-P", retained, "(", "-type", "f", "-o", "-type", "d", ")"]
        + ["-exec", "chmod", "u+w", "--", "{}", "+"],
    )
    steps.run(
        f"{name}-validate",
        [
            *PERF_DATA,
            "validate-producer",
            "--producer",
            kind,
            "--backend",
            backend,
            *(["--browser", browser] if browser else []),
            "--performance-root",
            retained,
        ],
    )
    metadata = {
        "name": name,
        "identity": output,
        "producer": kind,
        "backend": backend,
        "browser": browser or None,
        "retained_root": retained,
    }
    (producer_root / "metadata.json").write_text(
        _compact(metadata) + "\n", encoding="utf-8"
    )


def perf(
    profile: str,
    backends: str,
    browsers: str,
    workload: str,
    posts: str,
    authors: str,
    revisions: str,
) -> None:
    nonce = steps.text("perf-nonce", [*PERF_DATA, "nonce"])
    run_root = Path(".xtask/performance", nonce)
    (run_root / "producers").mkdir(parents=True, exist_ok=True)
    parameters = {
        "freshnessNonce": nonce,
        "profile": profile,
        "backends": backends,
        "browsers": browsers,
        "workload": workload,
        "posts": posts,
        "authors": authors,
        "revisions": revisions,
    }
    selection_path = run_root / "selection.json"
    selection_path.write_bytes(
        steps.output(
            "perf-select",
            ["nix", "eval", "--impure", "--json", "--expr", SELECT_PRODUCERS],
            {"JAUNDER_PERF_PARAMETERS": _compact(parameters)},
        )
    )
    steps.run(
        "perf-validate-selection",
        [*PERF_DATA, "validate-selection", "--selection", str(selection_path)],
    )
    selection = json.loads(selection_path.read_bytes())
    for selected in selection["producers"]:
        _retain(run_root, selected)
    invocation = {
        "profile": profile,
        "freshness_nonce": nonce,
        "selection": selection["selection"],
        "producers": [
            json.loads(path.read_bytes())
            for path in sorted(run_root.glob("producers/*/metadata.json"))
        ],
    }
    invocation_path = run_root / "invocation.json"
    invocation_path.write_text(
        json.dumps(invocation, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    steps.run(
        "perf-assemble", [*PERF_DATA, "assemble", "--invocation", str(invocation_path)]
    )


def perf_compare(result: str) -> None:
    """Comparison is advisory."""
    steps.run("perf-compare", [*PERF_DATA, "compare", "--result", result])


def perf_import_baseline(run_id: str) -> None:
    """Only this explicit authenticated import may replace the baseline."""
    steps.run("perf-import-baseline", [*PERF_DATA, "import-baseline", run_id])


# E2E: CI uses the same isolated producer and artifact-only reconciliation.


def e2e_producer(lane: str) -> None:
    steps.run("e2e-producer", ["bash", "scripts/e2e-workflow", "producer", lane])


def e2e_regenerate_coverage() -> None:
    """The intentional snapshot update; ordinary producers only verify."""
    steps.run(
        "e2e-regenerate-coverage",
        [
            "bash",
            "scripts/e2e-workflow",
            "producer",
            "sqlite-chromium-unsplit",
            "regenerate",
        ],
    )


def e2e_reconcile(backend: str, artifacts: str) -> None:
    """Artifact-only reconciliation by this checkout's Nix-built verifier."""
    verifier = steps.text(
        "host-proof", ["nix", "build", "--no-link", "--print-out-paths", ".#host-proof"]
    )
    steps.run(
        "e2e-reconcile",
        [
            f"{verifier}/bin/e2e-evidence",
            "reconcile",
            backend,
            "--catalog",
            "end2end/e2e-lanes.json",
            "--diagnostics-root",
            artifacts,
        ],
        report.render,
    )


# Issue discovery and PR front doors (issue mutations are in `issues`).
# Nothing here infers approval or ownership.


def issue_discover(milestone: str) -> None:
    """Exhaustive read-only milestone candidates; never claims or dispatches work."""
    steps.run("issue-discover", ["bash", "scripts/issue-discover.sh", milestone])


def pr_land(number: str, expected_head: str, mode: str) -> None:
    steps.run("pr-land", ["bash", "scripts/pr-land.sh", number, expected_head, mode])


def pr_cleanup(number: str, expected_head: str, ownership: str) -> None:
    steps.run(
        "pr-cleanup",
        ["bash", "scripts/pr-cleanup.sh", number, expected_head, ownership],
    )


# The CSR bundle, built with the checkout's locked wasm toolchain and Vite.


def build_csr(profile: str) -> None:
    checkout = Path.cwd()
    release = profile == "release"
    vite_tools = steps.text(
        "csr-vite-node-modules",
        ["nix", "build", ".#csr-vite-node-modules", "--no-link", "--print-out-paths"],
    )
    steps.run(
        "csr-wasm",
        [
            "cargo",
            "build",
            "--locked",
            "-p",
            "csr",
            "--target",
            "wasm32-unknown-unknown",
            *(["--release"] if release else []),
        ],
    )
    shutil.rmtree(checkout / "target" / "site", ignore_errors=True)
    with tempfile.TemporaryDirectory() as temporary:
        work = Path(temporary)
        project = work / "project"
        system_artifacts = work / "system-artifacts"
        (project / "csr").mkdir(parents=True)
        (work / "input").mkdir()
        system_artifacts.mkdir()
        for name in ["package.json", "package-lock.json", "vite.config.js"]:
            shutil.copy(checkout / name, project)
        for name in ["index.html", "entry.js"]:
            shutil.copy(checkout / "csr" / name, project / "csr")
        (project / "node_modules").symlink_to(f"{vite_tools}/node_modules")
        steps.run(
            "system-artifacts",
            [
                "cargo",
                "run",
                "--locked",
                "--manifest-path",
                f"{checkout}/tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "system-artifacts",
                "--out",
                str(system_artifacts),
            ],
        )
        steps.run(
            "csr-assets",
            [
                "bash",
                f"{checkout}/scripts/build-csr-assets.sh",
                f"target/wasm32-unknown-unknown/{profile}/csr.wasm",
                str(project),
                str(work / "input"),
                str(system_artifacts),
                str(work / "dist"),
            ],
            environment={"CSR_VITE_PUBLIC_DIR": f"{checkout}/public"},
        )
        (checkout / "target" / "site").mkdir(parents=True, exist_ok=True)
        steps.run(
            "csr-site", ["cp", "-a", f"{work}/dist/.", f"{checkout}/target/site/"]
        )
