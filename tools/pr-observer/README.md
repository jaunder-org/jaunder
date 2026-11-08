# PR observer

Host-only, read-only GitHub PR evidence observation. From the checkout's
devShell:

```sh
just pr-watch 731
just pr-watch 731 --once
just pr-watch 731 --until merged
```

Omit the number to resolve the open PR for the current branch. The default polls
at 30-second intervals for up to 90 minutes and stops at the next
caller-actionable outcome. `--once` reports one observation; `--until merged`
passively continues across readiness while another actor may arm the merge.
Neither mode authorizes or performs landing, queue re-enqueue, cleanup, or
tracker changes.

Stdout is one JSON observer report (not an xtask command envelope); stderr
carries progress and diagnostics. `outcome` is authoritative:

- `ready-to-land` and `merged` exit 0. Readiness requires nonempty live-ruleset
  requirements, complete current-head evidence, and a landable GitHub state.
  Readiness is **not** approval to merge.
- `checks-failed` means a directly or transitively required check failed;
  `watcher-error` means observation could not establish trustworthy evidence.
- `timed-out` means the observation budget expired; `pending` is a one-shot
  observation whose evidence or checks are still incomplete.
- `blocked`, `conflicted`, `stale`, and `closed-unmerged` preserve GitHub's
  adverse state. `dequeued` means an observed same-head queue entry vanished;
  `ejected` identifies a failed current-head merge-group run.

All adverse/pending outcomes exit 1; invalid arguments or failure to establish a
subject exit 2. The report includes the exact head and available log pointers.
The tool reads required contexts from the live main ruleset, never a local job
allowlist. Empty/incomplete evidence cannot establish success.

Read-only continuation accepts `--expected-head <full-lowercase-40-hex-SHA>` and
`--acknowledged-ejection <GitHub-Actions-run-URL>`. Acknowledgment requires an
expected head. Every verdict, even merged, stays bound to that head; drift is
`stale`. Only the exact acknowledged historical URL is disregarded as an
ejection discriminator, never as a failed verdict; a fresh failure is terminal.
These inputs neither authorize nor perform mutation.

`--observed-queued-head <SHA>` carries positive live same-head queued readback
from the separate native action. It requires and must equal `--expected-head`;
missing/mismatched expected head refuses before observation. Just supplies this
input ONLY after positive queued readback, never from an arm alone or guesswork.
Existing `Progress` preserves a dequeue before observer establishment. No
default behavior, classifier/poller or mutation capability changes.

After recorded exact PR/head human approval, the separate frontdoor is
`just pr-land <PR> <approved-head-SHA> [--reenqueue]`. Recovery additionally
requires explicit one-shot authorization. Just captures the historical URL from
live ejection preflight, asks this SAME classifier for one-snapshot readiness
with only that run acknowledged, makes ONE head-bound native call, verifies
arm/queue/merged readback, then observes the same head. No retries or second
poller/classifier exist. Land exit 0 requires complete same-head merged
evidence; arming/queueing is not merging. Native/preflight refusal may have no
JSON.

The ordinary xtask PR frontdoor/report envelope is retired. Only the privileged
ADR promoter and its required mutation transport/predicate helpers remain there.
Unmanaged cleanup is separate:
`just pr-cleanup <PR> <exact-head-SHA> unmanaged`; never invoke for
managed/uncertain ownership. See CONTRIBUTING for full guards.
