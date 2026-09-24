# #1656 Org export and offline migration implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for a bounded
> independent task. This outline is required by a dependency fork, dual-backend
> schema/data migration, and cross-process startup/restore exclusion. The
> approved
> [spec](../specs/2026-09-24-issue-1656-org-special-strings-and-code-migrations.md)
> is authoritative.

## Scope

In: `orgize` `v0.10` fork; persistent queue of offline Rust operations seeded
only by companion SQLx migrations; migration-phase `database.lock`;
media-reference backfill cutover; reusable current-Post re-render; exact-schema
backup/restore and public-feed correctness.

Out: unrelated upstream fixes, `#+OPTIONS` support, rewriting authored source or
Post Revisions, online migration/batch checkpoints, cross-directory PostgreSQL
coordination, new migration CLI, per-SQL-migration ceremony. Operators must stop
older binaries before deployment; no new lock can coordinate with a binary that
predates it.

## Task outline

- [x] **1. Establish Org export parity at the pinned fork.** Before changing
      browser-visible bytes, capture transient baseline screenshots of a
      deterministic public Org Post and title with all three punctuation
      sequences at fixed route/theme/viewport. Fork the exact upstream `v0.10`
      tip (record its SHA), test prose (including nested inline prose) versus
      literal/code/link destination contexts and headings in the fork, assert
      that authored Org source remains unchanged, implement the smallest
      exporter change, and review the fork diff against that base. Pin Cargo,
      Nix vendoring/flake input, and cargo-deny source policy to the same
      immutable revision; add Jaunder body/title regression proof. Record the
      fork decision and project it into `docs/ARCHITECTURE.md` when the feature
      lands.
  - Contract: `host::render::render_post` and `render_title` keep using
    `orgize`; the fork changes only exporter text, not source parsing or stored
    source.
  - Verification: fork tests for nested inline prose and literal source
    preservation; focused Jaunder host body/title tests; pin/flake/lockfile
    agreement; no altered Markdown/HTML export caused by dependency wiring.
- [ ] **2. Define one migration-phase exclusion boundary across command paths.**
      Acquire one exclusive `<storage>/database.lock` before SQLx+queue draining
      for every production server/CLI DB open and release before ordinary work;
      keep `runtime.lock` as the server-lifetime upload/duplicate-serve guard. A
      CLI with pending work refuses while the same-directory server holds
      `runtime.lock`, whereas ordinary live CLI work without pending work
      remains permitted. Backup export participates in the migration lock for
      its snapshot; restore refuses a live server and owns the same lock
      continuously from before emptiness preflight through DB/theme/Media
      restoration and validation. Avoid recursive lock acquisition by inner open
      helpers.
  - Contract: the command/composition boundary owns lock lifetimes; storage DB
    open + queue drain run under one caller-owned guard. Lock identity is the
    storage directory for both supported backends, not PostgreSQL's remote DB
    name.
  - Verification: real competing-process same-directory lock serialization for
    server/CLI/backup; pause restore after DB import and prove openers cannot
    serve before Media is complete; startup/rollback/error paths release locks.
    The pending-queue refusal and sandbox live-command check follow in Task 3
    once the queue exists.
- [ ] **3. Enqueue and drain offline Rust work with SQLx on both backends.** Add
      matching SQL migrations for the queue and two enqueues (media repair
      before rebuild); normal SQL migrations remain unchanged. Implement a
      closed operation dispatcher, monotonically ordered queue rows with
      diagnostic timestamp, and one transaction per row wrapping work + row
      deletion. Move the current startup media-reference backfill to its queued
      operation and remove its unconditional open-time call. Keep each Rust
      operation in a semantically named source file and share mechanics rather
      than cloning them.
  - Contract: SQLx applies its normal migrations first. SQLx's private migration
    table is never read directly; a committed enqueue survives an interrupted
    Rust step. Unknown operation, SQLx failure, or Rust failure returns an error
    before serving/ordinary CLI use. Later SQLx migrations may enqueue the same
    operation name again.
  - Verification: `#[apply(backends)]` upgrade, ordering,
    exactly-once-on-success, repeat enqueue, unknown name, injected
    mid-operation rollback/crash and next-open retry; injected SQLx migration
    failure returns no ordinary storage handle or service and runs no Rust
    operation; a CLI with pending work refuses under a live server's
    `runtime.lock` while a no-pending CLI and sandbox command mode still
    operate. Direct `open_database` fixture semantics stay aligned. Record the
    offline queue and narrow ADR-0092 exception in a numberless ADR plus
    `docs/ARCHITECTURE.md`.
- [ ] **4. Rebuild current Post derivatives and dependent public projections.**
      The reusable `rebuild_rendered_posts` operation recomputes bodies and
      titles of every current Post format from canonical source, writing only
      changes, including retained Deleted Posts. Reconcile current-Post Media
      references if output changes while retaining revision references and Media
      Record ownership. Invalidate stale Syndication Feed caches and queue
      affected public-feed/WebSub work atomically; never manufacture a Post
      Revision or edit timestamp. Preserve existing per-Post lookup/media
      semantics rather than routing through a user edit.
  - Contract: offline single transaction per queue item; no intermediate
    visibility, no persistent completed-work history, no replacement of stored
    source/revision snapshots. The handler name is reusable in a later SQLx
    enqueue.
  - Verification: dual-backend upgrade fixture with all formats, deleted and
    public/non-public Posts, titles, historical revision,
    changed-media-reference fixture, unchanged-row no-op, and injected rollback;
    compare revision counts and edit timestamps before/after for changed and
    unchanged Posts, with byte-identical historical revisions; focused
    web/Atom/RSS/JSON Feed assertions including validator freshness and WebSub
    work.
- [ ] **5. Prove recovery, presentation, and final integration.** Include the
      queue table as portable backup data, test CLI/server-level pending-row
      export/import in both directions between SQLite and PostgreSQL at exactly
      matching schema versions, and verify restoration drains before serving.
      Pause a migration during a concurrent export to prove the snapshot holds
      either the pending pre-attempt row/data or completed post-migration
      row/data, never a mixed state. Capture a final screenshot at the baseline
      Org Post's route/theme/viewport and compare the transient before/after
      pair; do not commit visual artifacts. Review the final fork diff, ADR
      projections, and both-backend restore/CLI boundaries; run relevant focused
      suites and broader gates for the verified final tree.
  - Contract: no restore guarantee beyond the approved spec and existing backup
    failure contract; no fork PR merge or Jaunder PR merge without their
    respective approved lifecycle decisions.
  - Verification: targeted
    `devtool run -- cargo xtask test-local -- -p storage ...`, focused
    `cargo xtask e2e-local` for browser output, precommit/prepush hooks, and an
    explicit hermetic `cargo xtask validate --no-e2e` because Nix fork vendoring
    and storage migration are load-bearing. CI's `{backend}×{browser}` lanes
    remain authoritative.

## Risk checks

- The offline exception **narrows** ADR-0092 for the migration transaction only;
  normal write transactions remain bounded. No lock is held for the rest of
  server/CLI lifetime except the existing `runtime.lock` on `serve`.
- SQLite and PostgreSQL SQL migration filenames and queue semantics match.
  Sequence order is not wall-clock order. One queue-row transaction either
  commits derivatives, feed effects, and deletion or rolls them all back.
- Restore's filesystem phase follows DB import; the lock cannot drop between
  them. Backup and restore do not skip the queue as a cache, and exact
  schema-version matching stays intact.
- A fork revision descends from recorded `v0.10` SHA and is changed only for
  special-string export; Nix, Cargo, and deny policy point to that reviewed
  source. Existing source, title, and Post Revision representations do not
  silently become the fork's authored bytes.
- Public projector, Syndication Feed cache/fingerprints, and WebSub
  notifications reflect changed rendered output without changing unrelated
  eligibility or creating user edits. No test suppressions or lint suppressions
  are introduced without approval.
