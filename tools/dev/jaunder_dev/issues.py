"""Issue front doors: explicit decisions in, verified tracker state out.

Native `gh` owns every mutation; each reply is checked before the next call,
and the issue is read back in full before success is reported. Nothing is
rolled back: a failure names the issue to inspect before retrying.

Ported from the shell glue in `scripts/issue-metadata.sh`, which
`scripts/issue-discover.sh` still uses. Failures exit as that glue did: a
violated check made with `jq -e` exits 1, a jq `error(...)` 5, a selection
that yields nothing 4, unreadable JSON 2, and a failed `gh` its own status.
"""

import json
import re
import sys
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Any, NoReturn

from jaunder_dev import steps

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
PROJECT_ITEMS = (
    'query=query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){'
    "issue(number:$number){projectItems(first:100){nodes{id project{id} "
    'priority:fieldValueByName(name:"Priority"){... on '
    "ProjectV2ItemFieldSingleSelectValue{name}}} pageInfo{hasNextPage}}}}}"
)

# Exit statuses of the checks this replaces.
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
        reply = steps.output("gh", ["gh", *args], None, stdin)
        try:
            return json.loads(reply)
        except ValueError:
            self.fail(UNREADABLE, f"gh {args[0]} replied with unreadable JSON")

    def issues(self, endpoint: str) -> list[dict[str, Any]]:
        """Every page of a REST inventory, each row's issue identity unique.

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
        milestones = self.issues(f"repos/{REPO}/milestones?state=open&per_page=100")
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


class _Request:
    """An issue's explicit decisions, checked before any tracker write."""

    def __init__(self, operation: _Operation, decisions: Sequence[str]) -> None:
        title, issue_type, milestone, priority, labels, body_file = decisions
        if not TITLE.match(title):
            operation.fail(VIOLATED, "Expected Conventional Commit title")
        if issue_type not in ("Task", "Bug", "Feature"):
            operation.fail(VIOLATED, "Explicit type must be Task, Bug or Feature")
        if priority not in PRIORITY_OPTIONS:
            operation.fail(VIOLATED, "Explicit Priority must be P1/P2/P3/P4")
        self.priority = priority
        self.body_file = Path(body_file)
        milestone_number = operation.milestone(milestone)
        try:
            body = self.body_file.read_bytes().decode(errors="replace")
        except OSError as error:
            operation.fail(UNREADABLE, f"{body_file}: {error.strerror}")
        label_list = labels.split(",") if labels else []
        if not (
            label_list
            and all(label_list)
            and len(label_list) == len(set(label_list))
            and re.search(r"\S", body)
        ):
            operation.fail(ERROR, "explicit nonempty labels and body required")
        self.input = {
            "title": title,
            "type": issue_type,
            "milestone": milestone_number,
            "labels": label_list,
            "body": body,
        }

    def encoded(self) -> bytes:
        """The request as gh reads it from standard input."""
        return (json.dumps(self.input, indent=2, ensure_ascii=False) + "\n").encode()


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
    present = operation.issues(dependencies)
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
    if not any(row["id"] == blocker_id for row in operation.issues(dependencies)):
        operation.fail(VIOLATED, f"blocker {blocker} missing from readback")


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
    item_id = _get(item, "id")
    if not (isinstance(item_id, str) and item_id):
        operation.fail(NOTHING_SELECTED, "project item has no id")
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
    if readback["body"].encode() != request.body_file.read_bytes():
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


def _backlog(operation: _Operation, item_id: str, priority: str) -> Any:
    """The issue's one Backlog item, which must carry the requested priority."""
    project = operation.gh(
        "api", "graphql", "-F", f"number={operation.number}", "-f", PROJECT_ITEMS
    )
    items = _get(project, "data", "repository", "issue", "projectItems")
    nodes = _get(items, "nodes")
    if _get(project, "errors") or _get(items, "pageInfo", "hasNextPage") is not False:
        operation.fail(ERROR, "incomplete project readback")
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
    backlog = [node for node in nodes if node["project"]["id"] == PROJECT]
    if not (
        len(backlog) == 1
        and backlog[0]["id"] == item_id
        and _get(backlog[0], "priority", "name") == priority
    ):
        operation.fail(ERROR, "Backlog/Priority readback mismatch")
    return backlog[0]


def _reporting(operation: _Operation, body: Callable[[], None]) -> None:
    """Runs `body`; on any failure, names the issue to inspect, as no change
    is rolled back."""
    try:
        body()
    except steps.StepFailed:
        print(
            f"Issue operation failed; inspect {REPO}/issues/{operation.number} "
            "before retrying (no rollback).",
            file=sys.stderr,
            flush=True,
        )
        raise


def create(decisions: Sequence[str], blockers: Sequence[str]) -> None:
    operation = _Operation("issue-create")

    def body() -> None:
        request = _Request(operation, decisions)
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


def edit(number: str, decisions: Sequence[str], blockers: Sequence[str]) -> None:
    operation = _Operation("issue-edit", number)

    def body() -> None:
        if not POSITIVE.fullmatch(number):
            operation.fail(VIOLATED, "Expected issue number")
        request = _Request(operation, decisions)
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
