"""The daily RustSec advisory scan of `main` (`.github/workflows/advisories.yml`).

CI checks advisories only for a change to either Cargo workspace's
dependencies (`dev.scope`), so an advisory published against a dependency
already on `main` fails no pull request. This scan reports it instead: it checks
both workspaces, the product and `tools/`, and keeps one open issue per
advisory, commented on each day it still fails
(docs/adr/drafts/ci-checks-advisories-on-dependency-change.md). cargo-deny reads
the root `deny.toml` for both, so their ignores and unmaintained scope are the
gate's.
"""

import json
import os
import sys
from dataclasses import dataclass
from datetime import UTC, datetime

from dev import steps
from dev.issues import REPO

LABELS = "security,dependencies,needs-triage"
# Each Cargo workspace, by name, with the arguments that select it.
WORKSPACES = {
    "product": [],
    "tools": ["--manifest-path", "tools/Cargo.toml"],
}


@dataclass(frozen=True)
class Advisory:
    id: str
    crate: str
    summary: str
    # Where it was found: each crate version, with its workspace.
    found: tuple[str, ...]

    @property
    def title(self) -> str:
        return f"fix(deps): {self.id} {self.crate}: {self.summary}"


def findings(report: bytes, workspace: str) -> list[Advisory]:
    """The advisories that fail cargo-deny's JSON `report` on `workspace`, once
    each, in order.

    Any other failure files nothing; the scan's status still reports it.
    """
    found: dict[str, Advisory] = {}
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
        place = f"`{crate['name']}` {crate['version']} in the {workspace} workspace"
        found.setdefault(
            advisory["id"],
            Advisory(advisory["id"], crate["name"], advisory["title"], (place,)),
        )
    return list(found.values())


def merged(reports: list[list[Advisory]]) -> list[Advisory]:
    """One advisory per ID across the workspaces' findings, found wherever any
    workspace found it."""
    advisories: dict[str, Advisory] = {}
    for advisory in (advisory for report in reports for advisory in report):
        known = advisories.get(advisory.id)
        advisories[advisory.id] = (
            advisory
            if known is None
            else Advisory(
                known.id, known.crate, known.summary, known.found + advisory.found
            )
        )
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
    places = "".join(f"- {place}\n" for place in advisory.found)
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
    checked = [
        (
            workspace,
            *steps.captured(
                f"cargo-deny-advisories-{workspace}",
                ["cargo", "deny", *args, "--format", "json", "check", "advisories"],
            ),
        )
        for workspace, args in WORKSPACES.items()
    ]
    today, link = datetime.now(UTC).date().isoformat(), _run_link()
    found = merged([findings(report, workspace) for workspace, _, _, report in checked])
    for advisory in found:
        _report(advisory, today, link)
    failed = [(code, report) for _, code, _, report in checked if code]
    if failed:
        # A report is a failure's only account, such as a database that could
        # not be fetched, so each failing one reaches the run's log.
        for _, report in failed:
            sys.stderr.buffer.write(report)
        sys.stderr.flush()
        raise steps.StepFailed(failed[0][0], False)
