"""The daily RustSec advisory scan of `main` (`.github/workflows/advisories.yml`).

CI checks advisories only for a change to either Cargo workspace's
dependencies (`dev.scope`), so an advisory published against a dependency
already on `main` fails no pull request. This scan reports it instead: it checks
both workspaces, the product and `tools/`, and keeps one open issue per
advisory, commented on each day it still fails
(docs/adr/0276-ci-checks-advisories-on-dependency-change.md). cargo-deny reads
the root `deny.toml` for both, so their ignores and unmaintained scope are the
gate's.
"""

import json
import os
import sys
from dataclasses import dataclass, replace
from datetime import UTC, datetime

from dev import static, steps
from dev.issues import REPO

LABELS = "security,dependencies,needs-triage"
# Each Cargo workspace, by name, with the arguments that select it.
WORKSPACES: dict[str, tuple[str, ...]] = {
    "product": (),
    "tools": static.TOOLS_MANIFEST,
}


@dataclass(frozen=True)
class Place:
    """A crate version an advisory affects, in one workspace."""

    crate: str
    version: str
    workspace: str

    def described(self) -> str:
        return f"`{self.crate}` {self.version} in the {self.workspace} workspace"


@dataclass(frozen=True)
class Advisory:
    id: str
    crate: str
    summary: str
    found: tuple[Place, ...]

    @property
    def title(self) -> str:
        return f"fix(deps): {self.id} {self.crate}: {self.summary}"


@dataclass(frozen=True)
class Checked:
    """One workspace's advisory check: cargo-deny's status and JSON report."""

    workspace: str
    code: int
    report: bytes


def findings(report: bytes, workspace: str) -> list[Advisory]:
    """Each advisory error in cargo-deny's JSON `report` on `workspace`, as one
    finding at the crate version it names, in order.

    Any other failure files nothing; the scan's status still reports it.
    """
    found = []
    for line in report.splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        fields = record.get("fields", {})
        advisory = fields.get("advisory")
        if (
            record.get("type") != "diagnostic"
            or fields.get("severity") != "error"
            or not advisory
        ):
            continue
        crate = fields["graphs"][0]["Krate"]
        place = Place(crate["name"], crate["version"], workspace)
        found.append(
            Advisory(advisory["id"], crate["name"], advisory["title"], (place,))
        )
    return found


def merged(found: list[Advisory]) -> list[Advisory]:
    """One advisory per ID, first seen first, found at every distinct place any
    finding for it named."""
    advisories: dict[str, Advisory] = {}
    for finding in found:
        known = advisories.setdefault(finding.id, replace(finding, found=()))
        places = known.found + tuple(p for p in finding.found if p not in known.found)
        advisories[finding.id] = replace(known, found=places)
    return list(advisories.values())


def _run_link() -> str:
    run = os.environ.get("GITHUB_RUN_ID")
    if not run:
        return "a local run"
    server, repository = (
        os.environ["GITHUB_SERVER_URL"],
        os.environ["GITHUB_REPOSITORY"],
    )
    return f"{server}/{repository}/actions/runs/{run}"


def _body(advisory: Advisory, today: str, link: str) -> str:
    places = "".join(f"- {place.described()}\n" for place in advisory.found)
    return (
        f"cargo-deny reports [{advisory.id}]"
        f"(https://rustsec.org/advisories/{advisory.id}.html) on `main`:\n\n"
        f"{places}\n"
        f"Found on {today} by {link}. The daily advisory scan "
        "(`.github/workflows/advisories.yml`) comments here each day it still "
        "fails. Resolve it by updating the dependency, or by ignoring the "
        "advisory in `deny.toml` with its reason.\n"
    )


def _report(advisory: Advisory, today: str, link: str) -> None:
    """Comments on the open issue for `advisory`, or opens one."""
    listed = steps.output(
        "issue-search",
        [
            "gh",
            "issue",
            "list",
            "--repo",
            REPO,
            "--state",
            "open",
            "--search",
            f"{advisory.id} in:title",
            "--json",
            "number,title",
        ],
    )
    # The search is fuzzy; only an issue whose title carries the ID is this one.
    matches = [issue for issue in json.loads(listed) if advisory.id in issue["title"]]
    if matches:
        # Should two be open, the oldest is the one the scan filed and keeps.
        number = str(min(issue["number"] for issue in matches))
        comment = f"Still reported on {today} by {link}."
        steps.run(
            "issue-comment",
            ["gh", "issue", "comment", number, "--repo", REPO, "--body", comment],
        )
        return
    steps.run(
        "issue-create",
        [
            "gh",
            "issue",
            "create",
            "--repo",
            REPO,
            "--title",
            advisory.title,
            "--label",
            LABELS,
            "--body",
            _body(advisory, today, link),
        ],
    )


def scan() -> None:
    """Checks both workspaces' advisories and reports each failing one; fails
    whenever cargo-deny does on either."""
    checked = [_check(workspace, args) for workspace, args in WORKSPACES.items()]
    today, link = datetime.now(UTC).date().isoformat(), _run_link()
    found = [f for check in checked for f in findings(check.report, check.workspace)]
    for advisory in merged(found):
        _report(advisory, today, link)
    failed = [check for check in checked if check.code]
    if failed:
        # A report is a failure's only account, such as a database that could
        # not be fetched, so each failing one reaches the run's log.
        for check in failed:
            sys.stderr.buffer.write(check.report)
        sys.stderr.flush()
        raise steps.StepFailed(failed[0].code, False)


def _check(workspace: str, args: tuple[str, ...]) -> Checked:
    code, _, report = steps.captured(
        f"cargo-deny-advisories-{workspace}",
        ["cargo", "deny", *args, "--format", "json", "check", "advisories"],
    )
    return Checked(workspace, code, report)
