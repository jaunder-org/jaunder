"""Performance measurement: Nix selects the producers, and each is built,
retained and validated before the next can run.

A failure stops the run and keeps every finished producer's evidence.
Comparison with the committed baseline is advisory; only an explicit
authenticated import replaces that baseline.
"""

import dataclasses
import json
from pathlib import Path
from typing import Any

from jaunder_dev import steps
from jaunder_dev.commands import PERF_DATA

SELECT_PRODUCERS = (
    'let flake = builtins.getFlake ("git+file://" + toString ./.); '
    'parameters = builtins.fromJSON (builtins.getEnv "JAUNDER_PERF_PARAMETERS"); '
    "in flake.performanceLib.${builtins.currentSystem}.selectPerformanceProducers "
    "parameters"
)


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


def _compact(value: object) -> str:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def _retain(run_root: Path, selected: dict[str, Any]) -> None:
    """Builds one selected producer and keeps a writable copy of its evidence."""
    name = str(selected["name"])
    identity = str(selected["identity"])
    backend = str(selected["backend"])
    kind = str(selected["producer"])
    browser = selected.get("browser") or ""

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
        output=identity,
        refusal=refusal,
    )
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


def measure(request: Request) -> None:
    nonce = steps.text("perf-nonce", [*PERF_DATA, "nonce"])
    run_root = Path(".xtask/performance", nonce)
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
    steps.run(
        "perf-validate-selection",
        [*PERF_DATA, "validate-selection", "--selection", str(selection_path)],
    )
    selection = json.loads(selection_path.read_bytes())
    for selected in selection["producers"]:
        _retain(run_root, selected)
    invocation = {
        "profile": request.profile,
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


def compare(result: str) -> None:
    steps.run("perf-compare", [*PERF_DATA, "compare", "--result", result])


def import_baseline(run_id: str) -> None:
    steps.run("perf-import-baseline", [*PERF_DATA, "import-baseline", run_id])
