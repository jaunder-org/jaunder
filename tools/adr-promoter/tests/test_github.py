"""Offline GitHub process/JSON contracts, never a live API request."""

import pytest

from adr_promoter.github import ApiError, classify


def test_graphql_errors_fail_even_when_gh_exits_zero():
    with pytest.raises(ApiError, match="graphql"):
        classify(0, '{"data":null,"errors":[{"message":"bad query"}]}', "")


@pytest.mark.parametrize(("code", "body", "stderr", "kind"), [
    (0, "not JSON", "", "malformed"),
    (0, '{"errors":[{"type":"NOT_FOUND"}]}', "", "not-found"),
    (1, '{"status":"404"}', "", "not-found"),
    (1, '{"status":"401"}', "", "auth"),
    (1, '{"status":"403","message":"rate limit exceeded"}', "", "rate-limit"),
    (1, '{"status":"403","message":"forbidden"}', "", "transport"),
    (1, "", "connection failed", "transport"),
    (1, "", "gh: bad document", "graphql"),
    (127, "", "missing", "missing"),
])
def test_process_failures_are_never_success(code, body, stderr, kind):
    with pytest.raises(ApiError) as failure:
        classify(code, body, stderr)
    assert failure.value.kind == kind


def test_rate_limit_has_one_reset_lookup_and_no_request_retry(monkeypatch):
    from adr_promoter import github
    calls = []
    replies = iter([
        (1, '{"status":"403","message":"rate limit"}', ""),
        (0, '{"resources":{"graphql":{"remaining":0,"reset":10},"core":{"remaining":0,"reset":20}}}', ""),
    ])
    def spawn(arguments):
        calls.append(tuple(arguments))
        return next(replies)
    monkeypatch.setattr(github, "spawn", spawn)
    with pytest.raises(ApiError, match="rate limited until 20"):
        github.request("api", "/fixture")
    assert calls == [("api", "/fixture"), ("api", "rate_limit")]


def test_reset_failure_warns_once_and_preserves_rate_limit(monkeypatch, capsys):
    from adr_promoter import github
    replies = iter([(1, '{"status":"403","message":"rate limit"}', ""), (0, "{}", "")])
    monkeypatch.setattr(github, "spawn", lambda arguments: next(replies))
    with pytest.raises(ApiError, match="rate limited"):
        github.request("api", "/fixture")
    assert capsys.readouterr().err == "xtask: warning: xtask.pr.rate_limit_reset: ignored failure while looking up rate-limit reset\n"


def test_raw_write_prose_success_is_not_parsed_as_json(monkeypatch):
    from adr_promoter import github
    monkeypatch.setattr(github, "spawn", lambda arguments: (0, "Armed auto-merge", ""))
    assert github.write("pr", "merge", "7") is None


@pytest.mark.parametrize(("method", "arguments", "absent"), [
    ("get_pr", (7,), True), ("snapshot", (7,), True), ("ref_head", ("stable",), True),
    ("rules", ("main",), False), ("checks", ("head",), False),
    ("open_prs", ("stable", "main"), False),
    ("merge_groups", ("main", 7, "head"), False),
])
def test_not_found_is_absence_only_at_existing_lookup_sites(monkeypatch, method, arguments, absent):
    from adr_promoter import github
    def missing(*arguments):
        raise ApiError("not-found", "not found")
    monkeypatch.setattr(github, "request", missing)
    repo = github.Repository("fixture", "repo")
    if absent:
        assert getattr(repo, method)(*arguments) is None
    else:
        with pytest.raises(ApiError):
            getattr(repo, method)(*arguments)


def test_merge_group_query_is_one_page_and_requires_exact_commit_parent(monkeypatch):
    from adr_promoter import github
    calls = []
    def request(*arguments):
        calls.append(arguments)
        path = arguments[1]
        if "actions/runs" in path:
            return {"workflow_runs": [
                {"event": "merge_group", "head_branch": "gh-readonly-queue/main/pr-7-base", "head_sha": "good"},
                {"event": "merge_group", "head_branch": "gh-readonly-queue/main/pr-7-other", "head_sha": "unrelated"},
                {"event": "merge_group", "head_branch": "gh-readonly-queue/main/pr-8-base", "head_sha": "wrong-pr"},
                {"event": "push", "head_branch": "gh-readonly-queue/main/pr-7-base", "head_sha": "wrong-event"},
            ]}
        if path.endswith("/good"):
            return {"sha": "good", "parents": [{"sha": "main"}, {"sha": "head"}]}
        return {"sha": "unrelated", "parents": [{"sha": "old-head"}]}
    monkeypatch.setattr(github, "request", request)
    assert github.Repository("fixture", "repo").merge_groups("main", 7, "head") == ["good"]
    assert calls == [
        ("api", "/repos/fixture/repo/actions/runs?event=merge_group&per_page=100"),
        ("api", "/repos/fixture/repo/commits/good"),
        ("api", "/repos/fixture/repo/commits/unrelated"),
    ]


def test_malformed_successful_ref_payload_is_not_orphan_absence(monkeypatch):
    from adr_promoter import github
    monkeypatch.setattr(github, "request", lambda *arguments: {})
    with pytest.raises(ValueError, match="sha"):
        github.Repository("fixture", "repo").ref_head("stable")


@pytest.mark.parametrize(("method", "arguments"), [
    ("get_pr", (7,)), ("snapshot", (7,)), ("ref_head", ("stable",)),
])
def test_json_null_is_malformed_not_not_found(monkeypatch, method, arguments):
    from adr_promoter import github
    monkeypatch.setattr(github, "request", lambda *arguments: None)
    with pytest.raises(ValueError):
        getattr(github.Repository("fixture", "repo"), method)(*arguments)


def snapshot_json():
    return {"data": {"repository": {"pullRequest": {
        "number": 7, "state": "OPEN", "isInMergeQueue": False, "headRefName": "automation/adr-promoter",
        "baseRefOid": "base", "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN",
        "commits": {"nodes": [{"commit": {"oid": "head", "committedDate": "2026-01-01"}}]},
        "autoMergeRequest": None, "statusCheckRollup": {"contexts": {"nodes": []}},
    }}}}


@pytest.mark.parametrize("field,value", [("number", 0), ("number", True), ("state", "unknown"),
    ("isInMergeQueue", None), ("baseRefOid", None), ("commits", {"nodes": []})])
def test_malformed_graphql_observation_is_an_error(field, value):
    from adr_promoter import github
    body = snapshot_json()
    body["data"]["repository"]["pullRequest"][field] = value
    with pytest.raises(ValueError):
        github.parse_snapshot(body)


@pytest.mark.parametrize("value", [None, -1, True, 1.5, "7", 2**64])
def test_actions_check_ids_cannot_lose_correlation_types(value):
    from adr_promoter import github
    node = {"name": "CI", "status": "COMPLETED", "conclusion": "SUCCESS", "databaseId": value,
            "checkSuite": {"app": {"slug": "github-actions"}, "workflowRun": {"databaseId": 9}}}
    with pytest.raises(ValueError, match="unsigned integer"):
        github.parse_checks([node])


@pytest.mark.parametrize("conclusion,state", [("SUCCESS", "success"), ("NEUTRAL", "success"),
    ("SKIPPED", "success"), ("FAILURE", "failure"), (None, "failure")])
def test_complete_check_conclusions_and_pending_status(conclusion, state):
    from adr_promoter import github
    node = {"name": "CI", "status": "COMPLETED", "conclusion": conclusion}
    assert github.parse_checks([node])[0].state == state
    node["status"] = "IN_PROGRESS"
    assert github.parse_checks([node])[0].state == "pending"


def test_requested_snapshot_number_is_verified(monkeypatch):
    from adr_promoter import github
    body = snapshot_json()
    body["data"]["repository"]["pullRequest"]["number"] = 8
    monkeypatch.setattr(github, "request", lambda *arguments: body)
    with pytest.raises(ValueError, match="does not match"):
        github.Repository("fixture", "repo").snapshot(7)


def test_real_gh_json_adapter_drives_dequeue_through_verified_arm(monkeypatch, tmp_path):
    import json
    from adr_promoter import controller, github, git
    calls = []
    armed = False
    rest_pr = {"number": 7, "head": {"repo": {"owner": {"login": "fixture"}},
        "ref": "automation/adr-promoter", "sha": "head"}, "user": {"login": "jaunder-adr-promoter[bot]"},
        "base": {"ref": "main"}, "state": "open", "body": "<!-- jaunder-adr-promoter -->"}
    def spawn(arguments):
        nonlocal armed
        arguments = tuple(arguments)
        calls.append(arguments)
        if arguments[:2] == ("pr", "merge"):
            assert arguments == ("pr", "merge", "7", "--repo", "fixture/repo", "--auto", "--merge", "--match-head-commit", "head")
            armed = True
            return 0, "Armed", ""
        if arguments[:2] == ("api", "graphql"):
            if "number=7" in arguments:
                value = snapshot_json()
                if armed:
                    value["data"]["repository"]["pullRequest"]["autoMergeRequest"] = {"enabledAt": "2026-01-01"}
            else:
                oid = next(argument[4:] for argument in arguments if argument.startswith("oid="))
                value = {"data": {"repository": {"object": {"oid": oid, "statusCheckRollup": {"contexts": {
                    "nodes": [{"context": "CI", "state": "SUCCESS", "createdAt": "2026-01-01"}]}}}}}}
        else:
            path = arguments[1]
            if path.endswith("/pulls/7"):
                value = rest_pr
            elif "/git/ref/" in path:
                value = {"object": {"sha": "head"}}
            elif "/rules/" in path:
                value = [{"type": "merge_queue"}, {"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": "CI"}]}}]
            elif "actions/runs" in path:
                assert path.endswith("?event=merge_group&per_page=100")
                value = {"workflow_runs": [{"event": "merge_group", "head_branch": "gh-readonly-queue/main/pr-7-base", "head_sha": "group"}]}
            elif path.endswith("/commits/group"):
                value = {"sha": "group", "parents": [{"sha": "base"}, {"sha": "head"}]}
            else:
                pytest.fail(f"unexpected gh request {arguments}")
        return 0, json.dumps(value), ""
    monkeypatch.setattr(github, "spawn", spawn)
    monkeypatch.setattr(git, "process", lambda *args: pytest.fail("recovery must not execute local Git"))
    event = controller.PullRequestEvent("dequeued", 7, "automation/adr-promoter", "head", "main")
    result = controller.run(event, tmp_path, github.Repository("fixture", "repo"))
    assert (result.kind, result.detail) == ("Rearmed", "re-armed promoter PR 7")
    assert sum(call[:2] == ("pr", "merge") for call in calls) == 1
    assert [argument for call in calls for argument in call if argument.startswith("oid=")] == ["oid=head", "oid=group"]


def test_malformed_http_message_still_has_typed_error():
    with pytest.raises(ApiError) as failure:
        classify(1, '{"status":"403","message":7}', "")
    assert failure.value.kind == "transport"


def test_open_create_close_and_graphql_fields_use_exact_repository_arguments(monkeypatch):
    from adr_promoter import github
    reads, writes = [], []
    def request(*arguments):
        reads.append(arguments)
        return snapshot_json() if arguments[:2] == ("api", "graphql") else []
    monkeypatch.setattr(github, "request", request)
    monkeypatch.setattr(github, "write", lambda *arguments: writes.append(arguments))
    repo = github.Repository("fixture", "repo")
    assert repo.open_prs("automation/adr-promoter", "main") == []
    assert repo.snapshot(7).head_sha == "head"
    repo.create("automation/adr-promoter", "main", "docs(adr): promote pending ADR drafts", "<!-- jaunder-adr-promoter -->")
    repo.close(7)
    assert reads[0] == ("api", "--method", "GET", "/repos/fixture/repo/pulls", "-f", "state=open", "-f", "head=fixture:automation/adr-promoter", "-f", "base=main")
    assert reads[1][-6:] == ("-f", "owner=fixture", "-f", "name=repo", "-F", "number=7")
    assert writes == [
        ("pr", "create", "--repo", "fixture/repo", "--head", "automation/adr-promoter", "--base", "main", "--title", "docs(adr): promote pending ADR drafts", "--body", "<!-- jaunder-adr-promoter -->"),
        ("pr", "close", "7", "--repo", "fixture/repo"),
    ]


@pytest.mark.parametrize("slug", [None, "owner", "/repo", "owner/"])
def test_missing_or_malformed_repository_is_an_error(monkeypatch, slug):
    from adr_promoter import github
    if slug is None:
        monkeypatch.delenv("GITHUB_REPOSITORY", raising=False)
    else:
        monkeypatch.setenv("GITHUB_REPOSITORY", slug)
    with pytest.raises(ValueError, match="GITHUB_REPOSITORY"):
        github.Repository.from_environment()


def test_transport_argv_and_missing_executable_are_offline(monkeypatch):
    import subprocess
    from adr_promoter import github
    def process(arguments, **kwargs):
        assert arguments == ["gh", "api", "/fixture"]
        assert kwargs == {"capture_output": True, "check": False}
        return subprocess.CompletedProcess(arguments, 0, b"{}", b"")
    monkeypatch.setattr(subprocess, "run", process)
    assert github.request("api", "/fixture") == {}
    def missing(*args, **kwargs):
        raise FileNotFoundError("fake missing child")
    monkeypatch.setattr(subprocess, "run", missing)
    with pytest.raises(ApiError) as failure:
        github.request("api", "/fixture")
    assert failure.value.kind == "missing"


def test_optional_merge_classification_remains_unknown_without_authorizing_conflict():
    from adr_promoter import github
    body = snapshot_json()
    pr = body["data"]["repository"]["pullRequest"]
    pr.pop("mergeable")
    pr.pop("mergeStateStatus")
    observed = github.parse_snapshot(body)
    assert observed.mergeable == "UNKNOWN" and observed.merge_state == "UNKNOWN"


@pytest.mark.parametrize("body", ["NaN", "Infinity", "-Infinity", '{"data":{},"unexpected":NaN}'])
def test_nonstandard_json_constants_are_malformed_not_success(body):
    with pytest.raises(ApiError) as failure:
        classify(0, body, "")
    assert failure.value.kind == "malformed"
