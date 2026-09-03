# Issue #979 — split post storage by concern

## Outcome

`storage::posts` becomes a directory whose focused leaves own the current post
storage concerns without changing storage behavior, public paths, SQL semantics,
or backend parity. The object-safe `PostStorage` trait, generic `PostStore<DB>`,
and backend-divergent `PostDialect` seam remain intact.

## Load-bearing decisions

- `storage/src/posts/mod.rs` is assembly only: module declarations, module
  documentation, and explicit re-exports of the existing public API.
- The directory has nine leaves with these responsibilities:
  - `models.rs`: public post, revision, lifecycle, input, and result records.
  - `errors.rs`: typed post-operation error enums and their conversions.
  - `cursors.rs`: post, scheduled-post, collection, and revision cursor
    projections between database keysets and opaque wire cursors.
  - `tags.rs`: tag records, shared tag SQL, diffing, and tag write helpers.
  - `media.rs`: persisted media subjects/references, ownership evidence, bounded
    snapshots, backfill inputs, and shared media-reference SQL helpers.
  - `visibility.rs`: viewer-resolution SQL/binds, audience projection, equality,
    and replacement helpers.
  - `lifecycle.rs`: bookkeeping expectations, revision decoding/capture, and
    shared create/update/publication/deletion mutation support.
  - `syndication.rs`: hybrid publication-window, go-live, and feed-catch-up
    query construction and row handling.
  - `store.rs`: `PostStorage`, `PostDialect`, `PostStore<DB>`, the single
    generic `PostStorage for PostStore<DB>` implementation, and the public
    `fetch_post_record` and `list_by_tag_rows` orchestration helpers.
- `PostStorage` remains object-safe. `MockPostStorage`, all public records,
  errors, inputs, cursors, and helper functions retain their existing
  `storage::*` paths through explicit public re-exports.
- ADR-0019 remains the architecture: shared operations stay in the one generic
  store implementation; `PostDialect` continues to expose only genuine backend
  SQL or transaction divergence. No forwarding layer, replacement trait set,
  macro-generated implementation, or new backend abstraction is introduced.
- Crate-internal consumers migrate from the flat `crate::posts::*` namespace to
  owner-qualified `crate::posts::<leaf>::*` paths. Concern modules and moved
  bridge items receive only the minimum crate visibility required by existing
  SQLite, PostgreSQL, migration, media, SQL, and test-support consumers.
- Existing SQL text, placeholder and bind order, row projections, span names,
  transaction boundaries, lock ordering, cursor ordering, and error mappings are
  preserved byte-for-byte where movement does not require path changes.
- Tag writes preserve stable slug ordering, first-casing-wins behavior,
  concurrent upsert semantics, and the no-write fast path for an unchanged set.
- Visibility preserves each viewer variant's SQL and bind arity, audience OR
  semantics, owner access, time/deletion gates, and fail-closed behavior.
- Syndication reads preserve hybrid-window selection, per-surface predicates,
  `(after, up_to]` go-live bounds, ordering, malformed-feed handling, and
  catch-up maxima.
- Dual-backend tests proving the generic PostStore contract remain in `store.rs`
  and continue using the backend templates required by ADR-0053. Pure tests move
  beside their owning leaf. Existing server integration tests stay in place. The
  PostgreSQL-only `postgres_tag_revision_capture_waits_for_current_media_lock`
  test moves from the generic root into `storage/src/postgres/posts.rs`, its
  dialect home; the existing SQLite-only dialect test remains there.
- `docs/ARCHITECTURE.md` is updated where source locations or the root-module
  layout description become stale. This split records no new architecture
  decision and creates no ADR.

## Acceptance

- `storage/src/posts.rs` is replaced by the assembly-only `posts/mod.rs` and the
  nine specified leaves; each leaf has one documented responsibility.
- Existing downstream code compiles against unchanged public `storage::*`
  interfaces, including `PostStorage`, `MockPostStorage`, `PostStore`, records,
  inputs, errors, cursors, media evidence, and public helper functions.
- SQLite and PostgreSQL dialect implementations still satisfy the unchanged
  generic-store/dialect contract, with no backend-specific behavior moved into
  the generic layer or generic behavior duplicated into dialects.
- Existing generic post-storage tests retain both-backend coverage and their
  backend templates; pure tests remain with their owning leaf, and the
  PostgreSQL lock test moves to its dialect home without losing its assertion.
- Existing tests continue to prove tag reconciliation, cursor pagination,
  visibility/audiences, revisions/lifecycle, media-reference ownership, and
  syndication-window behavior without dropped cases.
- The architecture view names the directory layout and current owner paths, and
  the repository gate passes.

## Boundaries

- No schema, migration, SQL-policy, transaction, locking, pagination,
  visibility, publication, revision, media-ownership, or Syndication Feed
  behavior change.
- No public API addition, removal, rename, signature change, compatibility shim,
  wildcard re-export, new dependency, new lint suppression, or new gate.
- No redesign of backend dialect files, `AppState`, post orchestration, feed
  workers, projectors, web/AtomPub adapters, or server integration suites.
- All seven coordination issues named by #979 are already merged; this work
  preserves their delivered contracts and does not reopen their scope.
