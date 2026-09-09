# Composition-root Storage Factory Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch` when
> useful. This outline exists because the approved spec changes a public storage
> interface and carries SQLite/PostgreSQL opening and `WriteScope` invariants.

## Scope

In:

- Replace eager `AppState` results from the existing database openers with a
  pool-owning `StorageFactory`, then delete the aggregate itself.
- Migrate every workspace caller: observer consumers, initialization, serve,
  commands, router/context wiring, test harnesses, fixtures, integration tests,
  and the external seed binary.
- Preserve backend opening, observer, identity, command, fixture, and error
  behavior.
- Amend and project the completed ADR-0016 Phase B design.

Out:

- Storage trait, schema, migration, command syntax, output, backup, restore,
  PostgreSQL bootstrap, publisher semantics, or user-visible behavior changes.
- A compatibility opener family, factory injection into application/fixture
  modules, raw-pool escape hatch, replacement aggregate, or services bundle.

## Task outline

- [x] Task 1: Cut every opener caller over to `StorageFactory`
  - Contract: `open_database` and `open_existing_database` return
    `StorageFactory`; observer-bearing opening returns `OpenedDatabase` with a
    `factory`, the existing instance identity, and the existing observer. The
    factory owns one selected pool and mints every object-safe storage handle
    and `WriteScope` on demand. The initial cutover temporarily retained
    `app_state()` for serve while command composition was narrowed; Tasks 6–10
    remove that transitional aggregate from every workspace consumer. The
    existing `Backend` marker retains its current meaning and bounds.
  - Verification: the workspace compiles against the clean-cutover interface;
    focused dual-backend storage opening, migration, instance-identity,
    factory-handle, and observer tests pass; opening and initialization visibly
    construct no handles or `WriteScope` before a factory method requests them.

- [x] Task 2: Narrow account command composition
  - Contract: user creation receives user storage and `WriteScope`; App Password
    creation receives user storage, session storage, and `WriteScope`;
    Invitation issuance receives site-configuration storage, Invitation storage,
    and `WriteScope`; SMTP testing receives site-configuration storage.
  - Verification: focused both-backend account command tests preserve successful
    state changes and output, initialization guidance, and typed downstream
    source chains. Structural inspection confirms account logic accepts no
    `AppState`, `StorageFactory`, raw pool, or aggregate bundle and that each
    root requests only its listed dependencies.

- [x] Task 3: Narrow site-configuration command composition
  - Contract: reads receive site-configuration storage; ordinary mutations
    receive site-configuration storage and `WriteScope`; publisher-owned
    mutations receive publisher storage, `WriteScope`, and the existing storage
    path. Dependency construction follows the selected key path.
  - Verification: focused both-backend site-configuration tests preserve
    validation, read/write output, typed errors, and publisher generation
    fencing. Structural inspection covers every set/get/list/unset path and
    rejects `AppState`, `StorageFactory`, raw-pool, aggregate-bundle, unrelated
    handle, or unnecessary `WriteScope` construction.

- [x] Task 4: Narrow WebSub dead-letter command composition
  - Contract: listing receives feed-event storage; redrive receives feed-event
    storage and `WriteScope`.
  - Verification: focused both-backend WebSub tests preserve deterministic
    listing, typed opening errors, and exact-selection atomic redrive.
    Structural inspection confirms neither path accepts a forbidden aggregate
    dependency and listing does not mint a `WriteScope`.

- [x] Task 5: Record the initial composition-root architecture
  - Contract: the first projection recorded `StorageFactory`, lazy handle
    construction, and narrowed named commands while retaining transitional
    serve-owned `AppState` assembly.
  - Verification: documentation formatting, links, ADR projection parity, and
    the repository static check surface passed.

- [ ] Task 6: Replace the router and context aggregate seam atomically
  - Contract: remove every router/context function that accepts `AppState`.
    Production `prepare_server` and the server test harness construct the
    existing Media, Theme, publisher, and ownership modules plus exact Axum
    extension handles. They supply Leptos contexts through a zero-argument
    provider closure whose interface exposes behavior, not stored dependencies.
    Shared private router combinators accept only one route family's exact
    inputs; no `RouterDeps`, factory, raw pool, or other holder crosses them.
  - Callers: migrate `server/src/{lib,context,test_support}.rs`,
    `server/tests/helpers/http.rs`, direct router tests, and all mock-override
    constructors in the same task.
  - Verification: focused router, context, AtomPub, feed, media, and mock
    substitution tests preserve production-shaped request behavior on both
    backends; the workspace compiles after the interface cutover.

- [ ] Task 7: Replace fixture operation seams and all callers atomically
  - Contract: every `storage::test_support` and `test-support` fixture operation
    receives its exact trait handles, `WriteScope`, and genuine non-storage
    inputs. Change each fixture interface and every workspace caller together;
    `AppState` may remain only as transitional test-root storage until Task 8.
  - Callers: include storage cfg-tests, all `server/tests/**` fixture families,
    server helper functions such as `set_site_config`, projector/feed/session
    helpers, and `test-support/src/{lib,main}.rs`.
  - Verification: dual-backend storage and server fixture tests preserve
    transaction acknowledgement, mock substitution, command output, and source
    chains. External seed-command coverage exercises every supported
    user/session, Post, Theme, and feed-event root on SQLite and PostgreSQL;
    sandbox profile success/rejection retains its SQLite-only policy.

- [ ] Task 8: Remove assembled state from every test harness
  - Contract: `TestEnv` becomes a SQLite/PostgreSQL resource owner for a private
    `StorageFactory`, pool fault injection, test-root-only raw-SQL inspection,
    instance identity, and teardown. Tests mint named handles/scope at their
    composition root and pass them into fixture/application seams. Migrate every
    remaining cfg-test, integration test, full-state mock literal, and external
    seed command in the same task.
  - Verification: dual-backend storage/server tests preserve raw-SQL inspection,
    injected pool failures, SQLite temp-directory lifetime, PostgreSQL teardown,
    and fixture behavior; structural inspection finds no test or fixture
    function accepting an aggregate/factory/raw pool.

- [ ] Task 9: Remove serve state, delete `AppState`, and project the decision
  - Contract: `prepare_server` mints named exact handles once and passes them to
    lifecycle, worker, manager, metrics, and router modules. Split broad wiring
    into private route-family functions or lexical closures rather than a holder
    crossing an interface. Then remove `storage/src/app_state.rs`, its export,
    every live Rust reference, and `StorageFactory::app_state`; rename
    `AppStateBackend` for factory-owned scope construction. Add a dated ADR-0016
    addendum and update `docs/ARCHITECTURE.md` and `CONTRIBUTING.md`; consider
    `CONTEXT.md` and update only if domain vocabulary changes.
  - Verification: structural search finds no live Rust `AppState` or
    `.app_state()` reference; non-archive documentation contains no stale
    current-state guidance; focused lifecycle/metrics/startup tests, workspace
    static checks, and the full host-native product test lane pass.

## Risk checks

- SQLite still distinguishes create-on-initialization from existing-only open,
  configures WAL, timeout, and cache behavior, and converges concurrent opens on
  one instance identity.
- PostgreSQL still resolves runtime credentials and preserves existing-database,
  migration, identity, and media-reference backfill behavior.
- Every handle and `WriteScope` minted by one factory shares that factory's
  pool; downstream code cannot construct or select a scope backend.
- Observer-bearing serve and saturation-metrics paths retain instance identity
  and pool snapshots with exact dependency wiring.
- `TestEnv` preserves SQLite/PostgreSQL resource lifetimes, raw-SQL inspection,
  pool fault injection, and PostgreSQL teardown without exposing an aggregate.
- Existing contextual errors and typed SQLx source chains survive opener,
  command, fixture, and external seed-binary failures.
- Router argument pressure is resolved through deeper private wiring functions,
  not a public/private holder passed across module interfaces; a strictly
  serve-local implementation detail is considered only if explicit wiring proves
  materially worse.
- The branch contains no `AppState`, old return-type shim, second opener
  convention, injected factory, replacement aggregate, services bundle, or
  unrelated cleanup.
