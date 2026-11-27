"""PR land and cleanup front doors through `dev`, as a caller runs them, against
scripted gh, git and cargo: never a real PR, merge or checkout cleanup. One
cleanup also runs against real Git in disposable repositories, never this
checkout. `test_pr_land.py` proves landing's decisions in process.

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
from test_pr_watch import PRESSURE, E, V, annotations, green, merged, rules, runs

from dev import pr_watch

HEAD = "a" * 40
MERGE = "b" * 40
BASH = shutil.which("bash") or "/bin/sh"
# Each native call must be the next scripted one. Checkout discovery, which
# `dev` itself makes, goes to the real Git unrecorded; any other Git call fails
# unless the caller's repository redirects were scrubbed.
NATIVE = f"""#!{BASH}
set -euo pipefail
tool=${{0##*/}}
if [[ "$tool $*" == 'git rev-parse --show-toplevel' ]]; then exec "$REAL_GIT" "$@"; fi
if [[ "$tool" == git ]]; then
    [[ -z "${{GIT_DIR+x}}${{GIT_WORK_TREE+x}}${{GIT_INDEX_FILE+x}}${{GIT_OBJECT_DIRECTORY+x}}${{GIT_COMMON_DIR+x}}${{GIT_NAMESPACE+x}}" ]] || {{ echo 'Git redirect not scrubbed' >&2; exit 23; }}
fi
args=$(jq -cn --args '$ARGS.positional' -- "$@")
jq -cn --arg tool "$tool" --argjson args "$args" '{{tool:$tool,args:$args}}' >> "$FIXTURE/calls"
n=$(<"$FIXTURE/index"); printf '%s' "$((n+1))" > "$FIXTURE/index"
reply=$(jq -ce --argjson n "$n" --arg tool "$tool" --argjson args "$args" '.[$n] | if .tool==$tool and .args==$args then . else error("unexpected native call") end' "$FIXTURE/responses")
if [[ "$tool" == gh ]]; then jq -c '.body' <<< "$reply"; else jq -rj '.text // ""' <<< "$reply"; fi
if [[ $(jq -r '.exit // 0' <<< "$reply") != 0 ]]; then echo 'simulated native failure' >&2; exit 1; fi
"""


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

    def cleanup(
        self, ownership: str, calls: list[Any]
    ) -> subprocess.CompletedProcess[bytes]:
        return self.dev("pr-cleanup", "731", HEAD, ownership, calls=calls)

    def count(self) -> int:
        return int((self.checkout.root.parent / "index").read_text())


@pytest.fixture
def native(checkout: Checkout) -> Native:
    require(shutil.which("jq") is not None, "jq")
    checkout.from_repository("scripts/pr-cleanup.sh")
    for tool in ["gh", "git", "cargo"]:
        checkout.executable(tool, NATIVE)
    return Native(checkout)


ORIGIN = {
    "tool": "git",
    "args": ["remote", "get-url", "origin"],
    "text": "https://github.com/jaunder-org/jaunder.git\n",
}
PR_READ = ["api", "graphql", "-f", f"query={pr_watch.PR_QUERY}"]
PR_READ += ["-f", "owner=jaunder-org", "-f", "name=jaunder", "-F", "number=731"]
RUNS = "repos/jaunder-org/jaunder/actions/runs?event=merge_group&per_page=100"


def read(body: Any) -> dict[str, Any]:
    return {"tool": "gh", "args": PR_READ, "body": body}


def pressure(oid: str, body: Any) -> dict[str, Any]:
    """The duration-pressure annotation read of commit `oid`."""
    query = ["api", "graphql", "-f", f"query={pr_watch.ANNOTATIONS_QUERY}"]
    query += ["-f", "owner=jaunder-org", "-f", "name=jaunder", "-f", f"oid={oid}"]
    return {"tool": "gh", "args": query, "body": body}


def observed(*polls: Any) -> list[Any]:
    """One observation: origin, the ruleset, then each scripted read."""
    rules_read = ["api", "repos/jaunder-org/jaunder/rules/branches/main"]
    return [ORIGIN, {"tool": "gh", "args": rules_read, "body": rules(V, E)}, *polls]


def test_land_runs_end_to_end_as_a_caller_runs_it(native: Native) -> None:
    calls = [
        ORIGIN,
        read(green()),
        {
            "tool": "git",
            "args": ["branch", "--show-current"],
            "text": "issue-731-example\n",
        },
        {"tool": "git", "args": ["rev-parse", "--verify", "HEAD"], "text": HEAD},
        *observed(
            read(green()),
            {"tool": "gh", "args": ["api", RUNS], "body": runs()},
            pressure(HEAD, annotations()),
        ),
        read(green()),
        {
            "tool": "gh",
            "args": ["pr", "merge", "731", "--repo", "jaunder-org/jaunder", "--auto"]
            + ["--merge", "--match-head-commit", HEAD],
        },
        read(green(armed=True)),
        *observed(
            read(merged()),
            pressure(HEAD, annotations()),
            pressure(MERGE, annotations(e2e=[PRESSURE])),
        ),
    ]
    result = native.dev("pr-land", "731", HEAD, calls=calls)
    assert result.returncode == 0, result
    assert native.count() == len(calls)
    report = json.loads(result.stdout)
    assert (report["outcome"], report["pointer"], report["pr"]) == (
        "merged",
        MERGE,
        731,
    )
    # The merge-group run's duration pressure reaches the land handoff.
    assert [e["detail"] for e in report["events"] if e["kind"] == "warning"] == [
        "e2e: sqlite-chromium-1-of-3: 1 attempt(s)"
    ]


@pytest.mark.parametrize(
    "args", [["0", HEAD], ["731", "A" * 40], ["731", HEAD, "--recover"]]
)
def test_land_refuses_malformed_identity_before_any_call(
    native: Native, args: list[str]
) -> None:
    assert native.dev("pr-land", *args, calls=[]).returncode == 2
    assert native.count() == 0


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
