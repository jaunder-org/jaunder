"""`dev pr-watch`'s rules, in process, against a scripted `gh` and a virtual clock.

The replies are GitHub's JSON shapes; a `gh` call is answered by what it asks
for (`route`), so a test scripts only the replies it cares about. The
decision tests parse the same shapes and then judge them with no I/O at all.
`test_pr_watch_frontdoor.py` drives the real command with the same vocabulary.
"""

import base64
import json
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import pytest
from conftest import REPOSITORY, require
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter

from dev import pr_watch, steps, trace
from dev.pr_watch import (
    FAILURE,
    PENDING,
    SUCCESS,
    GhError,
    Kind,
    Rules,
    Snapshot,
    Step,
    Watch,
    classify,
    decide,
)

HEAD = "a" * 40
MERGE = "b" * 40
OTHER = "c" * 40
V, E = "Validate (no e2e)", "e2e gate"
AGGREGATE = "Aggregate verdict"
QUEUE_RULES = Rules((V, E), strict=False, queue=True)
WORKFLOW = ".github/workflows/ci.yml"
OPTIONAL_WORKFLOW = ".github/workflows/diagnostics.yml"
ORIGIN = "https://github.com/jaunder-org/jaunder.git\n"
URL = "https://x/1"

Reply = tuple[int, str, str]


def ok(body: Any) -> Reply:
    return 0, json.dumps(body), ""


def failed(stderr: str = "", stdout: str = "", code: int = 1) -> Reply:
    return code, stdout, stderr


TRANSPORT = failed("connection reset by peer")
RATE_LIMITED = failed(stdout='{"message":"API rate limit exceeded","status":"403"}')
NOT_FOUND = failed(stdout='{"message":"Not Found","status":"404"}')


def status(name: str, state: str = "SUCCESS", at: str = "2026-07-30T14:10:00Z") -> Any:
    return {
        "__typename": "StatusContext",
        "context": name,
        "state": state,
        "targetUrl": URL,
        "createdAt": at,
    }


def job(
    name: str,
    conclusion: str | None = "FAILURE",
    run: int = 1,
    attempt: int = 1,
    completed: str = "2026-07-30T14:10:00Z",
    workflow: str = WORKFLOW,
    started: str = "2026-07-30T14:00:00Z",
) -> Any:
    """A GitHub Actions CheckRun; no conclusion means still running."""
    return {
        "__typename": "CheckRun",
        "name": name,
        "status": "COMPLETED" if conclusion else "IN_PROGRESS",
        "conclusion": conclusion,
        "detailsUrl": URL,
        "startedAt": started,
        "completedAt": completed if conclusion else None,
        "checkSuite": {
            "app": {"slug": "github-actions"},
            "workflowRun": {
                "databaseId": run,
                "runAttempt": attempt,
                "file": {"path": workflow},
            },
        },
    }


GREEN = (status(V), status(E, at="2026-07-30T14:20:00Z"))
PENDING_CHECKS = (status(V, "PENDING"), status(E, "PENDING"))


def pr(
    *checks: Any,
    state: str = "OPEN",
    head: str = HEAD,
    queued: bool = False,
    position: int | None = None,
    armed: bool = False,
    mergeable: str = "MERGEABLE",
    merge_state: str = "CLEAN",
    committed: str = "2026-07-30T13:00:00Z",
    cursor: str | None = None,
) -> Any:
    """The PR query's reply; `cursor` marks a page with another after it."""
    merged = state == "MERGED"
    return {
        "data": {
            "repository": {
                "pullRequest": {
                    "number": 731,
                    "state": state,
                    "baseRefName": "main",
                    "headRefName": "issue-731-example",
                    "mergedAt": "2026-07-30T15:00:00Z" if merged else None,
                    "mergeCommit": {"oid": MERGE} if merged else None,
                    "mergeable": mergeable,
                    "mergeStateStatus": merge_state,
                    "isInMergeQueue": queued,
                    "mergeQueueEntry": None
                    if position is None
                    else {"position": position},
                    "autoMergeRequest": {"enabledAt": "now"} if armed else None,
                    "commits": {
                        "nodes": [{"commit": {"oid": head, "committedDate": committed}}]
                    },
                    "statusCheckRollup": {
                        "contexts": {
                            "nodes": list(checks),
                            "pageInfo": {
                                "hasNextPage": cursor is not None,
                                "endCursor": cursor,
                            },
                        }
                    },
                }
            }
        }
    }


def green(**changes: Any) -> Any:
    return pr(*GREEN, **changes)


def pending() -> Any:
    return pr(*PENDING_CHECKS)


def merged(head: str = HEAD) -> Any:
    return green(state="MERGED", head=head, mergeable="UNKNOWN", merge_state="UNKNOWN")


def queued(position: int) -> Any:
    return green(queued=True, position=position)


def rules(*contexts: str, strict: bool = False, queue: bool = True) -> Any:
    rule = {
        "type": "required_status_checks",
        "parameters": {
            "strict_required_status_checks_policy": strict,
            "do_not_enforce_on_create": False,
            "required_status_checks": [{"context": context} for context in contexts],
        },
    }
    return [{"type": "deletion"}, rule] + ([{"type": "merge_queue"}] if queue else [])


def run(
    created: str = "2026-07-30T14:30:00Z",
    conclusion: str | None = "failure",
    run_id: int = 9,
    number: int = 731,
    workflow: str | None = None,
) -> Any:
    """A merge-group run; only one naming its `workflow` can be classified."""
    found = {
        "id": run_id,
        "html_url": f"https://github.com/o/r/actions/runs/{run_id}",
        "created_at": created,
        "head_branch": f"gh-readonly-queue/main/pr-{number}-base",
        "conclusion": conclusion,
    }
    return found if workflow is None else found | {"path": workflow}


def runs(*found: Any) -> Any:
    return {"total_count": len(found), "workflow_runs": list(found)}


def annotations(**checks: list[tuple[str, str, str | None]]) -> Any:
    """One commit's check runs, each with its (level, title, message)
    annotations."""
    nodes = [
        {
            "name": name,
            "annotations": {
                "nodes": [
                    {"annotationLevel": level, "title": title, "message": message}
                    for level, title, message in found
                ]
            },
        }
        for name, found in checks.items()
    ]
    suites = {"nodes": [{"checkRuns": {"nodes": nodes}}]}
    return {"data": {"repository": {"object": {"checkSuites": suites}}}}


def jobs(*conclusions: tuple[str, Any]) -> Any:
    return {"jobs": [{"name": n, "conclusion": c} for n, c in conclusions]}


def contents(source: str) -> Any:
    return {"encoding": "base64", "content": base64.b64encode(source.encode()).decode()}


def workflow(*names: str) -> str:
    """A workflow file defining one job per name, keyed by its position."""
    return "jobs:\n" + "".join(f"  j{i}:\n    name: {n}\n" for i, n in enumerate(names))


def route(argv: Sequence[str]) -> str:
    """Which scripted replies a native call takes, by what it asks for."""
    if argv[0] == "git":
        return " ".join(argv[1:])
    if argv[1] == "pr":
        # `gh pr list` resolves the subject; `gh pr merge` is `pr-land`'s arm.
        return argv[2]
    if argv[2] == "graphql":
        return "annotations" if any(a.startswith("oid=") for a in argv) else "pr"
    if argv[2].endswith("/rules/branches/main"):
        return "rules"
    if "/contents/" in argv[2]:
        # Each workflow file is its own route.
        return "contents:" + argv[2].partition("/contents/")[2].partition("?")[0]
    if argv[2].endswith("/logs"):
        # Each job's log is its own route.
        return "log:" + argv[2].split("/")[-2]
    return "jobs" if "/jobs?" in argv[2] else "runs"


@dataclass
class Native:
    """Scripted `gh` and `git`: each route answers its replies in order and
    then repeats its last one, so one reply can last a whole watch."""

    routes: dict[str, list[Reply]]
    calls: list[list[str]] = field(default_factory=list)

    def __call__(self, argv: Sequence[str]) -> Reply:
        self.calls.append(list(argv))
        replies = self.routes[route(argv)]
        return replies.pop(0) if len(replies) > 1 else replies[0]

    def asked(self, name: str) -> list[list[str]]:
        return [call for call in self.calls if route(call).startswith(name)]


def native(*snapshots: Any, **routes: Any) -> Native:
    """PR snapshots as replies, plus any other route; the ruleset defaults to
    the live one, a probe finds no merge-group run, and no commit has
    annotations."""
    scripted: dict[str, list[Reply]] = {
        "remote get-url origin": [(0, ORIGIN, "")],
        "rules": [ok(rules(V, E))],
        "runs": [ok(runs())],
        "annotations": [ok(annotations())],
        "pr": [reply if isinstance(reply, tuple) else ok(reply) for reply in snapshots],
    }
    for name, replies in routes.items():
        scripted[name] = [
            reply if isinstance(reply, tuple) else ok(reply) for reply in replies
        ]
    return Native(scripted)


@dataclass
class Clock:
    now: float = 0.0

    def time(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.now += seconds


@dataclass
class Result:
    code: int
    report: dict[str, Any]
    stderr: str

    @property
    def outcome(self) -> str:
        outcome: str = self.report["outcome"]
        return outcome

    def events(self, kind: str) -> list[str]:
        return [e["detail"] for e in self.report["events"] if e["kind"] == kind]


@pytest.fixture
def observe(capsys: pytest.CaptureFixture[str]) -> Any:
    """Runs `dev pr-watch` for PR 731 in process: its exit status, its report
    and its stderr."""

    def watch(fake: Native, clock: Clock | None = None, **options: Any) -> Result:
        clock = clock or Clock()
        options.setdefault("number", 731)
        try:
            pr_watch.run(Watch(**options), fake, clock.time, clock.sleep)
            code = 0
        except steps.StepFailed as failure:
            code = failure.code
        out, err = capsys.readouterr()
        return Result(code, json.loads(out) if out else {}, err)

    return watch


def rules_for(*contexts: str) -> Rules:
    """A non-strict merge-queue ruleset requiring `contexts`."""
    return Rules(contexts, strict=False, queue=True)


def snap(body: Any) -> Snapshot:
    return pr_watch.parse_snapshot(body, 731)[0]


# ---- classify: what a gh call's status and streams mean ----


def test_success_parses_the_body() -> None:
    assert classify(0, '{"number":731}', "") == {"number": 731}


def error(code: int, stdout: str, stderr: str = "") -> GhError:
    with pytest.raises(GhError) as raised:
        classify(code, stdout, stderr)
    return raised.value


@pytest.mark.parametrize(
    ("code", "stdout", "stderr", "kind"),
    [
        # Captured from `gh api /repos/jaunder-org/jaunder/pulls/999999`.
        (
            1,
            (
                '{"message":"Not Found","documentation_url":"https://docs.github.com/'
                'rest/pulls/pulls#get-a-pull-request","status":"404"}'
            ),
            "gh: Not Found (HTTP 404)",
            Kind.NOT_FOUND,
        ),
        (
            1,
            '{"message":"API rate limit exceeded for user ID 1.","status":"403"}',
            "",
            Kind.RATE_LIMITED,
        ),
        (
            1,
            '{"message":"You have exceeded a secondary rate limit.","status":"403"}',
            "",
            Kind.RATE_LIMITED,
        ),
        (
            1,
            '{"message":"Resource not accessible by integration","status":"403"}',
            "",
            Kind.TRANSPORT,
        ),
        (1, '{"message":"Bad credentials","status":"401"}', "", Kind.UNAUTHENTICATED),
        (127, "", "gh: command not found", Kind.MISSING),
        (1, '{"message":"Server Error","status":"502"}', "", Kind.TRANSPORT),
        (1, '{"status":"500"}', "", Kind.TRANSPORT),
        (1, "", "connection reset by peer", Kind.TRANSPORT),
        (0, "not json", "", Kind.MALFORMED),
    ],
)
def test_each_gh_failure_has_its_own_kind(
    code: int, stdout: str, stderr: str, kind: Kind
) -> None:
    assert error(code, stdout, stderr).kind is kind


def test_a_graphql_schema_error_on_stderr_is_a_graphql_error() -> None:
    # Captured: gh writes this to stderr with empty stdout.
    found = error(1, "", "gh: Field 'nosuchfield' doesn't exist on type 'Repository'\n")
    assert found.kind is Kind.GRAPHQL
    assert "nosuchfield" in found.detail


def test_a_graphql_errors_array_on_exit_zero_is_still_an_error() -> None:
    found = error(0, '{"data":null,"errors":[{"message":"Something went wrong"}]}')
    assert found.kind is Kind.GRAPHQL
    assert "Something went wrong" in found.detail


@pytest.mark.parametrize("code", [0, 1])
def test_graphql_not_found_is_a_missing_subject_not_a_query_failure(code: int) -> None:
    # Captured from `gh api graphql` for a nonexistent PR.
    reply = (
        '{"data":{"repository":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND",'
        '"path":["repository","pullRequest"],"message":"Could not resolve to a '
        'PullRequest with the number of 999999."}]}'
    )
    assert error(code, reply).kind is Kind.NOT_FOUND


def test_a_long_error_body_is_truncated_for_display() -> None:
    blob = "…" * 400
    assert error(1, "", blob).detail == f"transport: {'…' * 300}…"
    assert error(0, blob).kind is Kind.MALFORMED


def test_transience_partitions_the_kinds() -> None:
    transient = {kind for kind in Kind if GhError(kind).transient}
    assert transient == {Kind.RATE_LIMITED, Kind.TRANSPORT, Kind.MALFORMED}
    assert GhError(Kind.RATE_LIMITED).detail == "rate limited"
    assert GhError(Kind.MISSING).detail == "`gh` is not on PATH"
    assert GhError(Kind.UNAUTHENTICATED).detail == "`gh` is not authenticated"


# ---- parsing: GitHub's JSON into evidence ----


LIVE_RULES = [
    {"type": "deletion", "ruleset_id": 18086446},
    {"type": "pull_request", "parameters": {"allowed_merge_methods": ["merge"]}},
    {
        "type": "required_status_checks",
        "parameters": {
            "strict_required_status_checks_policy": False,
            "do_not_enforce_on_create": False,
            "required_status_checks": [{"context": V}, {"context": E}],
        },
    },
    {"type": "merge_queue", "parameters": {"merge_method": "MERGE"}},
]


def test_required_checks_come_from_the_ruleset_not_a_hardcoded_list() -> None:
    assert pr_watch.parse_rules(LIVE_RULES) == Rules((V, E), strict=False, queue=True)


def test_a_strict_rollback_ruleset_parses_as_strict_without_a_queue() -> None:
    strict = json.loads(json.dumps(LIVE_RULES[:3]).replace("false,", "true,", 1))
    assert pr_watch.parse_rules(strict) == Rules((V, E), strict=True, queue=False)


def test_required_checks_collect_every_applicable_rule_and_strict_policy() -> None:
    parsed = pr_watch.parse_rules(
        [
            rules("first")[1],
            {"type": "merge_queue"},
            rules("second", "first", strict=True)[1],
        ]
    )
    assert parsed == Rules(("first", "second"), strict=True, queue=True)


VALID_RULE: Any = rules("retained")[1]
BAD_CHECK_LISTS: list[Any] = [
    None,
    {},
    [{"context": "valid"}, {}],
    [{"context": 7}],
    [None],
    [{"context": ""}],
]
BAD_POLICIES: list[dict[str, Any]] = [
    {},
    {"strict_required_status_checks_policy": None},
    {"strict_required_status_checks_policy": "false"},
]


@pytest.mark.parametrize(
    "malformed",
    [
        {"type": "required_status_checks"},
        {"type": "required_status_checks", "parameters": None},
        {"type": "required_status_checks", "parameters": {}},
        {
            "type": "required_status_checks",
            "parameters": {"strict_required_status_checks_policy": False},
        },
        *(
            {
                "type": "required_status_checks",
                "parameters": {
                    "required_status_checks": checks,
                    "strict_required_status_checks_policy": False,
                },
            }
            for checks in BAD_CHECK_LISTS
        ),
        *(
            {
                "type": "required_status_checks",
                "parameters": {
                    "required_status_checks": [{"context": "valid"}],
                    **policy,
                },
            }
            for policy in BAD_POLICIES
        ),
        {"parameters": {}},
        {"type": 7},
        "not a rule",
    ],
)
def test_required_checks_reject_partial_or_malformed_rule_evidence(
    malformed: Any,
) -> None:
    # A retained context must never hide a missing requirement or policy.
    for value in [[malformed], [VALID_RULE, malformed]]:
        with pytest.raises(GhError) as raised:
            pr_watch.parse_rules(value)
        assert raised.value.kind is Kind.MALFORMED
    with pytest.raises(GhError):
        pr_watch.parse_rules({"rules": []})


def test_required_checks_allow_absent_optional_fields_and_empty_gate_evidence() -> None:
    parsed = pr_watch.parse_rules(
        [
            {
                "type": "required_status_checks",
                "parameters": {
                    "required_status_checks": [
                        {"context": "check", "integration_id": 123}
                    ],
                    "strict_required_status_checks_policy": False,
                },
            },
            rules(strict=True)[1],
        ]
    )
    # Strictness has no effect in a rule without checks.
    assert parsed == Rules(("check",), strict=False, queue=False)
    assert pr_watch.parse_rules([]) == Rules((), strict=False, queue=False)


def test_checks_flatten_both_union_members() -> None:
    parsed = snap(pr(status(V), job("lane", "SUCCESS", run=34984065696, attempt=2)))
    assert [check.name for check in parsed.checks] == [V, "lane"]
    assert all(check.state == SUCCESS for check in parsed.checks)
    assert (parsed.checks[1].kind, parsed.checks[1].run) == ("actions", 34984065696)
    assert (parsed.checks[1].attempt, parsed.checks[1].workflow) == (2, WORKFLOW)
    assert parsed.checks[0].kind == "status"


@pytest.mark.parametrize(
    ("conclusion", "state"),
    [("SUCCESS", SUCCESS), ("NEUTRAL", SUCCESS), ("SKIPPED", SUCCESS)]
    + [
        ("FAILURE", FAILURE),
        ("CANCELLED", FAILURE),
        ("TIMED_OUT", FAILURE),
        (None, PENDING),
    ],
)
def test_check_run_states(conclusion: str | None, state: str) -> None:
    assert snap(pr(job("lane", conclusion))).checks[0].state == state


@pytest.mark.parametrize(
    ("reported", "state"),
    [("SUCCESS", SUCCESS), ("FAILURE", FAILURE), ("ERROR", FAILURE)]
    + [("PENDING", PENDING), ("EXPECTED", PENDING)],
)
def test_status_context_states(reported: str, state: str) -> None:
    check = snap(pr(status(V, reported))).checks[0]
    assert check.state == state
    assert check.completed == (None if state == PENDING else "2026-07-30T14:10:00Z")


def test_another_apps_check_run_and_an_unknown_node_carry_no_actions_identity() -> None:
    other = job("external", "SUCCESS")
    other["checkSuite"] = {"app": {"slug": "circleci"}}
    parsed = snap(pr(other, {"__typename": "Unknown"}))
    assert [(check.kind, check.run) for check in parsed.checks] == [("check-run", None)]


@pytest.mark.parametrize(
    "workflow_run",
    [
        None,
        {"databaseId": None, "runAttempt": 1, "file": {"path": WORKFLOW}},
        {"databaseId": -1, "runAttempt": 1, "file": {"path": WORKFLOW}},
        {"databaseId": 42.5, "runAttempt": 1, "file": {"path": WORKFLOW}},
        {"databaseId": "42", "runAttempt": 1, "file": {"path": WORKFLOW}},
        {"databaseId": True, "runAttempt": 1, "file": {"path": WORKFLOW}},
    ],
)
def test_malformed_actions_run_identity_fails_closed(workflow_run: Any) -> None:
    node = job("lane")
    node["checkSuite"]["workflowRun"] = workflow_run
    with pytest.raises(GhError) as raised:
        snap(pr(node))
    assert raised.value.kind is Kind.MALFORMED


@pytest.mark.parametrize(
    "workflow_run",
    [{"databaseId": 42, "file": None}, {"databaseId": 42, "runAttempt": None}],
)
def test_an_actions_check_needs_no_attempt_or_workflow_file(workflow_run: Any) -> None:
    # GraphQL may leave both out; such a failure is only ever optional.
    node = job("lane")
    node["checkSuite"]["workflowRun"] = workflow_run
    (check,) = snap(pr(node)).checks
    assert (check.kind, check.run, check.attempt, check.workflow) == (
        "actions",
        42,
        None,
        None,
    )


def test_maximum_u64_actions_database_ids_remain_exact() -> None:
    assert snap(pr(job("lane", run=2**64 - 1))).checks[0].run == 2**64 - 1


def test_the_query_requests_the_run_identity_the_membership_rule_needs() -> None:
    assert "workflowRun{databaseId runAttempt file{path}}" in pr_watch.PR_QUERY
    assert "fullDatabaseId" not in pr_watch.PR_QUERY


def test_merged_and_queued_snapshots_carry_their_evidence() -> None:
    finished = snap(merged())
    assert (finished.state, finished.merge_commit) == ("MERGED", MERGE)
    assert finished.merged_at == "2026-07-30T15:00:00Z"
    entry = snap(queued(2))
    assert (entry.state, entry.queued, entry.position) == ("OPEN", True, 2)
    assert snap(green(armed=True)).armed
    assert not snap(green()).armed


def test_head_sha_and_committed_at_are_populated() -> None:
    parsed = snap(green())
    assert parsed.head == HEAD
    assert parsed.committed.isoformat() == "2026-07-30T13:00:00+00:00"


def test_unknown_merge_dimensions_parse_as_unknown() -> None:
    body = green()
    body["data"]["repository"]["pullRequest"]["mergeable"] = None
    del body["data"]["repository"]["pullRequest"]["mergeStateStatus"]
    parsed = snap(body)
    assert (parsed.mergeable, parsed.merge_state) == ("UNKNOWN", "UNKNOWN")


def mangle(body: Any, *path: Any, value: Any = ...) -> Any:
    """`body` with the field at `path` replaced, or removed without a value."""
    node = body
    for key in path[:-1]:
        node = node[key]
    if value is ...:
        del node[path[-1]]
    else:
        node[path[-1]] = value
    return body


PULL = ("data", "repository", "pullRequest")
COMMIT = (*PULL, "commits", "nodes", 0, "commit")
PAGE = (*PULL, "statusCheckRollup", "contexts", "pageInfo")


@pytest.mark.parametrize(
    ("path", "value", "message"),
    [
        (PULL[:2], None, "no pullRequest node"),
        (PULL, None, "no pullRequest node"),
        ((*PULL, "number"), 999, "number"),
        ((*PULL, "number"), True, "number"),
        ((*PULL, "state"), "UNKNOWN", "state"),
        ((*PULL, "isInMergeQueue"), None, "isInMergeQueue"),
        ((*PULL, "commits", "nodes"), [], "oid"),
        ((*COMMIT, "oid"), None, "oid"),
        ((*COMMIT, "committedDate"), None, "committedDate"),
        ((*COMMIT, "committedDate"), "2026-07-30", "committedDate"),
        ((*COMMIT, "committedDate"), "yesterday", "committedDate"),
        ((*PULL, "baseRefName"), None, "branch name"),
        ((*PULL, "headRefName"), "", "branch name"),
        ((*PULL, "statusCheckRollup"), ..., "rollup"),
        ((*PULL, "statusCheckRollup", "contexts", "pageInfo"), ..., "hasNextPage"),
        ((*PAGE, "hasNextPage"), None, "hasNextPage"),
        ((*PAGE, "hasNextPage"), "false", "hasNextPage"),
        ((*PAGE, "hasNextPage"), True, "endCursor"),
        ((*PULL, "statusCheckRollup", "contexts", "nodes"), None, "nodes"),
    ],
)
def test_a_reply_missing_load_bearing_evidence_is_malformed_not_defaulted(
    path: tuple[Any, ...], value: Any, message: str
) -> None:
    # A defaulted head, timestamp or rollup would read as a healthy PR.
    with pytest.raises(GhError) as raised:
        snap(mangle(green(), *path, value=value))
    assert raised.value.kind is Kind.MALFORMED
    assert message in raised.value.detail


def test_an_explicit_null_status_rollup_is_an_empty_last_page() -> None:
    body = mangle(green(), *PULL, "statusCheckRollup", value=None)
    assert pr_watch.parse_snapshot(body, 731) == (snap(pr()), None)
    assert pr_watch.parse_snapshot(green(cursor="next"), 731)[1] == "next"


LIVE_RUNS = runs(
    run("2026-07-30T20:59:26Z", "success", 30581543928, 727),
    run("2026-07-24T19:31:52Z", "success", 30120914770, 646),
    run("2026-07-24T18:22:35Z", "failure", 30116668286, 646),
    run("2026-07-24T18:00:58Z", "failure", 30115267074, 646),
)


def test_the_ejection_run_matches_on_the_branch_prefix_and_ignores_other_prs() -> None:
    found = pr_watch.parse_runs(LIVE_RUNS, 727)
    assert found is not None and found.id == 30581543928
    assert pr_watch.parse_runs(LIVE_RUNS, 999) is None
    assert pr_watch.parse_runs(LIVE_RUNS, 72) is None


def test_the_ejection_run_is_the_newest_and_its_conclusion_is_verbatim() -> None:
    # PR 646's newest run succeeded though two older ones failed; judging
    # them belongs to the decision rules.
    found = pr_watch.parse_runs(LIVE_RUNS, 646)
    assert found is not None
    assert (found.id, found.conclusion) == (30120914770, "success")
    assert "/actions/runs/" in found.url


@pytest.mark.parametrize(
    "fault",
    [{"id": None}, {"id": "9"}, {"html_url": None}, {"created_at": None}]
    + [{"created_at": "2026-09-15"}, {"conclusion": 42}],
)
def test_a_malformed_merge_group_run_fails_instead_of_reading_as_no_ejection(
    fault: dict[str, Any],
) -> None:
    for value in [runs(run() | fault), {"total_count": 0}]:
        with pytest.raises(GhError):
            pr_watch.parse_runs(value, 731)
    absent = run()
    del absent["conclusion"]
    with pytest.raises(GhError):
        pr_watch.parse_runs(runs(absent), 731)


def github(**routes: Any) -> tuple[pr_watch.GitHub, Native]:
    fake = native(**routes)
    return pr_watch.GitHub(fake, "o", "r", 731), fake


def test_a_completed_merge_group_run_needs_no_jobs_call() -> None:
    reader, fake = github(runs=[runs(run())])
    found = reader.ejection()
    assert found is not None and found.conclusion == "failure"
    assert fake.calls == [
        ["gh", "api", "repos/o/r/actions/runs?event=merge_group&per_page=100"]
    ]


@pytest.mark.parametrize(
    ("conclusion", "ejecting"),
    [("failure", True), ("timed_out", True), ("startup_failure", True)]
    + [(None, False), ("success", False), ("cancelled", False)],
)
def test_an_in_progress_merge_group_run_reports_its_first_ejecting_job(
    conclusion: str | None, ejecting: bool
) -> None:
    reader, fake = github(
        runs=[runs(run(conclusion=None))],
        jobs=[jobs(("Chromium E2E", None), ("Validation host", conclusion))],
    )
    found = reader.ejection()
    assert found is not None
    assert found.failed_job == ("Validation host" if ejecting else None)
    assert fake.asked("jobs") == [
        ["gh", "api", "repos/o/r/actions/runs/9/jobs?filter=latest&per_page=100&page=1"]
    ]


def test_in_progress_jobs_are_read_across_pages() -> None:
    reader, fake = github(
        runs=[runs(run(conclusion=None))],
        jobs=[jobs(*[("pending", None)] * 100), jobs(("late", "failure"))],
    )
    found = reader.ejection()
    assert found is not None and found.failed_job == "late"
    assert fake.asked("jobs")[1][2].endswith("&page=2")


@pytest.mark.parametrize(
    "page",
    [TRANSPORT, ok({"total_count": 0}), ok(jobs(("odd", 42))), ok({"jobs": ["odd"]})]
    + [ok({"jobs": [{"conclusion": "failure"}]})],
)
def test_in_progress_job_failures_propagate_instead_of_reading_as_no_ejection(
    page: Reply,
) -> None:
    reader, _ = github(runs=[runs(run(conclusion=None))], jobs=[page])
    with pytest.raises(GhError):
        reader.ejection()


@pytest.mark.parametrize(
    "url",
    [
        "git@github.com:jaunder-org/jaunder.git",
        "https://github.com/jaunder-org/jaunder.git",
        "https://github.com/jaunder-org/jaunder",
        "https://github.com/jaunder-org/jaunder/\n",
    ],
)
def test_remote_urls_parse_to_owner_and_repo(url: str) -> None:
    assert pr_watch.parse_remote(url) == ("jaunder-org", "jaunder")


@pytest.mark.parametrize(
    "url",
    ["not-a-remote", "", "https://github.com", "https://github.com/a/b/c", "x:/b"],
)
def test_a_remote_without_owner_and_repo_does_not_parse(url: str) -> None:
    assert pr_watch.parse_remote(url) is None


def test_no_hardcoded_repository_literal_in_the_module() -> None:
    # The repository comes from the origin remote. The needle is built so
    # this file cannot match itself.
    needle = "jaunder-org" + "/jaunder"
    assert needle not in Path(pr_watch.__file__).read_text()


# ---- membership: which workflow jobs a ruleset context can match ----


def test_job_names_are_literal_names_or_keys_when_unnamed() -> None:
    source = """
on: push
jobs:
  build:
    name: Validate (no e2e)
  e2e-gate:
    runs-on: ubuntu-latest
  matrix:
    name: e2e (${{ matrix.lane }})
    needs: build
  odd:
    name: 7
  bare: null
  gate:
    needs: [build, matrix, 7]
  loose:
    needs: {build: true}
"""
    definitions = pr_watch.job_definitions(source)
    assert definitions is not None
    needs = {definition.key: definition.needs for definition in definitions}
    assert needs == {
        "build": frozenset(),
        "e2e-gate": frozenset(),
        "matrix": {"build"},
        "bare": frozenset(),
        "gate": {"build", "matrix"},
    }
    assert pr_watch.job_names(definitions) == frozenset(
        {
            V,
            "e2e-gate",
            "e2e (${{ matrix.lane }})",
            "bare",
            "gate",
        }
    )


@pytest.mark.parametrize("source", ["jobs: [", "jobs: []", "- a list", ""])
def test_an_unparseable_workflow_has_no_jobs(source: str) -> None:
    assert pr_watch.job_definitions(source) is None


CI_FILE = f"contents:{WORKFLOW}"


ONE_JOB = ((pr_watch.Definition("j0", E),), None)


def test_a_workflow_is_read_once_per_path_and_head() -> None:
    reader, fake = github(**{CI_FILE: [contents(workflow(E))]})
    assert reader.workflow_jobs(WORKFLOW, HEAD) == ONE_JOB
    assert reader.workflow_jobs(WORKFLOW, HEAD) == ONE_JOB
    assert reader.workflow_jobs(WORKFLOW, OTHER) == ONE_JOB
    assert fake.asked("contents") == [
        ["gh", "api", f"repos/o/r/contents/{WORKFLOW}?ref={ref}"]
        for ref in [HEAD, OTHER]
    ]


UNDECODABLE = "its contents are not base64 UTF-8 text"


@pytest.mark.parametrize(
    ("reply", "problem"),
    [
        (NOT_FOUND, "it is unreadable: not found"),
        (ok({"message": "no content"}), UNDECODABLE),
        (ok({"content": base64.b64encode(b"\xff").decode()}), UNDECODABLE),
        (ok({"content": "%%%"}), "it is not a workflow"),
        (ok(contents("jobs: [")), "it is not a workflow"),
    ],
)
def test_an_unusable_workflow_has_no_jobs_and_says_why_once(
    reply: Reply, problem: str
) -> None:
    reader, fake = github(**{CI_FILE: [reply, ok(contents(workflow(E)))]})
    assert reader.workflow_jobs(WORKFLOW, HEAD) == ((), problem)
    assert reader.workflow_jobs(WORKFLOW, HEAD) == ((), problem)
    assert len(fake.asked("contents")) == 1


@pytest.mark.parametrize("reply", [TRANSPORT, RATE_LIMITED, (0, "not json", "")])
def test_a_transient_read_failure_is_retried_on_the_next_poll(reply: Reply) -> None:
    reader, fake = github(**{CI_FILE: [reply, ok(contents(workflow(E)))]})
    definitions, problem = reader.workflow_jobs(WORKFLOW, HEAD)
    assert definitions == () and problem is not None
    assert problem.startswith("it is unreadable: ")
    assert reader.workflow_jobs(WORKFLOW, HEAD) == ONE_JOB
    assert len(fake.asked("contents")) == 2


def test_a_file_outside_the_workflows_directory_is_never_read() -> None:
    reader, fake = github()
    assert reader.workflow_jobs(".github/actions/x/action.yml", HEAD) == (
        (),
        "it is outside .github/workflows/",
    )
    assert fake.asked("contents") == []


# ---- the decision rules ----


def test_a_merged_pr_is_terminal_merged_with_the_commit() -> None:
    step = decide(snap(merged()), QUEUE_RULES)
    assert (step.outcome, step.pointer) == ("merged", MERGE)
    assert step.detail == "2026-07-30T15:00:00Z"


def test_a_failing_required_check_is_checks_failed_with_its_url() -> None:
    step = decide(snap(pr(status(V, "FAILURE"), status(E, "PENDING"))), QUEUE_RULES)
    assert step == Step("terminal", "checks-failed", f"required check failed: {V}", URL)


def test_a_conflicting_pr_is_terminal_conflicted() -> None:
    assert (
        decide(snap(green(mergeable="CONFLICTING")), QUEUE_RULES).outcome
        == "conflicted"
    )


def test_a_closed_unmerged_pr_is_terminal() -> None:
    assert decide(snap(green(state="CLOSED")), QUEUE_RULES).outcome == "closed-unmerged"


def test_conflict_outranks_a_failed_check() -> None:
    body = pr(status(V, "FAILURE"), mergeable="CONFLICTING")
    assert decide(snap(body), QUEUE_RULES).outcome == "conflicted"


def test_all_required_green_is_ready_for_approval() -> None:
    assert decide(snap(green()), QUEUE_RULES) == Step("ready-to-land")


def test_an_absent_required_context_is_not_terminal() -> None:
    # A late-appearing aggregate satisfies nothing until it reports.
    assert decide(snap(pr(status(V))), QUEUE_RULES) == Step("awaiting-checks")
    assert decide(snap(pr()), QUEUE_RULES) == Step("awaiting-checks")


def test_a_membership_required_failure_is_terminal_before_the_aggregate_settles() -> (
    None
):
    failure = snap(pr(job("renamed lane"), status(AGGREGATE, "PENDING")))
    step = decide(failure, rules_for(AGGREGATE), required=failure.checks[:1])
    assert step == Step(
        "terminal", "checks-failed", "required check failed: renamed lane", URL
    )


def test_an_optional_failure_never_decides() -> None:
    body = pr(*GREEN, job("unrelated diagnostic"))
    assert decide(snap(body), QUEUE_RULES) == Step("ready-to-land")
    body = pr(*PENDING_CHECKS, job("unrelated diagnostic"))
    assert decide(snap(body), QUEUE_RULES) == Step("awaiting-checks")


def test_a_failing_non_required_check_does_not_fail_the_pr() -> None:
    body = pr(*GREEN, status("some-optional-lint", "FAILURE"))
    assert decide(snap(body), QUEUE_RULES) == Step("ready-to-land")


def test_a_duplicate_context_resolves_to_the_latest_completion() -> None:
    # A red original followed by a green re-run reads green.
    body = pr(status(V, "FAILURE"), status(V, at="2026-07-30T14:40:00Z"), status(E))
    check = pr_watch.resolve(snap(body).checks, V)
    assert check is not None and check.state == SUCCESS
    assert decide(snap(body), QUEUE_RULES) == Step("ready-to-land")


def test_a_pending_rerun_outranks_an_earlier_failure() -> None:
    body = pr(
        status(V, "FAILURE", "2026-07-30T14:40:00Z"), status(V, "PENDING"), status(E)
    )
    assert decide(snap(body), QUEUE_RULES) == Step("awaiting-checks")


def test_behind_is_stale_only_when_the_ruleset_is_strict() -> None:
    behind = snap(green(merge_state="BEHIND"))
    assert decide(behind, Rules((V, E), strict=True, queue=False)).outcome == "stale"
    assert decide(behind, QUEUE_RULES) == Step("ready-to-land")


def test_an_unrelated_same_named_pending_check_does_not_suppress_a_failure() -> None:
    body = pr(
        job("shared display name", run=1), job("shared display name", None, run=2)
    )
    (failure,) = pr_watch.resolved_failures(snap(body).checks)
    assert (failure.name, failure.run) == ("shared display name", 1)


def ejection(
    created: str = "2026-07-30T14:30:00Z",
    conclusion: str | None = "failure",
    failed_job: str | None = None,
) -> pr_watch.Run:
    found = pr_watch.parse_runs(runs(run(created, conclusion)), 731)
    assert found is not None
    return pr_watch.Run(
        found.id, found.url, found.created, found.conclusion, failed_job
    )


def test_a_failed_merge_group_run_newer_than_the_head_is_ejected_without_history() -> (
    None
):
    # Reachable with no sight of the queue entry: --once agrees with a watch.
    step = decide(snap(green()), QUEUE_RULES, ejection=ejection())
    assert step.outcome == "ejected"
    assert step.pointer == "https://github.com/o/r/actions/runs/9"
    assert step.detail == "the front-of-queue merge_group run failed"


def test_a_merge_group_run_older_than_the_head_is_not_an_ejection() -> None:
    # A freshly pushed head is not ejected by its predecessor's run.
    body = green(committed="2026-07-30T15:00:00Z")
    assert decide(snap(body), QUEUE_RULES, ejection=ejection()) == Step("ready-to-land")


def test_a_successful_merge_group_run_is_not_an_ejection() -> None:
    found = ejection(conclusion="success")
    assert decide(snap(green()), QUEUE_RULES, ejection=found) == Step("ready-to-land")


def test_an_in_progress_run_with_a_failed_job_is_ejected_not_dequeued() -> None:
    # #1982: the queue ejects on the first failed job.
    found = ejection(conclusion=None, failed_job="Validation host")
    step = decide(snap(green()), QUEUE_RULES, ejection=found, queued_head=HEAD)
    assert step.outcome == "ejected"
    assert "Validation host" in (step.detail or "")
    assert step.pointer == found.url


def test_an_in_progress_run_without_a_failed_job_leaves_the_removal_dequeued() -> None:
    found = ejection(conclusion=None)
    step = decide(snap(green()), QUEUE_RULES, ejection=found, queued_head=HEAD)
    assert step.outcome == "dequeued"


def test_an_in_progress_failed_job_older_than_the_head_is_not_ejected() -> None:
    body = green(committed="2026-07-30T15:00:00Z")
    found = ejection(conclusion=None, failed_job="Validation host")
    assert decide(snap(body), QUEUE_RULES, ejection=found) == Step("ready-to-land")
    # Nor can the stale run explain a same-head removal.
    step = decide(snap(body), QUEUE_RULES, ejection=found, queued_head=HEAD)
    assert step.outcome == "dequeued"


def test_a_vanished_same_head_queue_entry_is_dequeued() -> None:
    step = decide(snap(green()), QUEUE_RULES, queued_head=HEAD)
    assert step.outcome == "dequeued"
    assert "queue entry vanished" in (step.detail or "")
    assert "no failed current-head merge-group run" in (step.detail or "")


def test_a_vanished_queue_entry_with_pending_checks_keeps_waiting() -> None:
    assert decide(snap(pending()), QUEUE_RULES, queued_head=HEAD) == Step(
        "awaiting-checks"
    )


def test_queued_and_armed_prs_report_their_phases() -> None:
    assert decide(snap(queued(2)), QUEUE_RULES) == Step("queued")
    assert decide(snap(green(armed=True)), QUEUE_RULES) == Step("armed")


def test_an_observed_dequeue_outranks_the_current_blocked_status() -> None:
    body = green(merge_state="BLOCKED")
    assert decide(snap(body), QUEUE_RULES, queued_head=HEAD).outcome == "dequeued"


def test_the_ejection_probe_fires_only_when_green_open_and_unqueued() -> None:
    # Cost control: never during the long pre-green phase.
    assert pr_watch.needs_ejection_probe(snap(green()), QUEUE_RULES)
    for body in [pending(), queued(1), merged()]:
        assert not pr_watch.needs_ejection_probe(snap(body), QUEUE_RULES)
    assert not pr_watch.needs_ejection_probe(
        snap(green()), Rules((V, E), strict=False, queue=False)
    )


def test_an_empty_required_set_fails_closed_without_an_ejection_probe() -> None:
    empty = rules_for()
    assert not pr_watch.needs_ejection_probe(snap(green()), empty)
    assert decide(snap(green()), empty).outcome == "watcher-error"


@pytest.mark.parametrize("merge_state", ["CLEAN", "HAS_HOOKS", "UNSTABLE", "BEHIND"])
def test_a_landable_merge_state_is_ready(merge_state: str) -> None:
    body = green(merge_state=merge_state)
    assert decide(snap(body), QUEUE_RULES) == Step("ready-to-land")


@pytest.mark.parametrize("merge_state", ["BLOCKED", "DRAFT", "DIRTY"])
@pytest.mark.parametrize("mergeable", ["MERGEABLE", "UNKNOWN"])
def test_a_blocked_merge_state_is_blocked_even_with_unknown_mergeability(
    merge_state: str, mergeable: str
) -> None:
    step = decide(
        snap(green(merge_state=merge_state, mergeable=mergeable)), QUEUE_RULES
    )
    assert step.outcome == "blocked"
    assert merge_state in (step.detail or "")


@pytest.mark.parametrize(
    ("mergeable", "merge_state"), [("UNKNOWN", "CLEAN"), ("MERGEABLE", "UNKNOWN")]
)
def test_unknown_mergeability_waits_for_a_stable_verdict(
    mergeable: str, merge_state: str
) -> None:
    body = green(mergeable=mergeable, merge_state=merge_state)
    assert decide(snap(body), QUEUE_RULES) == Step("awaiting-mergeability")


def test_queue_history_from_an_old_head_does_not_dequeue_the_new_head() -> None:
    assert decide(snap(green()), QUEUE_RULES, queued_head=OTHER) == Step(
        "ready-to-land"
    )


def test_no_check_name_is_hardcoded() -> None:
    body = pr(status("Alpha", "FAILURE"))
    assert decide(snap(body), rules_for("Alpha")).outcome == "checks-failed"


def test_the_expected_head_outranks_every_other_verdict() -> None:
    for body in [
        merged(OTHER),
        green(head=OTHER),
        pr(status(V, "FAILURE"), head=OTHER),
    ]:
        step = decide(snap(body), QUEUE_RULES, expected_head=HEAD)
        assert step.outcome == "stale"
        assert step.detail == pr_watch.head_drift_detail(HEAD, OTHER)


# ---- the watch loop ----


def test_five_consecutive_api_failures_yield_watcher_error(observe: Any) -> None:
    result = observe(native(TRANSPORT))
    assert (result.code, result.outcome) == (1, "watcher-error")
    assert result.report["detail"] == (
        "giving up after 5 failure(s): transport: connection reset by peer"
    )
    assert len(result.events("poll-error")) == 5


def test_a_transient_failure_before_success_does_not_end_the_watch(
    observe: Any,
) -> None:
    result = observe(native(TRANSPORT, merged()))
    assert (result.code, result.outcome) == (0, "merged")


@pytest.mark.parametrize("reply", [RATE_LIMITED, ok({"data": None})])
def test_a_rate_limit_or_malformed_reply_is_a_strike_that_can_recover(
    observe: Any, reply: Reply
) -> None:
    clock = Clock()
    result = observe(native(reply, merged()), clock)
    assert result.outcome == "merged"
    assert len(result.events("poll-error")) == 1
    assert clock.now == 30


def test_rate_limits_spend_the_strike_budget(observe: Any) -> None:
    result = observe(native(RATE_LIMITED))
    assert result.outcome == "watcher-error"
    assert result.events("poll-error") == ["rate limited"] * 5


@pytest.mark.parametrize(
    "reply", [failed(stdout='{"status":"401"}'), failed("gh: bad query")]
)
def test_a_non_transient_failure_ends_the_watch_at_once(
    observe: Any, reply: Reply
) -> None:
    clock = Clock()
    result = observe(native(reply, merged()), clock)
    assert result.outcome == "watcher-error"
    assert len(result.events("poll-error")) == 1
    assert clock.now == 0


def test_any_failure_ends_a_one_shot_observation(observe: Any) -> None:
    result = observe(native(TRANSPORT, merged()), once=True)
    assert result.outcome == "watcher-error"
    assert result.report["head_sha"] == ""


def test_a_failed_ruleset_read_is_a_poll_error(observe: Any) -> None:
    result = observe(native(merged(), rules=[TRANSPORT, ok(rules(V, E))]))
    assert result.outcome == "merged"
    assert result.events("poll-error") == ["transport: connection reset by peer"]


def test_identical_consecutive_snapshots_emit_exactly_one_phase_event(
    observe: Any,
) -> None:
    result = observe(native(pending(), pending(), pending(), merged()))
    assert result.events("phase") == ["awaiting-checks"]


def test_a_queue_position_change_emits_per_change(observe: Any) -> None:
    result = observe(native(queued(3), queued(2), merged()))
    # The merge that ends the watch also takes the PR out of the queue.
    assert result.events("queue") == ["position 3", "position 2", "not queued"]


def test_a_non_required_check_changing_emits_no_check_event(observe: Any) -> None:
    noisy = pr(*GREEN, status("optional-lint", "FAILURE"), queued=True, position=3)
    result = observe(native(queued(3), noisy, merged()))
    assert result.events("check") == [f"{V}: success", f"{E}: success"]


def test_checks_render_as_words(observe: Any) -> None:
    result = observe(native(pr(status(V, "PENDING")), merged()))
    assert result.events("check") == [
        f"{V}: pending",
        f"{E}: not yet reported",
        f"{V}: success",
        f"{E}: success",
    ]


def test_the_poll_that_ends_the_watch_still_reports_its_check_changes(
    observe: Any,
) -> None:
    # #2085: a check settling on the poll that ends the watch is still reported.
    result = observe(native(pr(status(V, "SUCCESS")), green()))
    assert result.outcome == "ready-to-land"
    assert result.events("check") == [
        f"{V}: success",
        f"{E}: not yet reported",
        f"{E}: success",
    ]
    assert result.events("phase") == ["awaiting-checks"]
    last, before = result.report["events"][-1], result.report["events"][-2]
    assert (last["kind"], last["detail"]) == ("terminal", "ready-to-land")
    assert (before["kind"], before["detail"]) == ("check", f"{E}: success")


def test_an_adverse_final_poll_reports_the_check_that_ended_it(observe: Any) -> None:
    result = observe(native(pr(status(V, "PENDING")), pr(status(V, "FAILURE"))))
    assert result.outcome == "checks-failed"
    assert result.events("check")[-1] == f"{V}: failure"
    assert result.report["events"][-1]["kind"] == "terminal"


def test_leaving_the_queue_is_a_queue_event(observe: Any) -> None:
    result = observe(native(queued(1), pr(*PENDING_CHECKS), merged()))
    assert result.events("queue") == ["position 1", "not queued"]
    unpositioned = green(queued=True)
    result = observe(native(unpositioned, merged()))
    assert result.events("queue") == ["queued", "not queued"]


def test_an_ejected_pr_is_reported_through_the_loop(observe: Any) -> None:
    result = observe(native(green(), runs=[runs(run())]))
    assert (result.code, result.outcome) == (1, "ejected")
    assert result.report["pointer"] == "https://github.com/o/r/actions/runs/9"


def test_a_probe_failure_is_a_poll_error_never_a_silent_not_ejected(
    observe: Any,
) -> None:
    result = observe(native(green(), runs=[failed("probe down")]))
    assert result.outcome == "watcher-error"
    assert result.events("poll-error") == ["transport: probe down"] * 5


def test_ten_minutes_of_stasis_emits_one_heartbeat(observe: Any) -> None:
    # Poll k lands at t = 30(k-1); poll 21 is exactly t = 600.
    result = observe(native(*[pending()] * 21, merged()))
    assert result.events("heartbeat") == ["still awaiting-checks, no change for 10m"]


def test_budget_expiry_is_timed_out_once_anything_was_read(observe: Any) -> None:
    clock = Clock()
    result = observe(native(pending()), clock)
    assert result.outcome == "timed-out"
    assert result.report["detail"] == "the watch budget expired; GitHub never finished"
    assert result.events("terminal") == ["timed-out"]
    assert clock.now == 90 * 60


def test_budget_expiry_without_a_single_read_is_watcher_error(observe: Any) -> None:
    result = observe(native(TRANSPORT), timeout=1)
    assert result.outcome == "watcher-error"
    assert result.report["detail"] == (
        "the watch budget expired without a single successful read"
    )
    assert len(result.events("poll-error")) == 2


@pytest.mark.parametrize("once", [False, True])
def test_the_ready_handoff_returns_without_sleeping_and_once_agrees(
    observe: Any, once: bool
) -> None:
    clock = Clock()
    result = observe(native(green()), clock, once=once)
    assert (result.code, result.outcome) == (0, "ready-to-land")
    assert result.report["head_sha"] == HEAD
    assert "obtain approval" in result.report["detail"]
    assert result.events("terminal") == ["ready-to-land"]
    assert clock.now == 0


def test_a_passive_watch_crosses_ready_to_merged(observe: Any) -> None:
    result = observe(
        native(green(), green(armed=True), queued(2), merged()), until="merged"
    )
    assert (result.code, result.outcome) == (0, "merged")
    assert result.events("phase") == ["ready-to-land", "armed", "queued"]


PRESSURE = ("WARNING", "Duration pressure", "sqlite-chromium-1-of-3: 1 attempt(s)")


def test_a_ready_watch_reports_the_heads_duration_pressure(observe: Any) -> None:
    fake = native(
        green(),
        annotations=[
            annotations(
                lane=[PRESSURE, ("WARNING", "Other", "x")],
                summary=[("NOTICE", "Duration pressure", "y")],
            )
        ],
    )
    result = observe(fake)
    assert (result.code, result.outcome) == (0, "ready-to-land")
    assert result.events("warning") == ["lane: sqlite-chromium-1-of-3: 1 attempt(s)"]
    assert [call[-1] for call in fake.asked("annotations")] == [f"oid={HEAD}"]


def test_a_merged_watch_also_reports_the_merge_groups_pressure(observe: Any) -> None:
    fake = native(
        merged(),
        annotations=[annotations(), annotations(e2e=[PRESSURE])],
    )
    result = observe(fake)
    assert result.outcome == "merged"
    assert result.events("warning") == ["e2e: sqlite-chromium-1-of-3: 1 attempt(s)"]
    assert [call[-1] for call in fake.asked("annotations")] == [
        f"oid={HEAD}",
        f"oid={MERGE}",
    ]


def test_unreadable_annotations_are_a_warning_not_a_verdict(observe: Any) -> None:
    result = observe(native(green(), annotations=[TRANSPORT]))
    assert (result.code, result.outcome) == (0, "ready-to-land")
    [warning] = result.events("warning")
    assert warning.startswith(f"duration-pressure annotations of {HEAD} unread:")


def test_an_unfinished_watch_reads_no_annotations(observe: Any) -> None:
    fake = native(pending())
    observe(fake, once=True)
    assert fake.asked("annotations") == []


@pytest.mark.parametrize(
    ("body", "found"),
    [
        ({"data": {"repository": {"object": None}}}, []),
        (annotations(lane=[PRESSURE]), ["lane: sqlite-chromium-1-of-3: 1 attempt(s)"]),
    ],
)
def test_pressure_parsing(body: Any, found: list[str]) -> None:
    assert pr_watch.parse_pressure(body) == found


@pytest.mark.parametrize(
    "body",
    [
        {"data": {"repository": {"object": {}}}},
        annotations(lane=[("WARNING", "Duration pressure", None)]),
    ],
)
def test_malformed_pressure_replies_are_refused(body: Any) -> None:
    with pytest.raises(GhError):
        pr_watch.parse_pressure(body)


def test_queue_disappearance_confirms_merge_with_metadata(observe: Any) -> None:
    result = observe(native(queued(2), green(), merged()))
    assert result.outcome == "merged"
    assert result.report["detail"] == "2026-07-30T15:00:00Z"
    assert result.report["pointer"] == MERGE


@pytest.mark.parametrize(
    ("fresh", "expected"),
    [
        (green(state="CLOSED"), "closed-unmerged"),
        (green(head=OTHER), "ready-to-land"),
        (merged(OTHER), "merged"),
        (pr(status(V, "FAILURE"), status(E)), "checks-failed"),
        (green(), "dequeued"),
    ],
)
def test_the_dequeue_confirmation_judges_fresh_evidence(
    observe: Any, fresh: Any, expected: str
) -> None:
    assert observe(native(queued(2), green(), fresh)).outcome == expected


@pytest.mark.parametrize("fresh", [queued(1), pending()])
def test_a_queued_or_pending_confirmation_is_not_a_dequeue(
    observe: Any, fresh: Any
) -> None:
    assert observe(native(queued(2), green(), fresh, merged())).outcome == "merged"


def test_the_confirmation_preserves_fresh_ejection_evidence(observe: Any) -> None:
    fake = native(queued(2), green(), green(), runs=[runs(), runs(run())])
    assert observe(fake).outcome == "ejected"


def test_a_failed_confirmation_retries_only_within_the_poll_budget(
    observe: Any,
) -> None:
    result = observe(native(queued(2), green(), TRANSPORT, green(), merged()))
    assert result.outcome == "merged"
    assert len(result.events("poll-error")) == 1
    result = observe(native(queued(2), green(), ok({"data": None})), timeout=1)
    assert result.outcome == "timed-out"
    assert len(result.events("poll-error")) == 1


@pytest.mark.parametrize("field_name", ["state", "isInMergeQueue", "number"])
@pytest.mark.parametrize("value", [..., None, {}, "UNKNOWN", 999])
def test_a_malformed_confirmation_uses_the_poll_error_budget(
    observe: Any, field_name: str, value: Any
) -> None:
    confirmation = mangle(green(), *PULL, field_name, value=value)
    result = observe(native(queued(2), green(), confirmation))
    assert result.outcome == "watcher-error"
    assert len(result.events("poll-error")) == 5
    assert field_name in result.events("poll-error")[0]
    result = observe(native(queued(2), green(), confirmation), timeout=1)
    assert result.outcome == "timed-out"


def test_a_same_head_dequeue_returns_after_exactly_one_poll_interval(
    observe: Any,
) -> None:
    clock = Clock()
    result = observe(native(queued(2), green(), green(), merged()), clock)
    assert result.outcome == "dequeued"
    assert "queue entry vanished" in result.report["detail"]
    assert clock.now == 30


def test_a_same_head_queue_disappearance_waits_while_checks_are_pending(
    observe: Any,
) -> None:
    result = observe(native(queued(2), pending(), merged()))
    assert result.outcome == "merged"
    assert "awaiting-checks" in result.events("phase")


def test_a_new_head_resets_queue_history(observe: Any) -> None:
    replacement = green(head=OTHER, committed="2026-07-30T16:00:00Z")
    result = observe(native(queued(2), replacement, merged()), until="merged")
    assert result.outcome == "merged"


MODES: list[dict[str, Any]] = [{}, {"once": True}, {"until": "merged"}]


@pytest.mark.parametrize("mode", MODES)
def test_an_empty_required_set_fails_closed_in_every_watch_mode(
    observe: Any, mode: dict[str, Any]
) -> None:
    clock = Clock()
    result = observe(native(green(), rules=[rules()]), clock, **mode)
    assert (result.code, result.outcome) == (1, "watcher-error")
    assert clock.now == 0


@pytest.mark.parametrize("merge_state", ["CLEAN", "HAS_HOOKS", "UNSTABLE", "BEHIND"])
def test_every_landable_merge_status_obeys_each_watch_mode(
    observe: Any, merge_state: str
) -> None:
    landable = green(merge_state=merge_state)
    for mode in MODES[:2]:
        assert observe(native(landable), **mode).outcome == "ready-to-land"
    result = observe(native(landable, merged()), until="merged")
    assert result.outcome == "merged"
    assert "ready-to-land" in result.events("phase")


@pytest.mark.parametrize("merge_state", ["BLOCKED", "DRAFT", "DIRTY"])
@pytest.mark.parametrize("mode", MODES)
def test_every_blocked_merge_status_stops_every_watch_mode(
    observe: Any, merge_state: str, mode: dict[str, Any]
) -> None:
    result = observe(native(green(merge_state=merge_state)), **mode)
    assert (result.code, result.outcome) == (1, "blocked")


@pytest.mark.parametrize(
    ("mergeable", "merge_state"), [("UNKNOWN", "CLEAN"), ("MERGEABLE", "UNKNOWN")]
)
def test_unknown_merge_dimensions_wait_in_every_watch_mode(
    observe: Any, mergeable: str, merge_state: str
) -> None:
    unknown = green(mergeable=mergeable, merge_state=merge_state)
    result = observe(native(unknown), once=True)
    assert (result.code, result.outcome) == (1, "pending")
    assert result.report["phase"] == "awaiting-mergeability"
    for mode in [{}, {"until": "merged"}]:
        result = observe(native(unknown, merged()), **mode)
        assert result.outcome == "merged"
        assert "awaiting-mergeability" in result.events("phase")


def test_once_mode_returns_pending_without_looping(observe: Any) -> None:
    clock = Clock()
    result = observe(native(pending()), clock, once=True)
    assert (result.code, result.outcome) == (1, "pending")
    assert result.report["phase"] == "awaiting-checks"
    assert "detail" not in result.report and "pointer" not in result.report
    assert clock.now == 0


def test_once_mode_reaches_a_terminal_outcome_when_one_exists(observe: Any) -> None:
    result = observe(native(merged()), once=True)
    assert (result.code, result.outcome) == (0, "merged")
    assert "phase" not in result.report


def membership(*snapshots: Any, files: dict[str, str] | None = None) -> Native:
    """An aggregate-only ruleset, with these workflow files at the head and
    no other: by default, `ci.yml` defines the aggregate and one lane."""
    files = files or {WORKFLOW: workflow("Renamed validation lane", AGGREGATE)}
    fake = native(*snapshots, rules=[rules(AGGREGATE)])
    fake.routes |= {f"contents:{path}": [ok(contents(s))] for path, s in files.items()}
    fake.routes |= {"contents:.github/workflows/missing.yml": [NOT_FOUND]}
    return fake


ON_HOLD = status(AGGREGATE, "PENDING")


@pytest.mark.parametrize("name", ["Renamed validation lane", "Arbitrary browser arm"])
@pytest.mark.parametrize("mode", MODES)
def test_a_failure_in_a_workflow_defining_a_ruleset_context_stops_every_mode(
    observe: Any, name: str, mode: dict[str, Any]
) -> None:
    # The second job is not named in the file at all: its workflow's
    # membership alone makes it required, before the aggregate settles.
    result = observe(membership(pr(job(name), ON_HOLD)), **mode)
    assert (result.code, result.outcome) == (1, "checks-failed")
    assert result.report["detail"] == f"required check failed: {name}"
    assert result.report["pointer"] == URL


def test_an_unrelated_same_named_pending_job_does_not_suppress_a_required_failure(
    observe: Any,
) -> None:
    failing = job("Renamed validation lane", run=1)
    other_run = job("Renamed validation lane", None, run=2)
    result = observe(membership(pr(failing, other_run, ON_HOLD)))
    assert result.outcome == "checks-failed"


OPTIONAL_FILES = {
    WORKFLOW: workflow(AGGREGATE),
    OPTIONAL_WORKFLOW: workflow("Unrelated diagnostic", "Optional A", "Optional B"),
}


def optional(name: str, **changes: Any) -> Any:
    return job(name, workflow=OPTIONAL_WORKFLOW, **changes)


def test_an_optional_failure_is_warned_once_and_does_not_block_readiness(
    observe: Any,
) -> None:
    failure = optional("Unrelated diagnostic")
    fake = membership(
        pr(failure, ON_HOLD), pr(failure, status(AGGREGATE)), files=OPTIONAL_FILES
    )
    result = observe(fake)
    assert result.outcome == "ready-to-land"
    assert result.events("warning") == [
        f"optional check failed: Unrelated diagnostic ({URL})"
    ]


def test_a_non_actions_optional_failure_reads_no_workflow(observe: Any) -> None:
    advisory = status("External advisory", "FAILURE")
    advisory["targetUrl"] = None
    fake = membership(pr(advisory, ON_HOLD))
    result = observe(fake, once=True)
    assert result.outcome == "pending"
    assert result.events("warning") == ["optional check failed: External advisory"]
    assert fake.asked("contents") == []


def test_an_unreadable_workflow_is_optional_and_says_why_once(observe: Any) -> None:
    missing = ".github/workflows/missing.yml"
    lane = job("Renamed validation lane", workflow=missing)
    result = observe(membership(pr(lane, ON_HOLD), pr(lane, ON_HOLD), merged()))
    assert result.outcome == "merged"
    assert result.events("poll-error") == []
    assert result.events("warning") == [
        f"workflow {missing} has no usable jobs: it is unreadable: not found",
        f"optional check failed: Renamed validation lane ({URL})",
    ]


def test_a_then_b_optional_failures_warn_exactly_once_each(observe: Any) -> None:
    first = optional("Optional A")
    second = optional("Optional B", completed="2026-07-30T14:20:00Z")
    fake = membership(
        pr(first, ON_HOLD), pr(first, second, ON_HOLD), merged(), files=OPTIONAL_FILES
    )
    assert observe(fake).events("warning") == [
        f"optional check failed: Optional A ({URL})",
        f"optional check failed: Optional B ({URL})",
    ]


def test_an_optional_failure_rerun_warns_for_its_new_attempt(observe: Any) -> None:
    attempt = optional("Optional A")
    rerun = optional("Optional A", attempt=2, completed="2026-07-30T14:30:00Z")
    fake = membership(
        pr(attempt, ON_HOLD),
        pr(attempt, rerun, ON_HOLD),
        merged(),
        files=OPTIONAL_FILES,
    )
    assert len(observe(fake).events("warning")) == 2


def test_a_pending_or_successful_rerun_suppresses_a_superseded_failure(
    observe: Any,
) -> None:
    failure = job("Renamed validation lane")
    rerun = job("Renamed validation lane", None, started="2026-07-30T14:30:00Z")
    result = observe(membership(pr(failure, rerun, ON_HOLD)), once=True)
    assert result.outcome == "pending"
    success = job(
        "Renamed validation lane", "SUCCESS", completed="2026-07-30T14:40:00Z"
    )
    result = observe(membership(pr(failure, success, status(AGGREGATE))))
    assert result.outcome == "ready-to-land"


def test_a_new_head_warns_about_the_same_shaped_failure_again(observe: Any) -> None:
    advisory = status("External advisory", "FAILURE")
    fake = native(
        pr(advisory, status(AGGREGATE, "PENDING")),
        pr(advisory, status(AGGREGATE), head=OTHER, committed="2026-07-30T14:20:00Z"),
        merged(),
        rules=[rules(AGGREGATE)],
    )
    result = observe(fake, until="merged")
    assert result.outcome == "merged"
    assert len(result.events("warning")) == 2


def test_every_event_reaches_both_stderr_and_the_report(observe: Any) -> None:
    result = observe(native(pending(), merged()))
    lines = [
        f"  {e['at']} [{e['kind']}] {e['detail']}" for e in result.report["events"]
    ]
    assert result.stderr.splitlines() == lines
    assert result.report["events"][0]["at"] == "1970-01-01T00:00:00Z"
    assert result.report["events"][-1] == {
        "at": "1970-01-01T00:00:30Z",
        "kind": "terminal",
        "detail": "merged",
    }


def test_the_status_rollup_is_read_across_pages(observe: Any) -> None:
    fake = native(pr(status(V), cursor="next"), pr(status(E)))
    result = observe(fake)
    assert result.outcome == "ready-to-land"
    assert fake.asked("pr")[1][-2:] == ["-f", "after=next"]
    fake = native(pr(status(V), cursor="next"), pr(status(E), head=OTHER))
    result = observe(fake, once=True)
    assert result.events("poll-error") == [
        "malformed response: PR head changed while paginating status checks"
    ]


# ---- read-only continuation: expected head, queue history, acknowledgment ----


HISTORICAL = "https://github.com/o/r/actions/runs/9"
FRESH = "https://github.com/o/r/actions/runs/10"


def test_positive_queued_readback_survives_an_unqueued_first_poll(observe: Any) -> None:
    result = observe(
        native(green()), until="merged", expected_head=HEAD, observed_queued_head=HEAD
    )
    assert (result.outcome, result.report["head_sha"]) == ("dequeued", HEAD)
    # Without the readback, the default observation is unchanged.
    assert observe(native(green()), expected_head=HEAD).outcome == "ready-to-land"


@pytest.mark.parametrize(
    ("snapshot", "expected"),
    [(merged(), "merged"), (merged(OTHER), "stale"), (green(), "ejected")],
)
def test_queue_continuation_keeps_the_head_merge_and_fresh_failure_guards(
    observe: Any, snapshot: Any, expected: str
) -> None:
    fake = native(snapshot, runs=[runs(run(run_id=10))])
    result = observe(
        fake,
        expected_head=HEAD,
        acknowledged_ejection=HISTORICAL,
        observed_queued_head=HEAD,
    )
    assert result.outcome == expected
    assert result.report["head_sha"] == HEAD
    if expected == "ejected":
        assert result.report["pointer"] == FRESH


def test_acknowledgment_disregards_only_the_historical_run(observe: Any) -> None:
    acknowledged = {"expected_head": HEAD, "acknowledged_ejection": HISTORICAL}
    fake = native(green(), runs=[runs(run()), runs(run(run_id=10))])
    result = observe(fake, until="merged", **acknowledged)
    assert (result.outcome, result.report["pointer"]) == ("ejected", FRESH)
    fake = native(green(), green(), merged(), runs=[runs(run())])
    assert observe(fake, until="merged", **acknowledged).outcome == "merged"


def test_an_acknowledged_in_progress_run_with_a_failed_job_is_not_a_fresh_ejection(
    observe: Any,
) -> None:
    fake = native(
        green(),
        runs=[runs(run(conclusion=None))],
        jobs=[jobs(("Validation host", "failure"))],
    )
    result = observe(
        fake,
        expected_head=HEAD,
        acknowledged_ejection=HISTORICAL,
        observed_queued_head=HEAD,
    )
    assert result.outcome == "dequeued"


@pytest.mark.parametrize("replacement", [green(head=OTHER), merged(OTHER)])
def test_the_head_guard_outranks_even_a_merged_replacement(
    observe: Any, replacement: Any
) -> None:
    result = observe(native(replacement, green()), expected_head=HEAD)
    assert (result.code, result.outcome) == (1, "stale")
    assert result.report["head_sha"] == HEAD
    assert result.report["detail"] == pr_watch.head_drift_detail(HEAD, OTHER)


@pytest.mark.parametrize(
    ("snapshot", "expected"),
    [
        (green(), "ready-to-land"),
        (green(merge_state="BLOCKED"), "blocked"),
        (green(mergeable="UNKNOWN"), "pending"),
        (pr(status(V, "FAILURE"), status(E)), "checks-failed"),
    ],
)
def test_an_acknowledged_preflight_still_requires_readiness(
    observe: Any, snapshot: Any, expected: str
) -> None:
    fake = native(snapshot, runs=[runs(run())])
    result = observe(
        fake, once=True, expected_head=HEAD, acknowledged_ejection=HISTORICAL
    )
    assert result.outcome == expected


def test_one_shot_observation_returns_readiness_and_only_reads(observe: Any) -> None:
    fake = native(green())
    clock = Clock()
    result = observe(fake, clock, once=True)
    assert (result.outcome, result.report["pr"]) == ("ready-to-land", 731)
    assert result.report["events"]
    assert clock.now == 0
    # The remote, the ruleset, the PR, the ejection probe and the head's
    # annotations.
    assert [call[:2] for call in fake.calls] == [
        ["git", "remote"],
        ["gh", "api"],
        ["gh", "api"],
        ["gh", "api"],
        ["gh", "api"],
    ]


# ---- the subject, and what is never a verdict ----


def test_the_pr_defaults_to_the_current_branch(observe: Any) -> None:
    fake = native(
        green(),
        **{"branch --show-current": [(0, "feature\n", "")]},
        list=[[{"number": 731}]],
    )
    result = observe(fake, number=None)
    assert result.outcome == "ready-to-land"
    assert fake.asked("list") == [
        ["gh", "pr", "list", "--head", "feature", "--state", "open"]
        + ["--repo", "jaunder-org/jaunder", "--json", "number"]
    ]


@pytest.mark.parametrize(
    ("routes", "message"),
    [
        (
            {"remote get-url origin": [(2, "", "no such remote")]},
            "no GitHub origin remote",
        ),
        (
            {"remote get-url origin": [(0, "not-a-remote\n", "")]},
            "no GitHub origin remote",
        ),
        ({"branch --show-current": [(0, "\n", "")]}, "no current branch"),
        ({"branch --show-current": [(128, "", "fatal")]}, "no current branch"),
        ({"list": [ok([])]}, "no open PR for branch feature"),
        ({"list": [ok([{"number": "731"}])]}, "no open PR for branch feature"),
        ({"list": [NOT_FOUND]}, "no open PR for branch feature"),
    ],
)
def test_no_subject_is_a_refusal_with_no_report(
    observe: Any, routes: dict[str, Any], message: str
) -> None:
    fake = native(
        green(), **{"branch --show-current": [(0, "feature\n", "")], **routes}
    )
    result = observe(fake, number=None)
    assert (result.code, result.report) == (2, {})
    assert result.stderr.startswith(f"dev: pr-watch: {message}")


def test_an_unknown_pr_is_a_refusal_with_no_report(observe: Any) -> None:
    graphql = (
        '{"data":{"repository":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND"}]}'
    )
    result = observe(native(failed(stdout=graphql)))
    assert (result.code, result.report) == (2, {})
    assert "no such pull request: #731 in jaunder-org/jaunder" in result.stderr


def test_broken_tooling_while_resolving_is_a_watcher_error_report(
    observe: Any,
) -> None:
    # "The tooling broke" is more actionable than "no such PR".
    branch = {"branch --show-current": [(0, "feature\n", "")]}
    result = observe(native(green(), **branch, list=[TRANSPORT]), number=None)
    assert (result.code, result.outcome) == (1, "watcher-error")
    assert (result.report["pr"], result.report["head_sha"]) == (0, "")
    assert result.report["detail"] == "transport: connection reset by peer"


def test_an_internal_defect_exits_2_without_a_report(observe: Any) -> None:
    def broken(argv: Sequence[str]) -> Reply:
        raise RuntimeError("defect")

    result = observe(broken)
    assert (result.code, result.report) == (2, {})
    assert "RuntimeError: defect" in result.stderr


# ---- classifying an ejection: infrastructure, or a decision for a human ----

# Recorded from PR #2016's ejection: merge-group run 44632169176, job
# 135941369307 (Validation test checks), whose log carried GitHub's escapes.
NIX_LOG = (
    "2026-11-19T22:50:50.0331455Z \x1b[31;1merror:\x1b[0m unable to download "
    "'https://jaunder-org.cachix.org/nar/edbc0fcd.nar.zst': HTTP error 416\n"
    "2026-11-19T22:50:50.0333085Z error: path '/nix/store/xnhf8331-cargo-package"
    "-tree-sitter-julia-0.23.1' is required, but there is no substituter that "
    "can build it\n"
)
# PR #2072's variant (job 137822809833): a 206, not a 416.
PARTIAL_LOG = (
    "error: unable to download 'https://jaunder-org.cachix.org/nar/78662aee.nar"
    ".zst': HTTP error 206 (curl error: Failure when receiving data from the "
    "peer)\nerror: path '/nix/store/x-perl' is required, but there is no "
    "substituter that can build it\n"
)
TEST_FAILURE_LOG = "FAILED tools/dev/tests/test_gates.py::test_population\n"
# The shape of the repository's ci.yml: lanes, the aggregates over them, and
# per-pair reconciliation that runs after a failed lane.
CI_SOURCE = """
jobs:
  merge-queue-verdict:
    name: Merge-queue verdict
  validate-host:
    name: Validation host
    needs: [merge-queue-verdict]
  validate-test-checks:
    name: Validation test checks
    needs: [merge-queue-verdict]
  validate-no-e2e:
    name: Validate (no e2e)
    needs: [merge-queue-verdict, validate-host, validate-test-checks]
  e2e-lane:
    name: e2e (${{ matrix.lane }})
    needs: [merge-queue-verdict]
  e2e-reconcile:
    name: e2e reconcile (${{ matrix.backend }}/${{ matrix.browser }})
    needs: [merge-queue-verdict, e2e-lane]
  e2e-gate:
    name: e2e gate
    needs: [merge-queue-verdict, e2e-lane, e2e-reconcile]
"""
LANE, GATE = "Validation test checks", V
RUN_URL = "https://github.com/o/r/actions/runs/9"


def failed_jobs(*named: tuple[int, str]) -> Any:
    return {
        "jobs": [{"id": i, "name": n, "conclusion": "failure"} for i, n in named]
        + [{"id": 1, "name": "E2E catalog", "conclusion": "success"}]
    }


def classified(**routes: Any) -> pr_watch.Ejection:
    """The repository's own workflow, PR #2016's failed jobs and its log,
    unless a route is scripted."""
    scripted: dict[str, Any] = {
        CI_FILE: [contents(CI_SOURCE)],
        "jobs": [failed_jobs((7, LANE), (8, GATE))],
        "log:7": [(0, NIX_LOG, "")],
    }
    reader, _ = github(**(scripted | routes))
    found = pr_watch.parse_runs(runs(run(workflow=WORKFLOW)), 731)
    assert found is not None
    return reader.classify_ejection(found, HEAD)


@pytest.mark.parametrize("log", [NIX_LOG, PARTIAL_LOG])
def test_a_nix_substitute_failure_is_infrastructure(log: str) -> None:
    assert pr_watch.signature(log) == "nix-substitute"


@pytest.mark.parametrize(
    "log",
    [NIX_LOG.splitlines()[0], NIX_LOG.splitlines()[1], TEST_FAILURE_LOG, ""],
)
def test_half_a_signature_or_another_failure_matches_nothing(log: str) -> None:
    assert pr_watch.signature(log) is None


def test_every_signature_names_its_real_example_job() -> None:
    assert [(s.name, s.example) for s in pr_watch.SIGNATURES] == [
        ("nix-substitute", 135941369307)
    ]


@pytest.mark.parametrize(
    ("template", "name", "matches"),
    [
        ("e2e (${{ matrix.lane }})", "e2e (sqlite/chromium)", True),
        ("e2e (${{ matrix.lane }})", "e2e ()", True),
        ("e2e (${{ matrix.lane }})", "e2e gate", False),
        ("e2e reconcile (${{ a }}/${{ b }})", "e2e reconcile (postgres/firefox)", True),
        ("Validation host", "Validation host", True),
        ("Validation host", "Validation hostile", False),
        ("a.b", "axb", False),
    ],
)
def test_a_name_expression_stands_for_any_text(
    template: str, name: str, matches: bool
) -> None:
    definition = pr_watch.Definition("key", template)
    assert pr_watch.defines(definition, name) is matches


@pytest.mark.parametrize("source", ["shape", "repository"])
def test_the_ci_workflow_drops_aggregates_and_keeps_lanes(source: str) -> None:
    if source == "repository":
        path = REPOSITORY / WORKFLOW
        require(path.is_file(), f"the repository's {WORKFLOW}")
        definitions = pr_watch.job_definitions(path.read_text(encoding="utf-8"))
    else:
        definitions = pr_watch.job_definitions(CI_SOURCE)
    assert definitions is not None
    failed = [
        pr_watch.FailedJob(1, "e2e (postgres-firefox-ordinary-1-of-2)"),
        pr_watch.FailedJob(2, "e2e reconcile (postgres/firefox)"),
        pr_watch.FailedJob(3, "e2e gate"),
        pr_watch.FailedJob(4, LANE),
        pr_watch.FailedJob(5, GATE),
    ]
    jobs = pr_watch.derive(failed, definitions)
    assert not isinstance(jobs, str)
    assert [job.derived for job in jobs] == [False, True, True, False, True]


def test_an_aggregate_failing_alone_is_primary() -> None:
    definitions = pr_watch.job_definitions(CI_SOURCE)
    assert definitions is not None
    jobs = pr_watch.derive([pr_watch.FailedJob(5, GATE)], definitions)
    assert jobs == (pr_watch.FailedJob(5, GATE),)


@pytest.mark.parametrize(
    ("names", "why"),
    [
        (["Validation hostile"], "job 'Validation hostile' matches 0 workflow jobs"),
        (["twin"], "job 'twin' matches 2 workflow jobs"),
    ],
)
def test_a_job_the_workflow_does_not_identify_cannot_be_judged(
    names: list[str], why: str
) -> None:
    definitions = [
        pr_watch.Definition("a", "twin"),
        pr_watch.Definition("b", "t${{ x }}"),
    ]
    failed = [pr_watch.FailedJob(1, name) for name in names]
    assert pr_watch.derive(failed, definitions) == why


def test_pr_2016s_ejection_is_infrastructure() -> None:
    ejection = classified()
    assert ejection.signature == "nix-substitute"
    assert ejection.report() == {
        "run": RUN_URL,
        "signature": "nix-substitute",
        "unclassified": None,
        "jobs": [
            {"id": 7, "name": LANE, "derived": False, "signature": "nix-substitute"},
            {"id": 8, "name": GATE, "derived": True, "signature": None},
        ],
    }
    assert ejection.detail == f"infrastructure: nix-substitute ({RUN_URL})"


UNCLASSIFIED = [
    # Infrastructure beside a real failure is a decision for a human.
    (
        {
            "jobs": [failed_jobs((7, LANE), (9, "Validation host"), (8, GATE))],
            "log:9": [(0, TEST_FAILURE_LOG, "")],
        },
        "job 'Validation host' matches no infrastructure signature",
    ),
    ({"log:7": [(0, TEST_FAILURE_LOG, "")]}, f"job '{LANE}' matches no"),
    ({"jobs": [failed_jobs()]}, "no failed job to classify"),
    ({"log:7": [TRANSPORT]}, f"job '{LANE}' log is unreadable: transport"),
    ({"log:7": [NOT_FOUND]}, f"job '{LANE}' log is unreadable: not found"),
    ({"jobs": [TRANSPORT]}, "its jobs are unreadable: transport"),
    (
        {"jobs": [{"jobs": [{"conclusion": "failure", "name": LANE}]}]},
        "its jobs are unreadable: malformed response: merge-group job omitted",
    ),
    ({CI_FILE: [NOT_FOUND]}, f"workflow {WORKFLOW}: it is unreadable: not found"),
    (
        {"jobs": [failed_jobs((7, "Renamed lane"))]},
        "job 'Renamed lane' matches 0 workflow jobs",
    ),
]


@pytest.mark.parametrize(("routes", "why"), UNCLASSIFIED)
def test_anything_but_known_infrastructure_is_unclassified(
    routes: dict[str, Any], why: str
) -> None:
    ejection = classified(**routes)
    assert ejection.signature is None
    assert ejection.unclassified is not None and ejection.unclassified.startswith(why)
    assert ejection.detail == f"unclassified: {ejection.unclassified} ({RUN_URL})"


def test_a_run_naming_no_workflow_is_unclassified_without_reading() -> None:
    reader, fake = github()
    found = pr_watch.parse_runs(runs(run()), 731)
    assert found is not None
    ejection = reader.classify_ejection(found, HEAD)
    assert ejection.unclassified == "the run names no workflow file"
    assert fake.calls == []


def test_a_job_log_is_read_as_escaped_text() -> None:
    reader, fake = github(**{"log:7": [(0, NIX_LOG, "")]})
    assert reader.job_log(7) == NIX_LOG
    assert fake.calls == [
        ["gh", "api", "repos/o/r/actions/jobs/7/logs", "--allow-escape-sequences"]
    ]


def test_the_classification_is_a_trace_phase_with_its_verdict(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    monkeypatch.setattr(trace, "tracer", lambda: provider.get_tracer("t"))
    classified()
    (span,) = exporter.get_finished_spans()
    assert span.name == "classify-ejection"
    assert dict(span.attributes or {}) == {
        pr_watch.EJECTION_RUN: RUN_URL,
        pr_watch.EJECTION_SIGNATURE: "nix-substitute",
    }


def infrastructure_ejection(**routes: Any) -> Native:
    scripted: dict[str, Any] = {
        "runs": [runs(run(workflow=WORKFLOW))],
        CI_FILE: [contents(CI_SOURCE)],
        "jobs": [failed_jobs((7, LANE), (8, GATE))],
        "log:7": [(0, NIX_LOG, "")],
    }
    return native(green(), **(scripted | routes))


def test_an_ejected_report_says_whether_infrastructure_caused_it(
    observe: Any,
) -> None:
    result = observe(infrastructure_ejection())
    assert (result.code, result.outcome) == (1, "ejected")
    assert result.report["ejection"]["signature"] == "nix-substitute"
    assert result.events("classify") == [f"infrastructure: nix-substitute ({RUN_URL})"]
    assert [e["kind"] for e in result.report["events"]][-2:] == ["classify", "terminal"]


@pytest.mark.parametrize("once", [False, True])
def test_a_failed_log_read_never_changes_the_outcome(observe: Any, once: bool) -> None:
    result = observe(infrastructure_ejection(**{"log:7": [TRANSPORT]}), once=once)
    assert (result.code, result.outcome) == (1, "ejected")
    assert result.report["ejection"]["signature"] is None
    assert result.events("poll-error") == []
