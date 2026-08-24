# Issue #507: Store-handle accessors for server storage dependencies

## Outcome

Server-side storage dependency call sites read through named borrow-only
accessors instead of spelling `Arc::as_ref()` at each use. The refactor reduces
mechanical `.as_ref()` noise while preserving the existing dependency-injection
architecture: storage handles remain `Arc<dyn *Storage>` at owning boundaries,
and consumers borrow `&dyn *Storage` for individual calls.

## Load-bearing decisions

- Add accessor methods on existing owning structs that already hold store
  handles:
  - `storage::AppState` for CLI/server composition helpers that receive the app
    storage bundle.
  - `server::atompub::posts::PostServices` for the AtomPub member/collection
    handlers' request-local storage bundle.
  - `server::feed::worker::FeedWorker` for background feed regeneration and
    event processing.
- Accessors return `&dyn Trait` and contain the only new `self.<field>.as_ref()`
  for those owned handles.
- Cover every issue-listed store-handle site. Dense ownership clusters use
  accessors; singleton direct-injection handlers (`feed/handlers.rs`,
  `atompub/service.rs`, `atompub/rsd.rs`, and AtomPub media handlers if still
  present) must either pass borrowed `&dyn` values through a narrow helper
  boundary or be explicitly left as a genuine Axum extraction boundary with no
  newly introduced wrapper. Do not hide an issue-listed site by leaving the file
  untouched.
- Do not replace `Arc<dyn Trait>` fields, remove constructor injection, or
  introduce a new service-locator/bundle. This must stay consistent with
  ADR-0016: dependency breadth remains at existing ownership/composition
  boundaries.
- Non-storage `.as_ref()` calls are out of scope: `Option`, `ETag`,
  `ContentType`, `ContentHash`, URL/domain newtypes, and similar value borrows
  keep their existing idioms.
- `Arc` clones or direct handle access needed to layer Axum/Leptos contexts,
  start workers, or transfer shared ownership remain legitimate.

## Acceptance

- Every issue-listed store-handle `.as_ref()` site is addressed in
  `server/src/atompub/posts.rs`, `server/src/feed/handlers.rs`,
  `server/src/feed/worker.rs`, `server/src/atompub/service.rs`,
  `server/src/atompub/rsd.rs`, AtomPub media handlers, and
  `server/src/commands.rs`.
- Dense clusters in `atompub/posts.rs`, `feed/worker.rs`, and `commands.rs` call
  accessors instead of borrowing owned fields directly.
- Accessor definitions are the only remaining `.as_ref()` sites for
  corresponding owned store handles in dense clusters.
- Any remaining issue-listed singleton store-handle `.as_ref()` is justified by
  an existing Axum extraction/ownership boundary rather than omission; no new
  wrapper exists only to erase one call.
- Existing behavior is unchanged: AtomPub collection/member/service/RSD
  operations, Syndication Feed serving/regeneration, feed worker processing, and
  CLI helpers still call the same storage traits with the same inputs.
- A focused server/storage compile or test command covering the touched crate(s)
  passes before shipping.

## Boundaries

- No storage trait, storage backend, database schema, or storage implementation
  changes.
- No public HTTP, AtomPub, feed, media, or CLI behavior changes.
- No new DI abstraction beyond accessors on existing owners.
- No attempt to remove every `.as_ref()` in the server crate; the scope is
  store-handle noise, not value-borrow idioms.
