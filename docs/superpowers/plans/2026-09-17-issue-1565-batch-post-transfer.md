# Explicit Batch Post Transfer Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a bounded task
> when useful. This outline exists because issue #1565 changes durable storage
> retention and AtomPub replay semantics, adds crash-recovery metadata, and
> introduces matched-file replacement invariants.

## Scope

In:

- Durable per-User AtomPub create-key replay and client create-intent recovery.
- A selectable reconciliation report with explicit sequential push, pull, and
  remote-delete batches.
- Revalidated matched-Post replacement, clean-buffer refresh, partial-success
  reporting, cancellation, and fresh post-batch inventory.
- Pure ERT, live ERT, and dual-backend Rust proof required by the approved spec.
- The two ADR drafts and their architecture projection.

Out:

- Automatic synchronization, concurrent requests, conflict override, merge, or
  absence-means-delete behavior.
- New bulk endpoints, nested-file discovery, Markdown/HTML publishing, Media
  management, purge/restore, or non-Post administration.
- ADR promotion or edits to the generated `docs/README.md`.

## Task outline

- [x] **Task 1: Make create recovery durable across server and client
      restarts.**
  - Contract: a committed `(User, IdempotencyKey)` never expires or authorizes a
    second Post. Active replay returns the original Member as `200`; replay of a
    soft-deleted original returns `409` and leaves the key consumed. Retire the
    idempotency-prune storage/maintenance surface and expiry metric while
    preserving the historical table shape and migration compatibility.
  - Contract: before any create POST, publish persists `JAUNDER_CREATE_KEY`, an
    exact sent-Entry digest, and attempt time. Recovery reuses the key until
    `JAUNDER_ID` is safely written, then removes the intent. A changed digest
    recovers identity without marking current content synced; the Post remains
    local-ahead for a later explicit PUT.
  - Verification: dual-backend storage tests and AtomPub HTTP tests cover active
    replay beyond one hour, maintenance survival, deleted-original `409`, and
    unchanged Post count. Pure/live ERT covers intent persistence, restart,
    response loss, changed-content recovery, ID-first cleanup, and ordinary
    single-Post publishing.

- [ ] **Task 2: Turn the reconciliation report into a selection and execution
      surface.**
  - Contract: a dedicated report mode owns stable row identities, arbitrary
    marks, contiguous-region selection, and state/reason display. It exposes no
    operation binding until the task implementing that operation lands.
    Selection never changes row eligibility.
  - Contract: one shared batch executor consumes rows in displayed order, keeps
    at most one operation in flight, catches independent item failures, and
    honors quit only between items. Before starting the next item it appends one
    result with `action`, stable `row-key`, `outcome`, optional Post
    ID/slug/ETag/ synchronization time/HTTP status, `local-effect`, and
    actionable reason/detail. Delete results deliberately omit synchronization
    metadata.
  - Contract: the report buffer owns the ordered last-batch result list
    separately from inventory rows. Completion or cancellation rebuilds rows
    from a fresh inventory and then renders the intact result summary, including
    failures whose original row no longer classifies the same way; the next
    batch replaces that summary.
  - Verification: pure ERT covers marks/region resolution, progress, complete
    result shapes and ordering, failure display after refresh, cancellation, and
    an executed 1,000-item synthetic batch proving stable order, one in-flight
    operation, retained successes, terminal-result cardinality, and continuation
    after an injected independent failure.

- [ ] **Task 3: Add explicit selected push and remote-delete operations.**
  - Contract: this slice adds the report bindings for push and delete. Each
    command previews its selected-operation count and requires one confirmation
    before its first mutation; delete uses distinct destructive wording and is
    never invoked by push or refresh.
  - Contract: push accepts only local drafts and safely local-ahead matched
    Posts and delegates each row to the durable publish path from Task 1;
    unchanged rows are no-ops and all other states return blocked results.
  - Contract: delete accepts only unambiguous `server-only`, `unchanged`,
    `local-ahead`, and `server-ahead` rows. Before its destructive confirmation
    it fetches and displays each current strong ETag; each DELETE revalidates
    that ETag. Confirmed `204` removes a matched local file but has no local
    effect for server-only rows; stale/error outcomes preserve local state and
    report reviewed identity, slug, ETag, status, and file effect.
  - Verification: pure ERT proves both complete state matrices, count previews,
    one confirmation per command, no implicit delete, and Task 2 result shapes.
    Live ERT proves mixed create/update/no-op/blocked push, server-only and
    matched deletion, stale `412`, soft-deletion wording, local-file safety, and
    continuation after an independent failure.

- [ ] **Task 4: Add revalidated selected pull for missing and matched Posts.**
  - Contract: this slice adds the report binding for pull. It previews the
    selected-operation count and requires one confirmation before its first
    mutation. Pull accepts `server-only` and safely `server-ahead` rows, treats
    `unchanged` as no-op, and returns a visible blocked result for every other
    state.
  - Contract: server-only rows retain ADR-0160's exclusive install. A
    `server-ahead` matched row carries its reviewed local path/SHA-256 and
    remote ETag; after staging the Member and Media, installation revalidates
    both snapshots and destination occupancy immediately before mutation.
  - Contract: matched install atomically replaces the current file, then
    atomically renames it when the canonical slug changed. A crash between those
    steps leaves one ID-bearing file that retry recognizes. A modified visited
    buffer blocks; a clean visited buffer follows the installed bytes and
    filename, remains unmodified, and preserves windows/point where possible.
    Every outcome uses Task 2's result schema.
  - Verification: pure ERT covers the complete state matrix, count preview and
    confirmation, remote/local races, modified and clean buffers, destination
    collision, slug rename, result fields, and between-step recovery. Live ERT
    covers server-only plus server-ahead selection across Collection pagination,
    Local Media Copies, ETag staleness, and failure isolation.

- [ ] **Task 5: Prove the integrated contract and finalize its documentation.**
  - Contract: integration coverage exercises one mixed multi-page report through
    explicit push, pull, and delete selections, including cancellation and a
    refreshed final classification. User documentation names the mark/region
    workflow, confirmations, partial success, soft deletion, and retry model.
  - Contract: once implementation matches the proposed decisions, convert the
    architecture projection from `Committed direction` to current behavior; keep
    both numberless ADR drafts proposed for post-merge promotion.
  - Verification: run the pure ERT runner and focused live ERT runner while
    iterating, then use the hook-backed commit boundary for the final slice.
    Before handoff, the branch must pass the authoritative non-e2e ladder via
    `devtool run -- cargo xtask validate --no-e2e`, including dual-backend
    coverage and Elisp coverage. `cargo xtask check` remains an optional broad
    diagnostic, not a substitute for that final gate.

## Risk checks

- Keep SQLite and PostgreSQL behavior identical with `#[apply(backends)]`; do
  not let removal of expiry weaken same-key serialization or `MutationOutcome`
  commit-indeterminate handling.
- Ensure a consumed key whose Post is Deleted cannot fall through to creation,
  and remove every maintenance/prune caller without disturbing ADR-0167's other
  cleanup domains.
- The persisted create-intent properties must not enter Atom content or make
  their own digest unstable. Key removal occurs only after durable Post-ID
  write-back.
- Never hold two HTTP mutations in flight. Cancellation and ordinary errors are
  item boundaries, not rollback requests for completed remote mutations.
- Recheck local bytes, visited-buffer modification, remote ETag, and destination
  occupancy after staging and immediately before matched replacement.
- Preserve ADR-0160's Media origin/instance/hash trust chain and retain verified
  Local Media Copies from safe partial work.
- No lint suppression may be added without explicit approval. Each completed
  task stages its intended tree and commits through `jaunder-commit`, with no
  `Co-Authored-By` trailer.
