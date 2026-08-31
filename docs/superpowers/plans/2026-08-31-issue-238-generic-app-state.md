# Generic AppState Construction Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded
> implementation work. This outline exists because the approved change
> reallocates an audited storage-mutation surface, changes concurrency/error
> semantics, and alters the backend composition boundary.

## Scope

In:

- Retire `AtomicOps` into exact-dependency, storage-owned account mutation
  functions.
- Add race-safe invite validation/claim and whole-user session revocation to
  their owning generic stores.
- Cut over registration/password-reset callers and every `AppState` constructor.
- Replace both production AppState builders with one generic builder and a
  `Backend` scope factory.
- Update the closed mutation census, focused dual-backend/web tests, proposed
  ADR, and architecture projection.

Out:

- ADR-0016 Phase B's on-demand backend handle factory.
- Database schema, token/password formats, transaction internals, lock
  acquisition, public protocol shapes, and unrelated storage deduplication.

## Task outline

- [x] Task 1: Replace AtomicOps with composable account mutations
  - Contract: `InviteStorage` gains a read-only validity precheck plus one
    capability-taking conditional claim that records `used_by`; its dedicated
    token-state error maps into `RegisterWithInviteError`. `SessionStorage`
    gains one capability-taking whole-user revocation.
    `storage::account_mutations` owns the two free functions and their existing
    public result errors; each takes `&mut WriteTransaction` plus exact trait
    dependencies.
  - Ordering: registration prechecks before Argon2, inserts the user, then
    conditionally claims the invite; a raced claim is `InviteAlreadyUsed` and
    rolls the insert back. Password reset claims the token, hashes, sets the
    password, then revokes sessions in one scope.
  - Cutover: remove the AtomicOps trait, concrete backend implementations,
    exports, AppState field, Leptos context, and all callers with no
    compatibility path. Update custom AppState test constructors and the
    structural census from `AtomicOps (2)` to `Invite (2)` plus `Session (4)`
    while retaining 48 total.
  - Verification: focused `#[apply(backends)]` storage tests prove successful
    attribution/reset, every token-state boundary, concurrent loser behavior,
    rollback after claim/insert, password source chains, and all-session
    revocation. Focused web tests prove reset-token failures become client
    validation errors and registration mappings remain stable.

- [x] Task 2: Make AppState construction generic
  - Contract: `Backend` adds `write_scope(Pool<Self>) -> WriteScope`,
    implemented only for SQLite/Postgres beside `write_connection`; it acquires
    no sqlx bind/executor bounds. `app_state` owns one crate-visible
    `make_app_state<DB>(Pool<DB>) -> Arc<AppState>` with one explicit
    `XStore<DB>: XStorage` coercion bound per handle; detailed sqlx bounds
    remain solely on their store implementations.
  - Cutover: both production openers call the generic function; delete their
    field wiring and obsolete alias imports. Preserve all thirteen object-safe
    storage handles, open-subscription policy, pool cloning, and sealed
    backend-specific scope construction.
  - Documentation: land the proposed account-mutation ADR and architecture
    projection with the final composition/census/error semantics; `CONTEXT.md`
    remains unchanged.
  - Verification: focused dual-backend database-opening/test-harness coverage
    proves both concrete instantiations and every AppState handle; repository
    static checks prove the complete explicit bound set and absence of obsolete
    AtomicOps symbols.

## Risk checks

- The invite conditional update must require unused and unexpired state and
  persist the created `UserId`; no unconditional PostgreSQL overwrite remains.
- The precheck is read-only and receives no `WriteTransaction`, so the audited
  mutation count stays 48.
- A race or any later failure returns through `WriteScopeError::Operation` and
  rolls back user insertion, invite/reset claims, password replacement, and
  session revocation as applicable.
- SQLite retains `BEGIN IMMEDIATE`; PostgreSQL retains its existing scope and
  row-lock discipline; neither account function begins or commits a transaction.
- Password-reset missing/expired/used errors use the typed validation
  conversion, while genuine database/password failures retain masked storage
  classification and source chains.
- Free functions stay qualified through `storage::account_mutations`; types and
  object-safe traits remain item imports under repository Rust path conventions.
- `mod.rs` files remain assembly-only; module documentation explains the
  ownership and transaction boundary.
- Every constructor/callsite/export/test/doc is migrated in the same clean
  cutover; no shim, alias, deprecated path, or replacement holder survives.
