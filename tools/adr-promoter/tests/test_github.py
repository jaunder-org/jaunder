"""Our consumed evidence and load-bearing SDK arguments, without network calls."""
from types import SimpleNamespace as NS

import pytest
from github import UnknownObjectException

from adr_promoter import github


def observed(**data):
    # SDK response data, deliberately without any lazy attribute access.
    return NS(_rawData=data)


def test_token_retry_and_expected_head_boundary(monkeypatch):
    calls = []
    pull = NS(enable_automerge=lambda **kw: calls.append(kw))
    repo = NS(get_pull=lambda number: pull)
    def client(**kw):
        calls.append(kw)
        return NS(get_repo=lambda slug, lazy: repo)
    monkeypatch.setattr(github, "Github", client)
    monkeypatch.setenv("GITHUB_REPOSITORY", "fixture/repo")
    monkeypatch.setenv("GH_TOKEN", "installation-token")
    api = github.Repository.from_environment()
    api.arm(7, "exact-head")
    assert calls[0]["auth"].token == "installation-token"
    assert calls[0]["retry"] is None and calls[0]["per_page"] == 100
    assert calls[1] == {"merge_method": "MERGE", "expected_head_oid": "exact-head"}


def test_merge_group_first_page_only_and_exact_parent_selection():
    pages, fetched = [], []
    runs = [observed(event="merge_group", head_branch="gh-readonly-queue/main/pr-7-base", head_sha="good"),
            observed(event="merge_group", head_branch="gh-readonly-queue/main/pr-7-old", head_sha="old"),
            observed(event="merge_group", head_branch="gh-readonly-queue/main/pr-8-base", head_sha="wrong")]
    def page(index):
        pages.append(index)
        return runs
    def commit(sha):
        fetched.append(sha)
        return observed(sha=sha, parents=[{"sha": "head" if sha == "good" else "old-head"}])
    repo = NS(get_workflow_runs=lambda **kw: NS(get_page=page), get_commit=commit)
    api = github.Repository("fixture", "repo", NS(), repo)
    assert api.merge_groups("main", 7, "head") == ["good"]
    assert pages == [0] and fetched == ["good", "old"]


def test_graphql_paging_keeps_identity_and_pending_rerun_evidence():
    calls = []
    def query(query, variables):
        calls.append(variables.copy())
        second = variables["after"] is not None
        contexts = {"nodes": [{"context": "CI", "state": "PENDING" if second else "SUCCESS", "createdAt": "now"}],
                    "pageInfo": {"hasNextPage": not second, "endCursor": "next"}}
        return {}, {"data": {"repository": {"object": {"oid": "head", "statusCheckRollup": {"contexts": contexts}}}}}
    api = github.Repository("fixture", "repo", NS(requester=NS(graphql_query=query)), NS())
    result = api.checks("head")
    assert result.sha == "head" and [c.state for c in result.checks] == ["success", "pending"]
    assert [c["after"] for c in calls] == [None, "next"]


def test_snapshot_and_rules_project_only_consumed_policy_evidence():
    pr = {"number": 7, "state": "OPEN", "headRefName": "stable", "headRefOid": "head",
          "baseRefOid": "base", "mergeable": "CONFLICTING", "mergeStateStatus": "DIRTY",
          "isInMergeQueue": False, "autoMergeRequest": {"enabledAt": "now"},
          "statusCheckRollup": {"contexts": {"nodes": [], "pageInfo": {"hasNextPage": False}}}}
    rules = [{"type": "merge_queue"}, {"type": "required_status_checks", "parameters": {
        "required_status_checks": [{"context": "CI"}]}}]
    requester = NS(graphql_query=lambda *args: ({}, {"data": {"repository": {"pullRequest": pr}}}),
                   requestJsonAndCheck=lambda *args: ({}, rules))
    api = github.Repository("fixture", "repo", NS(requester=requester), NS())
    assert api.snapshot(7) == github.Snapshot(7, "OPEN", "stable", "head", "base", "CONFLICTING",
                                               "DIRTY", True, False, ())
    assert api.rules("main") == github.Rules(("CI",), True)
    with pytest.raises(ValueError, match="does not match"):
        api.snapshot(8)


def test_graphql_repeated_cursor_is_an_error():
    contexts = {"nodes": [], "pageInfo": {"hasNextPage": True, "endCursor": "same"}}
    query = lambda *args: ({}, {"data": {"repository": {"object": {"oid": "head", "statusCheckRollup": {"contexts": contexts}}}}})
    api = github.Repository("fixture", "repo", NS(requester=NS(graphql_query=query)), NS())
    with pytest.raises(ValueError, match="pagination"):
        api.checks("head")


def test_absence_is_narrow_and_operational_errors_propagate():
    def missing(*args):
        raise UnknownObjectException(404, {"message": "Not Found"})
    api = github.Repository("fixture", "repo", NS(requester=NS(graphql_query=missing)),
                            NS(get_pull=missing, get_git_ref=missing))
    assert api.get_pr(7) is None and api.ref_head("stable") is None
    with pytest.raises(UnknownObjectException):
        api.checks("head")


def test_rate_limit_diagnostic_does_not_hide_failed_observation(capsys):
    from github import RateLimitExceededException
    def limited(*args):
        raise RateLimitExceededException(403, {"message": "rate limit"})
    from datetime import datetime, timezone
    limits = NS(resources=NS(core=NS(remaining=0, reset=datetime.fromtimestamp(20, timezone.utc)),
                             graphql=NS(remaining=0, reset=datetime.fromtimestamp(10, timezone.utc))))
    api = github.Repository("fixture", "repo", NS(get_rate_limit=lambda: limits), NS(get_pull=limited))
    with pytest.raises(RuntimeError, match="rate limited until 20"):
        api.get_pr(7)
    def failed_reset():
        raise OSError("reset unavailable")
    api = github.Repository("fixture", "repo", NS(get_rate_limit=failed_reset), NS(get_pull=limited))
    with pytest.raises(RateLimitExceededException):
        api.get_pr(7)
    assert "ignored failure while looking up rate-limit reset" in capsys.readouterr().err


def test_consumed_check_states_and_missing_evidence():
    checks = github.parse_checks([
        {"name": "CI", "status": "IN_PROGRESS", "conclusion": "SUCCESS"},
        {"name": "CI", "status": "COMPLETED", "conclusion": "FAILURE"},
        {"context": "external", "state": "SUCCESS", "createdAt": "now"},
    ])
    assert [check.state for check in checks] == ["pending", "failure", "success"]
    with pytest.raises(ValueError):
        github.parse_checks(None)
