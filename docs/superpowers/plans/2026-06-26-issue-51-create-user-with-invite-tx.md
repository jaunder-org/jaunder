# Issue #51 — `create_user_with_invite` SQLite transaction discipline: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the `SQLITE_BUSY`-on-upgrade failure mode in `SqliteAtomicOps::create_user_with_invite` by taking the write lock up front with `BEGIN IMMEDIATE`, with no change to observable behavior or backend parity.

**Architecture:** Replace the SQLite implementation's deferred sqlx `Transaction` with a raw pooled connection driven by manual `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK` (mirroring `storage/src/sqlite/backup.rs`). Validation stays before the Argon2 hash so a bogus invite is rejected without hashing (ADR-0022). Document the cost-ordering discipline as a new ADR and cross-reference it from ADR-0018. Postgres is unchanged (MVCC-safe).

**Tech Stack:** Rust, `sqlx` (SQLite + Postgres), `tokio`, `rstest`, `cargo xtask` gate.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-26-issue-51-create-user-with-invite-tx.md` (authoritative).
- **SQLite-only code change.** Do **not** touch `storage/src/postgres/mod.rs`. Parity is by behavior, not by line (ADR-0019).
- **Behavior-preserving.** The SQL statements, bind order, and the five result paths (`InviteNotFound`, `InviteAlreadyUsed`, `InviteExpired`, `UsernameTaken`, success) are unchanged. Every existing test must stay green without modification.
- **No `#[ignore]` tests inside instrumented dialect files** (`storage/src/sqlite/*.rs`): the SQLite coverage pass skips `#[ignore]` and the PG pass is `-p jaunder` only, so an ignored in-file test runs in neither and tanks per-file coverage. Add no in-file tests here.
- **Manual-transaction precedent:** `storage/src/sqlite/backup.rs:17-56` — raw `pool.acquire()` + `sqlx::query("BEGIN IMMEDIATE")`, body in an `async {}.await` block, then a `match` issuing `COMMIT` (with `?`) or `ROLLBACK` (best-effort `let _ =`). Follow it exactly.
- **Gate:** per-task `cargo xtask check --no-test` (clippy + fmt); pre-commit `cargo xtask validate --no-e2e`; full `cargo xtask validate` (sqlite + postgres e2e) before the PR. Invoke bare via context-mode (no trailing `;echo`/`|tee`); pass/fail is the exit code.
- **No commits without explicit user approval** (project rule). Each task below ends with a commit *step*, but hold the actual commit until the user approves at the per-task review gate.
- **ADR numbering:** the new ADR is **0022** (0021 lives only in the unmerged `b802c4f`; do not reuse it). The README ADR table will show a transient 0020 → 0022 gap until `b802c4f` lands — expected.
- **No Co-Authored-By trailers** in commits.

---

### Task 1: Reshape `create_user_with_invite` (SQLite) to `BEGIN IMMEDIATE`

**Files:**
- Modify: `storage/src/sqlite/mod.rs:149-211` (the `create_user_with_invite` body)
- Test (existing, unchanged — behavioral guard): `server/tests/storage/storage.rs:1242-1405`
  (`create_user_with_invite_creates_user_and_marks_invite_used`,
  `..._second_call_returns_already_used`, `..._expired_returns_invite_expired`,
  `..._unknown_code_returns_not_found`, `..._duplicate_username_returns_username_taken`)
- Test (existing, unchanged): `storage/src/sqlite/mod.rs:295` (`create_user_with_invite_hash_failure_returns_internal_error`)

**Interfaces:**
- Consumes: `SqlitePool::acquire`, `sqlx::query`/`query_as`/`query_scalar`, `crate::helpers::hash_password`, `RegisterWithInviteError` (already `From<sqlx::Error>`).
- Produces: unchanged public signature
  `async fn create_user_with_invite(&self, username: &Username, password: &Password, display_name: Option<&str>, is_operator: bool, invite_code: &str) -> Result<i64, RegisterWithInviteError>`.

This is a behavior-preserving refactor of well-tested code, so the cycle is **green baseline → refactor → still green** (the existing tests are the spec). Do not add a new in-file test (dialect-file coverage rule); do not weaken the existing tests.

- [ ] **Step 1: Establish the green baseline.** Confirm the behavioral guards pass against the current code before changing anything.

Run (bare, via context-mode): `cargo nextest run -p server --test main create_user_with_invite`
Expected: the five `create_user_with_invite_*` tests PASS. (If `-p server --test main` is not the harness target, fall back to `cargo nextest run -E 'test(create_user_with_invite)'`.)

- [ ] **Step 2: Replace the function body.** Edit `storage/src/sqlite/mod.rs` so `create_user_with_invite` reads exactly as below. The statements and binds are identical to the original; only the transaction control changes (deferred `tx` → raw connection + `BEGIN IMMEDIATE` + explicit `COMMIT`/`ROLLBACK`, with the body in an `async {}.await` block so every exit path settles the transaction).

```rust
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError> {
        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring sqlite/backup.rs.
        //
        // ADR-0022: the invite (a high-entropy secret) is validated *before* hashing, so
        // a bogus code is rejected without paying the Argon2 cost. The hash therefore runs
        // inside the immediate transaction on the success path only.
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<i64, RegisterWithInviteError> = async {
            let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
                "SELECT used_at, expires_at FROM invites WHERE code = $1",
            )
            .bind(invite_code)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(RegisterWithInviteError::InviteNotFound)?;

            let (used_at, expires_at) = row;
            if used_at.is_some() {
                return Err(RegisterWithInviteError::InviteAlreadyUsed);
            }

            let now = Utc::now();
            if expires_at <= now {
                return Err(RegisterWithInviteError::InviteExpired);
            }

            let password_hash = crate::helpers::hash_password(password.clone())
                .await
                .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?;

            let insert = sqlx::query_scalar::<_, i64>(
                "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
                 VALUES ($1, $2, $3, $4, $5)
                 RETURNING user_id",
            )
            .bind(username.as_str())
            .bind(&password_hash)
            .bind(display_name)
            .bind(now)
            .bind(is_operator)
            .fetch_one(&mut *conn)
            .await;

            let user_id = match insert {
                Ok(id) => id,
                Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                    return Err(RegisterWithInviteError::UsernameTaken);
                }
                Err(error) => return Err(RegisterWithInviteError::Internal(error)),
            };

            sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
                .bind(now)
                .bind(user_id)
                .bind(invite_code)
                .execute(&mut *conn)
                .await?;

            Ok(user_id)
        }
        .await;

        match result {
            Ok(user_id) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(user_id)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }
```

- [ ] **Step 3: Confirm the guards still pass.** Re-run the same tests; they must pass unchanged. The `..._duplicate_username_returns_username_taken` test (invite left unused) and `..._second_call_returns_already_used`/`..._expired_...`/`..._unknown_code_...` (no user created) prove the explicit `ROLLBACK` is correct; the happy-path test proves `COMMIT` works.

Run: `cargo nextest run -p server --test main create_user_with_invite`
Expected: all five `create_user_with_invite_*` tests PASS.

- [ ] **Step 4: Confirm the in-crate hash-failure test still passes** (covers the `Internal` → `ROLLBACK` path).

Run: `cargo nextest run -p storage create_user_with_invite_hash_failure_returns_internal_error`
Expected: PASS.

- [ ] **Step 5: Static gate.**

Run: `cargo xtask check --no-test`
Expected: exit 0 (clippy clean, fmt clean). Inspect detail via `jq '.steps' .xtask/last-result.json` if it goes red.

- [ ] **Step 6: Commit** (hold for user approval at the review gate).

```bash
git add storage/src/sqlite/mod.rs
git commit -m "fix(storage/sqlite): take write lock up front in create_user_with_invite (#51)

Replace the deferred read-then-write transaction with BEGIN IMMEDIATE on a raw
connection (manual COMMIT/ROLLBACK), eliminating the SQLITE_BUSY-on-upgrade
failure mode per ADR-0021. Behavior unchanged; validation still precedes hashing
(ADR-0022). Postgres untouched."
```

---

### Task 2: Add ADR-0022 and cross-reference it from ADR-0018

**Files:**
- Create: `docs/adr/0022-validate-before-expensive-work.md`
- Modify: `docs/adr/0018-constant-time-authentication.md` (add `Amended:` line + a scope-boundary subsection; decision/invariant unchanged)
- Modify: `docs/README.md:48` (add the ADR-0022 row to the ADR table)

**Interfaces:** Documentation only; no code. House style (per `docs/adr/0021` and the #18 plan): `# NNNN. Title`, then `- Status:` / `- Date:` / `- Deciders:`, then `## Context` / `## Decision` / `## Consequences`.

- [ ] **Step 1: Write ADR-0022.** Create `docs/adr/0022-validate-before-expensive-work.md` with exactly:

```markdown
# 0022. Validate cheaply before expensive work when the gate is a high-entropy secret

- Status: accepted
- Date: 2026-06-26
- Deciders: Michael Alan Dorman

## Context

Several storage operations gate an expensive computation behind a credential
check. The expensive step is almost always an Argon2id password hash (tens of
milliseconds, deliberately). Two different gates call for opposite orderings,
and conflating them creates either a username-enumeration oracle or a
denial-of-service amplifier.

- `UserStorage::authenticate` gates on a **username** — a low-entropy,
  user-chosen, enumerable identifier. Returning early when the username is
  absent leaks account existence through response timing. ADR-0018 resolves this
  by performing an equalizing dummy Argon2 verification on the absent path.

- `create_user_with_invite` and `confirm_password_reset` gate on a **high-entropy
  secret** — an invite code or password-reset token, each 32 cryptographically
  random bytes (`auth::generate_token`, ~256 bits). The enumeration concern does
  not apply: a timing oracle gives no usable advantage against a 2^256 space. But
  hashing *before* validating the secret turns every request bearing a bogus
  secret into wasted Argon2 work — a CPU-exhaustion amplifier — and, for
  invite-gated registration, destroys the one capability-based throttle available
  (stop issuing invites ⇒ the hashing surface drops to zero).

## Decision

When the value being validated is a **high-entropy secret** (not an enumerable
identifier), validate it with a cheap lookup **before** performing expensive work
(Argon2 hashing, large allocations, etc.), and reject invalid secrets without
paying that cost.

This is the deliberate complement to ADR-0018, not a contradiction of it. The
dividing line is the **entropy of the thing being validated**:

- **Enumerable identifier** (username, email): equalize timing — do the work
  anyway (ADR-0018).
- **High-entropy secret** (invite code, session/reset token): cheap-reject first
  — skip the work on invalid input (this ADR).

## Consequences

- `create_user_with_invite` validates the invite (a `SELECT`) before hashing; the
  SQLite implementation additionally takes its write lock up front per ADR-0021,
  so the hash runs inside the immediate transaction on the success path only
  (issue #51).
- `confirm_password_reset` currently hashes the new password *before* validating
  the reset token — it violates this ADR and is tracked as a follow-up issue.
- ADR-0018 carries a scope-boundary cross-reference to this ADR so the boundary
  is discoverable from both sides; its decision and durable invariant are
  unchanged.
- This is about *cost ordering*, not correctness: both orderings produce the same
  result for valid input, and no timing guarantee for enumerable identifiers is
  weakened — those remain governed by ADR-0018.
- Relates to ADR-0007 (auth mechanisms) and ADR-0018 (timing-equalized auth).
```

- [ ] **Step 2: Amend ADR-0018.** In `docs/adr/0018-constant-time-authentication.md`, add an `Amended:` line immediately after the `* Date: 2026-06-13` line:

```markdown
* Amended: 2026-06-26 — added the scope boundary vs. ADR-0022 (see *Scope boundary* below); the original decision and durable invariant are unchanged.
```

Then add this subsection immediately after the `### Durable invariant` block (before `### Alternatives considered`):

```markdown
### Scope boundary

This ADR governs validating an **enumerable identifier** (username/email), where
response timing must be equalized so it cannot reveal account existence.
Validating a **high-entropy secret** (invite code, password-reset token) is the
opposite case and is governed by **ADR-0022**: there, a cheap rejection of an
invalid secret *before* the Argon2 work is both safe (no useful timing oracle
exists against a ~256-bit space) and preferred (it bounds DoS amplification and
preserves capability-issuance as a throttle). Do not apply this ADR's
equalizing-dummy-hash rule to high-entropy-secret paths.
```

- [ ] **Step 3: Add the ADR-0022 row to the README table.** In `docs/README.md`, after the `| [0020]... |` row (line 48), add:

```markdown
| [0022](adr/0022-validate-before-expensive-work.md) | Validate Cheaply Before Expensive Work for High-Entropy Secrets | accepted |
```

(No 0021 row — that belongs to `b802c4f`. The transient gap is expected and reconciled when that branch lands.)

- [ ] **Step 4: Sanity-check the docs.** Confirm the three files are well-formed and internally consistent.

Run: `rg -n 'ADR-0022|0022-validate-before-expensive-work|Scope boundary|Amended: 2026-06-26' docs/adr/0018-constant-time-authentication.md docs/adr/0022-validate-before-expensive-work.md docs/README.md`
Expected: matches in all three files (README row, ADR-0022 title/body, ADR-0018 amendment + scope-boundary subsection).

- [ ] **Step 5: Commit** (hold for user approval).

```bash
git add docs/adr/0022-validate-before-expensive-work.md docs/adr/0018-constant-time-authentication.md docs/README.md
git commit -m "docs(adr): add ADR-0022 validate-before-expensive-work; cross-ref from ADR-0018 (#51)"
```

---

### Task 3: Full gate, then file the `confirm_password_reset` follow-up issue

**Files:** none (verification + GitHub bookkeeping).

- [ ] **Step 1: Run the full pre-PR gate.**

Run: `cargo xtask validate --no-e2e`
Expected: exit 0 (static + clippy + coverage). Read `jq '.steps' .xtask/last-result.json` on failure. (The full `cargo xtask validate` with sqlite+postgres e2e runs at the ship gate.)

- [ ] **Step 2: File the follow-up issue** via the `jaunder-issues` skill (assign to the **Robustness** project). Content:

  - **Title:** `storage: confirm_password_reset hashes the new password before validating the reset token (ADR-0022)`
  - **Body:** `confirm_password_reset` (`storage/src/sqlite/mod.rs:213` and `storage/src/postgres/mod.rs:139`) computes the Argon2 hash of the new password *before* validating the reset token, so a flood of bogus-token requests forces expensive hashing — a CPU-exhaustion amplifier. Per **ADR-0022**, a high-entropy secret (the reset token) must be validated *before* the expensive work. Unlike invite-gated registration, password-reset confirmation has **no capability gate**, so it is strictly more exposed (only request rate-limiting mitigates it). Remediation: validate/claim the token first (the `UPDATE … RETURNING` claim already exists), and hash the new password only on the success path. Preserve backend parity and the existing error variants (`NotFound`/`AlreadyUsed`/`Expired`). Reference: ADR-0022, ADR-0018 (scope boundary), issue #51.
  - **Labels:** `data-integrity` (match #51).

- [ ] **Step 3:** Record the new issue number; it becomes a sibling of #52/#53 under the Robustness project and is handed back to `jaunder-develop`.

---

## Self-Review

**Spec coverage:**
- Option-A reshape (`BEGIN IMMEDIATE`, validate-first, explicit COMMIT/ROLLBACK) → Task 1. ✓
- ADR-0022 → Task 2 Step 1. ✓
- Amend ADR-0018 (immutable decision + cross-reference) → Task 2 Step 2. ✓
- README ADR-0022 row → Task 2 Step 3. ✓
- Tests (existing guards stay green; no new in-file tests) → Task 1 Steps 1,3,4. ✓
- File `confirm_password_reset` follow-up issue → Robustness → Task 3 Step 2. ✓
- Backend parity invariant (ADR-0019): SQLite-only change, Postgres untouched ⇒ parity preserved by construction. `create_user_with_invite` runs on both backends via `invite_and_atomic_registration_work` (`storage.rs:719`, `#[apply(backends)]`) for happy + `InviteAlreadyUsed`; the SQLite-only detailed-path tests and the broader Postgres coverage gap are out of scope (tracked as #54). → Global Constraints + Task 1. ✓
- Sequencing note (ADR 0022, gap at 0021) → Global Constraints. ✓

**Placeholder scan:** none — full ADR text, full function body, and full issue text are inline.

**Type consistency:** the function signature and `RegisterWithInviteError` variants (`InviteNotFound`/`InviteAlreadyUsed`/`InviteExpired`/`UsernameTaken`/`Internal`) match `storage/src/atomic.rs:14-23` and the existing Postgres impl; the `async {}.await` block is annotated `Result<i64, RegisterWithInviteError>` to fix inference; `&mut *conn` matches the `backup.rs` precedent.
