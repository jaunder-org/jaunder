# Issue #238: Generic AppState construction

## Outcome

SQLite and PostgreSQL database openers construct the same `AppState` through one
generic, one-argument storage composition function. The obsolete `AtomicOps`
dependency holder and its duplicated backend implementations are removed;
registration and password-reset transactions instead compose shared storage
operations through the caller-owned `WriteScope`.

This is an internal architecture refactor with two intentional correctness
changes: PostgreSQL rejects concurrent reuse of one invite, and invalid
password-reset capabilities surface as client validation errors rather than
server storage errors. All other registration, invite attribution, password
replacement, session revocation, transaction-outcome, and backend-parity
behavior remains unchanged.

## Load-bearing decisions

- `storage` owns the two cross-store account-mutation orchestration functions
  because they encode database transaction invariants and are proved by the
  storage crate's dual-backend harness.
- The orchestration functions receive `&mut WriteTransaction` and only their
  exact object-safe storage dependencies. They do not receive `AppState`, a
  replacement service object, a pool, or a general executor.
- User registration performs a read-only invite-validity precheck before Argon2,
  creates the user inside the caller-owned transaction, then conditionally
  claims the still-valid invite with that `UserId`. A concurrent loser receives
  `InviteAlreadyUsed`; the surrounding scope rolls its inserted user back.
  Password reset composes the existing reset-token claim with password
  replacement and revocation of every session for that user.
- The primitive SQL operations move to their owning storage traits: the
  read-only validity check and conditional, attributing claim belong to
  `InviteStorage`; whole-user session revocation belongs to `SessionStorage`.
  The precheck is not part of the audited mutation census because it receives no
  write capability. Existing generic stores implement all three once for SQLite
  and PostgreSQL.
- `AtomicOps`, `SqliteAtomicOps`, `PostgresAtomicOps`, `AppState::atomic`, and
  the corresponding Leptos context are deleted in a clean cutover. No aliases,
  compatibility wrappers, deprecated exports, or replacement holder remain.
- The audited application-mutation census remains exactly 48 declarations. Its
  two `AtomicOps` entries are replaced by one additional `InviteStorage` entry
  and one additional `SessionStorage` entry; the structural gate remains
  fail-closed.
- Callers continue to open the transaction with `WriteScope::run`. The shared
  functions neither begin nor commit transactions and preserve `MutationOutcome`
  and `WriteScopeError` behavior.
- Invite validity and reset-token claiming still occur before password
  preparation. Invite claiming occurs after user insertion so it can persist
  `used_by`; its conditional update rechecks validity and rolls the user back on
  a race. The documented high-entropy-capability-before-Argon2 exception remains
  explicit, and failures after either claim roll the surrounding transaction
  back.
- Missing, expired, and already-used registration capabilities retain their
  existing public classifications. Password-reset capabilities now use the
  existing typed `From<ConfirmPasswordResetError>` conversion: missing, expired,
  and already-used tokens are client validation errors; internal storage and
  password-preparation failures remain masked storage errors with their source
  chains intact.
- Password reset still revokes all active sessions in the same transaction as
  token consumption and password replacement.
- `Backend` gains the inverse of its existing transaction-connection adapter: a
  factory method that converts `Pool<Self>` into the sealed, backend-erased
  `WriteScope`. This is the only backend-specific step in generic `AppState`
  composition. The new ADR supersedes ADR-0019's obsolete "`DB_SYSTEM` and
  nothing else" clause only for these two sealed transaction-adapter directions.
- The generic builder states one coercion bound per `XStore<DB>: XStorage`. Each
  store remains the sole owner of its detailed sqlx bind/executor/row bounds;
  neither the builder nor `Backend` duplicates that internal union. ADR-0019's
  per-consumer bound discipline remains intact.
- `AppState` remains a composition-root-only heterogeneous storage bundle under
  ADR-0016. The generic builder does not implement the separately decided
  on-demand backend handle factory.
- The structural change is recorded by
  `docs/adr/drafts/account-mutations-compose-storage-primitives.md` and
  projected into `docs/ARCHITECTURE.md`. `CONTEXT.md` is unchanged because no
  domain term changes.

## Acceptance

- Both production database openers call one generic one-argument
  `make_app_state(pool)` implementation; neither contains a backend-specific
  copy of the field wiring.
- `AppState` contains the thirteen object-safe storage handles and one sealed
  `WriteScope`, with no atomic-operation holder.
- The storage and web crate surfaces contain no `AtomicOps`, `SqliteAtomicOps`,
  or `PostgresAtomicOps` symbol or compatibility path.
- Registration on both backends prechecks a valid invite, consumes it with the
  created user's `UserId`, and creates exactly one user atomically.
- Registration rejects missing, expired, already-used, and raced invite claims
  with the existing error classifications. The concurrent PostgreSQL loser is
  `InviteAlreadyUsed`; a claim race, username conflict, password-preparation
  failure, or storage failure neither consumes the invite nor leaves a user
  behind.
- Password reset on both backends consumes a valid reset token, changes the
  password, and revokes every existing session atomically.
- Password reset reports missing, expired, and already-used token claims as
  client validation errors. A raced token claim has the same classification as
  the winning claim's resulting token state; password-preparation or storage
  failure rolls back the token claim, password change, and session revocation.
- The shared functions use exact trait dependencies at web callsites; no whole
  `AppState`, generic backend factory, pool, or service-locator-shaped
  replacement crosses the composition root.
- The structural mutation gate recognizes exactly 48 capability-taking
  application methods with `InviteStorage` and `SessionStorage` owning the two
  moved primitives and no `AtomicOps` category.
- Focused dual-backend storage tests cover the successful compositions,
  persisted invite `used_by`, token-state boundaries, concurrent invite/reset
  losers, rollback paths, password-error source chain, and whole-user session
  revocation.
- Changed-contract web tests prove invalid reset capabilities use the typed
  client-validation mapping; existing registration and password-reset behavior
  otherwise remains green.
- The storage crate builds for both concrete database backends, proving every
  generic store's object-safe coercion and complete store-owned sqlx bound set.
- The proposed ADR and architecture view agree with the delivered ownership,
  census, transaction boundary, and dependency-injection shape.

## Boundaries

- Do not implement ADR-0016 Phase B's on-demand backend handle factory or change
  which CLI commands construct a full `AppState`.
- Do not expose `WriteTransaction` internals, permit arbitrary SQL through it,
  or add a second transaction-start path.
- Do not change SQLite `BEGIN IMMEDIATE`, PostgreSQL locking,
  commit-indeterminate semantics, or rollback-on-drop behavior.
- Do not redesign password hashing, token formats, error messages, session
  lifetime, registration policy, or public HTTP/server-function protocols.
- Do not deduplicate unrelated backend dialect code, backup storage, database
  opening/migration behavior, or test-only custom `AppState` construction.
- Do not weaken the closed mutation census, backend-parity policy,
  dependency-injection rules, or lint/test gates.
