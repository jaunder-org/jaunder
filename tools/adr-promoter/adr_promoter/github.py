"""Small gh argv/JSON adapter; GitHub's response, not exit zero, is evidence."""

from dataclasses import dataclass
import json
import os
import subprocess
import sys


class ApiError(RuntimeError):
    def __init__(self, kind, detail):
        self.kind = kind
        super().__init__(detail)


def _brief(text):
    text = text.strip()
    return text[:300] + ("…" if len(text) > 300 else "")


def _invalid_constant(value):
    raise ValueError(f"invalid JSON constant {value}")


def decode_json(source):
    # Match JSON's grammar rather than Python's optional NaN/Infinity extension.
    return json.loads(source, parse_constant=_invalid_constant)


def classify(code, stdout, stderr):
    if code == 127 or "command not found" in stderr:
        raise ApiError("missing", "`gh` is not on PATH")
    valid_json = True
    try:
        body = decode_json(stdout.strip())
    except ValueError:
        body, valid_json = None, False
    errors = body.get("errors") if isinstance(body, dict) else None
    if isinstance(errors, list) and errors:
        if any(isinstance(error, dict) and error.get("type") == "NOT_FOUND" for error in errors):
            raise ApiError("not-found", "not found")
        raise ApiError("graphql", "graphql: " + json.dumps(errors))
    if code == 0:
        # JSON null is valid JSON; an empty/malformed stream is not.
        if not valid_json:
            raise ApiError("malformed", "malformed response: " + _brief(stdout))
        return body
    if isinstance(body, dict) and isinstance(body.get("status"), str):
        status = body["status"]
        message = body.get("message") if isinstance(body.get("message"), str) else ""
        if status == "401":
            raise ApiError("auth", "`gh` is not authenticated")
        if status == "404":
            raise ApiError("not-found", "not found")
        if status == "403" and "rate limit" in message.lower():
            raise ApiError("rate-limit", "rate limited")
        raise ApiError("transport", f"transport: HTTP {status}: {message}")
    if stderr.strip().startswith("gh: "):
        raise ApiError("graphql", "graphql: " + stderr.strip()[4:])
    raise ApiError("transport", "transport: " + _brief(stderr))


def spawn(arguments):
    try:
        result = subprocess.run(["gh", *arguments], capture_output=True, check=False)
    except FileNotFoundError as error:
        raise ApiError("missing", "`gh` is not on PATH") from error
    except OSError as error:
        raise ApiError("transport", f"could not run gh: {error}") from error
    return result.returncode, result.stdout.decode("utf-8", errors="replace"), result.stderr.decode("utf-8", errors="replace")


def _with_reset(error):
    if error.kind != "rate-limit":
        return error
    try:
        value = classify(*spawn(["api", "rate_limit"]))
        resources = value["resources"]
        resources = resources if isinstance(resources, dict) else {}
        resets = [bucket["reset"] for name in ("graphql", "core")
                  if isinstance(bucket := resources.get(name), dict)
                  and type(bucket.get("remaining")) is int and bucket["remaining"] == 0
                  and type(bucket.get("reset")) is int and 0 <= bucket["reset"] <= 0xFFFFFFFFFFFFFFFF]
        if resets:
            return ApiError("rate-limit", f"rate limited until {max(resets)} (unix)")
    except (ApiError, KeyError, TypeError, AttributeError):
        # One best-effort lookup, never a retry of the failed privileged request.
        print("xtask: warning: xtask.pr.rate_limit_reset: ignored failure while looking up rate-limit reset", file=sys.stderr)
    return error


def request(*arguments):
    try:
        return classify(*spawn(arguments))
    except ApiError as error:
        raise _with_reset(error)


def write(*arguments):
    code, stdout, stderr = spawn(arguments)
    if code != 0:
        try:
            classify(code, stdout, stderr)
        except ApiError as error:
            raise _with_reset(error)


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
    if type(value) is not int or not 0 <= value <= 0xFFFFFFFFFFFFFFFF:
        raise ValueError(f"GitHub response has no unsigned integer {name}")
    return value


def parse_pr(value):
    state = text(value, "state")
    if state not in ("open", "closed"):
        raise ValueError(f"promoter PR response has unknown state {state}")
    return PullRequest(
        unsigned(at(value, "number"), "number"), text(value, "head", "repo", "owner", "login"),
        text(value, "user", "login"), text(value, "head", "ref"), text(value, "head", "sha"),
        text(value, "base", "ref"), at(value, "body") if isinstance(at(value, "body"), str) else "",
        state == "open",
    )


def parse_checks(nodes):
    if not isinstance(nodes, list):
        return ()
    checks = []
    for node in nodes:
        name = at(node, "name")
        if isinstance(name, str):
            if at(node, "checkSuite", "app", "slug") == "github-actions":
                unsigned(at(node, "databaseId"), "check databaseId")
                unsigned(at(node, "checkSuite", "workflowRun", "databaseId"), "workflow run databaseId")
            state = "pending" if at(node, "status") != "COMPLETED" else (
                "success" if at(node, "conclusion") in ("SUCCESS", "NEUTRAL", "SKIPPED") else "failure"
            )
            started, completed = at(node, "startedAt"), at(node, "completedAt")
        elif isinstance(name := at(node, "context"), str):
            state = {"SUCCESS": "success", "FAILURE": "failure", "ERROR": "failure"}.get(at(node, "state"), "pending")
            started = at(node, "createdAt")
            completed = started if state != "pending" else None
        else:
            continue
        checks.append(Check(name, state, started if isinstance(started, str) else None,
                            completed if isinstance(completed, str) else None))
    return tuple(checks)


def parse_snapshot(value):
    pr = at(value, "data", "repository", "pullRequest")
    if not isinstance(pr, dict):
        raise ValueError("no pullRequest node in response")
    number = unsigned(at(pr, "number"), "PR number")
    state = at(pr, "state")
    if number == 0 or state not in ("OPEN", "MERGED", "CLOSED") or type(at(pr, "isInMergeQueue")) is not bool:
        raise ValueError("pull request has invalid number, state or isInMergeQueue")
    nodes = at(pr, "commits", "nodes")
    if not isinstance(nodes, list) or not nodes:
        raise ValueError("no head commit in response")
    head = at(nodes[0], "commit")
    text(head, "committedDate")
    mergeable = at(pr, "mergeable")
    merge_state = at(pr, "mergeStateStatus")
    return Snapshot(
        number, state, at(pr, "headRefName") if isinstance(at(pr, "headRefName"), str) else "",
        text(head, "oid"), text(pr, "baseRefOid"),
        mergeable if mergeable in ("MERGEABLE", "CONFLICTING") else "UNKNOWN",
        merge_state if merge_state in ("BEHIND", "BLOCKED", "CLEAN", "DIRTY", "DRAFT", "HAS_HOOKS", "UNSTABLE") else "UNKNOWN",
        isinstance(at(pr, "autoMergeRequest", "enabledAt"), str), pr["isInMergeQueue"],
        parse_checks(at(pr, "statusCheckRollup", "contexts", "nodes")),
    )


def parse_rules(value):
    if not isinstance(value, list):
        raise ValueError("branch rules response is not an array")
    status = next((rule for rule in value if at(rule, "type") == "required_status_checks"), {})
    entries = at(status, "parameters", "required_status_checks")
    contexts = tuple(entry["context"] for entry in entries if isinstance(at(entry, "context"), str)) if isinstance(entries, list) else ()
    return Rules(contexts, any(at(rule, "type") == "merge_queue" for rule in value))


PR_QUERY = '''query($owner:String!,$name:String!,$number:Int!,$after:String){
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      number state mergedAt mergeCommit { oid } mergeable mergeStateStatus
      isInMergeQueue mergeQueueEntry { position } autoMergeRequest { enabledAt }
      headRefName baseRefOid
      commits(last:1){ nodes { commit { oid committedDate } } }
      statusCheckRollup {
        contexts(first:100, after:$after){
          nodes {
            __typename
            ... on CheckRun { databaseId name conclusion status detailsUrl startedAt completedAt checkSuite { app { slug } workflowRun { databaseId } } }
            ... on StatusContext { context state targetUrl createdAt }
          }
          pageInfo { hasNextPage endCursor }
        }
      }
    }
  }
}'''

COMMIT_CHECKS_QUERY = '''query($owner:String!,$name:String!,$oid:GitObjectID!){
  repository(owner:$owner,name:$name){
    object(oid:$oid){
      ... on Commit {
        oid statusCheckRollup {
          contexts(first:100){
            nodes {
              __typename
              ... on CheckRun { databaseId name conclusion status detailsUrl startedAt completedAt checkSuite { app { slug } workflowRun { databaseId } } }
              ... on StatusContext { context state targetUrl createdAt }
            }
          }
        }
      }
    }
  }
}'''


@dataclass(frozen=True)
class Repository:
    owner: str
    name: str

    @classmethod
    def from_environment(cls):
        slug = os.environ.get("GITHUB_REPOSITORY")
        if slug is None:
            raise ValueError("GITHUB_REPOSITORY is not set")
        parts = slug.split("/", 1)
        if len(parts) != 2 or not all(parts):
            raise ValueError("GITHUB_REPOSITORY must be owner/repository")
        return cls(*parts)

    @property
    def slug(self):
        return f"{self.owner}/{self.name}"

    def _lookup(self, parse, *arguments):
        # Only typed NOT_FOUND is absence; a successful null body is malformed.
        try:
            value = request(*arguments)
        except ApiError as error:
            if error.kind == "not-found":
                return None
            raise
        return parse(value)

    def _graphql(self, query, key, value):
        return ("api", "graphql", "-f", f"query={query}", "-f", f"owner={self.owner}",
                "-f", f"name={self.name}", "-F" if key == "number" else "-f", f"{key}={value}")

    def open_prs(self, branch, base):
        value = request("api", "--method", "GET", f"/repos/{self.slug}/pulls", "-f", "state=open",
                        "-f", f"head={self.owner}:{branch}", "-f", f"base={base}")
        if not isinstance(value, list):
            raise ValueError("promoter PR list is not an array")
        return [parse_pr(pr) for pr in value]

    def get_pr(self, number):
        return self._lookup(parse_pr, "api", f"/repos/{self.slug}/pulls/{number}")

    def snapshot(self, number):
        result = self._lookup(parse_snapshot, *self._graphql(PR_QUERY, "number", number))
        if result is None:
            return None
        if result.number != number:
            raise ValueError("pull request number does not match requested PR")
        return result

    def ref_head(self, branch):
        return self._lookup(lambda value: text(value, "object", "sha"),
                            "api", f"/repos/{self.slug}/git/ref/heads/{branch}")

    def rules(self, base):
        return parse_rules(request("api", f"/repos/{self.slug}/rules/branches/{base}"))

    def checks(self, sha):
        value = request(*self._graphql(COMMIT_CHECKS_QUERY, "oid", sha))
        commit = at(value, "data", "repository", "object")
        return CommitChecks(text(commit, "oid"), parse_checks(at(commit, "statusCheckRollup", "contexts", "nodes")))

    def merge_groups(self, base, number, head):
        value = request("api", f"/repos/{self.slug}/actions/runs?event=merge_group&per_page=100")
        runs = at(value, "workflow_runs")
        prefix = f"gh-readonly-queue/{base}/pr-{number}-"
        candidates = sorted({run["head_sha"] for run in runs if at(run, "event") == "merge_group"
                             and isinstance(at(run, "head_branch"), str) and run["head_branch"].startswith(prefix)
                             and isinstance(at(run, "head_sha"), str)}) if isinstance(runs, list) else []
        correlated = []
        # Branch suffixes identify a base, not the PR head: prove exact parents.
        for candidate in candidates:
            commit = request("api", f"/repos/{self.slug}/commits/{candidate}")
            parents = at(commit, "parents")
            if at(commit, "sha") == candidate and isinstance(parents, list) and any(at(parent, "sha") == head for parent in parents):
                correlated.append(candidate)
        return correlated

    def create(self, branch, base, title, marker):
        write("pr", "create", "--repo", self.slug, "--head", branch, "--base", base,
              "--title", title, "--body", marker)

    def arm(self, number, head):
        write("pr", "merge", str(number), "--repo", self.slug, "--auto", "--merge", "--match-head-commit", head)

    def close(self, number):
        write("pr", "close", str(number), "--repo", self.slug)
