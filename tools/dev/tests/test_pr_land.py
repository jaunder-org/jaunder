"""`dev pr-land` in process, against a scripted `gh` and `git` and a virtual
clock: the real observer decides every verdict, so these replies are GitHub's
shapes, shared with `test_pr_watch.py`. Never a real PR or merge.

The PR route answers both the observer's polls and the lander's own live reads,
in the order they happen.
"""

import json
import os
from typing import Any

import pytest
from test_pr_watch import (
    CI_FILE,
    CI_SOURCE,
    GATE,
    HEAD,
    LANE,
    MERGE,
    NIX_LOG,
    NOT_FOUND,
    TEST_FAILURE_LOG,
    TRANSPORT,
    WORKFLOW,
    Clock,
    Native,
    contents,
    failed_jobs,
    green,
    merged,
    native,
    run,
    runs,
)

from dev import pr_land, pr_watch, steps

MERGE_CALL = ["gh", "pr", "merge", "731", "--repo", "jaunder-org/jaunder"]
MERGE_CALL += ["--auto", "--merge", "--match-head-commit", HEAD]
LOCAL = {
    "branch --show-current": [(0, "issue-731-example\n", "")],
    "rev-parse --verify HEAD": [(0, f"{HEAD}\n", "")],
}
# PR #2016's ejection replayed as PR 731's: its run and job ids, its failed
# jobs and its recorded log, over the shape of ci.yml. A later run fails afresh.
REPLAYED, LANE_JOB, GATE_JOB = 44632169176, 135941369307, 135954057965
INFRASTRUCTURE = run(run_id=REPLAYED, workflow=WORKFLOW)
RUN_URL = f"https://github.com/o/r/actions/runs/{REPLAYED}"
FRESH_URL = "https://github.com/o/r/actions/runs/10"
# The same run unclassified: its lane's log shows a real test failure.
REAL_FAILURE = {f"log:{LANE_JOB}": [(0, TEST_FAILURE_LOG, "")]}


def fresh(workflow: str | None = WORKFLOW) -> Any:
    return run(created="2026-07-30T16:00:00Z", run_id=10, workflow=workflow)


def armed() -> Any:
    return green(armed=True)


def scripted(*snapshots: Any, ejections: list[Any], **routes: Any) -> Native:
    """The PR replies in order, each merge-group probe's newest run in order,
    and PR #2016's recorded jobs, workflow and log."""
    replay: dict[str, Any] = {
        "runs": [runs(*([] if found is None else [found])) for found in ejections],
        CI_FILE: [contents(CI_SOURCE)],
        "jobs": [failed_jobs((LANE_JOB, LANE), (GATE_JOB, GATE))],
        f"log:{LANE_JOB}": [(0, NIX_LOG, "")],
        "merge": [(0, "", "")],
    }
    return native(*snapshots, **(replay | LOCAL | routes))


@pytest.fixture
def lands(capsys: pytest.CaptureFixture[str]) -> Any:
    """Runs `dev pr-land 731 HEAD`: its status, report and stderr."""

    def invoke(fake: Native, reenqueue: bool = False) -> tuple[int, Any, str]:
        clock = Clock()
        try:
            land = pr_land.Land(731, HEAD, reenqueue)
            pr_land.run(land, fake, clock.time, clock.sleep)
            code = 0
        except steps.StepFailed as failure:
            code = failure.code
        out, err = capsys.readouterr()
        return code, json.loads(out) if out else None, err

    return invoke


def merges(fake: Native) -> int:
    return fake.calls.count(MERGE_CALL)


def test_an_approved_head_is_armed_once_and_observed_until_merged(lands: Any) -> None:
    fake = scripted(green(), green(), green(), armed(), merged(), ejections=[None])
    code, report, err = lands(fake)
    assert code == 0, err
    assert (report["outcome"], report["pointer"]) == ("merged", MERGE)
    assert merges(fake) == 1
    assert "Native action readback verified" in err


def test_pr_2016s_ejection_is_re_enqueued_once_and_merges(lands: Any) -> None:
    # Replays PR #2016's infrastructure ejection after the arm.
    fake = scripted(
        *(green(), green(), green(), armed(), green()),
        *(green(), green(), armed(), merged()),
        ejections=[None, INFRASTRUCTURE],
    )
    code, report, err = lands(fake)
    assert code == 0, err
    assert report["outcome"] == "merged"
    assert merges(fake) == 2
    assert f"re-enqueueing once: nix-substitute ({RUN_URL})" in err
    assert f"infrastructure: nix-substitute ({RUN_URL})" in err


def test_an_infrastructure_ejection_seen_first_is_re_enqueued_too(lands: Any) -> None:
    fake = scripted(
        green(),
        green(),
        green(),
        green(),
        armed(),
        merged(),
        ejections=[INFRASTRUCTURE],
    )
    code, report, err = lands(fake)
    assert (code, report["outcome"], merges(fake)) == (0, "merged", 1)
    assert f"re-enqueueing once: nix-substitute ({RUN_URL})" in err


def test_a_second_ejection_is_terminal_whatever_its_cause(lands: Any) -> None:
    fake = scripted(
        green(),
        green(),
        green(),
        green(),
        armed(),
        green(),
        ejections=[INFRASTRUCTURE, INFRASTRUCTURE, fresh()],
    )
    code, report, _ = lands(fake)
    assert (code, report["outcome"], report["pointer"]) == (1, "ejected", FRESH_URL)
    assert report["ejection"]["signature"] == "nix-substitute"
    assert merges(fake) == 1


def test_an_explicit_recovery_spends_the_one_re_enqueue(lands: Any) -> None:
    # The preflight ejection is a real failure; the later one is infrastructure.
    fake = scripted(
        green(),
        green(),
        green(),
        green(),
        armed(),
        green(),
        ejections=[INFRASTRUCTURE, INFRASTRUCTURE, fresh()],
        **{f"log:{LANE_JOB}": [(0, TEST_FAILURE_LOG, ""), (0, NIX_LOG, "")]},
    )
    code, report, err = lands(fake, reenqueue=True)
    assert (code, report["outcome"], merges(fake)) == (1, "ejected", 1)
    assert report["ejection"]["signature"] == "nix-substitute"
    assert f"re-enqueueing once: --reenqueue ({RUN_URL})" in err


def test_explicit_recovery_requires_a_live_ejection(lands: Any) -> None:
    fake = scripted(green(), green(), ejections=[None])
    code, report, err = lands(fake, reenqueue=True)
    assert (code, report, merges(fake)) == (1, None, 0)
    assert "--reenqueue requires a live ejection; observed ready-to-land" in err


@pytest.mark.parametrize(
    "routes",
    [
        REAL_FAILURE,
        {
            "jobs": [
                failed_jobs((LANE_JOB, LANE), (9, "Validation host"), (GATE_JOB, GATE))
            ]
        },
        {f"log:{LANE_JOB}": [TRANSPORT]},
    ],
)
def test_an_unclassified_ejection_after_the_arm_is_terminal(
    lands: Any, routes: dict[str, Any]
) -> None:
    routes.setdefault("log:9", [(0, TEST_FAILURE_LOG, "")])
    fake = scripted(
        green(),
        green(),
        green(),
        armed(),
        green(),
        ejections=[None, INFRASTRUCTURE],
        **routes,
    )
    code, report, err = lands(fake)
    assert (code, report["outcome"], merges(fake)) == (1, "ejected", 1)
    assert report["ejection"]["signature"] is None
    assert "re-enqueueing" not in err


def test_an_unclassified_preflight_ejection_is_refused_without_action(
    lands: Any,
) -> None:
    fake = scripted(green(), green(), ejections=[INFRASTRUCTURE], **REAL_FAILURE)
    code, report, err = lands(fake)
    assert (code, report, merges(fake)) == (1, None, 0)
    assert "dev: pr-land: not ready to land: ejected" in err
    assert f"job '{LANE}' matches no infrastructure signature" in err


def test_a_re_enqueue_that_is_not_ready_again_ends_with_that_report(
    lands: Any,
) -> None:
    fake = scripted(
        green(),
        green(),
        green(),
        armed(),
        green(),
        green(merge_state="BLOCKED"),
        ejections=[None, INFRASTRUCTURE],
    )
    code, report, _ = lands(fake)
    assert (code, report["outcome"], merges(fake)) == (1, "blocked", 1)


def wrong_origin(fake: Native) -> None:
    fake.routes["remote get-url origin"] = [(0, "https://github.com/o/r.git\n", "")]


def no_origin(fake: Native) -> None:
    fake.routes["remote get-url origin"] = [(1, "", "no origin")]


def other_head(fake: Native) -> None:
    fake.routes["pr"][0] = (0, json.dumps(green(head=MERGE)), "")


def other_base(fake: Native) -> None:
    body = green()
    body["data"]["repository"]["pullRequest"]["baseRefName"] = "release"
    fake.routes["pr"][0] = (0, json.dumps(body), "")


def closed(fake: Native) -> None:
    fake.routes["pr"][0] = (0, json.dumps(green(state="CLOSED")), "")


def unreadable(fake: Native) -> None:
    fake.routes["pr"][0] = TRANSPORT


def diverged(fake: Native) -> None:
    fake.routes["rev-parse --verify HEAD"] = [(0, f"{MERGE}\n", "")]


def no_branch(fake: Native) -> None:
    fake.routes["branch --show-current"] = [(128, "", "not a repository")]


def not_ready(fake: Native) -> None:
    fake.routes["pr"][1] = (0, json.dumps(green(merge_state="BLOCKED")), "")


def moved_before_the_action(fake: Native) -> None:
    fake.routes["pr"][2] = (0, json.dumps(green(head=MERGE)), "")


def armed_already(fake: Native) -> None:
    fake.routes["pr"][2] = (0, json.dumps(armed()), "")


def missing_pr(fake: Native) -> None:
    fake.routes["pr"][0] = NOT_FOUND


@pytest.mark.parametrize(
    ("fault", "message"),
    [
        (wrong_origin, "requires the jaunder-org/jaunder origin"),
        (no_origin, "requires the jaunder-org/jaunder origin"),
        (other_head, f"requires PR head {HEAD} on main; observed {MERGE} on main"),
        (other_base, "on main; observed"),
        (closed, "the PR is closed"),
        (unreadable, "the live PR read failed: transport"),
        (diverged, "local PR branch head diverges"),
        (no_branch, "git branch --show-current failed"),
        (not_ready, "not ready to land: blocked (GitHub reports merge state BLOCKED)"),
        (moved_before_the_action, f"requires PR head {HEAD} on main; observed {MERGE}"),
        (armed_already, "neither queued nor armed"),
        (missing_pr, "no such pull request"),
    ],
)
def test_every_guard_refuses_before_the_action(
    lands: Any, fault: Any, message: str
) -> None:
    fake = scripted(green(), green(), green(), armed(), merged(), ejections=[None])
    fault(fake)
    code, report, err = lands(fake)
    assert (code, report, merges(fake)) == (1, None, 0)
    assert message in err


@pytest.mark.parametrize(
    ("readback", "action", "message"),
    [
        (green(), (0, "", ""), "did not establish arm, queue or complete merged"),
        (armed(), (1, "", "merge refused"), "the merge action failed"),
    ],
)
def test_a_failed_or_silent_action_is_never_retried(
    lands: Any, readback: Any, action: Any, message: str
) -> None:
    fake = scripted(
        green(),
        green(),
        green(),
        readback,
        merged(),
        ejections=[None],
        merge=[action],
    )
    code, _, err = lands(fake)
    assert (code, merges(fake)) == (1, 1)
    assert message in err


@pytest.mark.parametrize("readback", ["queued", "merged"])
def test_an_immediate_queue_or_merge_readback_is_the_arm(
    lands: Any, readback: str
) -> None:
    body = green(queued=True) if readback == "queued" else merged()
    fake = scripted(green(), green(), green(), body, merged(), ejections=[None])
    code, report, err = lands(fake)
    assert (code, report["outcome"], merges(fake)) == (0, "merged", 1), err


def test_a_queued_readback_seeds_the_observers_queue_history(lands: Any) -> None:
    # A same-head entry that vanishes before the first poll is a dequeue.
    fake = scripted(
        green(),
        green(),
        green(),
        green(queued=True),
        green(),
        green(),
        ejections=[None],
    )
    code, report, _ = lands(fake)
    assert (code, report["outcome"]) == (1, "dequeued")


def test_a_defect_exits_2_without_a_report(
    lands: Any, monkeypatch: pytest.MonkeyPatch
) -> None:
    def broken(*_: Any) -> Any:
        raise RuntimeError("defect")

    monkeypatch.setattr(pr_watch, "observe", broken)
    fake = scripted(green(), ejections=[None])
    code, report, err = lands(fake)
    assert (code, report, merges(fake)) == (2, None, 0)
    assert "RuntimeError: defect" in err


def test_a_termination_request_passes_through(
    lands: Any, monkeypatch: pytest.MonkeyPatch
) -> None:
    def terminated(*_: Any) -> Any:
        raise steps.StepFailed(143, signalled=True)

    monkeypatch.setattr(pr_watch, "observe", terminated)
    code, _, _ = lands(scripted(green(), ejections=[None]))
    assert code == 143


def test_the_callers_git_redirects_are_scrubbed_first(
    lands: Any, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in pr_land.GIT_REDIRECTS:
        monkeypatch.setenv(name, "foreign")
    lands(scripted(green(), green(), green(), armed(), merged(), ejections=[None]))
    assert not set(pr_land.GIT_REDIRECTS) & set(os.environ)


def test_a_head_pushed_after_the_arm_is_stale_not_merged(lands: Any) -> None:
    fake = scripted(
        green(), green(), green(), armed(), merged(head=MERGE), ejections=[None]
    )
    code, report, _ = lands(fake)
    assert (code, report["outcome"], merges(fake)) == (1, "stale", 1)


def test_merged_without_a_commit_is_not_merge_evidence(lands: Any) -> None:
    incomplete = merged()
    incomplete["data"]["repository"]["pullRequest"]["mergeCommit"] = None
    fake = scripted(green(), green(), green(), armed(), incomplete, ejections=[None])
    code, report, err = lands(fake)
    assert (code, report["outcome"]) == (1, "merged")
    assert "incomplete merged evidence" in err
