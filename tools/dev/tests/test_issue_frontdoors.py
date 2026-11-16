"""Issue front doors through `dev`, against a fake gh: never a real tracker write.

Discovery runs the repository's shell scripts and jq, so only the filtered
`checks.dev-python` source may skip its tests; creation and editing run anywhere.
"""

import json
import shutil
import subprocess
import sys
from dataclasses import dataclass
from typing import Any

import pytest
from conftest import Checkout, require

# Records each call and answers from the scripted replies in order. Inventory
# replies model `gh --paginate --slurp`, never an unpaginated request.
GH = f"""#!{sys.executable}
import fnmatch, json, os, pathlib, sys
fixture = pathlib.Path(os.environ["FIXTURE"])
args = sys.argv[1:]
data = sys.stdin.read() if " --input - " in f" {{' '.join(args)}} " else ""
if data and os.environ.get("MUTATION_REMOVES"):
    pathlib.Path(os.environ["MUTATION_REMOVES"]).unlink()
with (fixture / "calls.jsonl").open("a") as calls:
    calls.write(json.dumps({{"args": args, "input": data}}) + "\\n")
inventories = ["repos/*/milestones[?]*", "repos/*/issues[?]*",
               "repos/*/dependencies/blocked_by[?]*"]
if any(fnmatch.fnmatchcase(arg, pattern) for arg in args for pattern in inventories):
    if "--paginate" not in args or "--slurp" not in args:
        print("inventory requires --paginate --slurp", file=sys.stderr)
        sys.exit(19)
n = int((fixture / "index").read_text())
(fixture / "index").write_text(str(n + 1))
replies = json.loads((fixture / "responses.json").read_text())
if n >= len(replies):
    print("unexpected gh call", file=sys.stderr)
    sys.exit(5)
print(json.dumps(replies[n].get("body"), separators=(",", ":")))
if replies[n].get("exit", 0):
    print("simulated gh failure", file=sys.stderr)
    sys.exit(1)
"""
DECISIONS = ["fix(tooling): preserve metadata", "Task", "3", "P2", "tooling,dx"]
BODY = "Keep <placeholder> exactly.\n"


@dataclass
class Tracker:
    checkout: Checkout

    def reply(self, responses: list[Any]) -> None:
        fixture = self.checkout.root.parent
        (fixture / "responses.json").write_text(json.dumps(responses))
        (fixture / "index").write_text("0")
        (fixture / "calls.jsonl").unlink(missing_ok=True)

    def dev(self, *args: str, **extra: str) -> subprocess.CompletedProcess[bytes]:
        fixture = {"FIXTURE": str(self.checkout.root.parent), **extra}
        return self.checkout.run(
            self.checkout.command(*args), self.checkout.environment(**fixture)
        )

    def mutate(
        self, command: str, number: str | None = None, *blockers: str, **extra: str
    ) -> subprocess.CompletedProcess[bytes]:
        body = self.checkout.root.parent / "body.md"
        return self.dev(
            command,
            *([number] if number else []),
            *DECISIONS,
            str(body),
            *blockers,
            **extra,
        )

    def discover(self) -> subprocess.CompletedProcess[bytes]:
        return self.dev("issue-discover", "3")

    def calls(self) -> list[dict[str, Any]]:
        path = self.checkout.root.parent / "calls.jsonl"
        if not path.exists():
            return []
        return [json.loads(line) for line in path.read_text().splitlines()]


@pytest.fixture
def tracker(checkout: Checkout) -> Tracker:
    checkout.executable("gh", GH)
    (checkout.root.parent / "body.md").write_text(BODY)
    return Tracker(checkout)


@pytest.fixture
def discovery(tracker: Tracker) -> Tracker:
    require(shutil.which("jq") is not None, "jq")
    tracker.checkout.from_repository(
        "scripts/issue-metadata.sh", "scripts/issue-discover.sh"
    )
    return tracker


def readback() -> dict[str, Any]:
    return {
        "number": 731,
        "html_url": "https://github.com/jaunder-org/jaunder/issues/731",
        "title": "fix(tooling): preserve metadata",
        "body": BODY,
        "type": {"name": "Task"},
        "milestone": {"number": 3},
        "labels": [{"name": "dx"}, {"name": "tooling"}],
    }


def project_items(nodes: list[Any], more: bool = False) -> dict[str, Any]:
    return {
        "body": {
            "data": {
                "repository": {
                    "issue": {
                        "projectItems": {
                            "nodes": nodes,
                            "pageInfo": {"hasNextPage": more},
                        }
                    }
                }
            }
        }
    }


PROJECT = "PVT_kwDOECw7os4BblPP"


def responses() -> list[Any]:
    return [
        {"body": [[{"number": 3, "title": "Developer tooling & DX"}]]},
        {
            "body": {
                "number": 731,
                "html_url": "https://github.com/jaunder-org/jaunder/issues/731",
            }
        },
        {"body": {"id": "PVTI_1"}},
        {"body": {"id": "PVTI_1"}},
        {"body": readback()},
        project_items(
            [{"id": "PVTI_1", "project": {"id": PROJECT}, "priority": {"name": "P2"}}]
        ),
    ]


def test_edit_replaces_explicit_metadata_without_changing_issue_identity(
    tracker: Tracker,
) -> None:
    tracker.reply(responses())
    result = tracker.mutate("issue-edit", "731")
    assert result.returncode == 0, result
    assert tracker.calls()[1]["args"] == [
        "api",
        "--method",
        "PATCH",
        "repos/jaunder-org/jaunder/issues/731",
        "--input",
        "-",
    ]
    tracker.reply(responses())
    assert tracker.mutate("issue-edit", "732").returncode != 0
    assert len(tracker.calls()) == 2, "do not file a different issue in Backlog"


# Each bad reply, and the status each front door fails with (create, edit).
BAD_STEPS: list[tuple[int, Any, int, int]] = [
    (
        0,
        {
            "body": [
                [{"number": 3, "title": "Developer tooling & DX"}],
                [{"number": 3, "title": "duplicate"}],
            ]
        },
        5,
        5,
    ),
    (0, {"body": {"message": "not pages"}}, 5, 5),
    (1, {"body": {}}, 5, 1),
    (2, {"body": {}}, 4, 4),
    (3, {"exit": 1}, 1, 1),
    (3, {"body": {}}, 1, 1),
    (4, {"body": {"number": 731, "body": "partial"}}, 5, 5),
    (4, {"body": {**readback(), "body": "Keep  exactly.\n"}}, 5, 5),
    (5, {"body": {"errors": [{"message": "GraphQL failure"}]}}, 5, 5),
    (5, project_items([], more=True), 5, 5),
    (5, project_items([{"id": "PVTI_1", "project": {}}]), 5, 5),
    (5, project_items([]), 5, 5),
    (1, {"body": {"number": 731, "html_url": "https://example.invalid/731"}}, 4, 4),
]


@pytest.mark.parametrize("command", ["issue-create", "issue-edit"])
@pytest.mark.parametrize(("index", "bad", "create", "edit"), BAD_STEPS)
def test_mutations_fail_visibly_on_partial_malformed_or_mangled_evidence(
    tracker: Tracker, command: str, index: int, bad: Any, create: int, edit: int
) -> None:
    replies = responses()
    replies[index] = bad
    tracker.reply(replies)
    result = tracker.mutate(command, "731" if command == "issue-edit" else None)
    assert result.returncode == (create if command == "issue-create" else edit)
    assert result.stdout == b"", "must not emit a partial-success report"
    assert b"failed" in result.stderr


def dependency_replies(after_add: list[Any]) -> list[Any]:
    replies = responses()
    replies[4:4] = [
        {"body": {"id": 9001, "number": 42, "state": "open"}},
        {
            "body": [
                [{"id": 9000, "number": 40, "state": "closed"}],
                [{"id": 9002, "number": 43, "state": "open"}],
            ]
        },
        {"body": {"id": 9001, "number": 42, "state": "open"}},
        {"body": after_add},
    ]
    return replies


def test_explicit_dependencies_use_database_ids_and_verified_readback(
    tracker: Tracker,
) -> None:
    tracker.reply(
        dependency_replies(
            [
                [{"id": 9000, "number": 40, "state": "closed"}],
                [
                    {"id": 9002, "number": 43, "state": "open"},
                    {"id": 9001, "number": 42, "state": "open"},
                ],
            ]
        )
    )
    result = tracker.mutate("issue-create", None, "42")
    assert result.returncode == 0, result
    assert tracker.calls()[6]["args"] == [
        "api",
        "--method",
        "POST",
        "repos/jaunder-org/jaunder/issues/731/dependencies/blocked_by",
        "-F",
        "issue_id=9001",
    ]


def test_missing_dependency_readback_cannot_claim_success(tracker: Tracker) -> None:
    replies = dependency_replies([[]])
    replies[5] = {"body": [[]]}
    tracker.reply(replies)
    result = tracker.mutate("issue-create", None, "42")
    assert result.returncode == 1
    assert result.stdout == b""


def test_an_empty_body_is_refused_before_mutation(tracker: Tracker) -> None:
    tracker.reply(responses())
    (tracker.checkout.root.parent / "body.md").write_text(" \n")
    assert tracker.mutate("issue-create").returncode != 0
    assert len(tracker.calls()) == 1, "only read milestone metadata"


@pytest.mark.parametrize(
    ("issue_type", "priority", "labels"),
    [
        ("", "P2", "tooling"),
        ("Task", "P0", "tooling"),
        ("Task", "P2", ""),
        ("Task", "P2", "tooling,tooling"),
    ],
)
def test_missing_or_invalid_decisions_cannot_trigger_a_mutation(
    tracker: Tracker, issue_type: str, priority: str, labels: str
) -> None:
    tracker.reply(responses())
    body = tracker.checkout.root.parent / "body.md"
    result = tracker.dev(
        "issue-create", DECISIONS[0], issue_type, "3", priority, labels, str(body)
    )
    assert result.returncode != 0
    assert all("--method" not in call["args"] for call in tracker.calls())


def test_create_applies_explicit_metadata_and_checks_readback(
    tracker: Tracker,
) -> None:
    tracker.reply(responses())
    result = tracker.mutate("issue-create")
    assert result.returncode == 0, result
    report = json.loads(result.stdout)
    assert report["number"] == 731
    assert report["backlog"]["priority"]["name"] == "P2"
    calls = tracker.calls()
    assert json.loads(calls[1]["input"]) == {
        "title": "fix(tooling): preserve metadata",
        "type": "Task",
        "milestone": 3,
        "labels": ["tooling", "dx"],
        "body": BODY,
    }
    assert "item-add" in calls[2]["args"]
    assert "be01f9da" in calls[3]["args"]


def test_success_reports_the_readback_and_names_the_issue_to_verify(
    tracker: Tracker,
) -> None:
    tracker.reply(responses())
    result = tracker.mutate("issue-edit", "731")
    assert result.returncode == 0, result
    assert json.loads(result.stdout) == readback() | {
        "backlog": {
            "id": "PVTI_1",
            "project": {"id": PROJECT},
            "priority": {"name": "P2"},
        }
    }
    assert result.stderr.endswith(
        b"Verify issue #731: https://github.com/jaunder-org/jaunder/issues/731\n"
    )


@pytest.mark.parametrize(
    ("number", "blockers", "calls"),
    [("0", [], 0), ("731", ["731"], 1), ("731", ["x"], 1)],
)
def test_an_invalid_number_or_blocker_is_refused_before_mutation(
    tracker: Tracker, number: str, blockers: list[str], calls: int
) -> None:
    tracker.reply(responses())
    result = tracker.mutate("issue-edit", number, *blockers)
    assert result.returncode == 1
    assert len(tracker.calls()) == calls
    assert result.stderr.endswith(
        f"inspect jaunder-org/jaunder/issues/{number} before retrying "
        "(no rollback).\n".encode()
    )


@pytest.mark.parametrize("command", ["issue-create", "issue-edit"])
def test_the_body_file_is_read_once_before_the_mutation(
    tracker: Tracker, command: str
) -> None:
    # The readback is checked against the bytes that were sent, so the file
    # changing afterwards cannot fail an issue that was written as decided.
    tracker.reply(responses())
    body = tracker.checkout.root.parent / "body.md"
    number = "731" if command == "issue-edit" else None
    result = tracker.mutate(command, number, MUTATION_REMOVES=str(body))
    assert result.returncode == 0, result
    assert not body.exists()


def test_an_unexpected_failure_after_the_mutation_names_the_issue_to_inspect(
    tracker: Tracker,
) -> None:
    # A lone surrogate in a field nothing checks cannot be printed as UTF-8.
    replies = responses()
    replies[4] = {"body": readback() | {"note": "\ud800"}}
    tracker.reply(replies)
    result = tracker.mutate("issue-create")
    assert result.returncode == 1
    assert result.stdout == b""
    assert b"UnicodeEncodeError" in result.stderr
    assert (
        b"inspect jaunder-org/jaunder/issues/731 before retrying (no rollback).\n"
        in result.stderr
    )
    gate = tracker.checkout.merged()["dev issue-create"]["args"]
    assert (gate["jaunder.status"], gate["process.exit.code"]) == ("failed", 1)


def test_a_body_whose_bytes_change_on_readback_is_refused(tracker: Tracker) -> None:
    # Invalid UTF-8 reaches the tracker replaced; the readback text matches
    # what was sent, but not the file's bytes.
    (tracker.checkout.root.parent / "body.md").write_bytes(b"Keep \xff\n")
    replies = responses()
    replies[4] = {"body": readback() | {"body": "Keep \ufffd\n"}}
    tracker.reply(replies)
    result = tracker.mutate("issue-create")
    assert result.returncode == 1
    assert json.loads(tracker.calls()[1]["input"])["body"] == "Keep \ufffd\n"


def test_unreadable_replies_and_body_files_fail_as_unreadable(
    tracker: Tracker,
) -> None:
    tracker.checkout.executable("gh", f"#!{sys.executable}\nprint('not json')\n")
    tracker.reply(responses())
    assert tracker.mutate("issue-create").returncode == 2
    (tracker.checkout.root.parent / "body.md").unlink()
    tracker.checkout.executable("gh", GH)
    tracker.reply(responses())
    result = tracker.mutate("issue-create")
    assert result.returncode == 2
    assert len(tracker.calls()) == 1, "only read milestone metadata"


BLOCKER = {"id": 9001, "number": 42, "state": "open"}


@pytest.mark.parametrize(
    ("index", "bad", "code"),
    [
        (4, {"body": "not an issue"}, 5),
        (4, {"body": BLOCKER | {"state": "closed"}}, 4),
        (4, {"body": BLOCKER | {"pull_request": {}}}, 4),
        (5, {"body": [[{"id": "9000", "number": 40}]]}, 1),
        (6, {"body": {"id": 9001}}, 1),
    ],
)
def test_a_blocker_that_is_not_an_open_issue_or_not_recorded_fails(
    tracker: Tracker, index: int, bad: Any, code: int
) -> None:
    replies = dependency_replies([[BLOCKER]])
    replies[index] = bad
    tracker.reply(replies)
    result = tracker.mutate("issue-create", None, "42")
    assert result.returncode == code
    assert result.stdout == b""


def status(name: str) -> dict[str, Any]:
    return project_items(
        [{"id": "PVTI_1", "project": {"id": PROJECT}, "status": {"name": name}}]
    )


def issue(number: int, title: str, labels: list[Any]) -> dict[str, Any]:
    return {
        "number": number,
        "title": title,
        "html_url": f"https://github.com/jaunder-org/jaunder/issues/{number}",
        "labels": labels,
    }


def discovery_responses() -> list[Any]:
    return [
        {"body": [[{"number": 3, "title": "Developer tooling & DX"}]]},
        {
            "body": [
                [
                    issue(731, "available", [{"name": "tooling"}]),
                    issue(732, "claimed", []),
                    issue(733, "blocked", []),
                    {"number": 734, "pull_request": {}},
                ]
            ]
        },
        status("Todo"),
        {"body": [[]]},
        status("In Progress"),
        {"body": [[]]},
        status("Todo"),
        {"body": [[{"number": 42, "state": "open"}]]},
    ]


def test_discovery_is_read_only_and_skips_claimed_blocked_and_pull_requests(
    discovery: Tracker,
) -> None:
    discovery.reply(discovery_responses())
    result = discovery.discover()
    assert result.returncode == 0, result
    report = json.loads(result.stdout)
    assert [candidate["number"] for candidate in report["candidates"]] == [731]
    assert [skipped["reason"] for skipped in report["skipped"]] == [
        "claimed",
        "blocked",
    ]
    for call in discovery.calls():
        assert "--method" not in call["args"]
        assert not any(arg.startswith("query=mutation") for arg in call["args"])


def test_discovery_consumes_every_page_before_filtering_closed_prerequisites(
    discovery: Tracker,
) -> None:
    replies = discovery_responses()
    replies[0]["body"] = [
        [{"number": 2, "title": "Other milestone"}],
        [{"number": 3, "title": "Developer tooling & DX"}],
    ]
    issues = replies[1]["body"][0]
    replies[1]["body"] = [[issues[0]], issues[1:]]
    replies[7]["body"] = [
        [{"number": 40, "state": "closed"}],
        [{"number": 42, "state": "open"}],
    ]
    discovery.reply(replies)
    result = discovery.discover()
    assert result.returncode == 0, result
    report = json.loads(result.stdout)
    assert len(report["candidates"]) == 1
    assert len(report["skipped"]) == 2
    assert (report["skipped"][1]["number"], report["skipped"][1]["blocked_by"]) == (
        733,
        [42],
    )


@pytest.mark.parametrize(
    ("index", "row"),
    [
        (0, {"number": 3, "title": "Developer tooling & DX"}),
        (1, issue(731, "available", [])),
        (3, {"number": 42, "state": "closed"}),
    ],
)
def test_repeated_identities_across_pages_are_refused_before_filtering(
    discovery: Tracker, index: int, row: dict[str, Any]
) -> None:
    replies = discovery_responses()
    replies[index]["body"] = [[row], [row]]
    discovery.reply(replies)
    result = discovery.discover()
    assert result.returncode != 0
    assert result.stdout == b""
    assert b"invalid/repeated issue identity" in result.stderr


@pytest.mark.parametrize(
    ("index", "bad"),
    [
        (1, {"exit": 1, "body": [[]]}),
        (1, {"body": [[], {"message": "bad later page"}]}),
        (1, {"body": [[{"number": 731, "pull_request": None}]]}),
        (2, project_items([], more=True)),
        (2, project_items([{"id": "PVTI_1", "project": {"id": PROJECT}}])),
        (3, {"body": [[{"number": 42, "state": "unknown"}]]}),
    ],
)
def test_discovery_refuses_late_failures_and_incomplete_or_malformed_evidence(
    discovery: Tracker, index: int, bad: Any
) -> None:
    replies = discovery_responses()
    replies[index] = bad
    discovery.reply(replies)
    result = discovery.discover()
    assert result.returncode != 0
    assert result.stdout == b""
