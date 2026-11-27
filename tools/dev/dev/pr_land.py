"""`dev pr-land`: arm one approved pull request head and observe it until merged.

Running it is merge approval for that exact PR and head, which `dev` cannot
attest to: the caller must have recorded it. Each arm is one
`gh pr merge --auto --merge --match-head-commit` call, made only after the
observer (`pr_watch`, in this process) reports `ready-to-land` for the head,
and verified by a live readback. Nothing is retried on a failed, silent or
uncertain action.

An ejection is terminal unless this invocation re-enqueues, which it does at
most once: when the ejection's classification names a known infrastructure
signature, at preflight or after arming; or, for a preflight ejection of any
cause, when the caller passed `--reenqueue`, which then requires one. Any later
ejection ends the invocation, whatever its cause. The observer's final report
is printed on stdout; only a complete `merged` report for the head exits 0.
"""

import json
import os
import re
import sys
import time
import traceback
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, NoReturn

from dev import pr_watch, steps
from dev.pr_watch import GhError, GitHub, Runner, Snapshot, Watch

# The observer infers its repository from origin; landing acts only here.
REPOSITORY = ("jaunder-org", "jaunder")
ORIGINS = (
    "https://github.com/jaunder-org/jaunder.git",
    "https://github.com/jaunder-org/jaunder",
    "git@github.com:jaunder-org/jaunder.git",
)
_SHA = re.compile(r"[0-9a-f]{40}")
# A caller's repository redirects, such as a hook's, would point Git elsewhere.
GIT_REDIRECTS = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
)


@dataclass(frozen=True)
class Land:
    number: int
    head: str
    reenqueue: bool = False


class Refused(Exception):
    """The invocation stopped before or instead of an action."""


def _refuse(message: str) -> NoReturn:
    raise Refused(message)


class _Lander:
    def __init__(
        self,
        land: Land,
        runner: Runner,
        now: Callable[[], float],
        sleep: Callable[[float], None],
    ) -> None:
        self.land, self.runner, self.now, self.sleep = land, runner, now, sleep
        self.github = GitHub(runner, *REPOSITORY, land.number)
        # The one re-enqueue this invocation may make, and the ejection it
        # answered, which the observer then disregards as an ejection only.
        self.reenqueued = False
        self.acknowledged: str | None = None

    def live(self) -> Snapshot:
        """The PR now, which must still be the approved head on main."""
        try:
            live = self.github.snapshot()
        except GhError as error:
            _refuse(f"the live PR read failed: {error.detail}")
        if live.head != self.land.head or live.base != "main":
            _refuse(
                f"land requires PR head {self.land.head} on main; observed "
                f"{live.head} on {live.base}"
            )
        if live.state not in ("OPEN", "MERGED"):
            _refuse(f"the PR is {live.state.lower()}")
        return live

    def git(self, *args: str) -> str:
        code, output, _ = self.runner(["git", *args])
        if code:
            _refuse(f"git {' '.join(args)} failed")
        return output.strip()

    def observe(self, **options: Any) -> dict[str, Any]:
        watch = Watch(
            self.land.number,
            expected_head=self.land.head,
            acknowledged_ejection=self.acknowledged,
            **options,
        )
        return pr_watch.observe(watch, self.runner, self.now, self.sleep)

    def ready(self) -> None:
        """Requires a one-snapshot `ready-to-land`. A preflight ejection is
        re-enqueued when it is infrastructure, or with `--reenqueue`, which
        needs that live ejection."""
        report = self.observe(once=True)
        ejected = report["outcome"] == "ejected"
        if self.land.reenqueue and not ejected:
            _refuse(
                f"--reenqueue requires a live ejection; observed {report['outcome']}"
            )
        if ejected and self.claim_reenqueue(report, explicit=self.land.reenqueue):
            report = self.observe(once=True)
        if report["outcome"] != "ready-to-land":
            detail = report.get("detail")
            suffix = f" ({detail})" if detail else ""
            _refuse(f"not ready to land: {report['outcome']}{suffix}")

    def claim_reenqueue(self, report: dict[str, Any], explicit: bool) -> bool:
        """Spends the invocation's one re-enqueue on this ejection, if it is
        unspent and the ejection is infrastructure or `explicit`ly recovered;
        the ejection is then acknowledged."""
        signature = (report.get("ejection") or {}).get("signature")
        if self.reenqueued or not (signature or explicit):
            return False
        self.reenqueued, self.acknowledged = True, report["pointer"]
        cause = signature or "--reenqueue"
        print(f"re-enqueueing once: {cause} ({report['pointer']})", file=sys.stderr)
        return True

    def arm(self) -> bool:
        """The one native action, verified; whether the PR is now queued."""
        live = self.live()
        if live.state != "OPEN" or live.queued or live.armed:
            _refuse("land requires an open PR that is neither queued nor armed")
        merge = ["pr", "merge", str(self.land.number), "--repo", "/".join(REPOSITORY)]
        merge += ["--auto", "--merge", "--match-head-commit", self.land.head]
        code, output, diagnostics = self.runner(["gh", *merge])
        sys.stderr.write(output + diagnostics)
        if code:
            _refuse("the merge action failed; inspect, do not retry")
        live = self.live()
        merged = (
            live.state == "MERGED"
            and bool(live.merged_at)
            and _SHA.fullmatch(live.merge_commit or "") is not None
        )
        if not (live.queued or live.armed or merged):
            _refuse(
                "the merge action did not establish arm, queue or complete "
                "merged evidence; inspect, do not retry"
            )
        print(
            "Native action readback verified; observing approved head "
            "(not a merge verdict)",
            file=sys.stderr,
        )
        return live.queued

    def __call__(self) -> dict[str, Any]:
        code, origin, _ = self.runner(["git", "remote", "get-url", "origin"])
        if code or origin.strip() not in ORIGINS:
            _refuse("land requires the jaunder-org/jaunder origin")
        live = self.live()
        branch, local = (
            self.git("branch", "--show-current"),
            self.git("rev-parse", "--verify", "HEAD"),
        )
        if branch == live.branch and local != self.land.head:
            _refuse("the local PR branch head diverges from the approved head")
        self.ready()
        while True:
            # Positive queue readback seeds the observer's queue history, so a
            # dequeue before its first poll is still reported.
            queued = self.land.head if self.arm() else None
            report = self.observe(until="merged", observed_queued_head=queued)
            if report["outcome"] != "ejected" or not self.claim_reenqueue(
                report, explicit=False
            ):
                return report
            if (again := self.observe(once=True))["outcome"] != "ready-to-land":
                return again


def run(
    land: Land,
    runner: Runner = pr_watch.traced,
    now: Callable[[], float] = time.time,
    sleep: Callable[[float], None] = time.sleep,
) -> None:
    """Prints the final observer report; only `merged` exits 0."""
    for name in GIT_REDIRECTS:
        os.environ.pop(name, None)
    try:
        report = _Lander(land, runner, now, sleep)()
    except steps.StepFailed:
        raise
    except (Refused, pr_watch.NoSubject) as refusal:
        print(f"dev: pr-land: {refusal}", file=sys.stderr, flush=True)
        raise steps.StepFailed(1, signalled=False) from None
    except Exception as defect:
        traceback.print_exc()
        raise steps.StepFailed(2, signalled=False) from defect
    print(json.dumps(report, separators=(",", ":"), ensure_ascii=False), flush=True)
    complete = _SHA.fullmatch(report.get("pointer") or "") and report.get("detail")
    if report["outcome"] != "merged":
        raise steps.StepFailed(1, signalled=False)
    if not complete:
        # A merged outcome without its merge commit or detail is not evidence.
        print("dev: pr-land: incomplete merged evidence", file=sys.stderr)
        raise steps.StepFailed(1, signalled=False)
