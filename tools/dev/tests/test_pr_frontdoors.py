"""PR land and cleanup front doors through `dev`, against scripted gh, git and
cargo: never a real PR, merge or checkout cleanup. One cleanup also runs
against real Git in disposable repositories, never this checkout.

They run the repository's scripts and jq, so only the filtered
`checks.dev-python` source may skip them.
"""

import copy
import json
import os
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from conftest import REAL_GIT, REPOSITORY, Checkout, require

HEAD = "a" * 40
MERGE = "b" * 40
HISTORICAL = "https://github.com/jaunder-org/jaunder/actions/runs/42"
BASH = shutil.which("bash") or "/bin/sh"
# Each native call must be the next scripted one. Checkout discovery, which
# `dev` itself makes, goes to the real Git unrecorded. The observer's build
# only appends to `builds`: the fixture already put the scripted observer where
# the build leaves it.
NATIVE = f"""#!{BASH}
set -euo pipefail
tool=${{0##*/}}
if [[ "$tool $*" == 'git rev-parse --show-toplevel' ]]; then exec "$REAL_GIT" "$@"; fi
if [[ "$tool" == git ]]; then
    [[ -z "${{GIT_DIR+x}}${{GIT_WORK_TREE+x}}${{GIT_INDEX_FILE+x}}${{GIT_OBJECT_DIRECTORY+x}}${{GIT_COMMON_DIR+x}}${{GIT_NAMESPACE+x}}" ]] || {{ echo 'Git redirect not scrubbed' >&2; exit 23; }}
fi
if [[ "$tool $1" == 'cargo build' ]]; then printf '%s\\n' "$*" >> "$FIXTURE/builds"; exit 0; fi
args=$(jq -cn --args '$ARGS.positional' -- "$@")
jq -cn --arg tool "$tool" --argjson args "$args" '{{tool:$tool,args:$args}}' >> "$FIXTURE/calls"
n=$(<"$FIXTURE/index"); printf '%s' "$((n+1))" > "$FIXTURE/index"
reply=$(jq -ce --argjson n "$n" --arg tool "$tool" --argjson args "$args" '.[$n] | if .tool==$tool and .args==$args then . else error("unexpected native call") end' "$FIXTURE/responses")
if [[ "$tool" == gh ]]; then jq -c '.body' <<< "$reply"; else jq -rj '.text // ""' <<< "$reply"; fi
if [[ $(jq -r '.exit // 0' <<< "$reply") != 0 ]]; then echo 'simulated native failure' >&2; exit 1; fi
"""
LIVE_QUERY = (
    'query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){'
    "pullRequest(number:$number){number state baseRefName headRefName headRefOid "
    "isInMergeQueue autoMergeRequest{enabledAt} mergedAt mergeCommit{oid}}}}"
)


@dataclass
class Native:
    checkout: Checkout

    def dev(self, *args: str, calls: list[Any]) -> subprocess.CompletedProcess[bytes]:
        fixture = self.checkout.root.parent
        (fixture / "responses").write_text(json.dumps(calls))
        (fixture / "index").write_text("0")
        return self.checkout.dev(
            *args,
            FIXTURE=str(fixture),
            REAL_GIT=REAL_GIT,
            GIT_DIR="foreign",
            GIT_INDEX_FILE="foreign-index",
        )

    def land(
        self, recovery: bool, calls: list[Any]
    ) -> subprocess.CompletedProcess[bytes]:
        mode = "--reenqueue" if recovery else "ordinary"
        return self.dev("pr-land", "731", HEAD, mode, calls=calls)

    def cleanup(
        self, ownership: str, calls: list[Any]
    ) -> subprocess.CompletedProcess[bytes]:
        return self.dev("pr-cleanup", "731", HEAD, ownership, calls=calls)

    def count(self) -> int:
        return int((self.checkout.root.parent / "index").read_text())


@pytest.fixture
def native(checkout: Checkout) -> Native:
    require(shutil.which("jq") is not None, "jq")
    checkout.from_repository("scripts/pr-land.sh", "scripts/pr-cleanup.sh")
    for tool in ["gh", "git", "cargo"]:
        checkout.executable(tool, NATIVE)
    observer = checkout.root / "tools" / "target" / "debug" / "pr-observer"
    observer.parent.mkdir(parents=True)
    observer.write_text(NATIVE)
    observer.chmod(0o755)
    # pr-land.sh observes through `dev pr-watch`.
    checkout.dev_on_path()
    return Native(checkout)


def live(armed: bool) -> dict[str, Any]:
    return {
        "tool": "gh",
        "args": ["api", "graphql", "-f", f"query={LIVE_QUERY}", "-F", "number=731"],
        "body": {
            "data": {
                "repository": {
                    "pullRequest": {
                        "number": 731,
                        "state": "OPEN",
                        "baseRefName": "main",
                        "headRefName": "issue-731-example",
                        "headRefOid": HEAD,
                        "isInMergeQueue": False,
                        "autoMergeRequest": {"enabledAt": "now"} if armed else None,
                    }
                }
            }
        },
    }


def observation(outcome: str, recovery: bool, once: bool) -> dict[str, Any]:
    args = ["watch", "731"]
    args += ["--once"] if once else ["--until", "merged"]
    args += ["--expected-head", HEAD]
    if recovery:
        args += ["--acknowledged-ejection", HISTORICAL]
    report = {
        "outcome": outcome,
        "pr": 731,
        "head_sha": HEAD,
        "pointer": HISTORICAL if outcome == "ejected" else MERGE,
        "detail": "2026-11-08T00:00:00Z",
        "events": [],
    }
    return {
        "tool": "pr-observer",
        "args": args,
        "exit": 0 if outcome in ("merged", "ready-to-land") else 1,
        "text": json.dumps(report),
    }


def land_calls(recovery: bool) -> list[Any]:
    calls: list[Any] = [
        {
            "tool": "git",
            "args": ["remote", "get-url", "origin"],
            "text": "https://github.com/jaunder-org/jaunder.git\n",
        },
        live(False),
        {
            "tool": "git",
            "args": ["branch", "--show-current"],
            "text": "issue-731-example\n",
        },
        {"tool": "git", "args": ["rev-parse", "--verify", "HEAD"], "text": HEAD},
        observation("ejected" if recovery else "ready-to-land", False, True),
    ]
    if recovery:
        calls.append(observation("ready-to-land", True, True))
    calls += [
        live(False),
        {
            "tool": "gh",
            "args": ["pr", "merge", "731", "--repo", "jaunder-org/jaunder", "--auto"]
            + ["--merge", "--match-head-commit", HEAD],
        },
        live(True),
        observation("merged", recovery, False),
    ]
    return calls


def pull_request(call: dict[str, Any]) -> dict[str, Any]:
    pr: dict[str, Any] = call["body"]["data"]["repository"]["pullRequest"]
    return pr


def rewrite_report(call: dict[str, Any], **changes: Any) -> None:
    call["text"] = json.dumps(json.loads(call["text"]) | changes)


RECOVERY = [False, True]


@pytest.mark.parametrize("recovery", RECOVERY)
def test_an_approved_land_makes_one_head_bound_action_then_observes(
    native: Native, recovery: bool
) -> None:
    calls = land_calls(recovery)
    result = native.land(recovery, calls)
    assert result.returncode == 0, result
    assert native.count() == len(calls)
    report = json.loads(result.stdout)
    assert (report["outcome"], report["head_sha"]) == ("merged", HEAD)
    # Each observation is its own `dev pr-watch`, which builds the observer.
    observations = [call for call in calls if call["tool"] == "pr-observer"]
    builds = (native.checkout.root.parent / "builds").read_text().splitlines()
    assert builds == [
        (
            "build --locked --quiet --manifest-path tools/Cargo.toml "
            "--target-dir tools/target -p pr-observer --bin pr-observer"
        )
    ] * len(observations)


ADVERSE = ["pending", "blocked", "conflicted", "checks-failed", "ejected"]
ADVERSE += ["watcher-error", "stale", "merged"]


@pytest.mark.parametrize("recovery", RECOVERY)
@pytest.mark.parametrize("outcome", ADVERSE)
def test_land_refuses_an_adverse_or_incomplete_observation_before_mutation(
    native: Native, recovery: bool, outcome: str
) -> None:
    calls = land_calls(recovery)
    index = 5 if recovery else 4
    calls[index] = observation(outcome, recovery, True)
    assert native.land(recovery, calls).returncode != 0, outcome
    assert native.count() == index + 1, f"no action after {outcome}"


@pytest.mark.parametrize("recovery", RECOVERY)
def test_land_refuses_a_changed_live_head(native: Native, recovery: bool) -> None:
    for index in [1, 6 if recovery else 5]:
        calls = land_calls(recovery)
        pull_request(calls[index])["headRefOid"] = MERGE
        assert native.land(recovery, calls).returncode != 0
        assert native.count() == index + 1


@pytest.mark.parametrize(
    "outcome", ["ready-to-land", "pending", "checks-failed", "watcher-error"]
)
def test_recovery_requires_a_live_ejection(native: Native, outcome: str) -> None:
    calls = land_calls(True)
    calls[4] = observation(outcome, False, True)
    assert native.land(True, calls).returncode != 0
    assert native.count() == 5


@pytest.mark.parametrize(
    ("field", "value"),
    [("isInMergeQueue", True), ("autoMergeRequest", {"enabledAt": "now"})],
)
def test_recovery_refuses_an_armed_or_queued_preflight(
    native: Native, field: str, value: Any
) -> None:
    calls = land_calls(True)
    pull_request(calls[1])[field] = value
    assert native.land(True, calls).returncode != 0
    assert native.count() == 5


def wrong_origin(calls: list[Any]) -> int:
    calls[0]["text"] = "https://github.com/other/repo.git"
    return 1


def local_divergence(calls: list[Any]) -> int:
    calls[3]["text"] = MERGE
    return 4


def wrong_base(calls: list[Any]) -> int:
    pull_request(calls[1])["baseRefName"] = "release"
    return 2


def wrong_reported_head(calls: list[Any]) -> int:
    rewrite_report(calls[4], head_sha=MERGE)
    return 5


def missing_arm(calls: list[Any]) -> int:
    del pull_request(calls[1])["autoMergeRequest"]
    return 2


def malformed_arm(calls: list[Any]) -> int:
    pull_request(calls[1])["autoMergeRequest"] = "armed"
    return 2


def api_errors(calls: list[Any]) -> int:
    calls[1]["body"]["errors"] = [{"message": "unavailable"}]
    return 2


@pytest.mark.parametrize("recovery", RECOVERY)
@pytest.mark.parametrize(
    "fault",
    [wrong_origin, local_divergence, wrong_base, wrong_reported_head, missing_arm]
    + [malformed_arm, api_errors],
)
def test_land_rejects_wrong_repository_divergence_base_and_malformed_arm_evidence(
    native: Native, recovery: bool, fault: Any
) -> None:
    calls = land_calls(recovery)
    expected = fault(calls)
    assert native.land(recovery, calls).returncode != 0
    assert native.count() == expected


@pytest.mark.parametrize("recovery", RECOVERY)
def test_an_inconsistent_success_report_status_is_not_permission(
    native: Native, recovery: bool
) -> None:
    calls = land_calls(recovery)
    index = 5 if recovery else 4
    calls[index]["exit"] = 1
    assert native.land(recovery, calls).returncode != 0
    assert native.count() == index + 1


@pytest.mark.parametrize("recovery", RECOVERY)
@pytest.mark.parametrize("state", ["queued", "merged"])
def test_an_immediate_queue_or_merged_readback_is_not_a_silent_no_op(
    native: Native, recovery: bool, state: str
) -> None:
    calls = land_calls(recovery)
    readback = 8 if recovery else 7
    pr = pull_request(calls[readback])
    pr["autoMergeRequest"] = None
    if state == "queued":
        pr["isInMergeQueue"] = True
        # Only a positive same-head queue readback gets the history input.
        calls[readback + 1]["args"] += ["--observed-queued-head", HEAD]
    else:
        pr |= {
            "state": "MERGED",
            "mergedAt": "2026-11-08T00:00:00Z",
            "mergeCommit": {"oid": MERGE},
        }
    result = native.land(recovery, calls)
    assert result.returncode == 0, result
    assert native.count() == readback + 2


def action_fails(calls: list[Any], action: int, recovery: bool) -> int:
    calls[action]["exit"] = 1
    return action + 1


def action_is_a_no_op(calls: list[Any], action: int, recovery: bool) -> int:
    calls[action + 1] = live(False)
    return action + 2


def readback_fails(calls: list[Any], action: int, recovery: bool) -> int:
    calls[action + 1]["exit"] = 1
    return action + 2


def fresh_ejection(calls: list[Any], action: int, recovery: bool) -> int:
    calls[action + 2] = observation("ejected", recovery, False)
    return action + 3


def checks_failed(calls: list[Any], action: int, recovery: bool) -> int:
    calls[action + 2] = observation("checks-failed", recovery, False)
    return action + 3


def head_drift(calls: list[Any], action: int, recovery: bool) -> int:
    rewrite_report(calls[action + 2], head_sha=MERGE)
    return action + 3


def incomplete_merge(calls: list[Any], action: int, recovery: bool) -> int:
    rewrite_report(calls[action + 2], pointer=None)
    return action + 3


@pytest.mark.parametrize("recovery", RECOVERY)
@pytest.mark.parametrize(
    "fault",
    [action_fails, action_is_a_no_op, readback_fails, fresh_ejection, checks_failed]
    + [head_drift, incomplete_merge],
)
def test_no_action_error_or_post_action_uncertainty_ever_rearms(
    native: Native, recovery: bool, fault: Any
) -> None:
    calls = land_calls(recovery)
    expected = fault(calls, 7 if recovery else 6, recovery)
    assert native.land(recovery, calls).returncode != 0
    assert native.count() == expected, "no retry"


SNAPSHOT = {
    "number": 731,
    "state": "MERGED",
    "baseRefName": "main",
    "headRefName": "issue-731-example",
    "headRefOid": HEAD,
    "mergedAt": "2026-11-08T00:00:00Z",
    "mergeCommit": {"oid": MERGE},
}
VIEW = ["pr", "view", "731", "--repo", "jaunder-org/jaunder", "--json"]
VIEW += ["number,state,baseRefName,headRefName,headRefOid,mergedAt,mergeCommit"]


def cleanup_calls() -> list[Any]:
    return [
        {"tool": "gh", "args": VIEW, "body": copy.deepcopy(SNAPSHOT)},
        {
            "tool": "git",
            "args": ["branch", "--show-current"],
            "text": "issue-731-example\n",
        },
        {"tool": "git", "args": ["rev-parse", "--verify", "HEAD"], "text": f"{HEAD}\n"},
        {
            "tool": "git",
            "args": ["status", "--porcelain=v1", "--untracked-files=all"],
            "text": "",
        },
        {"tool": "git", "args": ["fetch", "origin"]},
        {"tool": "git", "args": ["merge-base", "--is-ancestor", HEAD, "origin/main"]},
        {"tool": "git", "args": ["checkout", "--detach", "origin/main"]},
        {
            "tool": "git",
            "args": ["branch", "-d", "--", "issue-731-example"],
            "text": "Deleted branch issue-731-example (was aaaaaaa).\n",
        },
        {"tool": "cargo", "args": ["clean"]},
    ]


def test_unmanaged_cleanup_keeps_the_exact_guarded_sequence_and_merge_evidence(
    native: Native,
) -> None:
    result = native.cleanup("unmanaged", cleanup_calls())
    assert result.returncode == 0, result
    report = json.loads(result.stdout)
    assert (report["outcome"], report["head_sha"], report["merge_commit"]) == (
        "cleaned",
        HEAD,
        MERGE,
    )
    assert native.count() == 9
    assert b"Deleted branch issue-731-example" in result.stderr


def test_a_fork_head_named_main_cannot_delete_the_protected_local_main(
    native: Native,
) -> None:
    calls = cleanup_calls()
    calls[0]["body"]["headRefName"] = "main"
    calls[1]["text"] = "main\n"
    assert native.cleanup("unmanaged", calls).returncode != 0
    assert native.count() == 4, "refuse local main before fetch"


@pytest.mark.parametrize("ownership", ["managed", "unknown", ""])
def test_managed_or_unknown_cleanup_refuses_before_native_operations(
    native: Native, ownership: str
) -> None:
    assert native.cleanup(ownership, []).returncode != 0
    assert native.count() == 0


@pytest.mark.parametrize(
    ("key", "value"),
    [
        ("number", 732),
        ("baseRefName", "release"),
        ("state", "OPEN"),
        ("headRefOid", MERGE),
        ("mergedAt", None),
        ("mergeCommit", None),
    ],
)
def test_wrong_or_incomplete_merge_evidence_refuses_cleanup(
    native: Native, key: str, value: Any
) -> None:
    calls = cleanup_calls()
    calls[0]["body"][key] = value
    assert native.cleanup("unmanaged", calls).returncode != 0, key
    assert native.count() == 1, key


@pytest.mark.parametrize(
    ("index", "text"),
    [(1, "different\n"), (1, ""), (2, "wrong-head\n"), (3, "?? untracked\n")],
)
def test_a_wrong_local_branch_head_or_dirty_tree_prevents_fetch(
    native: Native, index: int, text: str
) -> None:
    calls = cleanup_calls()
    calls[index]["text"] = text
    assert native.cleanup("unmanaged", calls).returncode != 0
    assert native.count() == 4


@pytest.mark.parametrize("index", range(9))
def test_a_native_failure_stops_before_later_cleanup_steps(
    native: Native, index: int
) -> None:
    calls = cleanup_calls()
    calls[index]["exit"] = 1
    result = native.cleanup("unmanaged", calls)
    assert result.returncode != 0
    assert native.count() == index + 1
    assert result.stdout == b""


def real_git(cwd: Path, *args: str) -> subprocess.CompletedProcess[str]:
    """Real Git with a fixture identity and no signing, so fixture commits
    cannot pick up configuration from the managed checkout."""
    return subprocess.run(
        [REAL_GIT, "-c", "user.name=Cleanup fixture"]
        + ["-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false"]
        + ["-C", str(cwd), *args],
        capture_output=True,
        check=False,
        text=True,
        env=os.environ | {"LC_ALL": "C"},
    )


def real_git_ok(cwd: Path, *args: str) -> str:
    result = real_git(cwd, *args)
    assert result.returncode == 0, (args, result.stderr)
    return result.stdout.strip()


def test_disposable_native_git_cleanup_fetches_detaches_and_preserves_local_main(
    tmp_path: Path,
) -> None:
    script = REPOSITORY / "scripts" / "pr-cleanup.sh"
    require(script.is_file(), "the repository's scripts/pr-cleanup.sh")
    require(shutil.which("jq") is not None, "jq")
    upstream = tmp_path / "upstream"
    clone = tmp_path / "clone"
    real_git_ok(tmp_path, "init", "--initial-branch=main", str(upstream))
    (upstream / "base.txt").write_text("base\n")
    real_git_ok(upstream, "add", "base.txt")
    real_git_ok(upstream, "commit", "-m", "fixture base")
    base = real_git_ok(upstream, "rev-parse", "HEAD")
    real_git_ok(tmp_path, "clone", "--no-hardlinks", str(upstream), str(clone))
    real_git_ok(clone, "checkout", "-b", "issue-731-example")
    (clone / "feature.txt").write_text("feature\n")
    real_git_ok(clone, "add", "feature.txt")
    real_git_ok(clone, "commit", "-m", "fixture feature")
    head = real_git_ok(clone, "rev-parse", "HEAD")
    real_git_ok(clone, "branch", "--set-upstream-to=origin/main")
    # Only the upstream advances; the clone's main and origin/main stay at
    # base until the cleanup script itself fetches.
    real_git_ok(upstream, "fetch", str(clone), "issue-731-example")
    real_git_ok(upstream, "merge", "--no-ff", "-m", "fixture merge", "FETCH_HEAD")
    merged = real_git_ok(upstream, "rev-parse", "HEAD")
    assert real_git_ok(clone, "rev-parse", "origin/main") == base
    pr = tmp_path / "pr.json"
    pr.write_text(
        json.dumps(
            {
                "number": 731,
                "state": "MERGED",
                "baseRefName": "main",
                "headRefName": "issue-731-example",
                "headRefOid": head,
                "mergedAt": "2026-11-08T00:00:00Z",
                "mergeCommit": {"oid": merged},
            }
        )
    )
    # gh and cargo are fakes; no fake git is installed. Invalid repository
    # redirects stay confined to the fixture if the script stops scrubbing them.
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    view = (
        "pr view 731 --repo jaunder-org/jaunder --json "
        "number,state,baseRefName,headRefName,headRefOid,mergedAt,mergeCommit"
    )
    for tool, body in [
        ("gh", f'[[ "$*" == \'{view}\' ]]\njq -c . "$PR_FIXTURE"\n'),
        ("cargo", '[[ "$*" == clean ]]\nprintf \'clean\\n\' >> "$CARGO_MARKER"\n'),
    ]:
        fake = fakes / tool
        fake.write_text(f"#!{BASH}\nset -euo pipefail\n{body}")
        fake.chmod(0o755)
    marker = tmp_path / "cargo-called"
    result = subprocess.run(
        [BASH, str(script), "731", head, "unmanaged"],
        cwd=clone,
        env=os.environ
        | {
            "LC_ALL": "C",
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "PR_FIXTURE": str(pr),
            "CARGO_MARKER": str(marker),
            "GIT_DIR": str(tmp_path / "nonexistent"),
            "GIT_INDEX_FILE": str(tmp_path / "foreign-index"),
        },
        capture_output=True,
        check=False,
        timeout=60,
    )
    assert result.returncode == 0, result.stderr
    report = json.loads(result.stdout)
    assert (report["outcome"], report["head_sha"], report["merge_commit"]) == (
        "cleaned",
        head,
        merged,
    )
    assert b"Deleted branch issue-731-example" in result.stderr
    assert real_git_ok(clone, "branch", "--show-current") == ""
    assert real_git_ok(clone, "rev-parse", "HEAD") == merged
    assert real_git_ok(clone, "rev-parse", "origin/main") == merged
    assert real_git_ok(clone, "rev-parse", "refs/heads/main") == base
    ref = "refs/heads/issue-731-example"
    assert real_git(clone, "show-ref", "--verify", "--quiet", ref).returncode == 1
    assert marker.read_text() == "clean\n"
