# #1890 E2E workflow implementation outline

> Approval provenance: Manager relayed the human's “Approve design; prepare
> outline” for `issue1890-design-v2` (background
> `f709fe05-cf23-4fc6-aa59-0948aa12dec1`) and “Approve outline and begin” for
> `issue1890-outline-v1` (background `b5f826ee-bcb5-473f-9b88-932bef7c7637`).
> Implementation remains a separate lifecycle step after this docs-preparation
> tree is accepted and committed; this preparation contains no implementation.
>
> Integration note: #1885 PR #1899 has landed as
> `a440e58f4a0c9589cf6cb04921ddee20a0a831f2`. This checkout remains at its
> pre-merge base; do not rebase this docs-only preparation. Before source
> implementation, integrate the actual landed change only. The reported green CI
> is historical evidence, not proof for this checkout.
>
> Execute with `dev-cycle-iterate`. Outline required because this migration
> moves durable workflow ownership across Just, CI and Rust host tooling.

## Execution summary

After this outline is approved, retain the approved spec and ADR with the normal
hook-backed workflow, then migrate callers onto the Just-first design before
deleting Rust scheduling. First verify the risky seam: a local `just e2e` must
continue through all eight lanes and both Firefox reconciliations after producer
failures, while server-function coverage uses only the successful authoritative
SQLite/Chromium output and its existing archive/population checks. CI keeps
distributed matrix and artifact choreography. This outline does not authorize
implementation until separately approved.

## Task outline

- [ ] **Task 1: Establish the minimal Just and CI workflow boundary.** Keep
      `end2end/e2e-lanes.json` the single enabled-lane source; derive CI
      producer matrix data at runtime and retain standard matrix, dependency,
      artifact and aggregate jobs. Define the default `just e2e` plus
      parameterized producer and reconcile entrypoints; use existing Just/COTS
      behavior wherever sufficient and add no framework or mandatory Nix
      workflow wrapper.
  - Contract: local full workflow attempts every enabled lane, settles
    producers, runs both backend reconciliations despite missing/failed
    artifacts, and returns aggregated nonzero status. CI reconciliation depends
    on all three backend Firefox producers and aggregation observes every
    result.
  - Verification: focused catalog-to-matrix equality and dependency checks;
    inject an early producer failure and confirm later producers/reconciliations
    still run.

- [ ] **Task 2: Move existing evidence verdicts behind artifact-input
      boundaries.** Adapt the narrow census/report ownership, retry-duration,
      boot-trace and server-function verifiers to explicit artifact paths and
      runtime lane/catalog expectations; remove presentation envelopes/build
      hooks without moving parsing/verdict logic unnecessarily. Keep static
      server-function inventory verification independent and regeneration
      explicit.
  - Contract: coverage runs only after successful authoritative SQLite/Chromium
    production, from that invocation's selected successful immutable Nix output
    and existing archive-finalization, retention and population checks. Reject
    missing, inconsistent, or wrong-output evidence. Do not claim detection of
    hidden omissions beyond those checks; add no receipt/schema.
  - Verification: focused artifact-input cases for missing, wrong-output,
    malformed, failed-producer and archive-incomplete/inconsistent evidence;
    verify static inventory remains independently checkable.

- [ ] **Task 3: Switch callers, then retire Rust scheduling.** Migrate local and
      CI callers to the Just/COTS boundaries and verify the single-command
      experience before deletion. Then remove E2E scheduling-only paths and
      redundant build/report wrappers in `xtask/src/steps/nix.rs`, remove
      catalog embedding/plumbing in `xtask/src/e2e_lanes.rs`, and adapt actual
      consumers of result/report envelopes. Preserve non-E2E functions, narrow
      verification, and artifacts with real consumers.
  - Contract: no parallel Rust scheduler, duplicate topology, manual user
    command chain, or changes to `e2e-local` absent mandatory coupling.
  - Verification: focused caller/consumer search and proof that all required
    lanes, reconciliation and coverage verdicts remain reachable through the new
    boundaries.

- [ ] **Task 4: Close acceptance and lifecycle.** Run focused controlled
      failure, aggregation, cleanup/retention, population and current CI
      topology proofs; account for complete
      production/configuration/adapter/test maintenance without a LOC quota.
      Complete conformance review, project enduring decisions into maintained
      docs as required, and retire planning artifacts before PR.
  - Contract: preserve browser/boot producer ownership in #1884, consume the
    landed #1885 census boundary without taking its scope, and leave general
    coverage/doctest work to #1891. #1889 may call the synchronous full Just
    workflow only after its own landing; no unlanded sibling imports or blanket
    dependency.
  - Verification: current CI covers all eight lanes; both Firefox
    reconciliations wait for their producers; any producer or reconciliation
    failure fails the aggregate; server-function coverage remains gated by
    successful authoritative production and existing capture/population checks.

## Risk checks

- Continue-all behavior must be demonstrated with an early failure; later lanes
  and both reconciliations still execute, while the final result remains failed.
- The selected successful immutable producer output, existing capture
  finalization/archive checks and independent population checks are the evidence
  authority. Do not create new attestation machinery or overstate detection of
  omissions those checks do not observe.
- Preserve the caller-facing synchronous Just contract and standard CI
  artifact/aggregate responsibilities while deleting the Rust scheduler; keep
  one writer and avoid transitional duplicate ownership.
