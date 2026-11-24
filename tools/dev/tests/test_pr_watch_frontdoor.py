"""`dev pr-watch` as a caller runs it: the real command against a scripted gh
in a throwaway Git checkout whose origin names the repository. Never a real
GitHub call.

`--once` keeps each observation to one poll; the loop's timing is proven in
process by `test_pr_watch.py`, whose reply vocabulary this shares.
"""

import inspect
import json
import subprocess
import sys
from dataclasses import dataclass
from typing import Any

import pytest
from conftest import Checkout
from test_pr_watch import (
    AGGREGATE,
    HEAD,
    MERGE,
    OTHER,
    TRANSPORT,
    URL,
    WORKFLOW,
    E,
    V,
    contents,
    green,
    job,
    merged,
    ok,
    pending,
    pr,
    route,
    rules,
    run,
    runs,
    status,
    workflow,
)

# Answers each call from its route's scripted replies in order, repeating the
# last, and records it. A reply's status -15 kills gh by SIGTERM instead.
GH = f"""#!{sys.executable}
import json, os, pathlib, signal, sys
{inspect.getsource(route)}
fixture = pathlib.Path(os.environ["FIXTURE"])
argv = ["gh", *sys.argv[1:]]
with (fixture / "calls.jsonl").open("a") as calls:
    calls.write(json.dumps(argv) + "\\n")
state = json.loads((fixture / "state.json").read_text())
name = route(argv)
replies = json.loads((fixture / "replies.json").read_text())[name]
code, stdout, stderr = replies[min(state.get(name, 0), len(replies) - 1)]
state[name] = state.get(name, 0) + 1
(fixture / "state.json").write_text(json.dumps(state))
sys.stdout.write(stdout)
sys.stderr.write(stderr)
sys.stdout.flush()
if code == -15:
    os.kill(os.getpid(), signal.SIGTERM)
sys.exit(code)
"""


@dataclass
class Observer:
    checkout: Checkout

    def git(self, *args: str) -> None:
        subprocess.run(["git", "-C", str(self.checkout.root), *args], check=True)

    def watch(self, *args: str, **routes: Any) -> subprocess.CompletedProcess[bytes]:
        """Runs `dev pr-watch ARGS` with the live ruleset unless scripted."""
        fixture = self.checkout.root.parent
        scripted = {"rules": [ok(rules(V, E))], "runs": [ok(runs())]} | {
            name: [
                reply if isinstance(reply, tuple) else ok(reply) for reply in replies
            ]
            for name, replies in routes.items()
        }
        (fixture / "replies.json").write_text(json.dumps(scripted))
        (fixture / "state.json").write_text("{}")
        (fixture / "calls.jsonl").write_text("")
        return self.checkout.dev("pr-watch", *args, FIXTURE=str(fixture))

    def calls(self) -> list[list[str]]:
        text = (self.checkout.root.parent / "calls.jsonl").read_text()
        return [json.loads(line) for line in text.splitlines()]


@pytest.fixture
def observer(checkout: Checkout) -> Observer:
    checkout.executable("gh", GH)
    found = Observer(checkout)
    found.git("remote", "add", "origin", "git@github.com:jaunder-org/jaunder.git")
    found.git("checkout", "-q", "-b", "issue-731-example")
    return found


def report(result: subprocess.CompletedProcess[bytes]) -> Any:
    assert result.stdout.endswith(b"\n") and result.stdout.count(b"\n") == 1, result
    return json.loads(result.stdout)


def test_help_documents_the_read_only_modes(observer: Observer) -> None:
    result = observer.watch("--help")
    assert result.returncode == 0, result
    text = " ".join(result.stdout.decode().split())
    assert "Never merges or mutates" in text
    for option in ["--once", "--until", "--expected-head", "--acknowledged-ejection"]:
        assert option in text
    assert "--observed-queued-head" in text
    assert "[default: 30; x>=5]" in text and "[default: 90; x>=1]" in text
    assert observer.calls() == []


@pytest.mark.parametrize(
    ("args", "message"),
    [
        (["731", "--reenqueue"], "No such option"),
        (["731", "--once", "--until", "merged"], "--once conflicts with --until"),
        (["731", "--until", "ready"], "'ready' is not 'merged'"),
        (["731", "--interval", "4"], "--interval"),
        (["731", "--timeout", "0"], "--timeout"),
        (["abc"], "NUMBER"),
        (["731", "--expected-head", "abc"], "full lowercase 40-hex"),
        (["731", "--expected-head", "A" * 40], "full lowercase 40-hex"),
        (["731", "--observed-queued-head", HEAD], "require --expected-head"),
        (
            ["731", "--acknowledged-ejection", "https://github.com/a/b/actions/runs/1"],
            "require --expected-head",
        ),
        (
            ["731", "--expected-head", HEAD, "--acknowledged-ejection", "not-a-url"],
            "a GitHub Actions run URL",
        ),
        (
            ["731", "--expected-head", HEAD, "--observed-queued-head", OTHER],
            "observed queued head must equal expected head",
        ),
    ],
)
def test_unsafe_arguments_are_refused_before_observation(
    observer: Observer, args: list[str], message: str
) -> None:
    result = observer.watch(*args)
    assert result.returncode == 2, result
    assert result.stdout == b""
    assert message in result.stderr.decode()
    assert observer.calls() == []


ONCE: list[tuple[Any, dict[str, Any], str, int]] = [
    (green(), {}, "ready-to-land", 0),
    (merged(), {}, "merged", 0),
    (pending(), {}, "pending", 1),
    (pr(status(V, "FAILURE"), status(E)), {}, "checks-failed", 1),
    (green(mergeable="CONFLICTING"), {}, "conflicted", 1),
    (green(state="CLOSED"), {}, "closed-unmerged", 1),
    (green(merge_state="BLOCKED"), {}, "blocked", 1),
    (green(merge_state="BEHIND"), {"rules": [rules(V, E, strict=True)]}, "stale", 1),
    (green(), {"rules": [rules()]}, "watcher-error", 1),
    (green(), {"runs": [runs(run())]}, "ejected", 1),
    (green(armed=True), {}, "pending", 1),
]


@pytest.mark.parametrize(("snapshot", "routes", "outcome", "code"), ONCE)
def test_each_outcome_has_its_exit_status_and_report(
    observer: Observer, snapshot: Any, routes: dict[str, Any], outcome: str, code: int
) -> None:
    result = observer.watch("731", "--once", pr=[snapshot], **routes)
    assert result.returncode == code, result
    found = report(result)
    assert (found["outcome"], found["pr"], found["head_sha"]) == (outcome, 731, HEAD)
    # Absent optional fields are omitted, never null.
    assert set(found) <= {"outcome", "pr", "head_sha", "phase", "detail"} | {
        "pointer",
        "events",
    }
    assert ("phase" in found) == (outcome == "pending")
    assert None not in found.values()
    terminal = [e["detail"] for e in found["events"] if e["kind"] == "terminal"]
    assert terminal == ([] if outcome == "pending" else [outcome])


def test_a_merged_report_carries_the_commit_and_time(observer: Observer) -> None:
    found = report(observer.watch("731", "--once", pr=[merged()]))
    assert (found["pointer"], found["detail"]) == (MERGE, "2026-07-30T15:00:00Z")


def test_the_expected_head_is_stale_even_when_merged(observer: Observer) -> None:
    result = observer.watch(
        "731", "--once", "--expected-head", HEAD, pr=[merged(OTHER)]
    )
    assert result.returncode == 1
    found = report(result)
    assert (found["outcome"], found["head_sha"]) == ("stale", HEAD)


def test_progress_reaches_stderr_as_it_happens(observer: Observer) -> None:
    result = observer.watch("731", "--once", pr=[pending()])
    lines = result.stderr.decode().splitlines()
    found = report(result)
    assert lines == [
        f"  {e['at']} [{e['kind']}] {e['detail']}" for e in found["events"]
    ]
    assert [e["kind"] for e in found["events"]] == ["phase", "check", "check"]


def test_the_pr_defaults_to_the_current_branch(observer: Observer) -> None:
    result = observer.watch("--once", list=[[{"number": 731}]], pr=[green()])
    assert result.returncode == 0, result
    assert observer.calls()[0] == [
        "gh",
        "pr",
        "list",
        "--head",
        "issue-731-example",
        "--state",
        "open",
        "--repo",
        "jaunder-org/jaunder",
        "--json",
        "number",
    ]


@pytest.mark.parametrize("reply", [ok([]), ok([{"number": None}])])
def test_a_branch_without_an_open_pr_is_refused(observer: Observer, reply: Any) -> None:
    result = observer.watch("--once", list=[reply])
    assert (result.returncode, result.stdout) == (2, b"")
    assert "no open PR for branch issue-731-example" in result.stderr.decode()


def test_a_checkout_without_an_origin_is_refused(observer: Observer) -> None:
    observer.git("remote", "remove", "origin")
    result = observer.watch("731", "--once")
    assert (result.returncode, result.stdout) == (2, b"")
    assert "no GitHub origin remote" in result.stderr.decode()
    assert observer.calls() == []


def test_an_unknown_pr_is_refused(observer: Observer) -> None:
    missing = (
        '{"data":{"repository":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND"}]}'
    )
    result = observer.watch("731", "--once", pr=[(1, missing, "")])
    assert (result.returncode, result.stdout) == (2, b"")
    assert "no such pull request: #731 in jaunder-org/jaunder" in result.stderr.decode()


@pytest.mark.parametrize(
    ("reply", "detail"),
    [
        ((127, "", "gh: command not found"), "`gh` is not on PATH"),
        (TRANSPORT, "transport: connection reset by peer"),
    ],
)
def test_broken_tooling_is_a_watcher_error_report(
    observer: Observer, reply: Any, detail: str
) -> None:
    result = observer.watch("731", "--once", pr=[reply])
    assert result.returncode == 1
    found = report(result)
    assert found["outcome"] == "watcher-error"
    assert found["detail"] == f"giving up after 1 failure(s): {detail}"


def test_a_terminated_gh_stops_the_watch_without_a_report(observer: Observer) -> None:
    result = observer.watch("731", "--once", pr=[(-15, "", "")])
    assert (result.returncode, result.stdout) == (143, b"")


def test_workflow_membership_decides_a_failure_outside_the_ruleset(
    observer: Observer,
) -> None:
    files = {
        f"contents:{WORKFLOW}": [contents(workflow("lane", AGGREGATE))],
        "contents:.github/workflows/other.yml": [contents(workflow("other"))],
    }
    snapshot = pr(
        job("other", workflow=".github/workflows/other.yml"),
        job("lane"),
        status(AGGREGATE, "PENDING"),
    )
    result = observer.watch(
        "731", "--once", pr=[snapshot], rules=[rules(AGGREGATE)], **files
    )
    found = report(result)
    assert (found["outcome"], found["pointer"]) == ("checks-failed", URL)
    assert found["detail"] == "required check failed: lane"
    warnings = [e["detail"] for e in found["events"] if e["kind"] == "warning"]
    assert warnings == [f"optional check failed: other ({URL})"]
    read = [call[2] for call in observer.calls() if "/contents/" in call[2]]
    assert read == [
        f"repos/jaunder-org/jaunder/contents/{path}?ref={HEAD}"
        for path in [".github/workflows/other.yml", WORKFLOW]
    ]
