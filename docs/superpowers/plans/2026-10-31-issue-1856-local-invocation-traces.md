# Local xtask Invocation Traces in Perfetto — Implementation Outline

> Execute with `jaunder-iterate`. This outline is needed because the capture
> lifecycle, thread-timeline semantics, and replacement timing projection have
> concurrency/compatibility risks.

Approved spec:
`docs/superpowers/specs/2026-10-31-issue-1856-local-invocation-traces.md`. Human
approval provenance: request `4de2c96f-f1ec-4f40-91b0-c89fee7b5500`, version
`1856-mvp-spec-v1`; exact answer: “Approve spec and outline preparation.” That
answer authorized spec and outline preparation only. Separate implementation
authorization: Manager request `8edc8273-fda7-428c-8f3e-b5588a813948`; exact
answer: “Approve outline and MVP delivery.” It authorizes the approved MVP
dependency/code changes, focused tests, gates, representative command, and
browser proof if capable. It does not authorize commits, push, or PR before Lead
review, merge, external-policy edits, issue closure, broader #1856 work, or
viewer-service startup.

## Execution summary

Inspect any existing resolved `tracing-chrome` version; if absent, select and
source-check a compatible version, then add and resolve it only after
implementation authorization. Verify the actually resolved API can provide the
specified synchronous `Threaded` JSON-array capture and arbitrary writer before
broader instrumentation. Then instrument one `test-local` invocation from xtask
checkout preparation through its synchronous preparation/test phases. Derive
only those selected phase durations from matched `B`/`E` intervals, finalize a
unique local trace, and preserve `.xtask/last-result.json` and all current
result interfaces. Do not add a Collector, child propagation, other command
timing, or trace payload families. If the selected crate/API cannot meet the
spec, stop and return to the spec approval boundary instead of substituting
another adapter.

## Task outline

- [x] Task 1: Verify the pinned adapter seam and trace interval contract.
  - Contract: inspect any existing resolved version; if absent, select and
    source-check a compatible candidate, but add/resolve the dependency only
    after implementation authorization. Then verify the resolved version's
    `Threaded`, args opt-in, arbitrary-writer, direct-array, `B`/`E`, and guard
    behavior before broader instrumentation. Match intervals by `(pid, tid)` and
    nesting order; args are strings and timestamps are floating-point
    microseconds. Do not infer a returned flush status from `FlushGuard`.
  - Verification: focused xtask host tests for a nested same-thread span,
    two-thread separate tracks, matched/missing `B`/`E`, and bounded string
    fields. If the resolved API check fails, stop before broader implementation.

- [x] Task 2: Instrument the selected synchronous `test-local` path and derive
      its existing duration fields.
  - Contract: enable capture only for `test-local`; span checkout preparation,
    hook setup, dispatch, artifact-lock wait, CSR bundle admission/build/
    validation, worker build, and test invocation. Preserve the existing
    dispatch timing boundary for `CommandResult.duration_ms`; expose startup
    preparation separately. Remove matching manual `Instant` timers and derive
    selected result durations from the trace; retain all current
    `.last-result.json`, stdout JSON, human, and GitHub outputs.
  - Verification: focused tests for phase identities/order, lock wait, early
    failure outcome, result projection, and absence of duplicate selected-path
    timers. Other commands and their timing remain unchanged.

- [x] Task 3: Finalize and persist one run trace, then prove the real view.
  - Contract: close spans, emit terminal outcome, drop/join the guard, validate
    the JSON array and expected intervals, derive result durations, then persist
    via fallible temporary-file/write/flush/rename steps to
    `.xtask/traces/<run-id>/trace.json`. Report existing outputs even if trace
    persistence fails; preserve the original command verdict and exit override.
    Keep one unique file per run; no automatic pruning.
  - Verification: test success, handled `Err`/exit override, malformed or
    missing intervals, temporary-file creation/write/flush/rename failures, and
    finalization before `process::exit`. Then run
    `devtool run -- cargo xtask test-local -- -p storage site_config_primitives_round_trip`;
    open its exact output in the intended browser at `https://ui.perfetto.dev`
    and verify local-file loading, preparation/phase timing, and thread-track
    behavior. Browser capability remains unverified until this interactive
    proof.

## Risk checks

- `Threaded` measures same-thread entered intervals, not async span lifetime;
  keep all MVP scopes synchronous. A sequential real run need not show sibling
  overlap; a controlled two-thread test verifies track separation.
- `tracing-chrome` args are optional/string-valued and its writer guard does not
  return dependable write status. Buffer one bounded host trace, validate the
  finalized bytes and expected phase pairs, and use fallible persistence.
- If a selected interval is absent/unmatched, leave the existing zero
  `duration_ms` default and warn; never substitute an independent timer.
- The MVP keeps `.xtask/last-result.json` and public result contracts. Sidecar
  removal, Manager-owned external policy migration, all-command timing, child
  propagation, and broader #1856 parent/child coverage are deferred, not task
  dependencies. Do not edit external policies or expand the trace payload.

## Handoff gate

After this outline is reviewed, resolve its findings and present the separate
outline approval packet. No implementation starts until that approval and a
concrete next authorization are received through Manager. The approved spec
remains authoritative if this outline conflicts with it.
