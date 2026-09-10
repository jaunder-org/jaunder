# Complete the composition-root storage factory

Issue: #1405

## Outcome

Opening Jaunder storage yields a pool-owning `StorageFactory` from which each
executable, CLI, or test-harness composition root constructs only the storage
dependencies it needs. The obsolete `AppState` aggregate is deleted; serve,
commands, application functions, fixtures, and tests receive exact storage
handles and `WriteScope`.

This is a behavior-preserving architectural cutover for SQLite and PostgreSQL.

## Load-bearing decisions

- `StorageFactory` is the canonical public name for the pool-owning storage
  factory. It is distinct from the existing `Backend` marker, which continues to
  identify a generic SQLx database implementation.
- The existing database-opening interface returns `StorageFactory`. This is a
  clean cutover: all callers migrate, `AppState` and `StorageFactory::app_state`
  are deleted, and no parallel legacy opener family or compatibility alias
  remains.
- Observer-bearing database opening returns the same factory together with the
  existing instance identity and pool observer information. Pool observation is
  not coupled to full-state assembly.
- The factory owns exactly one runtime-selected SQLite or PostgreSQL pool. Every
  storage handle and `WriteScope` minted from one factory uses that same pool.
- The factory can mint every storage trait handle required by the running
  server, but every composition root requests handles individually. Creating one
  handle must not create unrelated handles.
- `WriteScope` remains factory-owned construction. Application and command code
  may receive and use a scope but cannot select its backend, construct one from
  a raw pool, or execute arbitrary SQL through it.
- A factory is used only at an executable, CLI, or test-harness composition
  root. It is never injected into a command, application module, fixture
  operation, or long-lived service as a storage locator.
- No heterogeneous storage aggregate replaces `AppState` as an application
  dependency. Serve may use one private lifecycle-only root wiring value to
  organize its broad composition; that value never crosses into a router,
  context, worker, manager, metrics module, or other runtime subsystem, all of
  which receive exact handles.
- Each affected command receives its exact storage dependencies directly:
  - user creation: user storage and `WriteScope`;
  - App Password creation: user storage, session storage, and `WriteScope`;
  - Invitation issuance: site-configuration storage, Invitation storage, and
    `WriteScope`;
  - SMTP testing: site-configuration storage;
  - site-configuration reads: site-configuration storage;
  - ordinary site-configuration mutations: site-configuration storage and
    `WriteScope`;
  - publisher-owned site-configuration mutations: publisher storage and
    `WriteScope`;
  - WebSub dead-letter listing: feed-event storage;
  - WebSub dead-letter redrive: feed-event storage and `WriteScope`.
- Storage fixture operations and server test helpers receive only their exact
  storage handles and `WriteScope`. Test harnesses retain the factory and raw
  pool only at the test root for dependency minting, raw-SQL inspection, fault
  injection, and resource lifetime ownership; neither crosses a fixture or
  application seam.
- The external `test-support` seed binary opens storage at each command root and
  injects exact handles into sandbox, Post, Theme, session, and feed-event seed
  operations.
- Existing non-storage composition inputs remain explicit. In particular,
  publisher-owned site-configuration mutations retain their storage path for
  generation fencing; it does not become factory state.
- Database opening preserves the current ordering and semantics of connection
  setup, migrations, instance-identity establishment, media-reference backfill,
  and observer construction.
- SQLite preserves create-on-initialization versus existing-database-only
  behavior, WAL mode, busy timeout, cache configuration, and concurrent
  instance-identity convergence.
- PostgreSQL preserves runtime credential resolution, existing-database
  behavior, migrations, instance identity, and media-reference backfill.
- Opening and downstream storage failures retain their typed source chains.
  Existing command-specific initialization guidance and contextual errors remain
  unchanged.
- ADR-0016 is amended to record that the completed Phase B factory makes
  `AppState` obsolete; `docs/ARCHITECTURE.md` and `CONTRIBUTING.md` describe the
  exact composition-root and test-harness design accurately.

## Acceptance

- The ordinary and observer-bearing storage openers expose `StorageFactory`.
- Opening storage alone creates neither storage handles nor a `WriteScope`;
  requesting one dependency does not create any unrelated dependency.
- `AppState`, its module/export, `StorageFactory::app_state`, and every
  construction/import/reference in live Rust code are removed.
- The serve path mints exact dependencies from one factory and retains current
  pool-observer, instance-identity, worker, router, and startup behavior. Its
  private root wiring value may cross only lifecycle composition helpers; every
  runtime subsystem receives exact dependencies.
- `jaunder init` completes initialization without constructing storage handles.
- The account, site-configuration, and WebSub command roots listed in #1405 mint
  only the handles and `WriteScope` required by the selected command, then pass
  those dependencies directly into command logic.
- Router/context functions, runtime lifecycle components, storage fixtures,
  server test helpers, and the external seed binary accept exact dependencies
  rather than `AppState`, `StorageFactory`, a raw pool, or a replacement
  application aggregate. Private serve composition helpers may receive the
  lifecycle-only root wiring value described above.
- `TestEnv` preserves SQLite/PostgreSQL lifetime, raw-SQL inspection, and pool
  fault-injection behavior while ceasing to expose an assembled storage state.
- Existing successful command output and state changes remain unchanged for both
  SQLite and PostgreSQL.
- Missing or uninitialized storage retains the current create-versus-existing
  behavior, initialization guidance where present, and SQLx source-chain
  visibility for both backends.
- Site-configuration publisher mutations retain generation-fenced mutation and
  commit-outcome behavior; WebSub redrive retains exact-selection atomicity.
- Pool-observer saturation reporting and concurrent database-opening identity
  tests continue to pass.
- ADR-0016, repository architecture documentation, and contributor guidance
  describe the completed exact-dependency design rather than retaining
  `AppState` as a serve or test convenience.

## Boundaries

- No storage traits, persisted schema, migrations, command syntax, command
  output, or user-visible policy changes.
- No change to the public meaning or generic bounds of `Backend`.
- No factory or raw pool is injected into production, application, or fixture
  modules; the existing test-root-only raw-SQL, fault-injection, and resource
  lifetime capability remains. No service locator, replacement application
  aggregate, or second services bundle. The private serve root wiring value is
  an implementation detail of lifecycle composition and is never a runtime
  dependency.
- No refactor of backup, restore, PostgreSQL bootstrap, publisher mutation
  semantics, or unrelated application behavior.
- No compatibility shim for `AppState` or the former opener return type;
  workspace callers migrate in the same change.
- Tests change only where needed to use exact dependency seams and preserve the
  observable behavior above on both SQLite and PostgreSQL.
