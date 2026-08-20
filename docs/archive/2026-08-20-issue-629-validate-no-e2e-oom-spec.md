# Issue #629: Validate (no e2e) null-step runner loss

## Problem

Issue #629 reports one `Validate (no e2e)` GitHub Actions job that failed with
an empty step list and unavailable logs. The retained job metadata still matches
that signature:

- job `89231127491`, run `30014588401`, workflow `CI`, branch `main`;
- job name `Validate (no e2e)`, conclusion `failure`;
- `steps: []` in the Actions job API response;
- `gh run view --job 89231127491 --log` returns `log not found: 89231127491`.

That is runner-loss shaped, plausibly OOM, but not a test/build assertion and
not root-caused.

## Current evidence

The issue comment on 2026-07-24 already deprioritized #629 to P4 because the
signature occurred once in a 60-run sample and reruns passed.

A fresh sample of recent CI does not show recurrence:

- `gh run list --workflow ci.yml --limit 30` showed no completed workflow
  failures among the latest 30 runs; only current in-progress runs and
  successes.
- `gh run list --workflow ci.yml --status failure --limit 50` found historical
  failures. The newest inspected `Validate (no e2e)` failure, run `31842589601`
  / job `94902457387`, had normal logs, `xtask-done`, and Rust compile errors
  (`DbConnectOptions` not in scope), not the null-step/log-missing signature.
- Current CI pins `Validate (no e2e)` to `ubuntu-24.04`; GitHub's hosted-runner
  reference lists public `ubuntu-24.04` Linux runners as 4 CPU / 16 GB RAM. The
  original failing job metadata reports label `ubuntu-latest`, before the
  workflow pin documented in `.github/workflows/ci.yml`.

## Decision

Do **not** change CI concurrency or runner class from this issue.

Reason: there is no current red-capable feedback loop. A cargo job cap, nextest
thread cap, or larger runner would trade CI time/cost for an unmeasured and
unreproduced failure. That is the exact risk the issue comment warned about:
generalizing from one un-root-caused infra failure.

Resolve #629 as stale/unreproduced with an evidence comment, leaving the
recurrence criterion explicit: reopen or file a fresh issue only if the
null-step + `log not found` signature recurs in production CI.

## Out of scope

- No `cargo`/nextest/Nix concurrency cap.
- No larger GitHub runner.
- No synthetic repeated CI dispatch just to try to catch a one-off runner loss.
- No local reproduction claim: local `cargo xtask validate --no-e2e` is not the
  GitHub hosted-runner memory environment.

## Acceptance criteria

- #629 has a closing comment with the evidence above: original job metadata,
  missing original log, fresh recent-run sample, and inspected non-matching
  `Validate (no e2e)` failure.
- #629 is closed with `not planned` / stale-unreproduced rationale, not
  represented as fixed by code.
- The Jaunder Backlog project Status is released to `Done` after closure.
- No production, workflow, or lockfile changes are made for #629.
