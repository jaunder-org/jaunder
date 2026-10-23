# #1807 — Observe startup in the CLI `run_serve` smoke test

**Status:** Approved spec; implementation not yet started.  
**Issue:** [#1807](https://github.com/jaunder-org/jaunder/issues/1807)  
**Baseline:** checkout `/home/mdorman/src/jaunder/issue-1807-cli-startup-smoke`,
branch `issue-1807-cli-startup-smoke`, HEAD
`944a9ace135a91f5d8e30d5ccf6d9b5f7a0d388a`; clean tracked, staged, and untracked
inventory when recorded. Source evidence is this checkout revision, not a
fetched-current-main claim.

## Outcome

The `run_serve` unit smoke test passes only after the real CLI `Serve` dispatch
reaches its existing runtime-file startup readiness signal. It fails
informatively if Serve exits before readiness or does not become ready within
the neighboring test's existing five-second bound.

## Approach and tradeoffs

Change only `run_serve` in `server/src/main.rs`. Keep its initialized storage
fixture and real `run(cli)` task. Use the existing `serve_runtime_is_ready`
helper against `<storage_path>/runtime.json`, and the same five-second timeout
already used by `run_serve_dev_auto_inits`; do not create a new budget or alter
that neighboring test. On early task completion, retain/report the original
`JoinHandle` outcome (including the task's error or panic) rather than letting a
generic timeout hide it. On readiness, timeout, or readiness-read error, abort
the owned task and await its handle before surfacing the final outcome; cleanup
must not mask the primary readiness/early-exit failure.

The readiness signal is evidence of the startup boundary the helper observes,
not an HTTP request assertion: `prepare_server_with_trusted_proxies` performs
server preparation and binds the listener; after binding,
`runtime_guard.update_address(addr, start_time)` publishes its actual nonzero
address. `run_serve_dev_auto_inits` already treats that publication as
readiness. This is the smallest existing seam and avoids a new production API or
network probe; it does not claim an end-to-end request was served.

## Acceptance and post-approval proof plan

- The test cannot pass merely because the Serve task was spawned or time
  elapsed; it observes a nonzero published runtime port.
- Early task completion is surfaced with useful original task-error context;
  timeout and helper read failure also remain failures.
- The test aborts and joins its owned task on every readiness-wait outcome
  before reporting success or failure.
- The initialized fixture and real Serve dispatch remain intact; scope remains
  this test only.
- **After explicit approval only:** run the focused `run_serve` test first in a
  deliberately failing form to demonstrate the assertion catches
  absent/unreached readiness (red), then restore the intended code and run it
  green. Do not execute this plan before approval. No spec-time tests, builds,
  formatting, or gates are authorized.

## Boundaries and evidence limits

No production changes/API, new timeout or budget, retry, skip, weaker assertion,
neighboring-test refactor, broad gate, or unrelated cleanup. Approval
provenance: direct User approval `1807 approved`, relayed through Manager for
correlated decision `1807-scope-seam-v1`, applies to the reviewed spec supplied
with that decision. The contract above is unchanged; only this status/provenance
record was added. This authorizes this spec-only commit, not implementation. The
reviewed source baseline was HEAD `944a9ace135a91f5d8e30d5ccf6d9b5f7a0d388a`.
Before this commit, the assigned branch was fast-forwarded to fetched
`origin/main` `ecb80ed9fdee424959832060c381a03a6eb800eb` (merge #1814,
containing #1708); the spec remains applicable and no source contract was
changed. At refresh, GitHub reported #1807 OPEN, project `Jaunder Backlog`
status Todo, no assignees/comments, and native `blocked_by` empty. The issue
helper found no prior spec, plan, or PR; open-PR inventory contained #1813 and
#1812 only. Runtime roster showed zero direct agents; these observations do not
establish a global assignment lease. The supported claim transition now sets
project status In Progress. Tracker and remote-main observations are
time-bounded. No implementation has started.
