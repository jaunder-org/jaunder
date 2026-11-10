# Surviving Workflow Observations Implementation Outline

> Execute with dev-cycle-iterate after outline approval. Outline needed because:
> cross-process recording, partial/concurrent publication and retention
> invariants.

## Execution summary

Follow the
[approved spec](../specs/2026-11-10-issue-1933-surviving-observability.md).
First prove the existing trace writer can record across two short-lived
processes without executing the workload or changing its status when recording
fails. Then secure retained/partial evidence, connect the existing tsc recipe
owner, and finish native CI diagnostics, current guidance and export
demonstrations. All runtime proofs below follow separate outline approval, not
this preparation.

## Task outline

- [ ] Task 1: A real passive start/finish invocation preserves owner execution.
  - Contract: add `record-start tsc-request <verify|fix>` and
    `record-finish <handle> <exit-code> <none|setup|request>` to surviving
    traced-run; neither accepts a workload command, arbitrary path, argv or
    environment. Start returns a generated opaque handle to private, unique
    local start context; finish uses the same identity and existing named-trace
    publication, not receipts. Persist monotonic origin/start and clock identity
    across recorder processes; reject unavailable/mismatched origin rather than
    fabricate elapsed time. Fixed fields include mode, observed
    outcome/boundary, explicit identity unknowns and `raw output not archived`;
    do not join inherited launcher context.
  - Verification: public recorder CLI tracer bullet with a minimal owner fixture
    running real success/failure commands directly; unavailable recorder/start
    sink and failed finish still execute once and preserve the immediately saved
    status.
- [ ] Task 2: Complete and partial observations survive concurrent repeated
      runs.
  - Contract: reuse existing fragment/final-trace layout; retain useful start
    context when finish is absent; missing completion means unknown, never
    inferred signal. Exclusive private allocation and atomic no-overwrite
    publication reject malformed handles, duplicate finish and collisions
    without replacing another observation. Remove count pruning of runner
    stdout/stderr and completed named traces/records; no expiry, collector,
    parent catalog, new dependency or duplicate evidence schema.
  - Verification: focused missing/malformed context, write-failure, concurrent,
    duplicate/privacy tests; repeat beyond former pruning thresholds and retain
    full/partial observations and output files; failed capture never certifies
    completeness.
- [ ] Task 3: Ordinary tsc automatically records its recipe-owned request.
  - Contract: edit only the existing `Justfile` tsc owner; begin before setup,
    run unchanged setup then Cargo/devtool arguments with inherited
    stdout/stderr, save each status immediately, skip request on setup failure,
    finish best effort. Recorder refusal/missing binary/token/write failure
    cannot gate work or its result; no execution wrapper, signals, traps,
    timeout or cleanup ownership is added. Envelope excludes Just
    evaluation/pre-entry work; verify/fix behavior is preserved.
  - Verification: public callsite fixtures compare
    order/arguments/streams/status with recorder available/unavailable;
    interruption leaves partial evidence without new handling. Retain
    representative real `just tsc` success/failure demonstrations.
- [ ] Task 4: Native CI diagnostics and explicit export are useful without new
      authority.
  - Contract: Validation hermetic keeps native step timing/status/source/logs;
    optional `always()` upload retains available full/partial diagnostics for 14
    days. Missing/upload-failed diagnostics warn visibly without changing
    workload status; existing mandatory producer artifacts, checks and aggregate
    verdicts remain intact. Current docs explain unknown/cache/non-additive
    timing and accepted raw-output gap; use existing copy/hash commands for
    consumer-selected export, never a service.
  - Verification: CI config plus applicable native-run evidence cover retained
    partials and upload incidents; export selected files and compare
    identities/bytes, retaining source on success/failure. Keep
    demonstrations/review packets in ignored storage.

## Risk checks

- Review private modes, handle confinement, clock correlation and no-overwrite
  first.
- Keep backend/browser populations unchanged; no xtask/devtool, E2E or
  Performance edits.
- Coordinate concrete documentation overlap with independent #1864 through
  Manager.
