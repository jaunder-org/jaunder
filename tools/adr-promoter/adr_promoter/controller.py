"""Serialized promotion/recovery policy; Git and gh are the only side-effect seams."""

from dataclasses import dataclass
import os
from pathlib import Path

from . import git, github
from .promotion import promote


BRANCH = "automation/adr-promoter"
BASE = "main"
TITLE = "docs(adr): promote pending ADR drafts"
MARKER = "<!-- jaunder-adr-promoter -->"
BOT = "jaunder-adr-promoter[bot]"


@dataclass(frozen=True)
class PullRequestEvent:
    action: str
    number: int
    head_ref: str
    head_sha: str
    base_ref: str


@dataclass(frozen=True)
class Outcome:
    kind: str
    detail: str


def parse_event(value):
    if not isinstance(value, dict):
        raise ValueError("parsing GitHub event: expected an object")
    action = value.get("action")
    if action is not None and not isinstance(action, str):
        raise ValueError("parsing GitHub event: action is not text")
    pr = value.get("pull_request")
    if pr is None:
        return None
    if not isinstance(pr, dict):
        raise ValueError("parsing GitHub event: pull_request is not an object")
    def reference(key):
        ref = pr.get(key)
        if not isinstance(ref, dict) or not isinstance(ref.get("ref"), str) or not isinstance(ref.get("sha"), str):
            raise ValueError(f"parsing GitHub event: invalid {key} ref/sha")
        return ref
    head, base = reference("head"), reference("base")
    number = github.unsigned(pr.get("number"), "event PR number")
    return PullRequestEvent(action or "", number, head["ref"], head["sha"], base["ref"])


def resolve_context(checks, name):
    matching = [check for check in checks if check.name == name]
    pending = [check for check in matching if check.state == "pending"]
    pool = pending or matching
    if not pool:
        return None
    # A pending rerun wins even over an older settled failure/success. Ties retain
    # the last observation, matching the existing required-context selector.
    def key(item):
        index, check = item
        timestamp = check.started if pending else check.completed
        return timestamp is not None, timestamp or "", index
    return max(enumerate(pool), key=key)[1]


def _occupant(pr, owner):
    return pr.open and pr.owner == owner and pr.head_ref == BRANCH and pr.base_ref == BASE


def _identity(pr, owner):
    return _occupant(pr, owner) and pr.author == BOT and MARKER in pr.body


def _singleton(api):
    pulls = [pr for pr in api.open_prs(BRANCH, BASE) if _occupant(pr, api.owner)]
    if len(pulls) > 1:
        raise RuntimeError("multiple open pull requests occupy the ADR promoter branch")
    if not pulls:
        return None
    if not _identity(pulls[0], api.owner):
        raise RuntimeError("open pull request occupies the ADR promoter branch with a different identity")
    return pulls[0]


def _attempt(operation, *arguments):
    try:
        operation(*arguments)
    except (OSError, RuntimeError) as error:
        return error
    return None


def _postcondition(verified, error, message):
    if not verified:
        if error is not None:
            raise RuntimeError(message + f": {error}") from error
        raise RuntimeError(message)


def _arm(api, number, head):
    error = _attempt(api.arm, number, head)
    pr = api.get_pr(number)
    snapshot = api.snapshot(number)
    if pr is None or snapshot is None:
        raise RuntimeError("promoter PR or snapshot disappeared after auto-merge arm")
    verified = (
        _identity(pr, api.owner) and pr.number == number and pr.head_sha == head
        and api.ref_head(BRANCH) == head and snapshot.state == "OPEN"
        and snapshot.number == number and snapshot.head_ref == BRANCH and snapshot.head_sha == head
        and (snapshot.armed or snapshot.queued)
    )
    _postcondition(verified, error, "GitHub did not verify auto-merge or queue membership on the unchanged promoter head")


def _delete(repo, api, expected):
    error = _attempt(git.delete_remote, repo, BRANCH, expected)
    actual = api.ref_head(BRANCH)
    if actual is not None and actual != expected:
        raise RuntimeError("promoter branch changed during leased deletion")
    _postcondition(actual is None, error, "promoter branch remained after leased deletion")


def _close(api, expected):
    current = api.get_pr(expected.number)
    if (current is None or not _identity(current, api.owner) or current.number != expected.number
        or current.head_sha != expected.head_sha or current.head_ref != expected.head_ref
        or current.base_ref != expected.base_ref or api.ref_head(BRANCH) is not None):
        raise RuntimeError("promoter PR changed or disappeared before close")
    error = _attempt(api.close, expected.number)
    closed = api.get_pr(expected.number)
    verified = (
        closed is not None and not closed.open and closed.number == expected.number
        and closed.owner == expected.owner and closed.author == expected.author
        and closed.head_sha == expected.head_sha and closed.head_ref == expected.head_ref
        and closed.base_ref == expected.base_ref and MARKER in closed.body
    )
    _postcondition(verified, error, "promoter PR close was not verified")


def _queue_required(rules):
    if not rules.queue or not rules.contexts:
        raise RuntimeError("ADR promoter requires a merge queue with required contexts")


def _existing_or_retire(repo, api):
    head = api.ref_head(BRANCH)
    pr = _singleton(api)
    if pr is None:
        if head is not None:
            _delete(repo, api, head)
        return None
    if head is None:
        _close(api, pr)
        return pr.number
    if head != pr.head_sha:
        raise RuntimeError("open promoter PR head differs from stable branch")
    snapshot = api.snapshot(pr.number)
    if (snapshot is None or snapshot.number != pr.number or snapshot.state != "OPEN"
        or snapshot.head_ref != BRANCH or snapshot.head_sha != head):
        raise RuntimeError("promoter snapshot differs from the stable branch")
    if snapshot.mergeable == "CONFLICTING" and snapshot.merge_state == "DIRTY":
        git.fetch(repo, head)
        main = git.refresh_main(repo, BASE)
        if snapshot.base_sha == main:
            parent = git.sole_parent(repo, head)
            if parent is not None and git.strict_ancestor(repo, parent, main) and git.merge_conflicts(repo, main, head):
                _delete(repo, api, head)
                _close(api, pr)
        # Insufficient local proof preserves the immutable attempt, without arming.
        return pr.number
    if snapshot.armed or snapshot.queued:
        return pr.number
    required = api.rules(BASE)
    _queue_required(required)
    if any((check := resolve_context(snapshot.checks, name)) is not None and check.state == "failure"
           for name in required.contexts):
        return pr.number
    _arm(api, pr.number, head)
    return pr.number


def _generate(repo, api):
    retired = _existing_or_retire(repo, api)
    if retired is not None and _singleton(api) is not None:
        return Outcome("Existing", f"promoter PR {retired} already owns the branch")
    git.prepare_main(repo, BASE)
    promote(repo)
    if not git.staged_diff(repo):
        return Outcome("NoChanges", "no ADR promotion diff")
    _queue_required(api.rules(BASE))
    git.format_staged_markdown(repo)
    git.commit_promotion(repo, BOT, TITLE)
    head = git.head_sha(repo)
    error = _attempt(git.push_promoter, repo, BRANCH)
    _postcondition(api.ref_head(BRANCH) == head, error, "remote promoter head does not equal the generated commit")
    error = _attempt(api.create, BRANCH, BASE, TITLE, MARKER)
    created = _singleton(api)
    _postcondition(created is not None, error, "created promoter PR was not found")
    if created.head_sha != head:
        raise RuntimeError("promoter PR head does not equal the generated commit")
    _arm(api, created.number, head)
    if retired is not None:
        return Outcome("Replaced", f"replaced stale promoter PR {retired} with {created.number}")
    return Outcome("Created", f"created and armed promoter PR {created.number}")


def _not_rearmed(reason):
    return Outcome("NotRearmed", "promoter PR not re-armed: " + reason)


def _green(required, checks):
    return bool(required.contexts) and all(
        (check := resolve_context(checks, name)) is not None and check.state == "success"
        for name in required.contexts
    )


def _recover(event, api):
    if event.action != "dequeued" or event.head_ref != BRANCH or event.base_ref != BASE:
        return Outcome("IgnoredEvent", "event does not target the ADR promoter")
    pr = api.get_pr(event.number)
    if pr is None:
        return _not_rearmed("pull request is absent")
    if (not _identity(pr, api.owner) or pr.number != event.number or pr.head_ref != event.head_ref
        or pr.head_sha != event.head_sha or pr.base_ref != event.base_ref):
        return _not_rearmed("event identity does not match the open promoter")
    if api.ref_head(BRANCH) != event.head_sha:
        return _not_rearmed("remote promoter branch no longer matches the event head")
    required = api.rules(event.base_ref)
    if not required.queue:
        return _not_rearmed("merge queue is absent")
    if not required.contexts:
        return _not_rearmed("required contexts are absent")
    groups = api.merge_groups(event.base_ref, event.number, event.head_sha)
    if len(groups) != 1:
        return _not_rearmed("merge-group correlation is absent or ambiguous")
    head_checks, merge_checks = api.checks(event.head_sha), api.checks(groups[0])
    if head_checks.sha != event.head_sha or merge_checks.sha != groups[0]:
        return _not_rearmed("context evidence names a different commit")
    if not _green(required, head_checks.checks) or not _green(required, merge_checks.checks):
        return _not_rearmed("required contexts are missing, incomplete, or failed")
    _arm(api, event.number, event.head_sha)
    return Outcome("Rearmed", f"re-armed promoter PR {event.number}")


def run(event, repo, api):
    return _generate(repo, api) if event is None else _recover(event, api)


def execute(repo):
    path = os.environ.get("GITHUB_EVENT_PATH")
    if path is None:
        raise ValueError("GITHUB_EVENT_PATH is not set")
    event = parse_event(github.decode_json(Path(path).read_bytes()))
    return run(event, repo, github.Repository.from_environment())
