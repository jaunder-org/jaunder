# Issue #60 — `confirm_password_reset` validate-before-hash (ADR-0022): design

- Date: 2026-06-26
- Issue: [#60](https://github.com/jaunder-org/jaunder/issues/60) — `storage: confirm_password_reset hashes the new password before validating the reset token (ADR-0022)`
- Status: approved (design); awaiting spec review

## Problem

`AtomicOps::confirm_password_reset` — both `storage/src/sqlite/mod.rs:244` and
`storage/src/postgres/mod.rs:139` — computes the Argon2 hash of the **new password
before validating the reset token**:

```
token_hash = hash_token(raw_token)        -- cheap
password_hash = hash_password(new)        -- EXPENSIVE (Argon2), BEFORE any validation
BEGIN
claimed = UPDATE password_resets SET used_at WHERE token_hash = ? AND used_at IS NULL
          AND expires_at > now RETURNING user_id
  └─ zero rows → SELECT to disambiguate NotFound / AlreadyUsed / Expired
UPDATE users SET password_hash …
DELETE FROM sessions …
COMMIT
```

A flood of requests bearing bogus tokens forces an Argon2 hash per request — a
CPU-exhaustion amplifier. The reset token is a high-entropy secret (32 random bytes,
`auth::generate_token`), so per **ADR-0022** it must be validated *before* the
expensive hash. Unlike invite-gated registration (#51), password-reset confirmation
has **no capability gate** — anyone can POST a token — so it is strictly more exposed
(only request rate-limiting mitigates it).

## Decision: hash only after the token claim succeeds (both backends)

Move the `hash_password(new_password)` call from *before* `pool.begin()` to *after*
the claiming `UPDATE password_resets … RETURNING user_id` succeeds. Identical reorder
in both backends:

```
token_hash = hash_token(raw_token)
BEGIN
claimed = UPDATE password_resets … RETURNING user_id      -- validates + consumes the token
if None:
    SELECT … → NotFound / AlreadyUsed / Expired           -- ROLLBACK, return; NO hash
password_hash = hash_password(new)                         -- success path only (token valid)
UPDATE users SET password_hash WHERE user_id
DELETE FROM sessions WHERE user_id
COMMIT
```

A bogus / used / expired token yields zero rows from the claim → the existing `SELECT`
disambiguation returns the error **without hashing**. The hash runs only on the
success path. A hash failure there `?`-returns and drops the transaction → rollback →
the token claim reverts (token not consumed) — the same observable outcome as today.

## SQLite locking (ADR-0021)

No `BEGIN IMMEDIATE` is needed. The claim `UPDATE` is already the **first** statement
in the transaction (write-first), so there is no shared→reserved lock upgrade —
ADR-0021-safe as-is, and unchanged by this reorder. Moving the hash inside the
transaction does hold the SQLite write lock across the ~tens-of-ms hash on the
**success** path; password-reset success is rare, so the contention is negligible
(Option-A style, per the issue's remediation). The alternative (a cheap pre-read
validate, hash outside the lock, then re-claim inside) is not worth the added code and
TOCTOU re-check for this rare operation.

## Backend parity

Both backends get the identical reorder, preserving parity (ADR-0019). Postgres is
MVCC, so the in-transaction hash has no locking cost there. Error variants
(`NotFound`/`AlreadyUsed`/`Expired`/`Internal`) and observable behavior are unchanged.

## ADRs

The **fix** is governed by **ADR-0022** (on `main`) — its second deliberate application
after #51; no new ADR for the reorder itself. The **testing mechanism** does introduce a
decision worth recording: gating the fault-injection hook on `test-utils` instead of
`#[cfg(test)]` so the dual-backend integration harness can reach it (and so storage error
paths need not be single-store tests). That is **ADR-0026** (test-only fault-injection
hooks behind a `test-utils` feature), written this cycle; the README ADR table is updated
(and the omitted 0023–0025 rows backfilled while there).

## Testing

- **Behavior unchanged:** the existing dual-backend password-reset tests
  (`server/tests/storage/storage.rs`: `email_verification_and_password_reset_work`,
  `create_password_reset_and_use_returns_user_id`,
  `use_password_reset_already_used_returns_already_used`,
  `use_password_reset_expired_returns_expired`,
  `use_password_reset_unknown_token_returns_not_found`, and the web-layer
  `web_password_reset` tests) must stay green on both backends.
- **Dual-backend coverage, not SQLite-only.** `confirm_password_reset`'s
  success/expired/invalid/used-token paths are already dual-backend via
  `server/tests/web/web_password_reset.rs`. The storage-layer **hash-failure →
  `Internal`** path was covered only by a **SQLite-only in-file test**
  (`confirm_password_reset_hash_failure_returns_internal_error`, `sqlite/mod.rs:376`).
  Since this fix reorders **both** backends, that test must cover both. The setup needs
  no raw DB manipulation — `state.password_resets.create_password_reset(user_id,
  expires)` returns a raw token and `state.atomic.confirm_password_reset` runs on the
  configured backend. **One enabler is required:** the hash-failure injection in
  `storage::helpers::hash_password` is `#[cfg(test)]`-gated, so it compiles only in
  storage's *own* test build — the `server` integration tests (a normal dependency on
  storage) can't trigger it. Re-gate it on `#[cfg(any(test, feature = "test-utils"))]`;
  the `test-utils` feature already exists in `storage/Cargo.toml` and is already enabled
  by `server`'s `[dev-dependencies]`, so the integration/coverage build gets the hook
  while production (which enables neither `test` nor the feature) does not. With that,
  **remove the SQLite-only in-file test; add two dual-backend tests in
  `server/tests/storage/storage.rs`:**
  - `confirm_password_reset_hash_failure_returns_internal` — valid token (via
    `create_password_reset`) + a hash-failing new password → `Internal` (success-path
    hash failure; the failed hash rolls back the claim).
  - `confirm_password_reset_bogus_token_returns_not_found_without_hashing` — a bogus
    token + a hash-failing new password → **`NotFound`**, not `Internal`: the claim
    rejects the token before the hash is attempted (proves validate-before-hash; before
    this fix it would have hashed first and returned `Internal`).
- **No in-file `#[ignore]` tests** in the dialect files.
- **Gate:** `cargo xtask validate --no-e2e` per commit; full `cargo xtask validate`
  (sqlite + postgres e2e) at ship.

## Out of scope

- Any change to the SQL statements, error variants, or the token/claim logic.
- Request rate-limiting (an orthogonal, separately-trackable mitigation).
