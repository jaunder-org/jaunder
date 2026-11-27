"""The daily RustSec advisory scan of `main` (`.github/workflows/advisories.yml`).

CI checks advisories only for a change to the product dependencies
(`dev.scope`), so an advisory published against a dependency already on `main`
fails no pull request. This scan reports it instead, as one open issue per
advisory, commented on each day it still fails
(docs/adr/drafts/ci-checks-advisories-on-dependency-change.md). cargo-deny reads
`deny.toml`, so its ignores and unmaintained scope are the gate's.
"""

import json
import os
import sys
from dataclasses import dataclass
from datetime import UTC, datetime

from dev import steps
from dev.issues import REPO

LABELS = "security,dependencies,needs-triage"


@dataclass(frozen=True)
class Advisory:
    id: str
    crate: str
    version: str
    summary: str

    @property
    def title(self) -> str:
        return f"fix(deps): {self.id} {self.crate}: {self.summary}"


def findings(report: bytes) -> list[Advisory]:
    """The advisories that fail cargo-deny's JSON `report`, once each, in order.

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
        found.setdefault(
            advisory["id"],
            Advisory(
                advisory["id"], crate["name"], crate["version"], advisory["title"]
            ),
        )
    return list(found.values())


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
    return (
        f"cargo-deny reports [{advisory.id}]"
        f"(https://rustsec.org/advisories/{advisory.id}.html) against "
        f"`{advisory.crate}` {advisory.version} on `main`.\n\n"
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
    """Checks the checkout's advisories and reports each failing one; fails
    whenever cargo-deny does."""
    code, _, report = steps.captured(
        "cargo-deny-advisories",
        ["cargo", "deny", "--format", "json", "check", "advisories"],
    )
    today, link = datetime.now(UTC).date().isoformat(), _run_link()
    for advisory in findings(report):
        _report(advisory, today, link)
    if code:
        # The report is the failure's only account, such as a database that
        # could not be fetched, so it reaches the run's log.
        sys.stderr.buffer.write(report)
        sys.stderr.flush()
        raise steps.StepFailed(code, False)
