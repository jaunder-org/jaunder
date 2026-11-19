"""Issue front doors: read-only discovery, and explicit decisions in, verified
tracker state out.

Native `gh` owns every mutation; each reply is checked before the next call,
and the issue is read back in full before success is reported. Nothing is
rolled back: any failure of a mutating front door names the issue to inspect
before retrying. Discovery never writes: it reports every open issue of a
milestone as a candidate or as skipped, with the local evidence for resuming
it.

Each class of failure has its own exit status: a violated check exits 1, a
malformed, partial, ambiguous or unreadable reply 5, a selection that yields
nothing 4, an unreadable body file 2, and a failed `gh` its own status.
"""

import fnmatch
import json
import re
import sys
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, NoReturn

from dev import steps
from dev.checkout import traced_git

REPO = "jaunder-org/jaunder"
PROJECT = "PVT_kwDOECw7os4BblPP"
PRIORITY_FIELD = "PVTSSF_lADOECw7os4BblPPzhWUx50"
PRIORITY_OPTIONS = {
    "P1": "2d6fa1d1",
    "P2": "be01f9da",
    "P3": "0bba09bc",
    "P4": "4b1f556c",
}
TITLE = re.compile(
    r"(feat|fix|docs|refactor|test|chore|build|ci|perf)(\([a-zA-Z0-9_-]+\))?: .+",
    re.DOTALL,
)
POSITIVE = re.compile(r"[1-9][0-9]*")
ISSUE_URL = f"https://github.com/{REPO}/issues/"


def _project_items_query(field: str) -> str:
    """An issue's project items, each with `field` under its lowercase name."""
    return (
        'query=query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){'
        "issue(number:$number){projectItems(first:100){nodes{id project{id} "
        f'{field.lower()}:fieldValueByName(name:"{field}"){{... on '
        "ProjectV2ItemFieldSingleSelectValue{name}}} pageInfo{hasNextPage}}}}}"
    )


# Exit statuses by class of failure.
VIOLATED = 1
NOTHING_SELECTED = 4
ERROR = 5
UNREADABLE = 2


class _Operation:
    """One issue operation; it names the issue to inspect if it fails."""

    def __init__(self, command: str, number: str = "") -> None:
        self.command = command
        self.number = number

    def fail(self, code: int, message: str) -> NoReturn:
        print(f"dev: {self.command}: {message}", file=sys.stderr, flush=True)
        raise steps.StepFailed(code, signalled=False)

    def gh(self, *args: str, stdin: bytes | None = None) -> Any:
        """A gh call's JSON reply; a failed call keeps gh's own status."""
        reply = steps.output("gh", ["gh", *args], stdin=stdin)
        try:
            return json.loads(reply)
        except ValueError:
            self.fail(ERROR, f"gh {args[0]} replied with unreadable JSON")

    def inventory(self, endpoint: str) -> list[dict[str, Any]]:
        """Every row of a paginated REST inventory, each row's number unique.

        Malformed pages and repeated identities are refused before callers
        select rows, so partial data never becomes an empty or successful list.
        """
        pages = self.gh("api", "--paginate", "--slurp", endpoint)
        if not (
            isinstance(pages, list)
            and pages
            and all(isinstance(page, list) for page in pages)
        ):
            self.fail(ERROR, "invalid REST pages")
        rows = [row for page in pages for row in page]
        numbers = [_get(row, "number") for row in rows]
        if not all(_positive(number) for number in numbers) or len(numbers) != len(
            set(numbers)
        ):
            self.fail(ERROR, "invalid/repeated issue identity")
        return rows

    def milestone(self, wanted: str) -> Any:
        """The one open milestone whose number or title is `wanted`."""
        milestones = self.inventory(f"repos/{REPO}/milestones?state=open&per_page=100")
        if not all(
            isinstance(row.get("title"), str) and row["title"] for row in milestones
        ):
            self.fail(ERROR, "invalid milestone")
        matching = [
            row
            for row in milestones
            if json.dumps(row["number"]) == wanted or row["title"] == wanted
        ]
        if len(matching) != 1:
            self.fail(ERROR, "milestone must match exactly one open title/number")
        return matching[0]["number"]


def _positive(value: Any) -> bool:
    """A positive whole JSON number, as jq's `type == "number" and . > 0 and
    . == floor` checks."""
    return (
        isinstance(value, int | float)
        and not isinstance(value, bool)
        and value > 0
        and value == int(value)
    )


def _blockers_valid(blockers: Sequence[str], number: str) -> bool:
    return all(
        POSITIVE.fullmatch(blocker) and blocker != number for blocker in blockers
    )


@dataclass(frozen=True)
class Decisions:
    """An issue's explicit decisions, as both front doors take them."""

    title: str
    issue_type: str
    milestone: str
    priority: str
    labels: str
    body_file: str


@dataclass(frozen=True)
class _Request:
    """Checked decisions, ready to send; nothing is read again after the write."""

    priority: str
    body_file: str
    # The body file's bytes, read once: the readback must return exactly them.
    body: bytes
    # The issue fields gh sends.
    input: dict[str, Any]

    def encoded(self) -> bytes:
        """The request as gh reads it from standard input."""
        return (json.dumps(self.input, indent=2, ensure_ascii=False) + "\n").encode()


def _request(operation: _Operation, decisions: Decisions) -> _Request:
    """Checks every decision, resolving the milestone and reading the body."""
    if not TITLE.match(decisions.title):
        operation.fail(VIOLATED, "Expected Conventional Commit title")
    if decisions.issue_type not in ("Task", "Bug", "Feature"):
        operation.fail(VIOLATED, "Explicit type must be Task, Bug or Feature")
    if decisions.priority not in PRIORITY_OPTIONS:
        operation.fail(VIOLATED, "Explicit Priority must be P1/P2/P3/P4")
    milestone = operation.milestone(decisions.milestone)
    try:
        body = Path(decisions.body_file).read_bytes()
    except OSError as error:
        operation.fail(UNREADABLE, f"{decisions.body_file}: {error.strerror}")
    text = body.decode(errors="replace")
    labels = decisions.labels.split(",") if decisions.labels else []
    if not (
        labels
        and all(labels)
        and len(labels) == len(set(labels))
        and re.search(r"\S", text)
    ):
        operation.fail(ERROR, "explicit nonempty labels and body required")
    return _Request(
        priority=decisions.priority,
        body_file=decisions.body_file,
        body=body,
        input={
            "title": decisions.title,
            "type": decisions.issue_type,
            "milestone": milestone,
            "labels": labels,
            "body": text,
        },
    )


def _add_blocker(operation: _Operation, number: str, blocker: str) -> None:
    """Adds one explicit blocker by its database id, then reads it back.

    Blockers are additive: editing never erases an existing dependency.
    """
    if not _blockers_valid([blocker], number):
        operation.fail(VIOLATED, "Invalid/self blocker")
    found = operation.gh("api", f"repos/{REPO}/issues/{blocker}")
    if not isinstance(found, dict):
        operation.fail(ERROR, f"blocker {blocker} is not an issue")
    if not (
        found.get("number") == int(blocker)
        and found.get("state") == "open"
        and "pull_request" not in found
        and _positive(found.get("id"))
    ):
        operation.fail(NOTHING_SELECTED, f"blocker {blocker} is not an open issue")
    blocker_id = found["id"]
    dependencies = f"repos/{REPO}/issues/{number}/dependencies/blocked_by?per_page=100"
    present = operation.inventory(dependencies)
    if not all(_positive(row.get("id")) for row in present):
        operation.fail(VIOLATED, "invalid dependency ids")
    if not any(row["id"] == blocker_id for row in present):
        added = operation.gh(
            "api",
            "--method",
            "POST",
            f"repos/{REPO}/issues/{number}/dependencies/blocked_by",
            "-F",
            f"issue_id={json.dumps(blocker_id)}",
        )
        if not (_positive(_get(added, "id")) and _positive(_get(added, "number"))):
            operation.fail(VIOLATED, "dependency was not recorded")
    if not any(row["id"] == blocker_id for row in operation.inventory(dependencies)):
        operation.fail(VIOLATED, f"blocker {blocker} missing from readback")


def _add_to_backlog(operation: _Operation, url: str) -> str:
    """The issue's Backlog item id, adding the issue if it is not there.

    `item-add` answers an existing item for an issue already in the project,
    but fails if project automation adds a new issue at the same moment. A
    failed add therefore uses the issue's one Backlog item when there is one;
    with none, or when the add was signalled, the failure stands.
    """
    try:
        item = operation.gh(
            "project",
            "item-add",
            "1",
            "--owner",
            "jaunder-org",
            "--url",
            url,
            "--format",
            "json",
        )
    except steps.StepFailed as failure:
        if failure.signalled:
            raise
        backlog = _backlog_items(operation, operation.number, "Priority", "listing")
        if not backlog:
            raise
        if len(backlog) > 1:
            operation.fail(ERROR, "ambiguous Backlog item")
        item_id = str(backlog[0]["id"])
        print(
            f"dev: {operation.command}: item-add failed; "
            f"using existing Backlog item {item_id}",
            file=sys.stderr,
            flush=True,
        )
        return item_id
    item_id = _get(item, "id")
    if not (isinstance(item_id, str) and item_id):
        operation.fail(NOTHING_SELECTED, "project item has no id")
    return item_id


def _finish(
    operation: _Operation,
    request: _Request,
    mutated: Any,
    blockers: Sequence[str],
) -> None:
    """Adds the issue to the Backlog with its priority and blockers, then
    verifies every decision by reading the issue back."""
    number = _get(mutated, "number")
    if not _positive(number):
        operation.fail(ERROR, "missing issue number")
    operation.number = json.dumps(number)
    url = _get(mutated, "html_url")
    if url != ISSUE_URL + operation.number:
        operation.fail(NOTHING_SELECTED, "issue URL does not name this issue")
    print(f"Verify issue #{operation.number}: {url}", file=sys.stderr, flush=True)
    item_id = _add_to_backlog(operation, url)
    priority = operation.gh(
        "project",
        "item-edit",
        "--project-id",
        PROJECT,
        "--id",
        item_id,
        "--field-id",
        PRIORITY_FIELD,
        "--single-select-option-id",
        PRIORITY_OPTIONS[request.priority],
        "--format",
        "json",
    )
    if _get(priority, "id") != item_id:
        operation.fail(VIOLATED, "priority edit did not name the project item")
    for blocker in blockers:
        _add_blocker(operation, operation.number, blocker)
    readback = operation.gh("api", f"repos/{REPO}/issues/{operation.number}")
    if not _matches(readback, number, request.input):
        operation.fail(
            ERROR, "issue metadata/body readback mismatch (possible body mangling)"
        )
    if readback["body"].encode() != request.body:
        operation.fail(VIOLATED, f"{request.body_file} differs from the readback body")
    backlog = _backlog(operation, item_id, request.priority)
    print(json.dumps(readback | {"backlog": backlog}, indent=2, ensure_ascii=False))


def _get(value: Any, *keys: str) -> Any:
    """`value.keys...` as jq reads it from an object, or None."""
    for key in keys:
        value = value.get(key) if isinstance(value, dict) else None
    return value


def _matches(readback: Any, number: Any, expected: dict[str, Any]) -> bool:
    labels = _get(readback, "labels")
    return (
        _get(readback, "number") == number
        and _get(readback, "title") == expected["title"]
        and _get(readback, "body") == expected["body"]
        and _get(readback, "type", "name") == expected["type"]
        and _get(readback, "milestone", "number") == expected["milestone"]
        and isinstance(labels, list)
        and sorted(str(_get(label, "name")) for label in labels)
        == sorted(expected["labels"])
    )


def _backlog_items(
    operation: _Operation, number: str, field: str, purpose: str
) -> list[dict[str, Any]]:
    """Issue `number`'s Backlog project items, each carrying `field`.

    Only a complete, well-formed listing is answered; `purpose` names what was
    being read when it is not.
    """
    project = operation.gh(
        "api", "graphql", "-F", f"number={number}", "-f", _project_items_query(field)
    )
    items = _get(project, "data", "repository", "issue", "projectItems")
    nodes = _get(items, "nodes")
    if _get(project, "errors") or _get(items, "pageInfo", "hasNextPage") is not False:
        operation.fail(ERROR, f"incomplete project {purpose}")
    if not (
        isinstance(nodes, list)
        and all(
            isinstance(_get(node, "id"), str)
            and node["id"]
            and isinstance(_get(node, "project", "id"), str)
            and node["project"]["id"]
            for node in nodes
        )
    ):
        operation.fail(ERROR, "malformed project item")
    return [node for node in nodes if node["project"]["id"] == PROJECT]


def _backlog(operation: _Operation, item_id: str, priority: str) -> Any:
    """The issue's one Backlog item, which must carry the requested priority."""
    backlog = _backlog_items(operation, operation.number, "Priority", "readback")
    if not (
        len(backlog) == 1
        and backlog[0]["id"] == item_id
        and _get(backlog[0], "priority", "name") == priority
    ):
        operation.fail(ERROR, "Backlog/Priority readback mismatch")
    return backlog[0]


def _reporting(operation: _Operation, body: Callable[[], None]) -> None:
    """Runs `body`; on any failure, names the issue to inspect, as no change
    is rolled back.

    An unexpected exception is reported the same way before it propagates:
    the tracker may already hold a partial write.
    """
    try:
        body()
    except Exception:
        print(
            f"Issue operation failed; inspect {REPO}/issues/{operation.number} "
            "before retrying (no rollback).",
            file=sys.stderr,
            flush=True,
        )
        raise


def create(decisions: Decisions, blockers: Sequence[str]) -> None:
    operation = _Operation("issue-create")

    def body() -> None:
        request = _request(operation, decisions)
        if not _blockers_valid(blockers, ""):
            operation.fail(VIOLATED, "Invalid/self blocker")
        mutated = operation.gh(
            "api",
            "--method",
            "POST",
            f"repos/{REPO}/issues",
            "--input",
            "-",
            stdin=request.encoded(),
        )
        _finish(operation, request, mutated, blockers)

    _reporting(operation, body)


def edit(number: str, decisions: Decisions, blockers: Sequence[str]) -> None:
    operation = _Operation("issue-edit", number)

    def body() -> None:
        if not POSITIVE.fullmatch(number):
            operation.fail(VIOLATED, "Expected issue number")
        request = _request(operation, decisions)
        if not _blockers_valid(blockers, number):
            operation.fail(VIOLATED, "Invalid/self blocker")
        mutated = operation.gh(
            "api",
            "--method",
            "PATCH",
            f"repos/{REPO}/issues/{number}",
            "--input",
            "-",
            stdin=request.encoded(),
        )
        if _get(mutated, "number") != int(number):
            operation.fail(VIOLATED, f"the edit did not answer as issue #{number}")
        _finish(operation, request, mutated, blockers)

    _reporting(operation, body)


def _status(operation: _Operation, number: str) -> str:
    """The issue's Backlog Status, or "" when it has no item or no status."""
    backlog = _backlog_items(operation, number, "Status", "inventory")
    if not backlog:
        return ""
    if len(backlog) > 1:
        operation.fail(ERROR, "ambiguous Backlog item")
    if "status" not in backlog[0]:
        operation.fail(ERROR, "missing status evidence")
    if backlog[0]["status"] is None:
        return ""
    name = _get(backlog[0], "status", "name")
    if not isinstance(name, str):
        operation.fail(ERROR, "invalid status")
    return name


def _open_blockers(operation: _Operation, number: str) -> list[Any]:
    """The numbers of the open issues blocking `number`, in order."""
    blockers = []
    for row in operation.inventory(
        f"repos/{REPO}/issues/{number}/dependencies/blocked_by?per_page=100"
    ):
        if row.get("state") == "open":
            blockers.append(row["number"])
        elif row.get("state") != "closed":
            operation.fail(ERROR, "invalid blocker state")
    return sorted(blockers)


def _issue_rows(operation: _Operation, milestone: Any) -> list[dict[str, Any]]:
    """The milestone's open issues, pull requests left out."""
    rows = []
    for row in operation.inventory(
        f"repos/{REPO}/issues?state=open&milestone={json.dumps(milestone)}&per_page=100"
    ):
        if "pull_request" in row:
            if not isinstance(row["pull_request"], dict):
                operation.fail(ERROR, "invalid PR discriminator")
            continue
        labels = row.get("labels")
        if not (
            isinstance(row.get("title"), str)
            and isinstance(row.get("html_url"), str)
            and isinstance(labels, list)
            and all(isinstance(_get(label, "name"), str) for label in labels)
        ):
            operation.fail(ERROR, "malformed issue inventory")
        rows.append(row)
    return rows


def _one(operation: _Operation, found: list[str], what: str) -> str | None:
    """The single piece of resume evidence, if any: several are ambiguous."""
    if len(found) > 1:
        operation.fail(ERROR, f"ambiguous {what}")
    return found[0] if found else None


def _issue_docs(operation: _Operation, number: str, kind: str, label: str) -> list[str]:
    """Regular files in docs/superpowers/`kind` named for issue `number`."""
    directory = Path("docs/superpowers", kind)
    if not directory.is_dir():
        if directory.exists():
            operation.fail(VIOLATED, f"{label} path is not a directory")
        return []
    return [
        str(path)
        for path in directory.iterdir()
        if path.is_file()
        and not path.is_symlink()
        and any(
            fnmatch.fnmatchcase(path.name, pattern)
            for pattern in [f"*issue-{number}-*.md", f"*issue-{number}.md"]
        )
    ]


def _plan_progress(plan: str) -> dict[str, int]:
    """How many of the outline's checklist items are ticked."""
    items = [
        line
        for line in Path(plan).read_text(errors="replace").split("\n")
        if re.match(r"\s*- \[[ xX]\]", line)
    ]
    return {
        "total": len(items),
        "done": sum(bool(re.search(r"\[[xX]\]", line)) for line in items),
    }


def _open_pr(operation: _Operation, branch: str) -> Any:
    """The branch's one open pull request, if any."""
    prs = operation.gh(
        "pr",
        "list",
        "--repo",
        REPO,
        "--head",
        branch,
        "--state",
        "open",
        "--limit",
        "2",
        "--json",
        "number,url,state",
    )
    if not (
        isinstance(prs, list)
        and len(prs) <= 1
        and all(
            _number(_get(pr, "number"))
            and isinstance(_get(pr, "url"), str)
            and _get(pr, "state") == "OPEN"
            for pr in prs
        )
    ):
        operation.fail(ERROR, "ambiguous/malformed open PRs")
    return prs[0] if prs else None


def _number(value: Any) -> bool:
    return isinstance(value, int | float) and not isinstance(value, bool)


def _resume_evidence(operation: _Operation, number: str) -> dict[str, Any]:
    """The issue's local branch, spec and outline, and the branch's open PR.

    Ambiguous evidence is refused rather than resolved by picking one.
    """
    branches = traced_git(
        "issue-branches",
        "for-each-ref",
        "--format=%(refname:short)",
        f"refs/heads/issue-{number}-*",
    )
    branch = _one(operation, branches.splitlines(), "issue branches")
    specs = _issue_docs(operation, number, "specs", "Specs")
    plans = _issue_docs(operation, number, "plans", "Outlines")
    spec = _one(operation, specs, "specs")
    plan = _one(operation, plans, "outlines")
    return {
        "branch": branch,
        "spec": spec,
        "plan": plan,
        "plan_progress": None if plan is None else _plan_progress(plan),
        "open_pr": None if branch is None else _open_pr(operation, branch),
    }


def discover(milestone: str) -> None:
    """Prints every open issue of `milestone` as a candidate or skipped.

    An issue is skipped as claimed (In Progress), as lacking a Backlog status,
    or as blocked by an open issue; every other issue is a candidate.
    """
    operation = _Operation("issue-discover")
    number = operation.milestone(milestone)
    candidates, skipped = [], []
    for row in _issue_rows(operation, number):
        issue = json.dumps(row["number"])
        status = _status(operation, issue)
        blockers = _open_blockers(operation, issue)
        reason = (
            "claimed"
            if status == "In Progress"
            else "missing-project-status"
            if not status
            else "blocked"
            if blockers
            else ""
        )
        if reason:
            skipped.append(
                {
                    "number": row["number"],
                    "title": row["title"],
                    "reason": reason,
                    "blocked_by": blockers,
                }
            )
            continue
        candidates.append(
            {
                "number": row["number"],
                "title": row["title"],
                "url": row["html_url"],
                "labels": [label["name"] for label in row["labels"]],
                "status": status,
                "blocked_by": blockers,
                "local": _resume_evidence(operation, issue),
            }
        )
    report = {
        "milestone": number,
        "candidates": sorted(candidates, key=lambda row: row["number"]),
        "skipped": sorted(skipped, key=lambda row: row["number"]),
    }
    print(json.dumps(report, indent=2, ensure_ascii=False))
