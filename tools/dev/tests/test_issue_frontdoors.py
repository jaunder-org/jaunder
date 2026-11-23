"""Issue front doors through `dev`, against a fake gh: never a real tracker write.

Discovery's local resume evidence comes from a real Git checkout and files.
"""

import json
import subprocess
import sys
from dataclasses import dataclass
from typing import Any

import pytest
from conftest import Checkout

# Records each call and answers from the scripted replies in order. Inventory
# replies model `gh --paginate --slurp`, never an unpaginated request.
GH = f"""#!{sys.executable}
import fnmatch, json, os, pathlib, signal, sys
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
if "raw" in replies[n]:
    sys.stdout.write(replies[n]["raw"])
else:
    print(json.dumps(replies[n].get("body"), separators=(",", ":")))
if replies[n].get("killed"):
    os.kill(os.getpid(), signal.SIGTERM)
if replies[n].get("exit", 0):
    print(replies[n].get("stderr", "simulated gh failure"), file=sys.stderr)
    sys.exit(replies[n]["exit"])
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

    def discover(self, milestone: str = "3") -> subprocess.CompletedProcess[bytes]:
        return self.dev("issue-discover", milestone)

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


def test_a_dependency_already_recorded_is_not_added_again(tracker: Tracker) -> None:
    replies = responses()
    replies[4:4] = [{"body": BLOCKER}, {"body": [[BLOCKER]]}, {"body": [[BLOCKER]]}]
    tracker.reply(replies)
    result = tracker.mutate("issue-create", None, "42")
    assert result.returncode == 0, result
    assert not any(
        "--method" in call["args"] and "POST" in call["args"]
        for call in tracker.calls()[2:]
    )


def test_a_created_issue_cannot_block_itself(tracker: Tracker) -> None:
    tracker.reply(responses())
    result = tracker.mutate("issue-create", None, "731")
    assert result.returncode == 1
    assert b"Invalid/self blocker" in result.stderr
    assert not any("dependencies" in " ".join(c["args"]) for c in tracker.calls())


def test_an_invalid_blocker_is_refused_before_creation(tracker: Tracker) -> None:
    tracker.reply(responses())
    result = tracker.mutate("issue-create", None, "x")
    assert result.returncode == 1
    assert all("--method" not in call["args"] for call in tracker.calls())


def test_a_title_that_is_not_a_conventional_commit_is_refused(
    tracker: Tracker,
) -> None:
    tracker.reply(responses())
    body = tracker.checkout.root.parent / "body.md"
    result = tracker.dev("issue-create", "preserve metadata", *DECISIONS[1:], str(body))
    assert result.returncode == 1
    assert b"Expected Conventional Commit title" in result.stderr
    assert tracker.calls() == []


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


# `item-add` racing the project's own auto-add of a new issue (#2007).
ALREADY_EXISTS = {
    "exit": 7,
    "stderr": "GraphQL: Content already exists in this project (addProjectV2ItemById)",
}
RACED_ITEM = {"id": "PVTI_1", "project": {"id": PROJECT}, "priority": None}


def raced(listing: Any) -> list[Any]:
    """The create replies with `item-add` failing, then the Backlog listing."""
    replies = responses()
    replies[2:3] = [ALREADY_EXISTS, listing]
    return replies


def test_an_issue_added_to_the_backlog_concurrently_uses_that_item(
    tracker: Tracker,
) -> None:
    tracker.reply(raced(project_items([RACED_ITEM])))
    result = tracker.mutate("issue-create")
    assert result.returncode == 0, result
    assert json.loads(result.stdout)["backlog"]["id"] == "PVTI_1"
    assert (
        b"dev: issue-create: item-add failed; using existing Backlog item PVTI_1\n"
        in result.stderr
    )
    calls = tracker.calls()
    assert calls[3]["args"][:4] == ["api", "graphql", "-F", "number=731"]
    assert calls[4]["args"][:2] == ["project", "item-edit"]
    assert calls[4]["args"][calls[4]["args"].index("--id") + 1] == "PVTI_1"


@pytest.mark.parametrize(
    ("listing", "code"),
    [
        (project_items([]), 7),
        (project_items([RACED_ITEM, RACED_ITEM | {"id": "PVTI_2"}]), 5),
    ],
)
def test_a_failed_add_without_exactly_one_backlog_item_fails(
    tracker: Tracker, listing: Any, code: int
) -> None:
    tracker.reply(raced(listing))
    result = tracker.mutate("issue-create")
    assert result.returncode == code
    assert result.stdout == b""
    assert not any("item-edit" in call["args"] for call in tracker.calls())


def test_a_signalled_add_does_not_fall_back(tracker: Tracker) -> None:
    replies = raced(project_items([RACED_ITEM]))
    replies[2] = {"killed": 1}
    tracker.reply(replies)
    result = tracker.mutate("issue-create")
    assert result.returncode == 128 + 15
    assert result.stdout == b""
    assert len(tracker.calls()) == 3, "no listing or edit after the signal"


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


def test_unreadable_replies_are_malformed_and_unreadable_body_files_unreadable(
    tracker: Tracker,
) -> None:
    # As measured from the shell front doors: jq -e exits 5 on a reply it
    # cannot parse, and 2 on a body file it cannot read.
    tracker.checkout.executable("gh", f"#!{sys.executable}\nprint('not json')\n")
    tracker.reply(responses())
    assert tracker.mutate("issue-create").returncode == 5
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


# Discovery. Expected outputs and statuses were measured from the shell
# discovery this front door keeps the behaviour of: every malformed, partial or
# ambiguous answer exits 5, a failed gh keeps its status, and a docs path that
# is not a directory exits 1.
BACKLOG_QUERY = (
    'query=query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){'
    "issue(number:$number){projectItems(first:100){nodes{id project{id} "
    'status:fieldValueByName(name:"Status"){... on '
    "ProjectV2ItemFieldSingleSelectValue{name}} "
    'priority:fieldValueByName(name:"Priority"){... on '
    "ProjectV2ItemFieldSingleSelectValue{name}}} pageInfo{hasNextPage}}}}}"
)
MILESTONES = {"body": [[{"number": 3, "title": "Developer tooling & DX"}]]}


def status(name: str | None, priority: str | None = None) -> dict[str, Any]:
    """An issue's one Backlog item with this Status and Priority."""
    return project_items(
        [
            {
                "id": "PVTI_1",
                "project": {"id": PROJECT},
                "status": None if name is None else {"name": name},
                "priority": None if priority is None else {"name": priority},
            }
        ]
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
        MILESTONES,
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
        status("Todo", "P2"),
        {"body": [[]]},
        status("In Progress", "P1"),
        {"body": [[]]},
        status("Todo"),
        {"body": [[{"number": 42, "state": "open"}]]},
    ]


def one_issue(project: Any, dependencies: Any = None) -> list[Any]:
    """Discovery of a milestone holding issue 731 alone."""
    return [
        MILESTONES,
        {"body": [[issue(731, "available", [])]]},
        project,
        dependencies or {"body": [[]]},
    ]


DISCOVERED = """\
{
  "milestone": 3,
  "candidates": [
    {
      "number": 731,
      "title": "available",
      "url": "https://github.com/jaunder-org/jaunder/issues/731",
      "labels": [
        "tooling"
      ],
      "status": "Todo",
      "priority": "P2",
      "blocked_by": [],
      "local": {
        "branch": null,
        "spec": null,
        "plan": null,
        "plan_progress": null,
        "open_pr": null
      }
    }
  ],
  "skipped": [
    {
      "number": 732,
      "title": "claimed",
      "reason": "claimed",
      "priority": "P1",
      "blocked_by": []
    },
    {
      "number": 733,
      "title": "blocked",
      "reason": "blocked",
      "priority": null,
      "blocked_by": [
        42
      ]
    }
  ]
}
"""


def inventory(endpoint: str) -> list[str]:
    return ["api", "--paginate", "--slurp", f"repos/jaunder-org/jaunder/{endpoint}"]


def project(number: int) -> list[str]:
    return ["api", "graphql", "-F", f"number={number}", "-f", BACKLOG_QUERY]


@pytest.mark.parametrize("milestone", ["3", "Developer tooling & DX"])
def test_discovery_is_read_only_and_skips_claimed_blocked_and_pull_requests(
    tracker: Tracker, milestone: str
) -> None:
    tracker.reply(discovery_responses())
    result = tracker.discover(milestone)
    assert result.returncode == 0, result
    assert result.stdout.decode() == DISCOVERED
    assert [call["args"] for call in tracker.calls()] == [
        inventory("milestones?state=open&per_page=100"),
        inventory("issues?state=open&milestone=3&per_page=100"),
        *(
            call
            for number in [731, 732, 733]
            for call in [
                project(number),
                inventory(f"issues/{number}/dependencies/blocked_by?per_page=100"),
            ]
        ),
    ]
    # Every gh call is its own traced step.
    gate = tracker.checkout.merged()["dev issue-discover"]["args"]
    assert gate["jaunder.status"] == "ok"


def test_discovery_consumes_every_page_before_filtering_closed_prerequisites(
    tracker: Tracker,
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
        [{"number": 43, "state": "open"}, {"number": 42, "state": "open"}],
    ]
    tracker.reply(replies)
    result = tracker.discover()
    assert result.returncode == 0, result
    report = json.loads(result.stdout)
    assert [candidate["number"] for candidate in report["candidates"]] == [731]
    assert report["skipped"][1] == {
        "number": 733,
        "title": "blocked",
        "reason": "blocked",
        "priority": None,
        "blocked_by": [42, 43],
    }


@pytest.mark.parametrize(
    "project",
    [project_items([{"id": "A", "project": {"id": "OTHER"}}]), status(None)],
)
def test_an_issue_without_a_backlog_status_is_skipped(
    tracker: Tracker, project: Any
) -> None:
    tracker.reply(one_issue(project))
    result = tracker.discover()
    assert result.returncode == 0, result
    assert json.loads(result.stdout) == {
        "milestone": 3,
        "candidates": [],
        "skipped": [
            {
                "number": 731,
                "title": "available",
                "reason": "missing-project-status",
                "priority": None,
                "blocked_by": [],
            }
        ],
    }


def test_candidates_come_out_by_priority_then_number(tracker: Tracker) -> None:
    # Listed out of number order, so only the number tiebreak orders the ties.
    priorities = {734: "P3", 736: None, 733: "P1", 731: "P3", 735: "P2", 732: None}
    tracker.reply(
        [
            MILESTONES,
            {"body": [[issue(number, "t", []) for number in priorities]]},
            *(
                reply
                for priority in priorities.values()
                for reply in [status("Todo", priority), {"body": [[]]}]
            ),
        ]
    )
    result = tracker.discover()
    assert result.returncode == 0, result
    assert [
        (row["number"], row["priority"])
        for row in json.loads(result.stdout)["candidates"]
    ] == [
        (733, "P1"),
        (735, "P2"),
        (731, "P3"),
        (734, "P3"),
        (732, None),
        (736, None),
    ]


def test_skipped_issues_stay_in_number_order_whatever_their_priority(
    tracker: Tracker,
) -> None:
    tracker.reply(
        [
            MILESTONES,
            {"body": [[issue(731, "t", []), issue(732, "t", [])]]},
            status("In Progress", "P4"),
            {"body": [[]]},
            status("In Progress", "P1"),
            {"body": [[]]},
        ]
    )
    result = tracker.discover()
    assert result.returncode == 0, result
    assert [
        (row["number"], row["priority"]) for row in json.loads(result.stdout)["skipped"]
    ] == [(731, "P4"), (732, "P1")]


def test_closed_blockers_do_not_block(tracker: Tracker) -> None:
    tracker.reply(
        one_issue(status("Todo"), {"body": [[{"number": 40, "state": "closed"}]]})
    )
    result = tracker.discover()
    assert result.returncode == 0, result
    report = json.loads(result.stdout)
    assert [candidate["number"] for candidate in report["candidates"]] == [731]
    assert report["skipped"] == []


def test_an_empty_milestone_has_no_candidates(tracker: Tracker) -> None:
    tracker.reply([MILESTONES, {"body": [[]]}])
    result = tracker.discover()
    assert result.returncode == 0, result
    assert (
        result.stdout
        == b'{\n  "milestone": 3,\n  "candidates": [],\n  "skipped": []\n}\n'
    )


@pytest.mark.parametrize(
    ("milestone", "reply", "code", "message"),
    [
        # Only open milestones are listed, so a closed one is unknown too.
        ("99", MILESTONES, 5, "milestone must match exactly one open title/number"),
        ("Closed", MILESTONES, 5, "milestone must match exactly one open title/number"),
        (
            "3",
            {"body": [[{"number": 3, "title": "x"}, {"number": 4, "title": "3"}]]},
            5,
            "milestone must match exactly one open title/number",
        ),
        ("3", {"body": [[{"number": 3, "title": ""}]]}, 5, "invalid milestone"),
        ("3", {"body": {"message": "not pages"}}, 5, "invalid REST pages"),
        ("3", {"body": []}, 5, "invalid REST pages"),
        (
            "3",
            {"body": [[{"number": 3, "title": "a"}], [{"number": 3, "title": "a"}]]},
            5,
            "invalid/repeated issue identity",
        ),
        ("3", {"exit": 1, "body": [[]]}, 1, "simulated gh failure"),
        ("3", {"exit": 4, "body": [[]]}, 4, "simulated gh failure"),
        ("3", {"raw": "not json\n"}, 5, "unreadable JSON"),
    ],
)
def test_an_unknown_closed_ambiguous_or_unreadable_milestone_is_refused(
    tracker: Tracker, milestone: str, reply: Any, code: int, message: str
) -> None:
    tracker.reply([reply])
    result = tracker.discover(milestone)
    assert result.returncode == code
    assert result.stdout == b""
    assert message.encode() in result.stderr
    assert len(tracker.calls()) == 1


ITEM = {"id": "PVTI_1", "project": {"id": PROJECT}}


@pytest.mark.parametrize(
    ("replies", "code", "message"),
    [
        (
            [MILESTONES, {"exit": 1, "body": [[]]}],
            1,
            "simulated gh failure",
        ),
        (
            [MILESTONES, {"body": [[], {"message": "bad later page"}]}],
            5,
            "invalid REST pages",
        ),
        (
            [MILESTONES, {"body": [[{"number": 731, "pull_request": None}]]}],
            5,
            "invalid PR discriminator",
        ),
        (
            [MILESTONES, {"body": [[{"number": 731, "html_url": "u", "labels": []}]]}],
            5,
            "malformed issue inventory",
        ),
        (
            [MILESTONES, {"body": [[issue(731, "t", [{"name": 1}])]]}],
            5,
            "malformed issue inventory",
        ),
        (
            [MILESTONES, {"body": [[issue(731, "a", [])], [issue(731, "a", [])]]}],
            5,
            "invalid/repeated issue identity",
        ),
        (
            one_issue({"body": {**project_items([])["body"], "errors": [{}]}}),
            5,
            "incomplete project inventory",
        ),
        (one_issue(project_items([], more=True)), 5, "incomplete project inventory"),
        (
            one_issue(project_items([{"id": "PVTI_1", "project": {}}])),
            5,
            "malformed project item",
        ),
        (one_issue({"raw": "garbage"}), 5, "unreadable JSON"),
        (one_issue(project_items([ITEM])), 5, "missing status evidence"),
        (
            one_issue(project_items([ITEM | {"status": {"name": 5}}])),
            5,
            "invalid status",
        ),
        (
            one_issue(project_items([ITEM | {"status": {"name": "Todo"}}])),
            5,
            "missing priority evidence",
        ),
        (one_issue(status("Todo", "P5")), 5, "invalid priority"),
        (
            one_issue(
                project_items(
                    [ITEM | {"status": {"name": "Todo"}, "priority": {"name": 2}}]
                )
            ),
            5,
            "invalid priority",
        ),
        # A skipped issue's priority is checked too.
        (one_issue(status("In Progress", "P5")), 5, "invalid priority"),
        (
            one_issue(project_items([ITEM | {"status": {"name": "In Progress"}}])),
            5,
            "missing priority evidence",
        ),
        (
            one_issue(
                project_items(
                    [ITEM | {"status": None}, ITEM | {"id": "B", "status": None}]
                )
            ),
            5,
            "ambiguous Backlog item",
        ),
        (
            one_issue(status("Todo"), {"body": [[{"number": 42, "state": "unknown"}]]}),
            5,
            "invalid blocker state",
        ),
    ],
)
def test_discovery_refuses_late_failures_and_incomplete_or_malformed_evidence(
    tracker: Tracker, replies: list[Any], code: int, message: str
) -> None:
    tracker.reply(replies)
    result = tracker.discover()
    assert result.returncode == code
    assert result.stdout == b""
    assert message.encode() in result.stderr


PLAN = "# Plan\n- [x] one\n  - [ ] two\n- [X] three\nnot - [ ] item\n"
OPEN_PR = {
    "number": 900,
    "url": "https://github.com/jaunder-org/jaunder/pull/900",
    "state": "OPEN",
}


def local(
    tracker: Tracker,
    /,
    branches: tuple[str, ...] = (),
    specs: tuple[str, ...] = (),
    plans: tuple[str, ...] = (),
) -> None:
    """Gives the checkout the issue branches and docs discovery reports."""
    root = tracker.checkout.root
    if branches:
        subprocess.run(
            ["git", "-C", str(root), "-c", "user.name=f", "-c", "user.email=f@f"]
            + ["commit", "--quiet", "--allow-empty", "-m", "base"],
            check=True,
        )
    for branch in branches:
        subprocess.run(["git", "-C", str(root), "branch", branch], check=True)
    for kind, names in [("specs", specs), ("plans", plans)]:
        for name in names:
            path = root / "docs" / "superpowers" / kind / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(PLAN)


RESUMED = """\
{
  "milestone": 3,
  "candidates": [
    {
      "number": 731,
      "title": "available",
      "url": "https://github.com/jaunder-org/jaunder/issues/731",
      "labels": [],
      "status": "Todo",
      "priority": null,
      "blocked_by": [],
      "local": {
        "branch": "issue-731-example",
        "spec": "docs/superpowers/specs/2026-01-01-issue-731-example.md",
        "plan": "docs/superpowers/plans/2026-01-01-issue-731.md",
        "plan_progress": {
          "total": 3,
          "done": 2
        },
        "open_pr": {
          "number": 900,
          "url": "https://github.com/jaunder-org/jaunder/pull/900",
          "state": "OPEN"
        }
      }
    }
  ],
  "skipped": []
}
"""
PR_LIST = [
    "pr",
    "list",
    "--repo",
    "jaunder-org/jaunder",
    "--head",
    "issue-731-example",
    "--state",
    "open",
    "--limit",
    "2",
    "--json",
    "number,url,state",
]


def test_discovery_reports_branch_docs_outline_progress_and_open_pr(
    tracker: Tracker,
) -> None:
    local(
        tracker,
        branches=("issue-731-example", "issue-7310-other"),
        specs=("2026-01-01-issue-731-example.md", "2026-01-01-issue-7310.md"),
        plans=("2026-01-01-issue-731.md",),
    )
    tracker.reply(one_issue(status("Todo")) + [{"body": [OPEN_PR]}])
    result = tracker.discover()
    assert result.returncode == 0, result
    assert result.stdout.decode() == RESUMED
    assert tracker.calls()[-1]["args"] == PR_LIST


def test_a_branch_without_an_open_pr_reports_none(tracker: Tracker) -> None:
    local(tracker, branches=("issue-731-example",))
    tracker.reply(one_issue(status("Todo")) + [{"body": []}])
    result = tracker.discover()
    assert result.returncode == 0, result
    candidate = json.loads(result.stdout)["candidates"][0]
    assert candidate["local"] == {
        "branch": "issue-731-example",
        "spec": None,
        "plan": None,
        "plan_progress": None,
        "open_pr": None,
    }


@pytest.mark.parametrize(
    ("evidence", "pr", "code", "message"),
    [
        (
            {"branches": ("issue-731-a", "issue-731-b")},
            [],
            5,
            "ambiguous issue branches",
        ),
        ({"specs": ("a-issue-731.md", "b-issue-731-x.md")}, [], 5, "ambiguous specs"),
        (
            {"plans": ("a-issue-731.md", "b-issue-731-x.md")},
            [],
            5,
            "ambiguous outlines",
        ),
        (
            {"branches": ("issue-731-example",)},
            [{"body": [OPEN_PR, OPEN_PR | {"number": 901}]}],
            5,
            "ambiguous/malformed open PRs",
        ),
        (
            {"branches": ("issue-731-example",)},
            [{"body": [OPEN_PR | {"state": "CLOSED"}]}],
            5,
            "ambiguous/malformed open PRs",
        ),
        (
            {"branches": ("issue-731-example",)},
            [{"exit": 1, "body": []}],
            1,
            "simulated gh failure",
        ),
    ],
)
def test_ambiguous_or_malformed_resume_evidence_is_refused(
    tracker: Tracker,
    evidence: dict[str, tuple[str, ...]],
    pr: list[Any],
    code: int,
    message: str,
) -> None:
    local(tracker, **evidence)
    tracker.reply(one_issue(status("Todo")) + pr)
    result = tracker.discover()
    assert result.returncode == code
    assert result.stdout == b""
    assert message.encode() in result.stderr


@pytest.mark.parametrize(
    ("kind", "message"),
    [
        ("specs", "Specs path is not a directory"),
        ("plans", "Outlines path is not a directory"),
    ],
)
def test_a_docs_path_that_is_not_a_directory_is_refused(
    tracker: Tracker, kind: str, message: str
) -> None:
    path = tracker.checkout.root / "docs" / "superpowers" / kind
    path.parent.mkdir(parents=True)
    path.write_text("")
    tracker.reply(one_issue(status("Todo")))
    result = tracker.discover()
    assert result.returncode == 1
    assert result.stdout == b""
    assert message.encode() in result.stderr
