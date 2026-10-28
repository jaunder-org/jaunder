# Persistent browser Sessions Implementation Outline

> Execute with jaunder-iterate and jaunder-dispatch. This outline exists because
> durable schema migration, authentication semantics, storage concurrency, and
> cross-backend cleanup meet on one Session lifecycle.

## Scope

In: implement the approved #1845 spec exactly, including typed browser/machine
rows, legacy/unknown carveout, persistent cookie renewal, authoritative idle
expiry, bounded cleanup, SQLite/PostgreSQL parity, and required proofs.

Out: absolute lifetime, configurable retention, App Password expiry, token
rotation, auth/protocol changes, generic retention framework, production session
access/credentials, and any expansion of the accepted legacy carveout.

## Task outline

- [ ] Task 1: Add durable Session kind and preserve legacy rows.
  - Contract: explicit closed kind (`Browser`, `Machine`, `Unknown`) on every
    new creation; migration assigns pre-existing rows `Unknown`, never inferring
    from label, creation route, or request transport. Audit every
    `create_session` callsite and explicitly classify each: all password
    login/registration and Passkey establishment paths create Browser; every App
    Password path creates Machine; test/support and any other path deliberately
    chooses its kind. Update SQLite/PostgreSQL schemas, storage models/queries,
    fixtures, and backup/restore codecs. Export preserves stored kinds including
    Unknown; restore accepts only the closed set and treats an absent kind in
    old backups as Unknown. Preserve revocation/listing; never infer legacy
    kind.
  - Verification: additive upgrade from populated legacy schema, fresh schema,
    backup/restore round-trip and older fixture decode; classification on both
    backends via `#[apply(backends)]` across every audited creation path. Define
    rollback as forward-only unless old binaries tolerate the additive column;
    otherwise restore the pre-migration backup, not an unsupported downgrade.
- [ ] Task 2: Enforce expiry, atomic touch and same-cookie renewal.
  - Contract: one explicit UTC `now`; typed Browser is expired at
    `last_used_at <= now - 2_592_000s`. Define touch eligibility against the
    persisted timestamp and existing 60-second policy. Storage authentication
    must atomically establish active+unrevoked+not-expired eligibility and
    either return the current row without a write inside throttle, or
    conditionally touch and return its timestamp. Since storage runs in a
    caller-owned write transaction, success/cookie emission waits for
    acknowledged enclosing commit; commit error or ambiguous outcome emits no
    renewal. Expiry, touch and revoke have a single storage linearization order:
    once expired/revoked, stale-cookie auth cannot revive; a touch committed
    before expiry governs, otherwise auth fails. Only cookie auth whose
    enclosing transaction committed a touch renews the same token. Compute
    `floor_unix_seconds(committed last_used_at + 2_592_000 seconds)`; emit
    `Expires` at that absolute instant and `Max-Age` as nonnegative remaining
    duration, neither extending server validity. No renewal in throttle.
    Explicit Authorization precedence remains ADR-0132; Bearer/Basic never renew
    ambient cookies. Preserve logout/retirement cookie clearing.
  - Verification: controlled UTC clock at before/equal/after idle cutoff and
    60-second threshold; throttled request followed by idle expiry; commit
    failure; deterministic concurrent touch/expiry/revoke races; cookie
    attributes, expiry bound, and no token churn; password and Passkey paths.
- [ ] Task 3: Add bounded typed-browser cleanup and qualify retention policy.
  - Contract: extend the daily transient-data pass with repeated fixed-size
    statements of at most 500 rows, draining the eligible backlog in one pass
    and releasing locks between batches. Select only typed Browser rows
    at/beyond the UTC cutoff; Unknown and Machine are excluded. Ensure progress
    without a global transaction; report Session cleanup failure separately
    while other domains continue, retrying on a later pass per ADR-0167. Update
    ADR-0167's current Session exclusion only to reflect this policy; no generic
    retention abstraction or new schedule.
  - Verification: both backends cover eligible/ineligible classes, exact cutoff,
    batch bound and repeated passes, partial progress, injected domain failure
    with independent-domain continuation, and next-pass retry; legacy and
    machine rows survive.
- [ ] Task 4: Verify browser-visible persistence and integrated behavior.
  - Contract: browser proof observes persistent cookie and successful use
    without claiming browser restart policy guarantees; logout/retirement
    attributes remain intact. No production data or credentials.
  - Verification: focused auth tests and
    `devtool run -- cargo xtask e2e-local <auth spec>`; then applicable
    `prepush` and CI/validation ladder per CONTRIBUTING, escalating for the
    backend/browser integration boundary.

## Risk checks

- Inspect all `create_session` callsites and explicitly type each; especially
  auth login/register, Passkey establishment, CLI and web App Password creation.
- Schema migration must be additive/backward-safe, preserve existing rows as
  Unknown, and keep SQLite/PostgreSQL behavior aligned; inspect backup/restore
  and fixture implications.
- Ensure database write transaction commit—not merely statement
  success—determines renewal eligibility and the cookie expiry source; preserve
  revocation linearization and non-revival under simultaneous operations.
- Preserve ADR-0107 cookie-only establishment, ADR-0132 explicit-Authorization
  precedence/retirement, ADR-0194 Passkey Session semantics, and ADR-0016
  dependency-injection ownership.
- Inspect session schema types and migration conventions, storage transaction
  APIs, authentication middleware precedence, App Password and Passkey creation,
  backup/restore format compatibility, and the daily cleanup coordinator before
  choosing concrete files or migration identifiers. Keep SQLite/PostgreSQL
  behavior and rollback semantics aligned; no caller may infer kind.
- Keep cleanup bounded, domain-owned, PII/secret-free, independently
  failure-tolerant, and restricted to typed expired Browser Sessions.
- Tests use the established shared storage harness and clock seam; retain
  focused auth integration plus Playwright e2e for persistent cookie
  receipt/use. Follow CONTRIBUTING's verify ladder, and do not represent a local
  focused e2e as backend/browser CI parity.
- No implementation begins until this outline is approved; spec remains
  authoritative if outline detail conflicts.
