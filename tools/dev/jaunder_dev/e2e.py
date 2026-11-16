"""The E2E workflow: isolated producers, then artifact-only reconciliation.

CI runs the same producer and reconciliation as the host, so a lane's verdict
does not depend on where it ran.
"""

from jaunder_dev import report, steps


def run_all(jobs: str) -> None:
    """Every producer, then reconciliation of both backends.

    `jobs` bounds concurrent producers; empty runs every enabled lane at once.
    """
    steps.run("e2e", ["bash", "scripts/e2e-workflow", "all", jobs])


def producer(lane: str) -> None:
    steps.run("e2e-producer", ["bash", "scripts/e2e-workflow", "producer", lane])


def regenerate_coverage() -> None:
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


def reconcile(backend: str, artifacts: str) -> None:
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
