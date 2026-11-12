# Canonical instants — implementation outline for issue #1940

> Approval record: human “Approve outline” for `issue-1940-outline-v1`, relayed
> through Manager in background `6ce686e4-c49c-47da-ac97-8602513c09a3`; Lead
> acknowledged and accepted this outline after the post-spec soundness pass. The
> spec gate was approved separately. Preparation-state descriptions below are
> historical; this marker changes no governing contract or merge gate.

> Revised draft only. Execute with dev-cycle-iterate after separate spec and
> outline approvals. Needed for storage migration, queue/restore correctness,
> timestamp ordering and coordinated Org marker compatibility.

## Execution summary

After approvals, first commit the contracts and numberless ADR/architecture
projection, before source work. The first implementation activity is a bounded
feasibility test of the legacy-upgrade queue sequence, not a deployable partial
migration. Then make canonical encoding, affected typed writers, normalization,
enforcement and portable backup/restore coherent in one production commit.
Coordinate Org output next and finish the cursor/backend/failure matrix. This
draft accompanies the proposed
[spec](../specs/2026-11-11-issue-1940-canonical-instants.md); neither cold
review nor this ordering supplies human approval or bypasses
spec-before-active-outline.

## Task outline

- [x] Prerequisite: approved contracts and architectural decisions precede
      source work.
  - Contract: obtain spec approval, then reconcile/review and separately approve
    the outline. Commit approved contracts plus numberless ADR draft and its
    architecture projection before any source/test implementation. Explicitly
    supersede the specified ADR-0153/0182 provisions and extend ADR-0226's
    offline exception, including handler-owned enforcement DDL; retain queue
    authority, typed binds and exact-schema restore. Never promote/renumber ADRs
    or edit the generated index manually.
  - Verification: Lead accepts exact documentary tree and approval provenance;
    intended-tree hook checks follow existing commit policy, not this prep task.

- [x] Deliverable A feasibility proof: earlier legacy work reaches
      normalization.
  - Contract: use existing MigrationDatabase in storage/src/migrations.rs and
    Member/Collection/storage seams with human Post 5/266 values. SQL migrations
    ONLY enqueue normalize_comparable_instants_v1. Ascending queue IDs remain
    authoritative; no triggers precede the normalization handler on normal
    upgrade.
  - Verification: a pre-0041 fixture contains an older pending rendering
    rebuild, legacy Post timestamps and content whose rendered output actually
    changes. Exercise its outbox-default case; show those operations complete
    ahead of normalization. Then normalize all selected cells, verify, CREATE
    enforcement, delete the operation row and commit together. The cursor now
    includes Post 5. This tests the highest-risk sequence; do not ship it as
    partial enforcement.

- [x] Deliverable A coherent release: closed wrapper, writers and atomic
      transition.
  - Contract: checked constructor/TryFrom, parsing, serde, both SQLx decoders,
    civil conversion/clock and checked arithmetic use existing UtcInstant; one
    Jiff precision-nine formatter. PG native microsecond behavior remains.
    Storage-local read_legacy_instant_cells is identity-first: identity and raw
    class/value precede text/civil/Julian interpretation and checked
    construction; public decode remains checked, without a raw-bind escape.
  - Contract: convert the spec's exact selected population, verify, install
    fixed named INSERT/UPDATE rejection triggers, delete queue row and COMMIT in
    the existing SQLite BEGIN IMMEDIATE transaction. PG validates native cells
    and installs finite-bound constraints in its queue transaction, not SQL
    migration. No registry, UDF, queue reordering, table rebuild or FK
    disabling. Update selected omitted-default writers, including outbox
    single/batch paths, to typed sampled clocks. Required raw omissions
    deliberately reject. theme_revisions is excluded; no unrelated Theme
    default-writer changes.
  - Contract: pending normalization accepts absent enforcement or an identical
    full set on an initialized restore target; partial/different sets fail.
    Completed/absent operation requires the full expected set. Never drop or
    bypass an existing set. Canonical encoding, affected writers, normalization,
    enforcement and portable restore/export below ship in one coherent commit.
  - Verification: every ingress and calendar/type/null/shape/range predicate;
    fractions/offsets, pre-1970, year zero/BCE, true Jiff MAX,
    arithmetic/cutoffs, PG ±infinity and inclusive micros
    endpoints/one-step-outside values. Preserve IDs, versions, semantic times,
    nulls, exact SQLite ns and Revision/timezone state. Test all selected
    normal/raw/default/batch writers and trigger states.

- [x] Deliverable A portable backup/restore: strict admission without backend
      breakage.
  - Contract: at the existing PG export seam, selected native timestamps pass
    through UtcInstant canonical output rather than raw to_jsonb timestamp text;
    pad stored micros, never reconstruct lost ns. NULL/other cells remain
    unchanged. Restore validates canonical backup cells before PG native binding
    can erase spelling and the complete selected population before commit, even
    with a pending normalization row and NO triggers. Imported queue rows never
    disable validation or imply completion; retain exact-schema and closed
    restore roles. Apply the exact enforcement states above; never consume work
    during import.
  - Verification: actual same-schema PG→SQLite and SQLite→PG backup/restore,
    including NULLs, pre-2000 fractions and existing directional quantization.
    Valid queued/full-set and queued/absent-set targets, invalid raw timestamp
    cells and partial/missing-completed enforcement must be exercised
    explicitly. Retain database rollback, FK checks and existing media/Theme
    failure ownership.

- [x] Deliverable A durability proof: failure, cancellation and uncertain
      acknowledgment.
  - Contract: extend only the bounded MigrationDatabase fixture and a test-only
    pause immediately before/after the existing COMMIT in
    code_migrations/drain.rs. Use its existing rejecting queue-delete-trigger
    pattern for delete failure; a fixture-only deferred FK violation makes
    COMMIT actually reject. No general fault-injection infrastructure or new
    public testing API.
  - Verification: malformed text, unsupported BLOB/class and Julian zero report
    table/column/row with checked public decode installed. Inject conversion,
    trigger-DDL and delete failures. Known rejected COMMIT closes/discards any
    still-transactional connection; cancel before COMMIT, and separately
    suppress acknowledgment after successful COMMIT. Close/reopen connections,
    inspect RAW physical cells, operation row and trigger set: old/pending/no
    new triggers before commit versus canonical/absent/full after commit. An
    uncertain caller result must not be called rollback. Retry only the durable
    pending state; account separately for earlier committed SQL migrations/queue
    operations.

- [x] Deliverable B: Org owned-UTC emission and historical sync reads agree.
  - Contract: datetime module owns canonical output; publish/pull/reconcile
    reuse it. Accept historical valid sync spellings; preserve exact publication
    ns, local DATE, zone/alias and fold. Failed/uncertain writes do not
    checkpoint.
  - Verification: historical whole-second Z and offset/fraction markers plus new
    markers; publication precision/fold round trips through existing
    entrypoints.

- [x] Deliverable C: integrated acceptance, conformance and enduring
      documentation.
  - Contract: every timestamp cursor named in the spec walks ties/small pages
    without missing/duplicate IDs; PostgreSQL micros remain an explicit
    limitation. Reconcile the pre-source ADR/projection against the delivered
    system.
  - Verification: focused consumer proofs, then independent conformance review
    and applicable hook/ship/CI boundaries from current CONTRIBUTING and skills,
    not duplicate broad gates per task. After conformance, remove completed
    planning documents while retaining enduring ADR/docs and intermediate
    history. Production cutover/backup validation is operator-owned; no Agent
    production database/server/credential access or backup disposal.

## Risk checks and approval ordering

- Supplied cold findings are incorporated, not human sign-off. Lead accepts
  their dispositions and resolves remaining findings before the spec HALT brief.
  After spec approval, reconcile/review this outline and obtain its separate
  HALT approval. Material design changes return to the spec gate.
- Whole-population normalization and enforcement installation are atomic, not
  whole upgrade. Failure blocks startup; retain a verified recovery copy through
  validated fresh backup/restore. No indefinite legacy-backup restore framework.
- One writer/executor, including formatter-capable gates. Preserve managed
  branch and checkout. Rebase only if needed; first confirm same-agent immediate
  direnv reload capability with explicit 900-second tool timeout after
  successful rebase.
- This revision performs documentation only: no tests/builds, implementation,
  commits, nested delegation, new human approval or production access.
