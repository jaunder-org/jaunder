"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran.
"""

from dev import report, steps

CATALOG = "end2end/e2e-lanes.json"
# Server-function flow coverage is derived from this pair's reconciled lanes.
COVERAGE_PAIR = ("sqlite", "chromium")


def run_all(jobs: str) -> None:
    """Every producer, then reconciliation of every backend/browser pair.

    `jobs` bounds concurrent producers; empty means the script's default of 3.
    """
    steps.run("e2e", ["bash", "scripts/e2e-workflow", "all", jobs])


def producer(lane: str) -> None:
    """One lane's producer; its verification report is rendered."""
    steps.run(
        "e2e-producer",
        ["bash", "scripts/e2e-workflow", "producer", lane],
        report.render,
    )


def _verifier() -> str:
    return steps.nix_build("host-proof", ".#host-proof", keep=False)


def _coverage(verifier: str, action: str, artifacts: str) -> None:
    steps.run(
        f"server-fn-coverage-{action}",
        [
            f"{verifier}/bin/e2e-evidence",
            "server-fn-coverage",
            action,
            "--catalog",
            CATALOG,
            "--diagnostics-root",
            artifacts,
        ],
        report.render,
    )


def regenerate_coverage() -> None:
    """The intentional snapshot update from freshly produced coverage lanes."""
    steps.run(
        "e2e-coverage-producers",
        ["bash", "scripts/e2e-workflow", "pair", *COVERAGE_PAIR],
    )
    _coverage(_verifier(), "regenerate", ".xtask/diagnostics")


def reconcile(backend: str, browser: str, artifacts: str) -> None:
    """Artifact-only reconciliation by this checkout's Nix-built verifier.

    The coverage pair's verdict also includes server-function flow coverage.
    """
    verifier = _verifier()
    steps.run(
        "e2e-reconcile",
        [
            f"{verifier}/bin/e2e-evidence",
            "reconcile",
            backend,
            browser,
            "--catalog",
            CATALOG,
            "--diagnostics-root",
            artifacts,
        ],
        report.render,
    )
    if (backend, browser) == COVERAGE_PAIR:
        _coverage(verifier, "verify", artifacts)
