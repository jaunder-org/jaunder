# Residual Tooling Simplification — Issue #1898 Implementation Outline

> Execute with dev-cycle-iterate ONLY after outline/execution approval. Outline
> needed because Git/index mutation, source admission, process/evidence custody,
> workspace retirement and multi-agent interfaces have non-obvious invariants.

## Execution summary

First deliver one real safety/bootstrap + repository-policy consumer path and
challenge its TOTAL maintenance/source-isolation cost before multiplying
targets. Then migrate concern-sized workflows, keeping producer/verdict and
lifecycle owners distinct. Delete only last-consumer scaffolding. Do not
duplicate landed families or adopt PR1932 receipts. Approved design is
`amended-spec.md` at the SHA recorded in `amended-design-v2-approval.md`;
supporting finite map is ignored `surviving-owners.md`, not an enduring source
catalog.

The following paragraph records historical intake status. Current qualified
Task2 acceptance is recorded below, without claiming the overall cost outcome.

This file is an ignored PROPOSED outline, not permission to run its proof. A
later maintained target may be
`docs/superpowers/plans/YYYY-MM-DD-issue-1898-residual-tooling.md`. Original
staged design bytes/index and shared stashes remain untouched now.

## Task outline

- [x] 1. Real precommit safety and one repository policy work without omnibus
      glue.
  - Contract: start repository-policy with single-process git-index-safety and
    repository-docs ADR-format path. Native Git/caller selection/source
    admission precede shared best-effort hook preparation; artifact-only
    consumers bypass mutation. Just retains named independent verdicts,
    fail-fast population and mandatory after-failure reconciliation/docs
    post-check. No snapshot protocol, arbitrary check selector or generic report
    envelope. Remove replaced adapters for this matched slice; retain legacy
    paths only for unmigrated consumers.
  - First proof seam: existing Git/index fixtures plus public native binary/Just
    calls using real Git, not mocked Git correctness. Observe successful staged
    input, failed policy input, mixed/untracked state, unreadable admission and
    hook-config failure: exact status/diagnostics, no unsafe staging, one
    reconciliation, and no hook/workload before failed admission. Exercise
    actual `just adr-format` consumer against intended source; fixture failure
    uses its real executable, not only library assertions. Do not invoke a broad
    gate as a substitute for this focused seam.
  - Source/cost risk check BEFORE broader extraction: positive/negative Nix
    source-membership assertions for the selected native tool and unchanged
    product/CSR/WASM populations; relevant source edit changes selected tool
    identity while excluded input stays excluded. Source Cargo inclusion and
    selected-output admission are different cases. Measure matched old/new
    production/test/manifest/target/recipe/Nix/docs surfaces and dependency
    edges, including duplicated transitional adapters and parser bootstrap
    costs.
  - STOP expansion/redesign if isolation fails, a general dispatcher/wrapper
    layer appears, extra packages beyond approved grouping become necessary, or
    matched-scope total maintenance grows without a concrete contract benefit.
    Simplify within approved scope first; consequential contract changes return
    to design approval. Do not postpone this decision until final deletion or
    equate fewer Rust lines/one successful command with success.

- [x] 2. Shared static definitions and remaining repository policies preserve
      verdicts.
  - Qualified acceptance: human `1898-task2-maintenance-reconsideration-v1`,
    background `8156ef02-d6ad-46ed-a2f9-587fa0160e77`, **Accept qualified
    checkpoint** via Manager. This is a POSITIVE-COST INTERMEDIATE (+277
    supplied snapshot), not demonstrated reduction; overall #1898 lower-total
    remains binding. Approval-documentation costs are recounted separately.
    Original contract and verification requirements below remain unchanged; this
    acceptance authorizes no Tasks3–7 work.
  - Contract: each pinned invocation/argument set lives once in a named Just
    leaf used by host and hermetic consumer; retain Fix/Verify, offline homes,
    target/platform and cargo-deny differences. Preserve narrow Leptosfmt error/
    scope adapter. Complete repository-docs/source-contracts and informational
    census; Org extractor reuses common exact-ID/ambiguity/partial-result rules.
  - Verification: public host/hermetic callsite argument/mode/order fixtures,
    selected same-definition source admission/invalidation, actual selected
    successful/failing consumer cases and independent policy/read-error
    verdicts. Preserve current populations and compensating auxiliary tests; no
    library engine/parser test duplication. Coordinate TSC interface with #1933
    BEFORE touching its exact recipe; do not wait for unrelated telemetry scope.

- [x] 3. Coverage/evidence and PG custody move once to actual concern owners.
  - Delivered (human lifted pause 2026-11-12): PG custody is coverage's
    nondefault `ephemeral-postgres` module, consumed by devtool's coverage
    producer and the `pg run` frontdoor (host-proof takes the frontdoor in
    Task5). Canonical seed is `test-support seed-e2e`; e2e-local runs the
    selected test-support artifact instead of building devtool, and ordinary e2e
    guests no longer install devtool. Coverage/WASM/budget/Elisp/mutation/
    doctest/performance status interfaces unchanged.
  - Contract: coverage owns nondefault ephemeral-postgres module scoped closure;
    host-proof pg-run consumes coverage, never reverse. No PG product-host
    feature or new ProcessKit/Tokio dependency. Existing
    coverage/WASM/budget/Elisp/mutation/ diagnostic targets and
    doctest/performance concern entrypoints retain status verification; Nix/Just
    sequence workloads. Canonical seed-e2e moves to existing test-support, one
    shared host/VM fixture list, no second SQL/data writer.
  - Verification: meaningful existing algorithm fixtures relocated to
    interfaces; actual producer-to-verdict joins reject
    missing/malformed/wrong/incomplete evidence. PG setup/body/child failure and
    interruption preserve env/status, one-shot teardown-before-exit and current
    cleanup warnings; pg-run tests cross its library seam. Explicit PG positive
    admission to producer/pg-run and negative product/CSR/WASM membership plus
    relevant-input invalidation: feature gating alone is not isolation; shared
    manifest/lock invalidation stays honest. Use applicable existing
    backend/browser evidence only after input/consumer equivalence check;
    execute changed seams, not full family audits by default.

- [ ] 4. Qualification uses selected concern executable without changing
      evidence meaning.
  - Contract: qualification-host behind four existing recipes preserves harness/
    source/target preflight, exact compiled-output/helper admission, lease/root/
    VM custody and ordinary/styling/no-evidence distinctions. Separate system-
    artifacts target may execute in Nix; qualification-host never does. Keep
    performance product/WASM closure separate and schema engines in-process.
  - Verification: binary-boundary and source/tool/helper drift/refusal fixtures;
    ordinary valid failed observation vs inadmissible publication, styling
    pass-only, post-publication cleanup nuance, canary/parity/collision/missing
    evidence and restricted-state failures. Prove selected native-output
    identity, producer-vs-host source/dependency admission and source-filter
    exclusions. Reuse accepted VM/native evidence only when exact applicable
    inputs/consumer contracts are unchanged; assign a specific changed
    end-to-end qualification proof if required, not a blanket four-workflow VM
    rebuild.

- [ ] 5. Focused proof and manual diagnostics retain convenient bounded
      workflows.
  - Contract: direct test-local/e2e-local retain their DIFFERENT mutable-lock vs
    immutable-artifact/root, admission, worker/seed/environment,
    readiness/capture/ signal/cleanup/status contracts. Standalone sandbox stays
    separate. Trace- analysis/source-diagnostics and fixed serial
    trace/benchmark recipes preserve reports, single-worker/quiet/unloaded
    assertions; no scheduler/supervisor.
  - Verification: real narrow nextest and browser workflows plus focused
    failure/ interruption cases at changed custody seams; stopped-writer
    finalization, refusal before consumers, retained diagnostic failure evidence
    and no stale artifact fallback. Preserve existing manuals and named manual
    report fixtures; distinct host proof does not certify full CI
    backend/browser matrix. Keep existing timings/capture gaps honest; no new
    recorder/retention machinery.

- [ ] 6. Resolve finite defer and retire only proven last-consumer
      infrastructure.
  - Contract: R26 experimental coverage emit/worker/aggregate endpoints remain
    with required devtool build/tests. #1898 Lead owns fresh bounded
    caller/operator readback, then preserve/retire decision and explicit human
    confirmation for any material loss. No experiment run is implicit. Retained
    required endpoints prohibit a claim of full devtool retirement. Every other
    R01–R27/R17a row has actual survivor/interface/test/doc consumer migration
    or explicit honest defer.
  - Verification: bounded source/docs/CLI/alias/Nix/package/filter/vendor/test
    inventory at final integrated tree; no maintained caller silently broken, no
    lost auxiliary test population or bootstrap. Remove obsolete generic
    dispatch/result/launcher/workspace edges only after this proof; compare
    final combined maintenance AGAIN, including native target/test/Nix additions
    and retained deferrals. A closure/search miss is not behavior or absence
    proof.

- [ ] 7. Conformance, enduring docs and delivery close this implementation scope
      honestly.
  - Contract: independent whole-branch Standards/Spec review against approved
    amendment and outline; resolve findings, project enduring owner/interface
    decisions into docs/ADR, repair references and retire completed working
    plans under non-squash history. #1864-approved residual verification
    deferrals remain explicit, not fictional passed parity proof or a new
    sharing prerequisite.
  - Verification/gates: focused proofs stay tied to their changed inputs; normal
    hook-backed commits/push and required current CI/producer/aggregate verdicts
    remain binding. Final intended tree includes necessary planning cleanup;
    verify hook mutations rather than accept older green proof. Avoid duplicate
    broad local VM/validate work without named risk/missing CI equivalent. Open
    issue-linked PR only when its authorized deliverable is complete; observe
    once via supported tooling and obtain separate exact-head human merge
    approval. Managed branch/checkout and needed evidence are preserved; no
    early Done/cleanup/removal or full-retirement claim with retained endpoints.

## Coordination and risk checks

- Sole writer/checkout executor; settle writer before formatter-capable gates.
  Other worktrees need not stop: reserve exact shared hunks before a future
  writer, transfer canonical policy/communication and accepted
  artifacts/results.
- #1864: CONTRIBUTING parity/invalidation/Nix inventory (captured922–939,
  1902–1926,1990–2070); ARCHITECTURE workflow/static owner regions (4464–4620,
  4645–4703,5118–5138). Refresh anchors when integrating; don't overwrite their
  current guidance or turn evidence deferral into a new execution prerequisite.
- #1933: tools/traced-run record-only/retention; Justfile TSC20–23; ci.yml
  Validation hermetic86–105; retention/export/failure docs. Coordinate TSC
  replacement and launcher participant effects BEFORE writing these hunks;
  preserve direct workload/saved status/observation envelope. No code takeover,
  whole-project serialization or blanket tracing prerequisite; PR1932 excluded.
- Source/provenance/authentication, test/backend/browser/coverage populations,
  native CI joins and privileged promoter safeguards are invariants, not
  optional budget trades. No new public interface/material loss without proper
  gate.

## Approval packet / execution withheld

Qualified ask after Lead cold review: approve this outline and authorize ONLY
Task1's matched-scope safety/bootstrap + ADR-format tracer bullet and its named
focused/source-membership/cost proofs first; expand only after Lead accepts its
stop/redesign assessment. Remaining tasks define bounded continuation, not a
waiver of review/gates. Preserve original approved staged bytes/provenance
before any later staging replacement. Promotion of amended contracts/approved
outline and hook-backed contract commit requires a separate explicit assignment
and applicable proof; no staging/commit is performed by this planning task.

Design-only approval is not runtime/COTS/source-isolation evidence. Outline
approval/execution, material-loss confirmation and per-PR merge remain distinct.
