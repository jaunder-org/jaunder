"""Pinned PyGithub mechanics; only consumed evidence and queue policy projections."""

from dataclasses import dataclass, replace
import json
import os
import sys

from github import Auth, Github, RateLimitExceededException, UnknownObjectException
from github.GithubObject import GithubObject


def decode_json(source):
    def invalid(value):
        raise ValueError(f"invalid JSON constant {value}")
    return json.loads(source, parse_constant=invalid)


@dataclass(frozen=True)
class PullRequest:
    number: int
    owner: str
    author: str
    head_ref: str
    head_sha: str
    base_ref: str
    body: str
    open: bool


@dataclass(frozen=True)
class Check:
    name: str
    state: str
    started: str | None
    completed: str | None


@dataclass(frozen=True)
class Snapshot:
    number: int
    state: str
    head_ref: str
    head_sha: str
    base_sha: str
    mergeable: str
    merge_state: str
    armed: bool
    queued: bool
    checks: tuple[Check, ...]


@dataclass(frozen=True)
class Rules:
    contexts: tuple[str, ...]
    queue: bool


@dataclass(frozen=True)
class CommitChecks:
    sha: str
    checks: tuple[Check, ...]


def at(value, *keys):
    for key in keys:
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def text(value, *keys):
    result = at(value, *keys)
    if not isinstance(result, str) or not result:
        raise ValueError("GitHub response has no " + "/".join(keys))
    return result


def unsigned(value, name):
    if type(value) is not int or value < 0:
        raise ValueError(f"GitHub response has no unsigned integer {name}")
    return value


def response(obj):
    # The base accessor reads the acquired response without CompletableGithubObject's
    # implicit completion request. Missing consumed evidence must fail, not load more.
    return GithubObject.raw_data.fget(obj)


def parse_pr(value):
    state = text(value, "state")
    if state not in ("open", "closed"):
        raise ValueError(f"promoter PR response has unknown state {state}")
    return PullRequest(
        unsigned(at(value, "number"), "number"), text(value, "head", "repo", "owner", "login"),
        text(value, "user", "login"), text(value, "head", "ref"), text(value, "head", "sha"),
        text(value, "base", "ref"), at(value, "body") if isinstance(at(value, "body"), str) else "", state == "open",
    )


def parse_checks(nodes):
    if not isinstance(nodes, list):
        raise ValueError("missing check context nodes")
    checks = []
    for node in nodes:
        name = at(node, "name")
        if isinstance(name, str):
            state = "pending" if at(node, "status") != "COMPLETED" else (
                "success" if at(node, "conclusion") in ("SUCCESS", "NEUTRAL", "SKIPPED") else "failure"
            )
            started, completed = at(node, "startedAt"), at(node, "completedAt")
        elif isinstance(name := at(node, "context"), str):
            state = {"SUCCESS": "success", "FAILURE": "failure", "ERROR": "failure"}.get(at(node, "state"), "pending")
            started = at(node, "createdAt")
            completed = started if state != "pending" else None
        else:
            raise ValueError("check context has no identity")
        checks.append(Check(name, state, started, completed))
    return tuple(checks)


def parse_snapshot(pr, checks):
    number = unsigned(at(pr, "number"), "PR number")
    state = at(pr, "state")
    if not number or state not in ("OPEN", "MERGED", "CLOSED") or type(at(pr, "isInMergeQueue")) is not bool:
        raise ValueError("pull request has invalid number, state or isInMergeQueue")
    return Snapshot(
        number, state, text(pr, "headRefName"), text(pr, "headRefOid"), text(pr, "baseRefOid"),
        at(pr, "mergeable") or "UNKNOWN", at(pr, "mergeStateStatus") or "UNKNOWN",
        isinstance(at(pr, "autoMergeRequest", "enabledAt"), str), pr["isInMergeQueue"], checks,
    )


CONTEXTS = '''statusCheckRollup { contexts(first:100, after:$after) {
  nodes {
    ... on CheckRun { name conclusion status startedAt completedAt }
    ... on StatusContext { context state createdAt }
  }
  pageInfo { hasNextPage endCursor }
} }'''
PR_QUERY = '''query($owner:String!,$name:String!,$number:Int!,$after:String){
  repository(owner:$owner,name:$name){ pullRequest(number:$number){
    number state mergeable mergeStateStatus isInMergeQueue autoMergeRequest { enabledAt }
    headRefName headRefOid baseRefOid
    ''' + CONTEXTS + '''
  } }
}'''
COMMIT_CHECKS_QUERY = '''query($owner:String!,$name:String!,$oid:GitObjectID!,$after:String){
  repository(owner:$owner,name:$name){ object(oid:$oid){ ... on Commit {
    oid ''' + CONTEXTS + '''
  } } }
}'''


@dataclass(frozen=True)
class Repository:
    owner: str
    name: str
    client: object
    repo: object

    @classmethod
    def from_environment(cls):
        slug = os.environ.get("GITHUB_REPOSITORY", "")
        parts = slug.split("/")
        if len(parts) != 2 or not all(parts):
            raise ValueError("GITHUB_REPOSITORY must be owner/repository")
        token = os.environ.get("GH_TOKEN")
        if not token:
            raise ValueError("GH_TOKEN installation token is not set")
        client = Github(auth=Auth.Token(token), retry=None, per_page=100)
        return cls(*parts, client, client.get_repo(slug, lazy=True))

    def _call(self, operation, *args, **kwargs):
        try:
            return operation(*args, **kwargs)
        except RateLimitExceededException as error:
            try:
                limits = self.client.get_rate_limit().resources
                resets = [bucket.reset.timestamp() for bucket in (limits.core, limits.graphql) if bucket.remaining == 0]
            except Exception:
                print("xtask: warning: xtask.pr.rate_limit_reset: ignored failure while looking up rate-limit reset", file=sys.stderr)
            else:
                if resets:
                    raise RuntimeError(f"rate limited until {int(max(resets))} (unix)") from error
            raise

    def _lookup(self, operation, *args):
        # Absence is permitted only for PR/snapshot/ref lookup, never acquisition.
        try:
            return self._call(operation, *args)
        except UnknownObjectException:
            return None

    def open_prs(self, branch, base):
        return self._call(lambda: [parse_pr(response(pr)) for pr in self.repo.get_pulls(
            state="open", head=f"{self.owner}:{branch}", base=base).get_page(0)])

    def get_pr(self, number):
        return self._lookup(lambda: parse_pr(response(self.repo.get_pull(number))))

    def _evidence(self, query, key, value, node_name):
        after, seen, checks, identity = None, set(), (), None
        for _ in range(100):
            _, data = self._call(self.client.requester.graphql_query, query,
                                 {"owner": self.owner, "name": self.name, key: value, "after": after})
            node = at(data, "data", "repository", node_name)
            current = parse_snapshot(node, ()) if node_name == "pullRequest" else text(node, "oid")
            if identity is not None and identity != current:
                raise ValueError("GitHub evidence identity changed during pagination")
            identity = current
            rollup = at(node, "statusCheckRollup")
            if rollup is None:
                return identity, checks
            connection = at(rollup, "contexts")
            checks += parse_checks(at(connection, "nodes"))
            page = at(connection, "pageInfo")
            if type(at(page, "hasNextPage")) is not bool:
                raise ValueError("missing GraphQL pagination evidence")
            if not page["hasNextPage"]:
                return identity, checks
            after = text(page, "endCursor")
            if after in seen:
                raise ValueError("repeated GraphQL pagination cursor")
            seen.add(after)
        raise ValueError("GraphQL pagination bound exceeded")

    def snapshot(self, number):
        result = self._lookup(self._evidence, PR_QUERY, "number", number, "pullRequest")
        if result is None:
            return None
        snapshot, checks = result
        if snapshot.number != number:
            raise ValueError("pull request number does not match requested PR")
        return replace(snapshot, checks=checks)

    def ref_head(self, branch):
        return self._lookup(lambda: text(response(self.repo.get_git_ref(f"heads/{branch}")), "object", "sha"))

    def rules(self, base):
        # PyGithub has no effective-branch-rules method; use its authenticated requester.
        _, rules = self._call(self.client.requester.requestJsonAndCheck, "GET",
                              f"/repos/{self.owner}/{self.name}/rules/branches/{base}")
        if not isinstance(rules, list):
            raise ValueError("branch rules response is not an array")
        status = next((rule for rule in rules if at(rule, "type") == "required_status_checks"), {})
        entries = at(status, "parameters", "required_status_checks") or []
        return Rules(tuple(text(entry, "context") for entry in entries),
                     any(at(rule, "type") == "merge_queue" for rule in rules))

    def checks(self, sha):
        identity, checks = self._evidence(COMMIT_CHECKS_QUERY, "oid", sha, "object")
        return CommitChecks(identity, checks)

    def merge_groups(self, base, number, head):
        def acquire():
            # Explicit first 100 only: iterating PaginatedList would broaden policy.
            runs = self.repo.get_workflow_runs(event="merge_group").get_page(0)
            prefix = f"gh-readonly-queue/{base}/pr-{number}-"
            candidates = sorted({text(run, "head_sha") for obj in runs
                                 if at(run := response(obj), "event") == "merge_group"
                                 and text(run, "head_branch").startswith(prefix)})
            correlated = []
            for candidate in candidates:
                commit = response(self.repo.get_commit(candidate))
                parents = at(commit, "parents")
                if not isinstance(parents, list):
                    raise ValueError("merge-group commit has no parents")
                if text(commit, "sha") == candidate and any(text(parent, "sha") == head for parent in parents):
                    correlated.append(candidate)
            return correlated
        return self._call(acquire)

    def create(self, branch, base, title, marker):
        self._call(self.repo.create_pull, head=branch, base=base, title=title, body=marker)

    def arm(self, number, head):
        self._call(lambda: self.repo.get_pull(number).enable_automerge(merge_method="MERGE", expected_head_oid=head))

    def close(self, number):
        self._call(lambda: self.repo.get_pull(number).edit(state="closed"))
