# Split Post Storage Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded work through
> `jaunder-dispatch`. This outline exists because storage SQL/locking invariants
> and cross-module visibility contracts make an unplanned multi-file split
> risky.

## Scope

In:

- Replace `storage/src/posts.rs` with the approved assembly-only directory and
  nine leaves.
- Preserve public `storage::*`, generic-store/dialect, backend-parity, SQL,
  transaction, locking, and test-homing contracts.
- Migrate crate-internal consumers to owner-qualified concern modules and update
  stale architecture paths.

Out:

- Storage behavior, schema, SQL policy, trait redesign, backend redesign, new
  abstractions, and changes to server/web/feed/projector behavior.

## Task outline

- [x] Task 1: Establish the post module surface and foundational leaves
  - Contract: create assembly-only `posts/mod.rs`; move records and inputs to
    `models.rs`, typed failures to `errors.rs`, and keyset/wire projections to
    `cursors.rs`; explicitly re-export every existing public `storage::*` name.
    Consumers of the three created leaves move to those final owner paths;
    concerns not yet extracted remain owned by `store.rs` until their task. No
    flat crate-private façade is introduced.
  - Verification: run the pure model, error, and cursor unit-test subset plus
    `devtool run -- cargo xtask test-local -- -p storage` to prove the relocated
    generic store across both backends before its gated commit.
- [ ] Task 2: Extract tag and media ownership concerns
  - Contract: `tags.rs` owns tag rows, constants, diffing, and shared writes;
    `media.rs` owns persisted-reference evidence, bounds, backfill inputs, and
    shared query helpers. Migrate backend, SQL, migration, media, and
    test-support callers directly to those owner modules with minimum
    visibility.
  - Verification: run the generic dual-backend tag and media-reference tests,
    including unchanged-set, lock-order, backfill, and ownership cases.
- [ ] Task 3: Extract visibility and lifecycle policy
  - Contract: `visibility.rs` owns resolution SQL/binds and audience helpers;
    `lifecycle.rs` owns bookkeeping expectations, revision decoding/capture, and
    shared mutation support. Move each concern and all its consumers atomically
    to the final owner paths. `store.rs` retains the one `PostStorage` trait,
    `PostDialect` seam, `PostStore<DB>`, and indivisible generic trait impl.
  - Verification: run dual-backend visibility, audience, revision, publication,
    update, deletion, and no-op mutation contracts.
- [ ] Task 4: Extract syndication reads and finish test homing
  - Contract: `syndication.rs` owns hybrid-window, go-live, and catch-up query
    helpers while generic trait methods stay in `store.rs`; move its definitions
    and consumers atomically to the final owner paths, and move only pure tests
    to concern leaves. Move
    `postgres_tag_revision_capture_waits_for_current_media_lock` to
    `storage/src/postgres/posts.rs`; leave server integration and SQLite dialect
    tests in place.
  - Verification: run dual-backend publication-window/go-live/catch-up tests and
    the PostgreSQL lock test; preserve every pre-existing test case.
- [ ] Task 5: Complete architecture projection and ownership audit
  - Contract: verify every internal post import names its final owner, retain
    only explicit public re-exports, and update only stale post-storage
    locations/layout prose in `docs/ARCHITECTURE.md`.
  - Verification: run `devtool run -- cargo xtask test-local -- -p storage` and
    `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/^storage::(listing|tags|posts|media)/)'`;
    then use `jaunder-commit` for the final task boundary.

## Risk checks

- `PostStorage` stays object-safe and `MockPostStorage` remains at `storage::*`.
- `PostDialect` retains only genuine backend divergence; no forwarding layer or
  duplicated backend implementation appears.
- SQL placeholders/binds, direct row decoding, transaction boundaries, span
  names, PostgreSQL advisory/row locks, and SQLite immediate-write assumptions
  remain unchanged.
- Tag slug ordering and first-casing-wins, viewer bind arity, audience OR/time
  gates, revision atomicity, media evidence bounds, and syndication-window
  ordering/cutoffs remain covered on both backends where applicable.
- `posts/mod.rs` contains only documentation, declarations, attributes, and
  explicit re-exports; no wildcard or compatibility façade remains.
- Each task ticks its checkbox before its gated commit; no lint suppression or
  `Co-Authored-By` trailer is introduced.
