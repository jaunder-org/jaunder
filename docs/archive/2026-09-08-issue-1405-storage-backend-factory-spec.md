# Complete the composition-root storage factory

Issue: #1405

## Outcome

Opening Jaunder storage yields a pool-owning `StorageFactory` from which each
composition root constructs only the storage dependencies it needs. Non-serve
commands stop constructing the complete `AppState`; the serve composition root
continues to assemble the full storage state required by the running server.

This is a behavior-preserving architectural cutover for SQLite and PostgreSQL.

## Load-bearing decisions

- `StorageFactory` is the canonical public name for the pool-owning storage
  factory. It is distinct from the existing `Backend` marker, which continues to
  identify a generic SQLx database implementation.
- The existing database-opening interface returns `StorageFactory` rather than
  an eagerly assembled `AppState`. This is a clean cutover: all callers migrate,
  and no parallel legacy opener family or compatibility alias remains.
- Observer-bearing database opening returns the same factory together with the
  existing instance identity and pool observer information. Pool observation is
  not coupled to full-state assembly.
- The factory owns exactly one runtime-selected SQLite or PostgreSQL pool. Every
  storage handle and `WriteScope` minted from one factory uses that same pool.
- The factory can mint every storage trait handle needed to construct the full
  server state, but callers request handles individually. Creating one handle
  must not create unrelated handles.
- `WriteScope` remains factory-owned construction. Application and command code
  may receive and use a scope but cannot select its backend, construct one from
  a raw pool, or execute arbitrary SQL through it.
- A factory is used only at a composition root. It is never injected into a
  command, application module, or long-lived service as a storage locator.
- `AppState` remains the storage-only aggregate for the running server. The
  serve composition root explicitly asks the factory to assemble it; no other
  affected command receives or constructs it.
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
- The architecture projection is updated to describe the completed ADR-0016
  Phase B design and the current storage-handle count accurately.

## Acceptance

- The ordinary and observer-bearing storage openers expose `StorageFactory`, not
  an eagerly constructed `AppState`.
- Opening storage alone creates neither storage handles nor a `WriteScope`;
  requesting one dependency does not create any unrelated dependency.
- The serve path constructs its complete `AppState` from one factory and retains
  its current pool-observer and instance-identity behavior.
- `jaunder init` completes initialization without constructing the full server
  state.
- The account, site-configuration, and WebSub command roots listed in #1405 mint
  only the handles and `WriteScope` required by the selected command, then pass
  those dependencies directly into command logic.
- No affected command accepts `AppState`, `StorageFactory`, a raw pool, or a new
  aggregate dependency bundle.
- Existing successful command output and state changes remain unchanged for both
  SQLite and PostgreSQL.
- Missing or uninitialized storage retains the current create-versus-existing
  behavior, initialization guidance where present, and SQLx source-chain
  visibility for both backends.
- Site-configuration publisher mutations retain generation-fenced mutation and
  commit-outcome behavior; WebSub redrive retains exact-selection atomicity.
- Pool-observer saturation reporting and concurrent database-opening identity
  tests continue to pass.
- Repository architecture documentation no longer says that the Phase B factory
  is unbuilt or that affected non-serve commands construct full `AppState`.

## Boundaries

- No storage traits, persisted schema, migrations, command syntax, command
  output, or user-visible policy changes.
- No change to the public meaning or generic bounds of `Backend`.
- No factory injection, service-locator access, raw-pool escape hatch, or second
  services bundle.
- No refactor of backup, restore, PostgreSQL bootstrap, publisher mutation
  semantics, or unrelated command composition.
- No compatibility shim for the former opener return type; workspace callers
  migrate in the same change.
- Tests are added or changed only where needed to prove the new composition
  contract and preserve the observable behavior above.
