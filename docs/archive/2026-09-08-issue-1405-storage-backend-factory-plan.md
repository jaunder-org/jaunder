# Composition-root Storage Factory Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch` when
> useful. This outline exists because the approved spec changes a public storage
> interface and carries SQLite/PostgreSQL opening and `WriteScope` invariants.

## Scope

In:

- Replace eager `AppState` results from the existing database openers with a
  pool-owning `StorageFactory`.
- Migrate every workspace caller, including observer consumers, initialization,
  serve, affected non-serve commands, and tests.
- Preserve backend opening, observer, identity, command, and error behavior.
- Project the completed ADR-0016 Phase B design into architecture documentation.

Out:

- Storage trait, schema, migration, command syntax, output, backup, restore,
  PostgreSQL bootstrap, or publisher mutation changes.
- A compatibility opener family, factory injection, raw-pool escape hatch, or
  replacement aggregate bundle.

## Task outline

- [x] Task 1: Cut every opener caller over to `StorageFactory`
  - Contract: `open_database` and `open_existing_database` return
    `StorageFactory`; observer-bearing opening returns `OpenedDatabase` with a
    `factory`, the existing instance identity, and the existing observer. The
    factory owns one selected pool, mints every object-safe storage handle and
    `WriteScope` on demand, and can assemble `Arc<AppState>` for serve. All
    workspace callers migrate in this task; serve requests full state,
    initialization requests nothing, and affected command roots may preserve
    their existing full-state composition until their following focused tasks.
    The existing `Backend` marker retains its current meaning and bounds.
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

- [x] Task 5: Record the completed composition-root architecture
  - Contract: `docs/ARCHITECTURE.md` describes `StorageFactory`, root-only use,
    lazy exact dependency construction, serve-owned `AppState` assembly, and the
    accurate current handle count while continuing to cite ADR-0016 and the
    existing `Backend`/`WriteScope` decisions.
  - Verification: documentation formatting, links, ADR projection parity, and
    the repository static check surface pass; no text still describes Phase B as
    unbuilt or the affected commands as full-state consumers.

## Risk checks

- SQLite still distinguishes create-on-initialization from existing-only open,
  configures WAL, timeout, and cache behavior, and converges concurrent opens on
  one instance identity.
- PostgreSQL still resolves runtime credentials and preserves existing-database,
  migration, identity, and media-reference backfill behavior.
- Every handle and `WriteScope` minted by one factory shares that factory's
  pool; downstream code cannot construct or select a scope backend.
- Observer-bearing serve and saturation-metrics paths retain instance identity
  and pool snapshots without requiring eager full-state construction.
- Existing contextual errors and typed SQLx source chains survive both opener
  and downstream command failures.
- The branch contains no old return-type shim, second opener convention,
  injected factory, new services bundle, or unrelated cleanup.
