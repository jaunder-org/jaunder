# Issue #60 — `confirm_password_reset` validate-before-hash: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `confirm_password_reset` hash the new password only *after* the reset token is validated/claimed, in both backends, per ADR-0022.

**Architecture:** Move the `hash_password` call from before `pool.begin()` to after the claiming `UPDATE password_resets … RETURNING user_id` succeeds. Identical reorder in SQLite and Postgres. No SQL, no locking, no error-variant changes.

**Tech Stack:** Rust, `sqlx` (SQLite + Postgres), `tokio`, `rstest`, `cargo xtask` gate.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-26-issue-60-confirm-reset-hash-order.md` (authoritative).
- **Both backends, identical reorder.** `storage/src/sqlite/mod.rs` *and* `storage/src/postgres/mod.rs`. Parity by behavior (ADR-0019).
- **No `BEGIN IMMEDIATE`** — the claim `UPDATE` is already write-first (ADR-0021-safe); the SQLite transaction stays a deferred `pool.begin()`.
- **The fix is governed by ADR-0022** (on `main`); the *testing mechanism* adds **ADR-0026** (fault-injection hooks behind the `test-utils` feature) + the README ADR-table rows (0023–0026).
- **No SQL / error-variant changes.** `NotFound`/`AlreadyUsed`/`Expired`/`Internal` preserved.
- **No `#[ignore]` tests in dialect files;** the in-file SQLite tests are plain `#[tokio::test]`.
- **Gate:** per-commit `cargo xtask validate --no-e2e`; full `cargo xtask validate` (sqlite + postgres e2e) at ship. The reorder adds no branches, so **no CRAP bump is expected**. Bare via context-mode with `cd <worktree> &&`.
- **Local test runs:** dual-backend `case_2_postgres` cases fail locally with `Connection refused` (no local PG); filter to sqlite (`& test(sqlite)`). The Nix gate exercises PG.
- **No commits without explicit user approval; no Co-Authored-By trailers.**

---

### Task 1: Reorder `confirm_password_reset` (both backends) + fix/add the in-file tests

**Files:**
- Modify: `storage/src/sqlite/mod.rs` (`confirm_password_reset` reorder, ~244-300; **remove** the SQLite-only in-file test `confirm_password_reset_hash_failure_returns_internal_error`, ~376-406)
- Modify: `storage/src/postgres/mod.rs` (`confirm_password_reset` reorder, ~139-196)
- Modify: `storage/src/helpers.rs` — re-gate the `hash_password` fault-injection hook on `#[cfg(any(test, feature = "test-utils"))]` (the enabler for dual-backend tests)
- Modify: `server/tests/storage/storage.rs` — add two dual-backend (`#[apply(backends)]`) tests after `email_verification_and_password_reset_work` (ends ~line 818); add `ConfirmPasswordResetError` to the `use storage::{…}` group
- Create: `docs/adr/0026-test-fault-injection-hooks-feature.md`; Modify: `docs/README.md` (add ADR rows 0023–0026)
- Note: trim the now-unused `ConfirmPasswordResetError`/`UserStorage` imports from `storage/src/sqlite/mod.rs`'s test module
- Test (existing, unchanged — behavioral guard): the dual-backend `email_verification_and_password_reset_work` (`storage.rs:757`, exercises `confirm_password_reset` success on both backends) and `server/tests/web/web_password_reset.rs` (`#[apply(backends)]`: success/expired/invalid/used token)

**Interfaces:** unchanged trait method `async fn confirm_password_reset(&self, raw_token: &str, new_password: &Password) -> Result<(), ConfirmPasswordResetError>` in both backends.

- [x] **Step 1: Green baseline (sqlite).**

Run: `cargo nextest run -E '(test(password_reset) | test(email_verification_and_password_reset_work)) & test(sqlite)'`
Expected: PASS (sqlite cases), including `confirm_password_reset_hash_failure_returns_internal_error`.

- [x] **Step 2: Reorder the SQLite impl.** In `storage/src/sqlite/mod.rs::confirm_password_reset`, (a) delete the pre-transaction hash block:

```rust
        let password_hash = crate::helpers::hash_password(new_password.clone())
            .await
            .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

        let mut tx = self.pool.begin().await?;
```

→ becomes:

```rust
        let mut tx = self.pool.begin().await?;
```

and (b) insert the hash after the claim's `else { … };` block, immediately before the `UPDATE users` query:

```rust
        };

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
```

→ becomes:

```rust
        };

        // ADR-0022: hash only after the token claim succeeds, so a bogus/used/expired
        // token is rejected above without paying the Argon2 cost.
        let password_hash = crate::helpers::hash_password(new_password.clone())
            .await
            .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
```

- [x] **Step 3: Reorder the Postgres impl.** Apply the **identical** two edits to `storage/src/postgres/mod.rs::confirm_password_reset` (delete the pre-`begin()` hash block; insert the same hash-with-comment after the claim's `else { … };` block, before `UPDATE users`). The surrounding code is line-for-line the same as SQLite.

- [x] **Step 4: Remove the SQLite-only in-file hash-failure test.** Delete `confirm_password_reset_hash_failure_returns_internal_error` (`storage/src/sqlite/mod.rs`, ~376-406). It is replaced by the dual-backend tests in Step 5 (which cover the SQLite *and* Postgres reorder). Its setup also relied on hash-before-claim — a non-matching token that only returned `Internal` because the hash ran first — so it would assert the wrong variant post-reorder anyway.

- [x] **Step 5: Add two dual-backend ordering tests.** In `server/tests/storage/storage.rs`, after `email_verification_and_password_reset_work` (ends ~line 818), add (uses `state` + storage-trait methods only — no raw SQL):

```rust
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_hash_failure_returns_internal(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("reset_hash_fail"), &password("password123"), None, false)
        .await
        .unwrap();
    let reset_token = state
        .password_resets
        .create_password_reset(user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    // Valid token → the claim succeeds, then hashing the new password fails → Internal
    // (success-path hash failure; the failed hash rolls the claim back).
    let result = state
        .atomic
        .confirm_password_reset(&reset_token, &password("force-hash-error-for-test-coverage"))
        .await;
    assert!(matches!(result, Err(ConfirmPasswordResetError::Internal(_))));
}

#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_bogus_token_returns_not_found_without_hashing(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    // No password_resets row matches this token. A hash-failing new password proves the
    // hash is NOT attempted: the claim rejects the token first → NotFound, not Internal
    // (ADR-0022). Before the reorder this would have hashed first and returned Internal.
    let result = state
        .atomic
        .confirm_password_reset("dGVzdA", &password("force-hash-error-for-test-coverage"))
        .await;
    assert!(matches!(result, Err(ConfirmPasswordResetError::NotFound)));
}
```

If `ConfirmPasswordResetError` is not already in the test module's `use storage::{…}` group, add it. (`username`, `password`, `Backend` are already imported and used by neighboring tests; `password("force-hash-error-for-test-coverage")` parses the magic hash-failing string via the same helper the in-file test used.)

- [x] **Step 6: Run the updated tests (sqlite cases).**

Run: `cargo nextest run -E 'test(confirm_password_reset) & test(sqlite)'`
Expected: PASS — `confirm_password_reset_hash_failure_returns_internal::case_1_sqlite` (`Internal`) and `confirm_password_reset_bogus_token_returns_not_found_without_hashing::case_1_sqlite` (`NotFound`). (Run `email_verification_and_password_reset_work & test(sqlite)` too to confirm the success path still passes.)

- [x] **Step 7: Static gate.**

Run: `cargo xtask check --no-test`
Expected: exit 0 (clippy + fmt clean).

- [x] **Step 8: Coverage gate.**

Run: `cargo xtask validate --no-e2e`
Expected: exit 0 — clean, **0 new-uncovered, 0 CRAP regressions** (the reorder relocates a line without adding branches; the two in-file tests keep the success-path hash and the bogus-token paths covered). If a CRAP regression or new-uncovered nonetheless appears, accept the baseline / add a focused test as in #51.

- [x] **Step 9: Commit** (hold for user approval).

```bash
git add storage/src/sqlite/mod.rs storage/src/postgres/mod.rs
git commit -m "fix(storage): validate the reset token before hashing in confirm_password_reset (#60)

Move the Argon2 hash of the new password from before the transaction to after the
claiming UPDATE password_resets ... RETURNING succeeds, in both backends. A bogus/
used/expired token is now rejected without paying the hash cost (ADR-0022); the
token is a high-entropy secret with no capability gate, so this closes a CPU-
exhaustion amplifier. No SQL, locking, or error-variant changes. Re-gate
hash_password's fault-injection hook on the test-utils feature (ADR-0026) so the
integration harness can reach it, and replace the SQLite-only in-file hash-failure
test with two dual-backend tests (hash-failure -> Internal; bogus-token -> NotFound,
proving validate-before-hash) covering both backends."
```

Stage `storage/src/{sqlite,postgres}/mod.rs`, `storage/src/helpers.rs`, and
`server/tests/storage/storage.rs` for the fix commit (plus `crap-manifest.json` if a
heal changed it). **ADR-0026 + the `docs/README.md` rows go in a separate
`docs(adr): …` commit**, mirroring #51.

---

### Task 2: Ship gate

- [x] **Step 1: Full pre-PR gate.**

Run: `cargo xtask validate --no-e2e` (full `cargo xtask validate` with e2e runs at `jaunder-ship`).
Expected: exit 0.

No follow-up issues or ADRs emerged from this cycle.

---

## Self-Review

**Spec coverage:**
- Both-backends reorder (hash after claim) → Task 1 Steps 2-3. ✓
- No BEGIN IMMEDIATE / no new ADR → Global Constraints. ✓
- SQLite-only in-file hash-failure test removed; converted to dual-backend → Task 1 Step 4. ✓
- Two dual-backend tests (hash-failure → Internal; bogus-token → NotFound ordering) → Task 1 Step 5. ✓
- Behavior/parity preserved; both backends' reorder covered (storage + web dual-backend guards) → Global Constraints + Task 1 Steps 1,6. ✓

**Placeholder scan:** none — full before/after edits, full test code inline.

**Type consistency:** `ConfirmPasswordResetError` variants (`NotFound`/`AlreadyUsed`/`Expired`/`Internal`), `crate::helpers::hash_password`, the test helpers `username`/`password`/`Backend` and `state.users`/`state.password_resets.create_password_reset`/`state.atomic.confirm_password_reset` match `storage/src/{sqlite,postgres}/mod.rs` and the patterns in `email_verification_and_password_reset_work` (`storage.rs:757`).
