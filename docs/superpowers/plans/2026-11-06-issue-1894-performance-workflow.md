# Performance workflow without xtask orchestration Implementation Outline

> Execute with dev-cycle-iterate. Outline needed because: this changes the
> durable Just/Nix/host-Rust ownership seam, public workflow entrypoints, and
> ordered failure/evidence-retention behavior. Human approval is required before
> implementation.

## Execution summary

First prove the expensive assumption through one real existing Nix producer: Nix
selects it from native Just parameters, Just builds and retains it serially, and
the host-only typed command assembles a result with its diagnostic references
intact. In the same first slice, inject a later build failure and a
producer-status failure to show completed earlier evidence remains in the
invocation upload tree. If the tracer bullet fails, diagnose that bounded path
and preserve its evidence; failure does not authorize a full matrix. Any broader
proof requires separate justification and approval. Then move the remaining
typed policy, compare/import operations, and actual callers to the new workflow;
remove the old performance scheduler only after equivalent focused proofs pass.
The approved spec is the scope contract.

## Task outline

- [x] Task 1: Introduce the production assembly path and prove Nix selection,
      serial Just retention, and typed assembly with one producer.
  - Contract: Nix selects producer descriptions from the native Just
    profile/backend/browser/workload/count parameters and one fresh invocation
    nonce, returning an ordered per-invocation list of producer names,
    installables, and identity metadata. Just is the sole consumer of
    installables for scheduling: it builds each selected installable separately
    and serially, retaining its complete `performance/` output under the
    nonce-owned `producers/<name>/` path before status validation and before the
    next producer starts. A build or status failure stops later producers
    without deleting earlier retained evidence. The host Rust leaf consumes
    producer identity and retained-root metadata for validation/assembly only;
    it does not consume installables, schedule builds, or maintain a second
    producer catalog. Keep `medium` as the public profile default; explicitly
    select `small` for the focused producer proof. Preserve `sqlite,postgres`,
    browser-list omission, `all`, and unset counts as the other defaults. Empty
    browser selection normalizes to Chromium only for browser work; explicit
    browser with storage-only rejects. Canonicality follows normalized
    selection: explicit default-equivalent selectors remain canonical;
    nondefault selection and every count override are noncanonical; baseline
    eligibility separately requires medium.
  - Verification: add focused Nix selection evaluation for all three profiles,
    default, storage-only, browser-only, multiple and duplicate selectors, and
    counts; verify pinned producer inputs and one fresh nonce. Fixture-driven
    serial-loop tests inject a second-build failure and a producer-status
    failure after one retained output and assert that the first output survives
    and no subsequent producer runs. Then run one real replacement-frontdoor
    invocation using an existing Nix producer and verify retained
    status/fragment/diagnostic files and host assembly. This is the first tracer
    bullet and one producer only, not the full matrix.

- [x] Task 2: Extend the same production assembly path with complete evidence
      validation and publication checks.
  - Contract: extend Task 1's host-only CLI path; do not create a throwaway or
    duplicate assembler. It consumes normalized selection, profile, nonce,
    provenance context, and producer identity/retained-root metadata. Complete
    checks cover status version/success, fragment schema/completeness,
    compatibility, and relative in-tree diagnostic references; publication
    atomically writes the nonce-scoped result and stable upload copy. The CLI
    never consumes installables, selects/schedules producers, invokes Nix, or
    alters producer measurement semantics. Preserve local-vs-GitHub provenance
    and keep Nix derivations from invoking any host executable.
  - Verification: fixture tests cover missing/failed status,
    malformed/incomplete fragments, incompatible plans/selection, absent or
    escaping diagnostic paths, complete assembly, fresh invocation identity, and
    atomic result publication. Use existing `tools/performance`
    contract/statistics tests rather than duplicating their internals.

- [ ] Task 3: Migrate comparison and trusted baseline import to host domain
      commands behind native Just parameters.
  - Contract: `just perf-compare <result-path>` stays advisory against the
    committed baseline. `just perf-import-baseline <positive-run-id>` remains
    the only baseline mutation. Retain all existing repository, workflow, job,
    event, branch/head, run-attempt, artifact, success/expiry, provenance,
    canonical-selection, medium-profile, result-completeness, compatibility, and
    atomic-publication checks. Rejected imports leave existing baseline bytes
    unchanged; PR/local/noncanonical evidence cannot seed it.
  - Verification: focused comparison fixtures prove compatible/incompatible
    behavior, absolute median/p95/spread reporting, and advisory timing deltas.
    Import fixtures independently reject each trust/identity and evidence
    failure class, assert byte-for-byte baseline preservation on every
    rejection, and prove a valid canonical trusted run atomically replaces it.
    No live import or credentials are needed.

- [ ] Task 4: Migrate actual workflow callers and retire obsolete performance
      scheduling paths.
  - Contract: `.github/workflows/performance.yml` uses the same Just run/compare
    commands, compares only after successful production when a baseline exists,
    and retains `if: always()` upload of all available invocation evidence.
    Update documented commands in `CONTRIBUTING.md` and `docs/ARCHITECTURE.md`;
    preserve the proposed/current-runtime distinction until migration is
    actually shipped. Remove performance scheduling/catalog/collection
    entrypoints from xtask CLI/dispatch only after the replacement calls and
    paths are in use. Leave unrelated xtask utilities, `tools/performance`
    workspace ownership/tests, ADR-0141, generated `docs/README.md`, and the
    committed baseline unchanged.
  - Verification: inspect every existing `cargo xtask perf`/workflow caller and
    confirm each has a replacement or is intentionally removed. Focused
    workflow/config checks prove run failure skips comparison but does not skip
    artifact upload; compare/import Just invocation smoke tests use local
    fixtures. Run the required reviewed-tree documentation/project gate at the
    established commit boundary; do not substitute a full performance matrix for
    these checks.

## Risk checks

- The first slice must establish all four costly facts before wider migration:
  Nix selects from runtime parameters; Just serializes producer execution;
  evidence is retained before the next build/status check; and the host typed
  command assembles real producer output and preserves diagnostic references.
- Failed later builds/status must not erase an earlier completed producer's
  evidence; failed, missing, malformed, incompatible, stale, or noncanonical
  inputs never become a successful canonical run or baseline.
- One fresh nonce identifies every producer and result in one invocation;
  outputs from another invocation are never silently reused. Preserve default
  matrix and count semantics exactly.
- Do not introduce a generic runner, a large shell script, duplicate producer
  catalog, schema framework, or one-for-one Rust scheduler rewrite. Keep the
  Just loop small, Nix authoritative for producer selection, and Rust
  authoritative only for typed domain policy.
- Keep comparison advisory and baseline import explicitly authenticated and
  atomic. ADR-0141 workspace separation and auxiliary test requirements remain
  unchanged; the narrow proposed ADR-0028 clarification and its architecture
  projection must stay consistent with the implementation.
- The first tracer bullet and each completed deliverable receive diff review.
  After the queue, run the applicable hook-backed/project verification through
  the normal iterate/commit path; no implementation begins before this outline
  is approved.
