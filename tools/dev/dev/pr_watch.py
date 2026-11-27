"""`dev pr-watch`: read-only observation of one pull request.

It polls until the next caller-actionable outcome and prints one JSON report on
stdout: `outcome`, `pr`, `head_sha`, `phase` (only for `pending`), `detail`,
`pointer` and the `events` log; absent optional fields are omitted. Each event
also reaches stderr as it happens: only changes, plus a heartbeat after ten
quiet minutes. `ready-to-land` and `merged` exit 0, every other report 1, and a
refusal, an unknown pull request or an internal defect 2 with no report, so a
defect can never read as a verdict.

The observer only reads: it never arms, merges, re-enqueues, cleans up or edits
the tracker; `scripts/pr-land.sh` is the only armer and observes through this
same command. Readiness is not approval to merge. The repository comes from the
origin remote and the requirements from its live `main` ruleset, read once per
invocation; no check, job or workflow name is hard-coded.

A failed check the ruleset names ends the watch `checks-failed`. A failed GitHub
Actions check the ruleset does not name ends it too when its workflow file, read
at the PR head, defines a job whose literal name (or key, when unnamed) is a
ruleset context; that is the early stop on a failed lane before its `always()`
aggregate settles. Any other failure, including one whose workflow file cannot
be read, is an optional warning. Misjudging toward optional only delays
`checks-failed` until the aggregate itself fails. A successful watch reads,
once, the `Duration pressure` warning annotations on the head's check runs and,
once merged, the merge commit's (the merge-group run's), and logs each as a
`warning` event; they never change the outcome. Every `gh` call is a traced
step whose status and streams `classify` turns into data or a typed error; the
decision rules are pure functions of parsed snapshots, and the clock is
injected.
"""

import base64
import json
import sys
import time
import traceback
from collections.abc import Callable, Sequence
from dataclasses import dataclass, replace
from datetime import UTC, datetime
from enum import StrEnum
from typing import Any, NamedTuple

import yaml

from dev import steps
from dev.e2e_evidence import PRESSURE_TITLE

# Runs one command (`gh` or `git`): its status, stdout and stderr.
Runner = Callable[[Sequence[str]], tuple[int, str, str]]

MAX_STRIKES = 5
HEARTBEAT_SECONDS = 600
SUCCESSFUL = ("ready-to-land", "merged")
PENDING, SUCCESS, FAILURE = "pending", "success", "failure"
# A completed CheckRun with any other conclusion failed.
PASSING_CONCLUSIONS = ("SUCCESS", "NEUTRAL", "SKIPPED")
# Job conclusions that fail a merge-group check and so eject the entry;
# cancellation follows a removal rather than causing it.
EJECTING_CONCLUSIONS = ("failure", "timed_out", "startup_failure")
READY_DETAIL = (
    "all required checks passed; obtain approval for this exact PR/head, "
    "then run `dev pr-land`"
)

PR_QUERY = """\
query($owner:String!,$name:String!,$number:Int!,$after:String){
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      number state mergedAt mergeCommit{oid} mergeable mergeStateStatus
      isInMergeQueue mergeQueueEntry{position} autoMergeRequest{enabledAt}
      commits(last:1){nodes{commit{oid committedDate}}}
      statusCheckRollup{
        contexts(first:100,after:$after){
          nodes{
            __typename
            ... on CheckRun{name conclusion status detailsUrl startedAt completedAt
              checkSuite{app{slug} workflowRun{databaseId runAttempt file{path}}}}
            ... on StatusContext{context state targetUrl createdAt}
          }
          pageInfo{hasNextPage endCursor}
        }
      }
    }
  }
}"""
# The latest check runs of one commit and their annotations, read once when a
# watch succeeds (50 x 100 x 50 stays under GitHub's node limit).
ANNOTATIONS_QUERY = """\
query($owner:String!,$name:String!,$oid:GitObjectID!){
  repository(owner:$owner,name:$name){
    object(oid:$oid){
      ... on Commit{
        checkSuites(first:50){
          nodes{
            checkRuns(first:100,filterBy:{checkType:LATEST}){
              nodes{name annotations(first:50){nodes{annotationLevel title message}}}
            }
          }
        }
      }
    }
  }
}"""


class Kind(StrEnum):
    """What a failed `gh` call means."""

    MISSING = "missing"
    UNAUTHENTICATED = "unauthenticated"
    NOT_FOUND = "not-found"
    GRAPHQL = "graphql"
    RATE_LIMITED = "rate-limited"
    TRANSPORT = "transport"
    MALFORMED = "malformed"


# A poll failing for one of these spends a strike; any other ends the watch.
TRANSIENT = (Kind.RATE_LIMITED, Kind.TRANSPORT, Kind.MALFORMED)
# How each kind reads in an event or report; `{}` is the error's message.
_DETAILS = {
    Kind.MISSING: "`gh` is not on PATH",
    Kind.UNAUTHENTICATED: "`gh` is not authenticated",
    Kind.NOT_FOUND: "not found",
    Kind.RATE_LIMITED: "rate limited",
    Kind.GRAPHQL: "graphql: {}",
    Kind.TRANSPORT: "transport: {}",
    Kind.MALFORMED: "malformed response: {}",
}


class GhError(Exception):
    def __init__(self, kind: Kind, message: str = "") -> None:
        super().__init__(kind, message)
        self.kind = kind
        self.message = message

    @property
    def transient(self) -> bool:
        return self.kind in TRANSIENT

    @property
    def detail(self) -> str:
        return _DETAILS[self.kind].format(self.message)


class NoSubject(Exception):
    """There is no pull request to observe: a refusal, never a report."""


def _malformed(message: str) -> GhError:
    return GhError(Kind.MALFORMED, message)


def _truncate(text: str) -> str:
    text = text.strip()
    return text if len(text) <= 300 else text[:300] + "…"


def classify(code: int, stdout: str, stderr: str) -> Any:
    """One `gh` call's JSON reply, or the typed error its outputs describe.

    `gh api` exits 1 for every failure, so the body decides; a nonzero exit is
    never success, and a GraphQL `errors` array is an error even on exit 0.
    """
    # `steps` reports an executable it cannot find as a shell's 127.
    if code == 127:
        raise GhError(Kind.MISSING)
    try:
        body, parsed = json.loads(stdout), True
    except ValueError:
        body, parsed = None, False
    errors = _get(body, "errors")
    if isinstance(errors, list) and errors:
        # GraphQL reports a missing pull request this way, inside a 200.
        if any(_get(error, "type") == "NOT_FOUND" for error in errors):
            raise GhError(Kind.NOT_FOUND)
        raise GhError(Kind.GRAPHQL, json.dumps(errors))
    if code == 0:
        if not parsed:
            raise _malformed(_truncate(stdout))
        return body
    status = _get(body, "status")
    if isinstance(status, str):
        message = _get(body, "message")
        message = message if isinstance(message, str) else ""
        if status == "401":
            raise GhError(Kind.UNAUTHENTICATED)
        if status == "404":
            raise GhError(Kind.NOT_FOUND)
        # A 403 is a rate limit or a refusal; only the message says which.
        if status == "403" and "rate limit" in message.lower():
            raise GhError(Kind.RATE_LIMITED)
        raise GhError(Kind.TRANSPORT, f"HTTP {status}: {message}")
    # No body: `gh: ...` is gh rejecting the request, anything else the
    # transport failing underneath it.
    stderr = stderr.strip()
    if stderr.startswith("gh: "):
        raise GhError(Kind.GRAPHQL, stderr.removeprefix("gh: "))
    raise GhError(Kind.TRANSPORT, _truncate(stderr))


def _get(value: Any, *keys: str) -> Any:
    """`value.keys...` through JSON objects, or None."""
    for key in keys:
        value = value.get(key) if isinstance(value, dict) else None
    return value


def _is_natural(value: Any) -> bool:
    """A JSON whole number that can identify something; never a boolean."""
    return type(value) is int and value >= 0


def _text(value: Any, *keys: str) -> str | None:
    found = _get(value, *keys)
    return found if isinstance(found, str) else None


def _instant(value: Any) -> datetime | None:
    """A GitHub timestamp with its zone; anything else is None."""
    try:
        parsed = datetime.fromisoformat(value)
    except TypeError, ValueError:
        return None
    return parsed if parsed.tzinfo is not None else None


def parse_remote(url: str) -> tuple[str, str] | None:
    """Owner and repository from a Git remote URL, in either form Git writes."""
    url = url.strip().rstrip("/")
    scheme, separator, rest = url.partition("://")
    path = rest.partition("/")[2] if separator else scheme.partition(":")[2]
    owner, _, repository = path.removesuffix(".git").partition("/")
    if not owner or not repository or "/" in repository:
        return None
    return owner, repository


@dataclass(frozen=True)
class Check:
    """One status-rollup entry, a CheckRun and a StatusContext alike.

    `kind` is `actions` for a GitHub Actions CheckRun, which also carries its
    workflow run, attempt and workflow file; `check-run` for any other
    CheckRun; `status` for a StatusContext.
    """

    name: str
    state: str
    url: str | None = None
    started: str | None = None
    completed: str | None = None
    kind: str = "status"
    run: int | None = None
    attempt: int | None = None
    workflow: str | None = None


@dataclass(frozen=True)
class Snapshot:
    state: str
    head: str
    # Git refreshes it on rebase and amend, so a re-pushed head post-dates a
    # merge-group run from before the push.
    committed: datetime
    checks: tuple[Check, ...]
    mergeable: str
    merge_state: str
    armed: bool
    queued: bool
    position: int | None
    merged_at: str | None
    merge_commit: str | None


@dataclass(frozen=True)
class Rules:
    """The live ruleset's requirements: its contexts, strictness and queue."""

    contexts: tuple[str, ...]
    strict: bool
    queue: bool


@dataclass(frozen=True)
class Run:
    """The PR's newest merge-group run; `failed_job` only while in progress."""

    id: int
    url: str
    created: datetime
    conclusion: str | None
    failed_job: str | None = None


def _invalid_rule(index: int, field: str) -> GhError:
    return _malformed(f"branch rule {index} has missing or invalid {field}")


def parse_rules(value: Any) -> Rules:
    """The union of every applicable rule; no malformed entry may shrink it."""
    if not isinstance(value, list):
        raise _malformed("branch rules response is not an array")
    contexts: list[str] = []
    strict = queue = False
    for index, rule in enumerate(value):
        kind = _get(rule, "type")
        if not (isinstance(kind, str) and kind):
            raise _invalid_rule(index, "type")
        queue |= kind == "merge_queue"
        if kind != "required_status_checks":
            continue
        parameters = rule.get("parameters")
        if not isinstance(parameters, dict):
            raise _invalid_rule(index, "parameters")
        checks = parameters.get("required_status_checks")
        rule_strict = parameters.get("strict_required_status_checks_policy")
        if not isinstance(checks, list):
            raise _invalid_rule(index, "required_status_checks")
        if not isinstance(rule_strict, bool):
            raise _invalid_rule(index, "strict_required_status_checks_policy")
        for position, check in enumerate(checks):
            context = _get(check, "context")
            if not (isinstance(context, str) and context):
                field = f"required_status_checks[{position}].context"
                raise _invalid_rule(index, field)
            if context not in contexts:
                contexts.append(context)
        # Strictness takes effect only in a rule that requires a check.
        strict |= rule_strict and bool(checks)
    return Rules(tuple(contexts), strict, queue)


def parse_check(node: Any) -> Check | None:
    name = _get(node, "name")
    if isinstance(name, str):
        state = (
            PENDING
            if _get(node, "status") != "COMPLETED"
            else SUCCESS
            if _get(node, "conclusion") in PASSING_CONCLUSIONS
            else FAILURE
        )
        check = Check(
            name,
            state,
            _text(node, "detailsUrl"),
            _text(node, "startedAt"),
            _text(node, "completedAt"),
            kind="check-run",
        )
        if _get(node, "checkSuite", "app", "slug") != "github-actions":
            return check
        run = _get(node, "checkSuite", "workflowRun")
        # The run identifies a re-run, so it must be there. The attempt only
        # tells warnings apart, and a check without a workflow file is optional.
        if not _is_natural(_get(run, "databaseId")):
            raise _malformed(f"GitHub Actions check {name} has no workflow run")
        attempt = run.get("runAttempt")
        return replace(
            check,
            kind="actions",
            run=run["databaseId"],
            attempt=attempt if _is_natural(attempt) else None,
            workflow=_text(run, "file", "path"),
        )
    context = _get(node, "context")
    if not isinstance(context, str):
        return None
    reported = _get(node, "state")
    state = (
        SUCCESS
        if reported == "SUCCESS"
        else FAILURE
        if reported in ("FAILURE", "ERROR")
        else PENDING
    )
    created = _text(node, "createdAt")
    completed = None if state == PENDING else created
    return Check(context, state, _text(node, "targetUrl"), created, completed)


def parse_snapshot(value: Any, number: int) -> tuple[Snapshot, str | None]:
    """One page of the PR query, and the cursor of the next page, if any.

    Every field a verdict turns on must be present: a missing head, timestamp
    or rollup is a broken reply, never a PR with nothing to report.
    """
    pr = _get(value, "data", "repository", "pullRequest")
    if not isinstance(pr, dict):
        raise _malformed("no pullRequest node in response")
    if not (_is_natural(pr.get("number")) and pr["number"] == number):
        raise _malformed("pull request number does not match the requested PR")
    if pr.get("state") not in ("OPEN", "MERGED", "CLOSED"):
        raise _malformed("pull request has invalid state")
    if not isinstance(pr.get("isInMergeQueue"), bool):
        raise _malformed("pull request has invalid isInMergeQueue")
    nodes = _get(pr, "commits", "nodes")
    commit = _get(nodes[0] if isinstance(nodes, list) and nodes else None, "commit")
    head = _text(commit, "oid")
    committed = _instant(_get(commit, "committedDate"))
    if head is None:
        raise _malformed("head commit has no oid")
    if committed is None:
        raise _malformed("head commit has no committedDate")
    if "statusCheckRollup" not in pr:
        raise _malformed("status-check response omitted rollup")
    checks: list[Check] = []
    cursor = None
    contexts = _get(pr["statusCheckRollup"], "contexts")
    if pr["statusCheckRollup"] is not None:
        page = _get(contexts, "pageInfo")
        more = _get(page, "hasNextPage")
        if not isinstance(more, bool):
            raise _malformed("status-check pageInfo omitted hasNextPage")
        cursor = _text(page, "endCursor") if more else None
        if more and cursor is None:
            raise _malformed("status-check page hasNextPage without endCursor")
        if not isinstance(_get(contexts, "nodes"), list):
            raise _malformed("status-check response omitted nodes")
        checks = [
            check for check in map(parse_check, contexts["nodes"]) if check is not None
        ]
    mergeable = _text(pr, "mergeable")
    merge_state = _text(pr, "mergeStateStatus")
    position = _get(pr, "mergeQueueEntry", "position")
    snapshot = Snapshot(
        state=pr["state"],
        head=head,
        committed=committed,
        checks=tuple(checks),
        mergeable=mergeable or "UNKNOWN",
        merge_state=merge_state or "UNKNOWN",
        # Only a recorded auto-merge request is an arm.
        armed=_text(pr, "autoMergeRequest", "enabledAt") is not None,
        queued=pr["isInMergeQueue"],
        position=position if _is_natural(position) else None,
        merged_at=_text(pr, "mergedAt"),
        merge_commit=_text(pr, "mergeCommit", "oid"),
    )
    return snapshot, cursor


def parse_runs(value: Any, number: int) -> Run | None:
    """The PR's newest merge-group run, whatever its conclusion.

    Its branch is `gh-readonly-queue/main/pr-<N>-<base sha>`: the suffix names
    the base, so only the prefix identifies the PR, and recency comes from
    `created_at`.
    """
    runs = _get(value, "workflow_runs")
    if not isinstance(runs, list):
        raise _malformed("response omitted array workflow_runs")
    newest = None
    for run in runs:
        branch = _get(run, "head_branch")
        if not (
            isinstance(branch, str)
            and branch.startswith(f"gh-readonly-queue/main/pr-{number}-")
        ):
            continue
        created = _instant(run.get("created_at"))
        conclusion = run.get("conclusion", 0)
        if not (
            _is_natural(run.get("id"))
            and isinstance(run.get("html_url"), str)
            and created is not None
            and (conclusion is None or isinstance(conclusion, str))
        ):
            raise _malformed("merge-group run is incomplete")
        if newest is None or created > newest.created:
            newest = Run(run["id"], run["html_url"], created, conclusion)
    return newest


def _nodes(value: Any, *keys: str) -> list[Any]:
    nodes = _get(value, *keys, "nodes")
    if not isinstance(nodes, list):
        raise _malformed(f"response omitted array {'.'.join([*keys, 'nodes'])}")
    return nodes


def parse_pressure(value: Any) -> list[str]:
    """Each duration-pressure warning annotation on one commit's check runs,
    as `<check>: <message>`; a commit GitHub does not know has none."""
    commit = _get(value, "data", "repository", "object")
    if commit is None:
        return []
    found = []
    for suite in _nodes(commit, "checkSuites"):
        for run in _nodes(suite, "checkRuns"):
            for annotation in _nodes(run, "annotations"):
                if (
                    _get(annotation, "annotationLevel") != "WARNING"
                    or _get(annotation, "title") != PRESSURE_TITLE
                ):
                    continue
                name, message = _text(run, "name"), _text(annotation, "message")
                if name is None or message is None:
                    raise _malformed("duration-pressure annotation is incomplete")
                found.append(f"{name}: {message}")
    return found


def job_names(source: str) -> frozenset[str] | None:
    """What a ruleset context can match in one workflow file: each job's
    literal `name`, or its key when it has none; None if it is no workflow."""
    try:
        jobs = _get(yaml.safe_load(source), "jobs")
    except yaml.YAMLError:
        return None
    if not isinstance(jobs, dict):
        return None
    names = (
        job.get("name", key) if isinstance(job, dict) else key
        for key, job in jobs.items()
    )
    return frozenset(name for name in names if isinstance(name, str))


# The decision rules: pure functions of parsed evidence.


@dataclass(frozen=True)
class Step:
    """One snapshot's verdict: a phase to keep watching, or an outcome."""

    phase: str
    outcome: str | None = None
    detail: str | None = None
    pointer: str | None = None


def _end(outcome: str, detail: str | None = None, pointer: str | None = None) -> Step:
    return Step("terminal", outcome, detail, pointer)


def _latest(checks: Sequence[Check]) -> Check | None:
    """The entry that speaks for same-named checks: an in-flight re-run
    outranks any completion, else the latest completion wins."""
    pending = [check for check in checks if check.state == PENDING]
    if pending:
        return max(reversed(pending), key=lambda check: check.started or "")
    return max(reversed(checks), key=lambda check: check.completed or "", default=None)


def resolve(checks: Sequence[Check], name: str) -> Check | None:
    """The state of the ruleset context `name`; None while it has not appeared."""
    return _latest([check for check in checks if check.name == name])


def _identity(check: Check) -> tuple[str, int | None, str]:
    # An Actions check is a re-run only within its own workflow run, so a
    # same-named job of another workflow cannot hide its failure.
    return check.kind, check.run, check.name


def resolved_failures(checks: Sequence[Check]) -> list[Check]:
    """Each logical check's failure, after re-run precedence."""
    latest = (
        _latest([check for check in checks if _identity(check) == identity])
        for identity in dict.fromkeys(map(_identity, checks))
    )
    return [check for check in latest if check and check.state == FAILURE]


def _all_green(snapshot: Snapshot, rules: Rules) -> bool:
    return all(
        (check := resolve(snapshot.checks, name)) is not None and check.state == SUCCESS
        for name in rules.contexts
    )


def needs_ejection_probe(snapshot: Snapshot, rules: Rules) -> bool:
    """Ejection presupposes a queue entry, which presupposes green checks."""
    return (
        snapshot.state == "OPEN"
        and bool(rules.contexts)
        and rules.queue
        and not snapshot.queued
        and _all_green(snapshot, rules)
    )


def _ejection_detail(run: Run, snapshot: Snapshot) -> str | None:
    """Why `run` ejected the current head, if it did.

    A run from before the head's commit belongs to an earlier push. The queue
    ejects on a failed job even while slower jobs keep the run going.
    """
    if run.created <= snapshot.committed:
        return None
    if run.conclusion == "failure":
        return "the front-of-queue merge_group run failed"
    if run.conclusion is None and run.failed_job is not None:
        return (
            f"job '{run.failed_job}' in the front-of-queue merge_group run "
            "failed; the run is still in progress"
        )
    return None


def head_drift_detail(expected: str, observed: str) -> str:
    return (
        f"refusing to land: expected PR head {expected}, observed {observed}; "
        "a fresh qualified landing invocation is required"
    )


def _settled(snapshot: Snapshot, expected_head: str | None) -> Step | None:
    """The outcome the snapshot alone decides; no check evidence can change it."""
    if expected_head is not None and snapshot.head != expected_head:
        return _end("stale", head_drift_detail(expected_head, snapshot.head))
    if snapshot.state == "MERGED":
        return _end("merged", snapshot.merged_at, snapshot.merge_commit)
    if snapshot.state == "CLOSED":
        return _end("closed-unmerged")
    if snapshot.mergeable == "CONFLICTING":
        detail = "the branch conflicts with the base and needs a rebase"
        return _end("conflicted", detail)
    return None


def decide(
    snapshot: Snapshot,
    rules: Rules,
    *,
    expected_head: str | None = None,
    ejection: Run | None = None,
    queued_head: str | None = None,
    required: Sequence[Check] = (),
) -> Step:
    """The verdict on one snapshot, adverse outcomes first.

    `required` are the failed checks outside the ruleset that workflow
    membership makes required; `queued_head` the head last seen queued.
    """
    if settled := _settled(snapshot, expected_head):
        return settled
    ruleset = [resolve(snapshot.checks, name) for name in rules.contexts]
    failed = [check for check in ruleset if check and check.state == FAILURE]
    failed += required
    if failed:
        detail = f"required check failed: {failed[0].name}"
        return _end("checks-failed", detail, failed[0].url)
    if rules.strict and snapshot.merge_state == "BEHIND":
        detail = "behind the base branch; the strict ruleset blocks the merge"
        return _end("stale", detail)
    if not rules.contexts:
        detail = "the required-check set is empty; readiness cannot be established"
        return _end("watcher-error", detail)
    if ejection is not None and (why := _ejection_detail(ejection, snapshot)):
        return _end("ejected", why, ejection.url)
    if snapshot.queued:
        return Step("queued")
    if snapshot.armed:
        return Step("armed")
    if not _all_green(snapshot, rules):
        return Step("awaiting-checks")
    # A same-head queue entry that vanished is the more specific diagnosis,
    # even if GitHub now also reports a blocked merge state.
    if queued_head == snapshot.head:
        detail = (
            "the queue entry vanished and no failed current-head merge-group "
            "run explains it"
        )
        return _end("dequeued", detail)
    if snapshot.merge_state in ("BLOCKED", "DRAFT", "DIRTY"):
        return _end("blocked", f"GitHub reports merge state {snapshot.merge_state}")
    if snapshot.mergeable != "MERGEABLE" or snapshot.merge_state not in (
        "CLEAN",
        "HAS_HOOKS",
        "UNSTABLE",
        "BEHIND",
    ):
        return Step("awaiting-mergeability")
    return Step("ready-to-land")


# Reading GitHub.


def _traced(argv: Sequence[str]) -> tuple[int, str, str]:
    code, output, diagnostics = steps.captured(argv[0], argv)
    return code, output.decode(errors="replace"), diagnostics.decode(errors="replace")


class GitHub:
    """The read-only `gh` calls of one watch of one pull request."""

    def __init__(
        self, runner: Runner, owner: str, repository: str, number: int
    ) -> None:
        self.runner = runner
        self.slug = f"{owner}/{repository}"
        self.owner, self.repository, self.number = owner, repository, number
        self._jobs: dict[tuple[str, str], tuple[frozenset[str], str | None]] = {}

    def call(self, *args: str) -> Any:
        return classify(*self.runner(["gh", *args]))

    def rules(self) -> Rules:
        return parse_rules(self.call("api", f"repos/{self.slug}/rules/branches/main"))

    def snapshot(self) -> Snapshot:
        """The PR with every page of its status rollup, all for one head."""
        pages: list[Snapshot] = []
        cursor: str | None = None
        while not pages or cursor is not None:
            args = ["-f", f"query={PR_QUERY}", "-f", f"owner={self.owner}"]
            args += ["-f", f"name={self.repository}", "-F", f"number={self.number}"]
            args += [] if cursor is None else ["-f", f"after={cursor}"]
            try:
                reply = self.call("api", "graphql", *args)
            except GhError as error:
                if error.kind is Kind.NOT_FOUND:
                    raise NoSubject(
                        f"no such pull request: #{self.number} in {self.slug}"
                    ) from error
                raise
            page, cursor = parse_snapshot(reply, self.number)
            if pages and page.head != pages[0].head:
                raise _malformed("PR head changed while paginating status checks")
            pages.append(page)
        checks = tuple(check for page in pages for check in page.checks)
        return replace(pages[0], checks=checks)

    def pressure(self, oid: str) -> list[str]:
        """The duration-pressure warnings on commit `oid`'s latest check runs."""
        args = ["-f", f"query={ANNOTATIONS_QUERY}", "-f", f"owner={self.owner}"]
        args += ["-f", f"name={self.repository}", "-f", f"oid={oid}"]
        return parse_pressure(self.call("api", "graphql", *args))

    def ejection(self) -> Run | None:
        """The newest merge-group run, with its first ejecting job while it
        is still in progress."""
        runs = f"repos/{self.slug}/actions/runs?event=merge_group&per_page=100"
        run = parse_runs(self.call("api", runs), self.number)
        page = 1
        while run is not None and run.conclusion is None:
            jobs = _get(
                self.call(
                    "api",
                    f"repos/{self.slug}/actions/runs/{run.id}/jobs"
                    f"?filter=latest&per_page=100&page={page}",
                ),
                "jobs",
            )
            if not isinstance(jobs, list):
                raise _malformed("response omitted array jobs")
            for job in jobs:
                conclusion = job.get("conclusion", 0) if isinstance(job, dict) else 0
                if not (conclusion is None or isinstance(conclusion, str)):
                    raise _malformed("merge-group job omitted its conclusion")
                if conclusion in EJECTING_CONCLUSIONS:
                    if not isinstance(job.get("name"), str):
                        raise _malformed("merge-group job omitted its name")
                    return replace(run, failed_job=job["name"])
            if len(jobs) < 100:
                break
            page += 1
        return run

    def workflow_jobs(self, path: str, head: str) -> tuple[frozenset[str], str | None]:
        """The job names in workflow file `path` at commit `head`, and why the
        file could not be used, if it could not.

        An unusable file has no jobs, so its failures stay optional rather than
        failing the poll. Each answer is kept for the watch, except a transient
        read failure: the next poll reads the file again.
        """
        if (path, head) in self._jobs:
            return self._jobs[path, head]
        names: frozenset[str] | None = None
        problem, settled = "it is outside .github/workflows/", True
        if path.startswith(".github/workflows/"):
            contents = f"repos/{self.slug}/contents/{path}?ref={head}"
            try:
                encoded = _get(self.call("api", contents), "content")
                names = job_names(base64.b64decode(encoded).decode())
                problem = "it is not a workflow"
            except GhError as error:
                problem, settled = (
                    f"it is unreadable: {error.detail}",
                    not error.transient,
                )
            except TypeError, ValueError:
                problem = "its contents are not base64 UTF-8 text"
        found = (frozenset(), problem) if names is None else (names, None)
        if settled:
            self._jobs[path, head] = found
        return found


# The watch loop.


@dataclass(frozen=True)
class Watch:
    """One invocation's options, as `dev pr-watch` takes them."""

    number: int | None = None
    # Seconds between polls, and minutes before `timed-out`; also the CLI's.
    interval: int = 30
    timeout: int = 90
    once: bool = False
    until: str | None = None
    expected_head: str | None = None
    acknowledged_ejection: str | None = None
    observed_queued_head: str | None = None


class _Log:
    """The one event log: each event reaches stderr as it happens, and the
    report."""

    def __init__(self, now: Callable[[], float]) -> None:
        self.now = now
        self.events: list[dict[str, str]] = []
        self.last = now()

    def __call__(self, kind: str, detail: str) -> None:
        self.last = self.now()
        at = datetime.fromtimestamp(self.last, UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        print(f"  {at} [{kind}] {detail}", file=sys.stderr, flush=True)
        self.events.append({"at": at, "kind": kind, "detail": detail})


class _View(NamedTuple):
    """What progress reports: the phase, each ruleset context's state and the
    queue. Nothing that changes on its own, or every poll would report."""

    phase: str
    checks: tuple[tuple[str, str], ...]
    queue: tuple[bool, int | None]


def _label(snapshot: Snapshot, name: str) -> str:
    check = resolve(snapshot.checks, name)
    return "not yet reported" if check is None else check.state


def _queue_detail(snapshot: Snapshot) -> str:
    if snapshot.position is not None:
        return f"position {snapshot.position}"
    return "queued" if snapshot.queued else "not queued"


def _progress(
    log: _Log, previous: _View | None, current: _View, snapshot: Snapshot
) -> None:
    """The checks and queue: the first view whole, then only what changed."""
    for index, (name, state) in enumerate(current.checks):
        if previous is None or previous.checks[index][1] != state:
            log("check", f"{name}: {state}")
    changed = snapshot.queued if previous is None else previous.queue != current.queue
    if changed:
        log("queue", _queue_detail(snapshot))


# A warning, and what makes it the same warning as an earlier one.
Notice = tuple[tuple[Any, ...], str]


def _optional(check: Check, head: str) -> Notice:
    """One warning per run attempt of an optional failure."""
    identity = (head, check.kind, check.run, check.attempt, check.name)
    where = f" ({check.url})" if check.url else ""
    detail = f"optional check failed: {check.name}{where}"
    return (*identity, check.started, check.completed), detail


def _poll(
    github: GitHub, rules: Rules, watch: Watch, queued_head: str | None
) -> tuple[Snapshot, Step, list[Notice]]:
    """One snapshot, its verdict and its warnings."""
    snapshot = github.snapshot()
    warnings: list[Notice] = []
    required: list[Check] = []
    ejection = None
    if _settled(snapshot, watch.expected_head) is None:
        for check in resolved_failures(snapshot.checks):
            if check.name in rules.contexts:
                continue
            names: frozenset[str] = frozenset()
            if check.kind == "actions" and check.workflow is not None:
                names, problem = github.workflow_jobs(check.workflow, snapshot.head)
                if problem is not None:
                    detail = f"workflow {check.workflow} has no usable jobs: {problem}"
                    # Once per file and head: a transient problem's detail can
                    # differ on every retry.
                    warnings.append(((check.workflow, snapshot.head), detail))
            if names.isdisjoint(rules.contexts):
                warnings.append(_optional(check, snapshot.head))
            else:
                required.append(check)
        if needs_ejection_probe(snapshot, rules):
            ejection = github.ejection()
            # Only this invocation's acknowledged run is disregarded, and only
            # as an ejection; a fresh failure still ends the watch.
            if ejection is not None and ejection.url == watch.acknowledged_ejection:
                ejection = None
    step = decide(
        snapshot,
        rules,
        expected_head=watch.expected_head,
        ejection=ejection,
        queued_head=queued_head,
        required=required,
    )
    return snapshot, step, warnings


def _pressure(log: _Log, github: GitHub, snapshot: Snapshot) -> None:
    """Logs each duration-pressure warning of a successful watch: on the PR
    head and, once merged, on the merge commit, which is the merge-group
    run's commit. The warnings never change the outcome, and neither does a
    failure to read them, which is itself a warning."""
    commits = [snapshot.head]
    if snapshot.merge_commit is not None and snapshot.merge_commit != snapshot.head:
        commits.append(snapshot.merge_commit)
    for oid in commits:
        try:
            warnings = github.pressure(oid)
        except GhError as error:
            log(
                "warning",
                f"duration-pressure annotations of {oid} unread: {error.detail}",
            )
            continue
        for detail in warnings:
            log("warning", detail)


def _watch(
    watch: Watch,
    runner: Runner,
    now: Callable[[], float],
    sleep: Callable[[float], None],
) -> dict[str, Any]:
    """The report: polls until an outcome, the deadline or the strike budget
    ends the watch. Raises NoSubject without a pull request to watch."""
    log = _Log(now)
    deadline = log.last + watch.timeout * 60
    number, head = watch.number or 0, watch.expected_head or ""
    queued_head = watch.observed_queued_head
    rules: Rules | None = None
    strikes, ever_read = 0, False
    previous: _View | None = None
    warned: set[tuple[Any, ...]] = set()

    def end(
        outcome: str,
        detail: str | None = None,
        pointer: str | None = None,
        phase: str | None = None,
    ) -> dict[str, Any]:
        optional = {"phase": phase, "detail": detail, "pointer": pointer}
        present = {key: value for key, value in optional.items() if value is not None}
        report = {"outcome": outcome, "pr": number, "head_sha": head}
        return report | present | {"events": log.events}

    try:
        owner, repository, number = _subject(watch.number, runner)
    except GhError as error:
        # The tooling broke, which is more actionable than "no such PR".
        return end("watcher-error", error.detail)
    github = GitHub(runner, owner, repository, number)
    while True:
        if now() >= deadline:
            # Whether GitHub never finished or we never could see.
            if ever_read:
                outcome = "timed-out"
                detail = "the watch budget expired; GitHub never finished"
            else:
                outcome = "watcher-error"
                detail = "the watch budget expired without a single successful read"
            log("terminal", outcome)
            return end(outcome, detail)
        try:
            if rules is None:
                rules = github.rules()
            snapshot, step, warnings = _poll(github, rules, watch, queued_head)
            if step.outcome == "dequeued":
                # Disappearance alone cannot tell a dequeue from a merge:
                # confirm once with fresh evidence.
                snapshot, step, warnings = _poll(github, rules, watch, queued_head)
        except GhError as error:
            # An absorbed failure is still an event: silence would read as
            # "nothing changed".
            log("poll-error", error.detail)
            strikes += 1
            if watch.once or not error.transient or strikes >= MAX_STRIKES:
                return end(
                    "watcher-error",
                    f"giving up after {strikes} failure(s): {error.detail}",
                )
            sleep(watch.interval)
            continue
        strikes, ever_read = 0, True
        head = watch.expected_head or snapshot.head
        # A push starts a new queue history.
        if queued_head != snapshot.head:
            queued_head = None
        ready = step.phase == "ready-to-land"
        current = _View(
            step.phase,
            tuple((name, _label(snapshot, name)) for name in rules.contexts),
            (snapshot.queued, snapshot.position),
        )
        # Passive readiness stays a phase; default readiness ends the watch.
        terminal: tuple[str, str | None, str | None] | None = None
        if step.outcome is not None:
            terminal = (step.outcome, step.detail, step.pointer)
        elif ready and not watch.until:
            terminal = ("ready-to-land", READY_DETAIL, None)
        # The terminal event stands in for the final phase, but check and
        # queue changes on that poll still report.
        if terminal is None and (previous is None or previous.phase != step.phase):
            log("phase", step.phase)
        _progress(log, previous, current, snapshot)
        previous = current
        for identity, detail in warnings:
            if identity not in warned:
                warned.add(identity)
                log("warning", detail)
        if snapshot.queued:
            queued_head = snapshot.head
        if terminal is not None:
            if terminal[0] in SUCCESSFUL:
                _pressure(log, github, snapshot)
            log("terminal", terminal[0])
            return end(*terminal)
        if watch.once:
            return end("pending", phase=step.phase)
        if now() - log.last >= HEARTBEAT_SECONDS:
            minutes = HEARTBEAT_SECONDS // 60
            log("heartbeat", f"still {step.phase}, no change for {minutes}m")
        sleep(watch.interval)


def _subject(number: int | None, runner: Runner) -> tuple[str, str, int]:
    """The repository from the origin remote, and the PR: `number`, or the
    current branch's open PR."""
    code, url, _ = runner(["git", "remote", "get-url", "origin"])
    remote = None if code else parse_remote(url)
    if remote is None:
        raise NoSubject("no GitHub origin remote")
    if number is not None:
        return *remote, number
    code, branch, _ = runner(["git", "branch", "--show-current"])
    branch = branch.strip()
    if code or not branch:
        raise NoSubject("no current branch; pass a PR number")
    listing = ["pr", "list", "--head", branch, "--state", "open"]
    listing += ["--repo", "/".join(remote), "--json", "number"]
    try:
        found = classify(*runner(["gh", *listing]))
    except GhError as error:
        if error.kind is not Kind.NOT_FOUND:
            raise
        found = None
    first = _get(found[0] if isinstance(found, list) and found else None, "number")
    if not _is_natural(first):
        raise NoSubject(f"no open PR for branch {branch}; pass a PR number")
    return *remote, first


def run(
    watch: Watch,
    runner: Runner = _traced,
    now: Callable[[], float] = time.time,
    sleep: Callable[[float], None] = time.sleep,
) -> None:
    """Prints the report; its outcome is the exit status."""
    try:
        report = _watch(watch, runner, now, sleep)
    except steps.StepFailed:
        raise
    except NoSubject as refusal:
        print(f"dev: pr-watch: {refusal}", file=sys.stderr, flush=True)
        raise steps.StepFailed(2, signalled=False) from None
    except Exception as defect:
        # A defect, not a verdict: exit 2, which `pr-land.sh` never reads as one.
        traceback.print_exc()
        raise steps.StepFailed(2, signalled=False) from defect
    print(json.dumps(report, separators=(",", ":"), ensure_ascii=False), flush=True)
    if report["outcome"] not in SUCCESSFUL:
        raise steps.StepFailed(1, signalled=False)
