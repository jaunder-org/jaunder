"""The upgrade test: a site an earlier build wrote survives an in-place upgrade.

The earlier build creates and verifies the baseline records, the service is
upgraded in place to this checkout, and this checkout verifies the same
records (nix/checks.nix `mkUpgradeCheck`). `test` builds that check here for
one backend; `dispatch` starts the CI workflow, which runs both.
"""

import shutil
from pathlib import Path

from dev import steps
from dev.checkout import traced_git

BACKENDS = ("sqlite", "postgres")
WORKFLOW = "upgrade.yml"
DIAGNOSTICS = Path(".xtask/diagnostics")
# The earlier build is this checkout's own Git object, so any fetched commit
# works, including one never pushed. Neither flake's lock file is touched.
UPGRADE_CHECK = (
    'let checkout = "git+file://" + toString ./.; '
    "current = builtins.getFlake checkout; "
    "previous = builtins.getFlake "
    '(checkout + "?rev=" + builtins.getEnv "JAUNDER_UPGRADE_FROM"); '
    "in (current.upgradeLib.${builtins.currentSystem}.mkUpgradeCheck "
    '{ inherit previous; backend = builtins.getEnv "JAUNDER_UPGRADE_BACKEND"; })'
)


def _commit(revision: str) -> str:
    return traced_git(
        "upgrade-from",
        "rev-parse",
        "--verify",
        "--end-of-options",
        f"{revision}^{{commit}}",
    )


def earlier(revision: str | None, tag: str | None) -> str:
    """The build to upgrade from, as a commit.

    An explicit revision wins. A tag run upgrades from the previous `v*` tag;
    any other run from the newest green run of the workflow on `main`. With
    neither, the build upgrades from itself.
    """
    if revision is not None:
        return _commit(revision)
    if tag is not None:
        previous = traced_git(
            "upgrade-previous-tag",
            "tag",
            "--list",
            "v*",
            "--sort=-version:refname",
            "--merged",
            f"{tag}^",
        ).split()
        return _commit(previous[0] if previous else tag)
    green = steps.text(
        "upgrade-last-green",
        [
            "gh",
            "run",
            "list",
            "--workflow",
            WORKFLOW,
            "--branch",
            "main",
            "--status",
            "success",
            "--limit",
            "1",
            "--json",
            "headSha",
            "--jq",
            '.[0].headSha // ""',
        ],
    )
    return _commit(green or "HEAD")


def _lift(output: str, destination: Path) -> None:
    """Copies the check's artifacts out of its (possibly failed) output."""
    if not output or not Path(output).is_dir():
        return
    destination.mkdir(parents=True, exist_ok=True)
    for artifact in Path(output).iterdir():
        if artifact.is_file():
            shutil.copyfile(artifact, destination / artifact.name)


def test(revision: str | None, tag: str | None, backend: str) -> None:
    environment = {
        "JAUNDER_UPGRADE_FROM": earlier(revision, tag),
        "JAUNDER_UPGRADE_BACKEND": backend,
    }
    check = f"upgrade-{backend}"

    def attribute(name: str) -> str:
        return steps.text(
            f"upgrade-{name}",
            ["nix", "eval", "--impure", "--raw", "--expr", f"({UPGRADE_CHECK}).{name}"],
            environment,
        )

    derivation = attribute("drvPath")
    output = attribute("outPath")
    try:
        # A failed VM test keeps its copied-out artifacts in its kept output.
        steps.nix_build(
            check, f"{derivation}^out", flags=(*steps.NIX_FLAGS, "--keep-failed")
        )
    finally:
        _lift(output, DIAGNOSTICS / check)


def dispatch(revision: str | None, ref: str) -> None:
    """Starts the CI workflow on `ref`; without `revision` it picks its own."""
    fields: list[str] = []
    if revision is not None:
        earlier = _commit(revision)
        # The runner fetches from origin, so the earlier build must be there.
        traced_git(
            "upgrade-from-on-origin",
            "branch",
            "--remotes",
            "--contains",
            earlier,
            refusal=lambda branches: (
                None if branches else f"{earlier} is on no origin branch"
            ),
        )
        fields = ["-f", f"from={earlier}"]
    steps.run(
        "upgrade-dispatch", ["gh", "workflow", "run", WORKFLOW, "--ref", ref, *fields]
    )
