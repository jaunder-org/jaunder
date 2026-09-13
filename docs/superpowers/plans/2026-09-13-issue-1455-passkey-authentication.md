# Passkey Authentication Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch` when useful. This outline exists because the approved spec
> adds a cryptographic protocol, durable and transient schemas, authentication
> transactions, and cross-backend concurrency invariants.

## Scope

In:

- Add discoverable, UV-required Passkey enrollment, management, and cookie-
  Session login while preserving password authentication and recovery.
- Add durable credential/user-handle storage, transient ceremony storage,
  cleanup, backup policy, and RP-host configuration guards on SQLite and
  PostgreSQL.
- Add the browser WebAuthn bridge, Login and Passkeys surfaces, exact server-
  function orchestration, observability, and real-browser verification.
- Pin a narrowly adapted `webauthn-rs` 0.5.5 fork because its safe upstream API
  hardcodes non-resident registration, conditional mediation for discovery, and
  rejection of non-monotonic counters contrary to the approved contract.

Out:

- Every exclusion in the approved specification, especially conditional UI,
  username-first WebAuthn, attestation policy, passkey-only accounts, multiple
  origins, alternate Sessions, and machine-client authentication.
- Direct use of `webauthn-rs-core`, catching `CredentialPossibleCompromise` as
  successful verification, manual cryptographic verification, or a second
  WebAuthn implementation.

## Key contracts

- `host::passkey` is the sole relying-party adapter. Storage and web use its
  typed credential/ceremony values; no caller reaches fork internals.
- One exact `PasskeyStorage` owns durable User handles/credentials and two
  distinct ceremony types: registration binds User and cookie Session;
  signed-out authentication binds neither until finish identifies the account.
- The browser holds protocol JSON and a raw ceremony handle only. Storage holds
  only its hash; prohibited sinks hold neither.
- A confirmed ceremony claim commits before verification and permits exactly one
  finish. A claim with indeterminate commit never proceeds; the client starts a
  fresh ceremony, while storage correctly treats consumption as unknown.
- Credential/Session and deletion/revocation use separate all-or-nothing
  `WriteScope` transactions. Session cookies are absent after operation rollback
  but follow existing login reconciliation on `CommitIndeterminate`.
- All management operations use only an ambient cookie Session; authentication
  start/finish remain signed-out operations.

## Task outline

- [x] Task 1: Establish the pinned relying-party adapter
  - Contract: create a `jaunder-org/webauthn-rs` fork with three narrow, opt-in
    changes: resident-required/no-attestation registration; empty-allow-list
    discovery with mediation omitted; and verified anomalous-counter results,
    leaving upstream defaults unchanged. Pin the required fork package(s) under
    root `[patch.crates-io]` by exact commit, ensure `Cargo.lock` selects that
    revision without a registry duplicate, add the required `deny.toml`
    organization allowance, and record rationale/removal condition beside the
    patch and in the ADR. `host::passkey` wraps only the safe `Webauthn` API,
    enables server-state serialization, and constructs one exact origin/RP.
  - Verification: fork tests prove resident key required, UV required,
    attestation none, mediation absent, and allow-list empty. Signature,
    origin/RP, UP, and UV failures still reject. Upstream mode accepts stored
    zero/returned zero but rejects stored-nonzero/returned zero and equal or
    decreased nonzero counters; opt-in mode returns verified results for those
    anomalies. Focused host tests cover state/credential serialization, exact
    origin, anomaly classification, and disabled permissive origin options.

- [x] Task 2: Add dual-backend Passkey persistence
  - Contract: add domain values for label, credential identity, random opaque
    User handle, and ceremony handle/hash. Paired next-numbered migrations
    create a durable User-handle table, durable credentials, and separate
    registration and authentication ceremony tables. Migrations backfill a
    unique random handle for every existing User; `UserStore::create_user`
    inserts one for every new User in the same transaction. Registration state
    requires User, cookie-Session, label, origin, and RP ID bindings;
    authentication state has no User/Session binding. Both require hashed
    handle, purpose, origin, RP ID, expiry, and one-time state. Generic
    `PasskeyStore<DB>` implements one object-safe `PasskeyStorage`; row decode
    rejects credential-ID/state disagreement. `StorageFactory`, restore order,
    fixture seeds, and `TestEnv` include the durable handle.
  - Verification: `#[apply(backends)]` tests cover existing/new User backfill,
    handle and credential uniqueness, labels/list order, ownership,
    serialization, purpose-specific constraints, expiry cutoff, atomic and
    concurrent claim, and cleanup predicates. Migration, restore-order, and
    schema parity pass on both backends.

- [x] Task 3: Enforce cross-domain transaction and lifecycle policy
  - Contract: extend `SessionStorage` with transaction-scoped revoke-all-except-
    current. Authentication updates backup state/high-water counter/last use and
    creates a Session atomically; deletion removes one owned credential and
    revokes other Sessions atomically. One internal transaction-scoped RP lock
    is shared by registration finish and every `site.base_url` set, aggregate,
    and unset path: SQLite uses its immediate writer transaction; PostgreSQL
    uses one deterministic advisory-xact key independent of optional rows.
    Registration re-reads exact origin/RP and inserts under that lock; config
    writes lock, inspect credential presence, and enforce the resulting host.
    Backup excludes both ceremony tables, includes handles/credentials, and
    retention wires Passkey pruning through `DatabaseMaintenance`, lifecycle,
    `StorageFactory`, and a new bounded `host::retention::Domain`.
  - Verification: both-backend fault injection proves atomic rollback for
    credential/Session and deletion/revocation paths. Concurrency races first
    enrollment against hostname set/unset and admits no stranded credential. CLI
    and web tests cover every configuration door and scheme/port/HTTPS-to- HTTP
    changes. Backup round trips preserve durable state and exclude all ceremony
    states; maintenance tests cover cutoffs, batches, composition, and
    telemetry.

- [ ] Task 4: Implement typed ceremony and management server functions
  - Contract: add typed start/finish registration and authentication plus list,
    delete, and availability operations. A dedicated cookie-only guard protects
    list/delete and both registration endpoints, rejecting Bearer/Basic without
    fallback; availability and signed-out authentication remain public.
    Registration start verifies current password; finish confirms the same
    User/Session and, after confirmed claim, takes Task 3's RP lock for final
    config recheck plus credential insert. Delete verifies current password and
    credential ownership before its atomic preserve-current/revoke-others
    mutation. Authentication finish neutrally cross-checks User handle and
    credential ID, verifies, audits, and establishes the existing
    `SessionUser`/cookie behavior. Wire `PasskeyStorage` and exact companions
    through `server/src/context.rs`, production lifecycle and integration
    `make_app` roots; update the explicit server-function registrar and
    wire-path inventory for every endpoint.
  - Verification: direct domain tests plus actual `/api/...` HTTP tests cover
    malformed payload decoding, status/body mapping, header/cookie precedence,
    neutral failures, insecure/config-changed/replayed state, identity mismatch,
    no token body, rollback without cookie, and commit-indeterminate Session
    reconciliation. Dual-backend deletion cases prove that wrong password,
    missing/revoked or explicit-Authorization Session, and foreign credential ID
    leave every Passkey and Session unchanged; success preserves current and
    revokes others. Integration enrolls a Passkey, completes real email reset,
    observes prior Session revocation, then lists and authenticates with that
    Passkey; password registration/login, invitations, forgot-password,
    Bearer/Basic/AtomPub, and explicit-auth cookie retirement remain covered.

- [ ] Task 5: Add browser WebAuthn and Passkeys UX
  - Contract: domain-free `client::webauthn` code capability-checks the
    standards API, converts typed JSON options/results, invokes explicit
    `navigator.credentials.create/get` without conditional mediation, and
    distinguishes success, cancellation, unsupported capability, and thrown
    failure. Add exact `web-sys` Credential/PublicKeyCredential features in
    `client/Cargo.toml`; if current bindings lack required JSON methods, keep
    the minimal audited `wasm_bindgen` extern inside this module. Login reuses
    Session reconciliation. A routed Passkeys page follows existing Field,
    mutation-feedback, Resource/list, sidebar, and navigation conventions.
  - Verification: `client` run-in-browser `wasm_bindgen-test` covers protocol
    conversion, capability, cancellation, and error outcomes. Host decision-
    fold tests cover UI state; Playwright covers component interaction,
    unsupported messaging, cancellation, validation, and navigation. Successful
    real ceremonies wait for Task 6's authenticator fixture.

- [ ] Task 6: Close integration, browser, and documentation proof
  - Contract: add a Chromium Playwright CDP virtual-authenticator fixture using
    the repository's wrapped interaction/readiness helpers, then cover real
    enrollment, account-picker assertion, two credentials, metadata refresh,
    deletion, preserved current Session, and revoked sibling Sessions. The CI
    gate's Chromium path carries real ceremonies; Firefox keeps standards-based
    production code and tests non-virtual UI behavior, while WebKit remains the
    existing local-only project. Do not mock a successful ceremony on engines
    without a virtual authenticator. Finish documentation with the exact fork
    revision/removal condition and operator-visible configuration behavior;
    remove throwaway probes.
  - Verification: focused storage/host/web/client/server lanes pass; actual
    Chromium WebAuthn smoke passes on SQLite and PostgreSQL; Firefox gated and
    local WebKit non-ceremony coverage remain green; then `jaunder-commit` runs
    the required full commit gate against the staged tree.

## Risk checks

- The fork changes policy selection only; upstream cryptographic, challenge,
  origin/RP, signature, UP, UV, and account-binding checks remain intact and
  defaults retain upstream behavior. The revision, license allowlist, patch
  rationale, removal condition, and dependency lock are reviewed together.
- Resident-key request, empty allow-list discovery, and omitted mediation are
  characterized from emitted JSON; no feature name is treated as proof.
- Unknown User handles, unknown credential IDs, and cross-User pairs share one
  neutral response and cannot update credentials or create Sessions.
- Raw ceremony handles are high entropy, hashed before persistence, confirmed-
  claim single-use, excluded from backup, and absent from telemetry.
- Registration state cannot omit its User/Session binding; authentication state
  cannot acquire one before discoverable identification.
- Current-password verification retains absent-User timing equalization and
  never moves Argon2 ahead of cheap high-entropy handle rejection.
- The RP lock has one backend-stable identity shared by registration and every
  config door; unset cannot bypass it, while scheme/port remain legal.
- Counter anomalies cannot lower the high-water mark, skip backup updates,
  bypass UV, or create a Session without PII-free telemetry.
- Operation rollback emits no cookie; `CommitIndeterminate` retains the existing
  raw-token cookie and authoritative reconciliation contract. Indeterminate
  ceremony claim never proceeds to verification.
- Backup derives all durable Passkey state and excludes both ceremony tables;
  restore order, cleanup composition/batching, and migrations stay symmetric.
- Every new server function is registered, receives production/test contexts,
  and has HTTP-level status/body/cookie evidence in addition to direct tests.
- Cancellation is an expected no-op; unsupported environments retain password
  auth. Real ceremony automation is Chromium-only because the current Playwright
  Firefox/WebKit harness exposes no virtual authenticator.
