"""Controller policy at fake Git/GitHub seams: no live publication capability."""

from dataclasses import replace
from types import SimpleNamespace

import pytest

from adr_promoter import controller
from adr_promoter.github import PullRequest, Snapshot, Rules, Check, CommitChecks


def pr(number=7, head="head"):
    return PullRequest(number, "fixture", controller.BOT, controller.BRANCH, head,
                       "main", controller.MARKER, True)


def snap(number=7, head="head"):
    return Snapshot(number, "OPEN", controller.BRANCH, head, "main-new", "MERGEABLE", "CLEAN",
                    False, False, (Check("CI", "success", None, "2026-01-01"),))


class FakeGitHub:
    owner = "fixture"
    name = "repo"

    def __init__(self, trace):
        self.trace = trace
        self.pulls = []
        self.snapshots = {}
        self.remote = None
        self.required = Rules(("CI",), True)
        self.groups = ["group"]
        self.commit_checks = {
            "head": CommitChecks("head", (Check("CI", "success", None, "2026-01-01"),)),
            "group": CommitChecks("group", (Check("CI", "success", None, "2026-01-01"),)),
        }
        self.fail = set()
        self.suppress = set()

    def touch(self, name):
        self.trace.append(name)
        if name in self.fail:
            raise RuntimeError(name + " failed")

    def open_prs(self, branch, base):
        assert (branch, base) == (controller.BRANCH, "main")
        self.touch("list")
        return self.pulls[:]

    def get_pr(self, number):
        self.touch("get")
        return next((value for value in self.pulls if value.number == number), None)

    def snapshot(self, number):
        self.touch("snapshot")
        return self.snapshots.get(number)

    def ref_head(self, branch):
        assert branch == controller.BRANCH
        self.touch("ref")
        return self.remote

    def rules(self, base):
        assert base == "main"
        self.touch("rules")
        return self.required

    def merge_groups(self, base, number, head):
        assert (base, number, head) == ("main", 7, "head")
        self.touch("groups")
        return self.groups

    def checks(self, sha):
        self.touch("checks")
        return self.commit_checks[sha]

    def create(self, branch, base, title, marker):
        assert (branch, base, title, marker) == (controller.BRANCH, "main", controller.TITLE, controller.MARKER)
        self.trace.append("create")
        if "create" not in self.suppress:
            self.pulls.append(pr(8, "new-head"))
            self.snapshots[8] = snap(8, "new-head")
        if "create" in self.fail:
            raise RuntimeError("create failed")

    def arm(self, number, head):
        self.trace.append("arm")
        assert self.snapshots[number].head_sha == head
        if "arm" not in self.suppress:
            self.snapshots[number] = replace(self.snapshots[number], armed=True)
        if "arm" in self.fail:
            raise RuntimeError("arm failed")

    def close(self, number):
        self.trace.append("close")
        if "close" not in self.suppress:
            self.pulls = [replace(value, open=False) if value.number == number else value for value in self.pulls]
        if "close" in self.fail:
            raise RuntimeError("close failed")


@pytest.fixture
def world(monkeypatch, tmp_path):
    trace = []
    api = FakeGitHub(trace)
    local = SimpleNamespace(arguments=[], staged=True, parent="old-main", ancestor=True, conflicts=True,
                            main="main-new", fail=set(), suppress=set(), trace=trace)
    def operation(name, value):
        def invoke(*arguments):
            trace.append(name)
            local.arguments.append((name, arguments))
            if name in local.fail:
                raise RuntimeError(name + " failed")
            return value() if callable(value) else value
        return invoke
    for method, name, value in [
        ("prepare_main", "prepare", None), ("staged_diff", "diff", lambda: local.staged),
        ("format_staged_markdown", "format", None), ("commit_promotion", "commit", None),
        ("head_sha", "head", "new-head"), ("fetch", "fetch", None),
        ("refresh_main", "main", lambda: local.main), ("sole_parent", "parent", lambda: local.parent),
        ("strict_ancestor", "ancestor", lambda: local.ancestor), ("merge_conflicts", "conflicts", lambda: local.conflicts),
    ]:
        monkeypatch.setattr(controller.git, method, operation(name, value))
    def delete(repo, branch, expected):
        trace.append("delete")
        assert branch == controller.BRANCH
        assert expected == api.remote
        if "delete" not in local.suppress:
            api.remote = None
        if "delete" in local.fail:
            raise RuntimeError("lease refused")
    def push(repo, branch):
        trace.append("push")
        assert branch == controller.BRANCH
        if "push" not in local.suppress:
            api.remote = "new-head"
        if "push" in local.fail:
            raise RuntimeError("push failed")
    monkeypatch.setattr(controller.git, "delete_remote", delete)
    monkeypatch.setattr(controller.git, "push_promoter", push)
    monkeypatch.setattr(controller, "promote", operation("promote", None))
    return SimpleNamespace(api=api, local=local, trace=trace, repo=tmp_path)


def existing(world):
    world.api.pulls = [pr()]
    world.api.snapshots[7] = snap()
    world.api.remote = "head"
    return world


def dequeue():
    return controller.PullRequestEvent("dequeued", 7, controller.BRANCH, "head", "main")


def run(world, event=None):
    return controller.run(event, world.repo, world.api)


def test_no_drafts_or_staged_diff_never_formats_commits_pushes_or_creates(world):
    world.local.staged = False
    result = run(world)
    assert result.kind == "NoChanges"
    assert result.detail == "no ADR promotion diff"
    assert world.trace == ["ref", "list", "prepare", "promote", "diff"]


def test_sdk_write_failure_is_accepted_only_after_exact_arm_postcondition(world, monkeypatch):
    from github import GithubException
    arm = world.api.arm
    def failed_response(number, head):
        arm(number, head)
        raise GithubException(502, {"message": "response lost"})
    monkeypatch.setattr(world.api, "arm", failed_response)
    assert run(world).kind == "Created"
    world.api.snapshots[8] = replace(world.api.snapshots[8], armed=False)
    world.api.suppress.add("arm")
    with pytest.raises(RuntimeError, match="did not verify"):
        run(world)


def test_created_observes_format_commit_push_create_arm_order(world):
    result = run(world)
    assert (result.kind, result.detail) == ("Created", "created and armed promoter PR 8")
    phases = {"prepare", "promote", "diff", "rules", "format", "commit", "head", "push", "create", "arm"}
    assert [name for name in world.trace if name in phases] == [
        "prepare", "promote", "diff", "rules", "format", "commit", "head", "push", "create", "arm",
    ]
    assert ("prepare", (world.repo, "main")) in world.local.arguments
    assert ("commit", (world.repo, "jaunder-adr-promoter[bot]", "docs(adr): promote pending ADR drafts")) in world.local.arguments


@pytest.mark.parametrize("field", ["author", "body"])
def test_foreign_occupant_is_an_error_without_cleanup_or_generation(world, field):
    existing(world)
    world.api.pulls = [replace(pr(), **{field: "foreign"})]
    with pytest.raises(RuntimeError, match="different identity"):
        run(world)
    assert world.trace == ["ref", "list"]


def test_multiple_occupants_are_an_error(world):
    existing(world)
    world.api.pulls.append(pr(8))
    with pytest.raises(RuntimeError, match="multiple"):
        run(world)
    assert "delete" not in world.trace and "prepare" not in world.trace


@pytest.mark.parametrize("change", ["ref", "snapshot-head", "snapshot-ref", "snapshot-state", "snapshot-absent"])
def test_changed_or_inconsistent_existing_attempt_is_an_error(world, change):
    existing(world)
    if change == "ref":
        world.api.remote = "changed"
    elif change == "snapshot-absent":
        world.api.snapshots.clear()
    else:
        field, value = {"snapshot-head": ("head_sha", "changed"), "snapshot-ref": ("head_ref", "changed"),
                        "snapshot-state": ("state", "CLOSED")}[change]
        world.api.snapshots[7] = replace(snap(), **{field: value})
    with pytest.raises(RuntimeError):
        run(world)
    assert not {"delete", "close", "arm", "prepare"}.intersection(world.trace)


@pytest.mark.parametrize("field", ["armed", "queued"])
def test_armed_or_queued_attempt_is_preserved_without_required_context_reads(world, field):
    existing(world)
    world.api.snapshots[7] = replace(snap(), **{field: True})
    assert run(world).kind == "Existing"
    assert not {"rules", "arm", "prepare", "delete"}.intersection(world.trace)


@pytest.mark.parametrize("checks", [(), (Check("CI", "pending", "new", None),),
    (Check("CI", "failure", None, "old"), Check("CI", "pending", "new", None)),
    (Check("CI", "failure", None, "2026-01"), Check("CI", "success", None, "2026-02")),
])
def test_existing_unarmed_attempt_can_be_armed_when_no_resolved_required_failure(world, checks):
    existing(world)
    world.api.snapshots[7] = replace(snap(), checks=checks)
    assert run(world).kind == "Existing"
    assert world.trace.count("arm") == 1
    assert not {"prepare", "promote", "push", "delete"}.intersection(world.trace)


def test_existing_required_failure_stays_visible_without_rearming(world):
    existing(world)
    world.api.snapshots[7] = replace(snap(), checks=(Check("CI", "failure", None, "new"),))
    assert run(world).kind == "Existing"
    assert "arm" not in world.trace and "delete" not in world.trace


@pytest.mark.parametrize("rules", [Rules((), True), Rules(("CI",), False)])
@pytest.mark.parametrize("has_pr", [False, True])
def test_generation_requires_queue_contexts_before_publication(world, rules, has_pr):
    if has_pr:
        existing(world)
    world.api.required = rules
    with pytest.raises(RuntimeError, match="merge queue with required contexts"):
        run(world)
    assert not {"format", "commit", "push", "create", "arm"}.intersection(world.trace)


def test_no_diff_does_not_require_queue_configuration(world):
    world.local.staged = False
    world.api.required = Rules((), False)
    assert run(world).kind == "NoChanges"
    assert "rules" not in world.trace


@pytest.mark.parametrize("phase", ["prepare", "promote", "diff", "format", "commit", "head", "rules"])
def test_operational_generation_errors_stop_publication(world, phase):
    (world.api.fail if phase == "rules" else world.local.fail).add(phase)
    with pytest.raises(RuntimeError, match="failed"):
        run(world)
    assert "push" not in world.trace and "create" not in world.trace


@pytest.mark.parametrize("operation", ["push", "create", "arm"])
def test_write_failure_is_success_only_when_exact_postcondition_completed(world, operation):
    (world.local.fail if operation == "push" else world.api.fail).add(operation)
    assert run(world).kind == "Created"
    assert world.trace.count(operation) == 1


@pytest.mark.parametrize("operation", ["push", "create", "arm"])
@pytest.mark.parametrize("reports_error", [False, True])
def test_unverified_write_is_an_error_even_when_command_exits_zero(world, operation, reports_error):
    owner = world.local if operation == "push" else world.api
    owner.suppress.add(operation)
    if reports_error:
        owner.fail.add(operation)
    with pytest.raises(RuntimeError):
        run(world)
    assert world.trace.count(operation) == 1


@pytest.mark.parametrize("orphan", ["branch", "pr"])
def test_only_documented_orphan_states_resume(world, orphan):
    if orphan == "branch":
        world.api.remote = "observed-orphan"
    else:
        existing(world)
        world.api.remote = None
    result = run(world)
    assert result.kind == ("Created" if orphan == "branch" else "Replaced")
    expected = "delete" if orphan == "branch" else "close"
    assert world.trace.index(expected) < world.trace.index("prepare")
    assert ("close" if orphan == "branch" else "delete") not in world.trace


@pytest.mark.parametrize("operation", ["delete", "close"])
@pytest.mark.parametrize("completed", [False, True])
def test_retirement_command_failure_requires_verified_postcondition(world, operation, completed):
    if operation == "delete":
        world.api.remote = "orphan"
        owner = world.local
    else:
        existing(world)
        world.api.remote = None
        owner = world.api
    owner.fail.add(operation)
    if not completed:
        owner.suppress.add(operation)
        with pytest.raises(RuntimeError):
            run(world)
        assert "prepare" not in world.trace
    else:
        assert run(world).kind in ("Created", "Replaced")


def test_changed_ref_during_lease_does_not_close_pr_or_generate(world, monkeypatch):
    world.api.remote = "observed"
    def raced(repo, branch, expected):
        assert expected == "observed"
        world.api.remote = "raced"
        raise RuntimeError("lease refused")
    monkeypatch.setattr(controller.git, "delete_remote", raced)
    with pytest.raises(RuntimeError, match="changed during leased deletion"):
        run(world)
    assert "prepare" not in world.trace and "close" not in world.trace


@pytest.mark.parametrize("missing", ["base", "parent", "ancestor", "conflicts"])
def test_github_conflict_without_complete_local_proof_preserves_head_and_diff(world, missing):
    existing(world)
    world.api.snapshots[7] = replace(snap(), mergeable="CONFLICTING", merge_state="DIRTY")
    if missing == "base":
        world.local.main = "advanced-main"
    elif missing == "parent":
        world.local.parent = None
    else:
        setattr(world.local, missing, False)
    assert run(world).kind == "Existing"
    assert world.trace.index("fetch") < world.trace.index("main")
    assert not {"delete", "close", "arm", "prepare"}.intersection(world.trace)


def test_complete_conflict_proof_retires_then_rebuilds_from_main(world):
    existing(world)
    world.api.snapshots[7] = replace(snap(), mergeable="CONFLICTING", merge_state="DIRTY")
    result = run(world)
    assert (result.kind, result.detail) == ("Replaced", "replaced stale promoter PR 7 with 8")
    phases = {"fetch", "main", "parent", "ancestor", "conflicts", "delete", "close", "prepare"}
    assert [name for name in world.trace if name in phases] == [
        "fetch", "main", "parent", "ancestor", "conflicts", "delete", "close", "prepare",
    ]


@pytest.mark.parametrize("phase", ["fetch", "main", "parent", "ancestor", "conflicts"])
def test_conflict_probe_failure_is_not_retirement_authorization(world, phase):
    existing(world)
    world.api.snapshots[7] = replace(snap(), mergeable="CONFLICTING", merge_state="DIRTY")
    world.local.fail.add(phase)
    with pytest.raises(RuntimeError, match="failed"):
        run(world)
    assert "delete" not in world.trace and "prepare" not in world.trace


@pytest.mark.parametrize("event", [
    controller.PullRequestEvent("opened", 7, controller.BRANCH, "head", "main"),
    controller.PullRequestEvent("dequeued", 7, "other", "head", "main"),
    controller.PullRequestEvent("dequeued", 7, controller.BRANCH, "head", "other"),
])
def test_irrelevant_events_are_success_without_reads_or_writes(world, event):
    assert run(world, event).kind == "IgnoredEvent"
    assert world.trace == []


@pytest.mark.parametrize("field,value", [
    ("author", "foreign"), ("body", "foreign"), ("owner", "foreign"),
    ("head_ref", "foreign"), ("head_sha", "stale"), ("base_ref", "other"), ("open", False),
])
def test_dequeue_identity_mismatch_is_safe_refusal_without_writes(world, field, value):
    existing(world)
    world.api.pulls = [replace(pr(), **{field: value})]
    assert run(world, dequeue()).kind == "NotRearmed"
    assert "arm" not in world.trace


@pytest.mark.parametrize("head", [None, "changed"])
def test_dequeue_changed_or_absent_stable_ref_is_not_rearmed(world, head):
    existing(world)
    world.api.remote = head
    assert run(world, dequeue()).kind == "NotRearmed"
    assert not {"arm", "rules", "prepare"}.intersection(world.trace)


@pytest.mark.parametrize("rules", [Rules((), True), Rules(("CI",), False)])
def test_dequeue_missing_queue_or_contexts_is_safe_refusal(world, rules):
    existing(world)
    world.api.required = rules
    assert run(world, dequeue()).kind == "NotRearmed"
    assert "groups" not in world.trace and "arm" not in world.trace


@pytest.mark.parametrize("groups", [[], ["one", "two"]])
def test_dequeue_needs_one_unique_parent_correlated_group(world, groups):
    existing(world)
    world.api.groups = groups
    assert run(world, dequeue()).kind == "NotRearmed"
    assert "checks" not in world.trace and "arm" not in world.trace


@pytest.mark.parametrize("sha", ["head", "group"])
@pytest.mark.parametrize("checks", [(), (Check("CI", "pending", "new", None),),
    (Check("CI", "failure", None, "new"),),
    (Check("other", "success", None, "new"),),
    (Check("CI", "success", None, "new"), Check("CI", "pending", "old", None)),
])
def test_required_contexts_must_be_green_on_both_exact_shas(world, sha, checks):
    existing(world)
    world.api.commit_checks[sha] = CommitChecks(sha, checks)
    assert run(world, dequeue()).kind == "NotRearmed"
    assert "arm" not in world.trace


@pytest.mark.parametrize("sha", ["head", "group"])
def test_wrong_commit_identity_is_safe_refusal(world, sha):
    existing(world)
    world.api.commit_checks[sha] = replace(world.api.commit_checks[sha], sha="other")
    assert run(world, dequeue()).kind == "NotRearmed"
    assert "arm" not in world.trace


def test_dequeue_green_contexts_rearm_without_local_generation(world):
    existing(world)
    assert run(world, dequeue()).kind == "Rearmed"
    assert world.trace.count("arm") == 1
    assert not {"prepare", "promote", "format", "commit", "push", "delete", "close"}.intersection(world.trace)


@pytest.mark.parametrize("read", ["get", "ref", "rules", "groups", "checks", "snapshot"])
def test_recovery_api_errors_are_not_safe_refusals(world, read):
    existing(world)
    world.api.fail.add(read)
    with pytest.raises(RuntimeError, match="failed"):
        run(world, dequeue())


@pytest.mark.parametrize("change", ["head_sha", "head_ref", "state", "unarmed", "queued"])
def test_arm_requires_exact_snapshot_or_live_queue_postcondition(world, monkeypatch, change):
    existing(world)
    original = world.api.arm
    def arm(number, head):
        original(number, head)
        updates = {"head_sha": "changed"} if change == "head_sha" else (
            {"head_ref": "changed"} if change == "head_ref" else (
                {"state": "CLOSED"} if change == "state" else {"armed": False, "queued": change == "queued"}))
        world.api.snapshots[number] = replace(world.api.snapshots[number], **updates)
    monkeypatch.setattr(world.api, "arm", arm)
    if change == "queued":
        assert run(world, dequeue()).kind == "Rearmed"
    else:
        with pytest.raises(RuntimeError, match="did not verify"):
            run(world, dequeue())


def test_pending_rerun_dominates_settled_result_and_completed_reruns_use_latest_timestamp():
    good = Check("CI", "success", None, "2026-02")
    bad = Check("CI", "failure", None, "2026-01")
    pending = Check("CI", "pending", "2025-01", None)
    assert controller.resolve_context((good, bad, pending), "CI") == pending
    assert controller.resolve_context((good, bad), "CI") == good
    assert controller.resolve_context((bad, good), "other") is None
    assert controller.resolve_context((bad, replace(good, completed="2026-01")), "CI").state == "success"


def test_conflict_probe_binds_every_local_operation_to_observed_objects(world):
    existing(world)
    world.api.snapshots[7] = replace(snap(), mergeable="CONFLICTING", merge_state="DIRTY")
    assert run(world).kind == "Replaced"
    observed = [(name, arguments[1:]) for name, arguments in world.local.arguments
                if name in {"fetch", "main", "parent", "ancestor", "conflicts"}]
    assert observed == [
        ("fetch", ("head",)), ("main", ("main",)), ("parent", ("head",)),
        ("ancestor", ("old-main", "main-new")), ("conflicts", ("main-new", "head")),
    ]


@pytest.mark.parametrize("change", ["ref", "owner", "author", "marker", "number"])
def test_arm_rejects_changed_post_write_identity(world, monkeypatch, change):
    existing(world)
    original = world.api.arm
    def arm(number, head):
        original(number, head)
        if change == "ref":
            world.api.remote = "changed"
        elif change == "number":
            world.api.snapshots[number] = replace(world.api.snapshots[number], number=99)
        else:
            field = "body" if change == "marker" else change
            world.api.pulls = [replace(pr(), **{field: "changed"})]
    monkeypatch.setattr(world.api, "arm", arm)
    with pytest.raises(RuntimeError, match="did not verify"):
        run(world, dequeue())


def test_changed_ref_before_close_prevents_closing_even_after_delete(world, monkeypatch):
    existing(world)
    world.api.remote = None
    original = world.api.get_pr
    def observed(number):
        current = original(number)
        world.api.remote = "raced"
        return current
    monkeypatch.setattr(world.api, "get_pr", observed)
    with pytest.raises(RuntimeError, match="before close"):
        run(world)
    assert "close" not in world.trace and "prepare" not in world.trace


@pytest.mark.parametrize("operation", ["close", "delete"])
def test_exit_zero_without_retirement_postcondition_is_not_success(world, operation):
    if operation == "close":
        existing(world)
        world.api.remote = None
        world.api.suppress.add("close")
    else:
        world.api.remote = "observed"
        world.local.suppress.add("delete")
    with pytest.raises(RuntimeError, match="not verified|remained"):
        run(world)
    assert "prepare" not in world.trace


@pytest.mark.parametrize("field,value", [("head_sha", "wrong-head"), ("author", "wrong-author")])
def test_created_pr_must_have_generated_head_and_exact_app_identity(world, monkeypatch, field, value):
    original = world.api.create
    def create(*arguments):
        original(*arguments)
        world.api.pulls = [replace(world.api.pulls[-1], **{field: value})]
    monkeypatch.setattr(world.api, "create", create)
    with pytest.raises(RuntimeError):
        run(world)
    assert "arm" not in world.trace


@pytest.mark.parametrize("value", [[], {"action": 7}, {"pull_request": False},
    {"pull_request": {}},
    {"pull_request": {"number": -1, "head": {"ref": "x", "sha": "y"}, "base": {"ref": "main", "sha": "b"}}},
    {"pull_request": {"number": True, "head": {"ref": "x", "sha": "y"}, "base": {"ref": "main", "sha": "b"}}},
    {"pull_request": {"number": 7, "head": {"ref": "x"}, "base": {"ref": "main", "sha": "b"}}},
])
def test_malformed_events_are_errors(value):
    with pytest.raises(ValueError):
        controller.parse_event(value)


def test_event_parser_distinguishes_generation_from_exact_pr_event():
    assert controller.parse_event({}) is None
    value = {"action": "dequeued", "pull_request": {"number": 7,
        "head": {"ref": controller.BRANCH, "sha": "head"}, "base": {"ref": "main", "sha": "base"}}}
    assert controller.parse_event(value) == dequeue()


def test_real_python_mutation_owner_runs_inside_controller_before_fake_publication(tmp_path, monkeypatch):
    import subprocess
    from adr_promoter import git
    from test_promotion import promotion_fixture, assert_complete_promotion
    before = promotion_fixture(tmp_path)
    trace = []
    api = FakeGitHub(trace)
    original_process = subprocess.run
    def process(arguments, **kwargs):
        if arguments[0] == "prettier":
            trace.append("format")
            assert all(path.endswith(".md") for path in arguments[2:])
            return subprocess.CompletedProcess(arguments, 0)
        return original_process(arguments, **kwargs)
    monkeypatch.setattr(subprocess, "run", process)
    def prepare(repo, base):
        assert base == "main" and not git.run(repo, "status", "--porcelain").strip()
        trace.append("prepare")
    def commit(repo, bot, title):
        assert bot == controller.BOT and title == controller.TITLE
        assert "format" in trace and git.staged_diff(repo)
        trace.append("commit")
    def push(repo, branch):
        assert branch == controller.BRANCH and "commit" in trace
        trace.append("push")
        api.remote = "new-head"
    monkeypatch.setattr(git, "prepare_main", prepare)
    monkeypatch.setattr(git, "commit_promotion", commit)
    monkeypatch.setattr(git, "head_sha", lambda repo: "new-head")
    monkeypatch.setattr(git, "push_promoter", push)
    result = controller.run(None, tmp_path, api)
    assert result.kind == "Created"
    assert trace.index("prepare") < trace.index("format") < trace.index("commit") < trace.index("push") < trace.index("create") < trace.index("arm")
    summary = ("docs/adr/drafts/alpha.md -> docs/adr/0002-alpha.md (status: proposed -> accepted); "
        "docs/adr/drafts/beta.md -> docs/adr/0003-beta.md (status: proposed -> accepted)"
        " — README table synced (3 rows (2 added, 0 removed)); staged")
    assert_complete_promotion(tmp_path, before, summary)


def test_dequeue_absent_pr_does_not_probe_or_mutate_other_state(world):
    result = run(world, dequeue())
    assert result.kind == "NotRearmed" and "pull request is absent" in result.detail
    assert world.trace == ["get"]


def test_dequeue_requires_the_requested_pr_number(world, monkeypatch):
    existing(world)
    monkeypatch.setattr(world.api, "get_pr", lambda number: pr(8))
    assert run(world, dequeue()).kind == "NotRearmed"
    assert "arm" not in world.trace


def test_missing_workflow_event_path_is_an_error_before_adapter_construction(tmp_path, monkeypatch):
    monkeypatch.delenv("GITHUB_EVENT_PATH", raising=False)
    with pytest.raises(ValueError, match="GITHUB_EVENT_PATH is not set"):
        controller.execute(tmp_path)
