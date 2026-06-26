# Issue #18 — SQLite transaction lock flake: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the e2e `database is locked` flake by removing SQLite read-then-write deferred transactions, document the anti-pattern as ADR-0021, and audit + triage every remaining explicit-transaction SQLite path.

**Architecture:** SQLite WAL deferred transactions that `SELECT` then `UPDATE` must upgrade a shared lock to a reserved lock against a possibly-stale snapshot → `SQLITE_BUSY`-on-upgrade that `busy_timeout` cannot rescue. The fix is to express each claim as a single autocommit statement (`UPDATE … WHERE … RETURNING`), taking the write lock immediately. This is already the established convention for `confirm_password_reset`, `use_email_verification`, and `touch_and_load`; this work brings the outliers into line and records the rule.

**Tech Stack:** Rust, sqlx (SQLite + Postgres), async-trait, tokio, tempfile, rstest; xtask verify ladder; Nix-VM e2e.

## Global Constraints

- **Backend parity (CONTRIBUTING):** SQLite and Postgres must remain behaviorally identical. Any shared (backend-agnostic) change must keep both backends' tests green.
- **Portable SQL convention:** shared (`storage/src/*.rs`) files use `$N` placeholders and bind duplicate values separately (e.g. `now` bound twice as `$1` and `$3`), not numbered-param reuse. SQLite ≥ 3.35 and Postgres both support `UPDATE … RETURNING`.
- **ADR format:** house style is `# NNNN. Title`, then `- Status:` / `- Date:` / `- Deciders:`, then `## Context` / `## Decision` / `## Consequences`. One decision per file. Next free number is **0021**.
- **No `#[ignore]` tests inside instrumented dialect files** (`storage/src/sqlite/*.rs`): the SQLite coverage pass skips `#[ignore]` and the PG pass is `-p jaunder` only, so an in-file ignored test's instrumented-but-unexecuted lines tank that file's per-file coverage. Concurrency repro test goes in `server/tests/feed/`.
- **Commits:** no `Co-Authored-By` trailers. One clean, verified commit per task. Do not commit without the user's approval at the cycle's halt points; per-task commits during iterate are authorized once execution begins.
- **Per-task gate:** `cargo xtask check --no-test` (clippy + fmt) plus the task's targeted tests. **Final gate:** `cargo xtask validate` (full CI-faithful, incl. e2e sqlite+postgres). Run all gate commands from the worktree root.

---

## Audit findings (Phase 2 grounding)

Pre-classified during planning (verified against source). The vulnerable pattern is a deferred `pool.begin()` transaction that **reads then writes**; write-first / write-only / read-only transactions are safe.

| Site | Sequence | Classification | Disposition |
|---|---|---|---|
| `sqlite/feed_events.rs:33` `claim_pending_batch` | SELECT → UPDATE → SELECT | VULNERABLE | **Task 1** (fix here) |
| `invites.rs:104` `use_invite` (shared) | SELECT → UPDATE | VULNERABLE / TRIVIAL | **Task 3** (fix here) |
| `sqlite/mod.rs:157` `create_user_with_invite` | SELECT(invite) → INSERT(user) RETURNING → UPDATE(invite) | VULNERABLE / SUBSTANTIAL | **Task 4** (file issue) |
| `sqlite/posts.rs:31` `update_post` | SELECT → INSERT(revision) → UPDATE(post) → replace_audiences | VULNERABLE / SUBSTANTIAL | **Task 4** (file issue) |
| `sqlite/posts.rs:106` `tag_post` | SELECT → INSERT tag → SELECT tag_id → INSERT join | VULNERABLE / SUBSTANTIAL | **Task 4** (file issue) |
| `sqlite/mod.rs:225` `confirm_password_reset` | UPDATE RETURNING (write-first) | SAFE | no action |
| `sqlite/sessions.rs:19` `touch_and_load` | UPDATE → SELECT (write-first) | SAFE | no action (ADR-0019 PoC; already compliant) |
| `posts.rs:538` `create_post` (shared) | INSERT RETURNING → replace_audiences (write-first) | SAFE | no action |
| `audiences.rs:181` `delete_audience` (shared) | DELETE → DELETE (write-only) | SAFE | no action |
| `email.rs:98` `create_email_verification` (shared) | UPDATE → INSERT (write-first) | SAFE | no action |

All four backend-agnostic sites are generic over the DB/pool and execute on SQLite.

---

## Task 1: Collapse `claim_pending_batch` + concurrency reproduction test

**Files:**
- Modify: `storage/src/sqlite/feed_events.rs:27-94` (the `claim_pending_batch` impl)
- Create: `server/tests/feed/feed_events_concurrency.rs`
- Modify: `server/tests/feed/main.rs` (register the new module)

**Interfaces:**
- Consumes: `Backend::Sqlite.setup() -> TestEnv { state, base }` (`server/tests/helpers/mod.rs:128`); `state.feed_events: Arc<dyn FeedEventStorage>`; `FeedEventStorage::enqueue(&self, &str) -> Result<i64, FeedEventError>` and `claim_pending_batch(&self, limit: i64, lease: chrono::Duration) -> Result<Vec<FeedEventRecord>, FeedEventError>`.
- Produces: no new public API; behavior of `claim_pending_batch` is unchanged.

- [ ] **Step 1: Write the failing concurrency reproduction test**

Create `server/tests/feed/feed_events_concurrency.rs`:

```rust
//! Reproduction harness for issue #18: the SQLite `claim_pending_batch` lock
//! flake. With the old SELECT→UPDATE→SELECT deferred transaction, concurrent
//! claimers upgrade a shared lock to a reserved lock against a stale snapshot
//! and SQLite returns `database is locked` (busy_timeout cannot rescue an
//! upgrade). With the single-statement `UPDATE … RETURNING` (ADR-0021) the
//! writes serialize cleanly under busy_timeout.
//!
//! Timing-based, so it is `#[ignore]`d — excluded from CI to avoid being a
//! flake source itself. Run on demand:
//!   cargo test -p jaunder --test feed -- --ignored claim_pending_batch_no_lock_contention
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use crate::helpers::Backend;
use chrono::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "timing-based #18 reproduction; run manually with --ignored"]
async fn claim_pending_batch_no_lock_contention() {
    let env = Backend::Sqlite.setup().await;
    let feed_events = env.state.feed_events.clone();

    // Seed a populated queue.
    for i in 0..200 {
        feed_events
            .enqueue(&format!("/feed-{i}.rss"))
            .await
            .expect("enqueue");
    }

    // Many concurrent claimers re-contending the same rows (zero lease keeps
    // every row claimable each pass → maximal UPDATE-upgrade contention).
    let mut handles = Vec::new();
    for _ in 0..16 {
        let fe = Arc::clone(&feed_events);
        handles.push(tokio::spawn(async move {
            for _ in 0..50 {
                fe.claim_pending_batch(200, Duration::zero()).await?;
            }
            Ok::<(), storage::FeedEventError>(())
        }));
    }

    for h in handles {
        h.await.expect("task panicked").expect("no database-is-locked error");
    }
}
```

Register it in `server/tests/feed/main.rs` (add alongside the existing `mod` lines):

```rust
mod feed_events_concurrency;
```

- [ ] **Step 2: Run the test against current code — verify it FAILS**

Run: `cargo test -p jaunder --test feed -- --ignored claim_pending_batch_no_lock_contention`
Expected: FAIL — a task returns `Err(FeedEventError…)` wrapping SQLite `database is locked` (the `.expect("no database-is-locked error")` panics). If it does not reliably fail, raise the task/iteration counts until it does, confirming the bug is reproduced before fixing.

- [ ] **Step 3: Collapse `claim_pending_batch` to a single statement**

Replace the whole method body in `storage/src/sqlite/feed_events.rs` (lines 27-94) with:

```rust
    async fn claim_pending_batch(
        pool: &Pool<Sqlite>,
        now: DateTime<Utc>,
        lease_cutoff: DateTime<Utc>,
        limit_i: i64,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        // Single autocommit statement: SQLite takes the write lock immediately,
        // so there is no deferred read-then-write lock upgrade (ADR-0021) and the
        // 5s busy_timeout applies cleanly. Mirrors the Postgres CTE claim.
        let rows = sqlx::query(
            "UPDATE feed_events SET status = 'claimed', claimed_at = $1 \
             WHERE id IN ( \
                 SELECT id FROM feed_events \
                 WHERE (status = 'pending' AND next_attempt_at <= $2) \
                    OR (status = 'claimed' AND claimed_at < $3) \
                 ORDER BY next_attempt_at ASC \
                 LIMIT $4 \
             ) \
             RETURNING id, feed_url, status, attempts, last_error, next_attempt_at, claimed_at, \
                       created_at, regenerated_at, pinged_at",
        )
        .bind(now)
        .bind(now)
        .bind(lease_cutoff)
        .bind(limit_i)
        .fetch_all(pool)
        .await?;

        let records = rows
            .into_iter()
            .map(|r| {
                let attempts: i64 = r.get("attempts");
                FeedEventRecord {
                    id: r.get("id"),
                    feed_url: r.get("feed_url"),
                    status: parse_status(r.get::<&str, _>("status")),
                    attempts: i32::try_from(attempts).unwrap_or(i32::MAX),
                    last_error: r.get("last_error"),
                    next_attempt_at: r.get("next_attempt_at"),
                    claimed_at: r.get("claimed_at"),
                    created_at: r.get("created_at"),
                    regenerated_at: r.get("regenerated_at"),
                    pinged_at: r.get("pinged_at"),
                }
            })
            .collect();
        Ok(records)
    }
```

Notes: `now` is bound twice (`$1` for the SET and `$2` for the eligibility predicate), matching the `$N`-with-duplicate-binds convention. The `tx`, the empty-`ids` early return, and the `placeholders()` call in this method are gone; the `placeholders()` helper stays (still used by `mark_*`). `use sqlx::{Pool, Row, Sqlite};` already imports `Row`.

- [ ] **Step 4: Run the reproduction test — verify it PASSES**

Run: `cargo test -p jaunder --test feed -- --ignored claim_pending_batch_no_lock_contention`
Expected: PASS (no task returns a lock error).

- [ ] **Step 5: Run the existing `claim_pending_batch` unit tests — verify parity**

Run: `cargo test -p storage --lib sqlite::feed_events`
Expected: PASS — `claim_returns_eligible_pending_row`, `double_claim_returns_no_rows_within_lease`, `lease_expired_rows_are_reclaimable`, `mark_pinged_marks_done_and_removes_from_queue`, `mark_failed_increments_attempts_and_reschedules`, `mark_exhausted_marks_failed_terminal`, `empty_id_arrays_are_noops`, `enqueue_creates_pending_row`, `parse_status_handles_all_statuses` all green.

- [ ] **Step 6: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: clippy + fmt clean (exit 0).

- [ ] **Step 7: Commit**

```bash
git add storage/src/sqlite/feed_events.rs server/tests/feed/feed_events_concurrency.rs server/tests/feed/main.rs
git commit -m "fix(storage): collapse SQLite claim_pending_batch to a single UPDATE...RETURNING (#18)"
```

---

## Task 2: ADR-0021 — SQLite dialect transaction discipline

**Files:**
- Create: `docs/adr/0021-sqlite-transaction-discipline.md`

**Interfaces:** none (documentation). Referenced by Task 4's issue bodies and the PR.

- [ ] **Step 1: Write the ADR**

Create `docs/adr/0021-sqlite-transaction-discipline.md`:

```markdown
# 0021. SQLite dialect transaction discipline: avoid read-then-write deferred transactions

- Status: accepted
- Date: 2026-06-26
- Deciders: Michael Alan Dorman

## Context

The Nix-VM e2e suite intermittently failed with SQLite `database is locked`
(issue #18). The cause was the *shape* of `claim_pending_batch` in the SQLite
feed-event dialect: a deferred transaction that did `SELECT … ; UPDATE … ;
SELECT …`. sqlx's `pool.begin()` issues a bare `BEGIN`, which in SQLite is
*deferred* — it acquires no lock. The opening `SELECT` takes a shared (read)
lock; the `UPDATE` must then upgrade it to a reserved (write) lock. In WAL mode,
if another connection commits a write between the `SELECT` and the `UPDATE`, the
upgrade is against a stale snapshot and SQLite returns `SQLITE_BUSY` — and the
`busy_timeout` cannot rescue it, because retrying does not refresh the
transaction's snapshot. Under concurrent load (the always-on feed worker plus
live request handlers writing the single shared db) this surfaced as the flake.

Postgres is immune: MVCC plus `FOR UPDATE SKIP LOCKED` lets it express the same
claim as one atomic `UPDATE … RETURNING` statement.

## Decision

In SQLite dialect code, prefer a **single autocommit statement** over a
read-then-write deferred transaction:

- Express "read to validate, then write" as one `UPDATE/INSERT/DELETE … WHERE
  <validation> RETURNING …` (or a write driven by a subquery). The write takes
  the reserved lock immediately; there is no shared→reserved upgrade, and
  `busy_timeout` applies cleanly to the one statement. A failure-path read (zero
  rows affected → `SELECT` to disambiguate the error) is fine: it runs outside
  any write and holds only a read lock.
- When multiple interdependent statements are genuinely required, take the write
  lock up front with `BEGIN IMMEDIATE` rather than a deferred `BEGIN`.
- Read-only transactions and write-first / write-only transactions are
  unaffected — they never perform a shared→reserved upgrade.

This codifies an already-established convention: `confirm_password_reset`,
`use_email_verification`, and the session `touch_and_load` (whose comment notes
it was restructured update-then-select specifically to avoid `SQLITE_BUSY`)
already follow it.

## Consequences

- `claim_pending_batch` and `use_invite` are collapsed to single-statement
  claims (issue #18). The classification rule here is what their audit used.
- An audit of every explicit-transaction SQLite path against this rule found
  three remaining read-then-write sites too substantial for a mechanical
  collapse (`create_user_with_invite`, `update_post`, `tag_post`); they are
  tracked as follow-up issues rather than reshaped here.
- Builds on ADR-0001 (storage backends) and ADR-0019 (generic store + per-trait
  dialect): this is transaction *discipline within* a SQLite dialect, distinct
  from ADR-0019's structural mechanism.
```

- [ ] **Step 2: Per-task gate (docs only)**

Run: `cargo xtask check --no-test`
Expected: clean (exit 0); confirms no link/format check trips on the new doc.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0021-sqlite-transaction-discipline.md
git commit -m "docs(adr): add ADR-0021 SQLite transaction discipline (#18)"
```

---

## Task 3: Collapse `use_invite` (trivial audit fix)

**Files:**
- Modify: `storage/src/invites.rs:101-140` (the `use_invite` impl)
- Test: existing invite tests are the parity guard (locate with the command in Step 1).

**Interfaces:**
- Consumes: `crate::helpers::InviteRow`, `crate::helpers::invite_record_from_row` (already imported/used in the file); `UseInviteError::{NotFound, AlreadyUsed, Expired}`.
- Produces: `use_invite` behavior is unchanged; it becomes a single-statement claim. Backend-agnostic — affects both SQLite and Postgres identically.

- [ ] **Step 1: Locate the existing `use_invite` tests (the parity guard)**

Run: `rg -n "use_invite|AlreadyUsed|UseInviteError::Expired" server/tests storage/src`
Expected: find the tests covering success, already-used, expired, and not-found. Confirm a "used invite is rejected" and an "expired invite is rejected" case exist. If a double-use (concurrent or sequential) case is missing, add a sequential one in the same test module:

```rust
// Second use of an already-claimed invite is rejected.
let err = state.invites.use_invite(&code, other_user_id).await.unwrap_err();
assert!(matches!(err, storage::UseInviteError::AlreadyUsed));
```

- [ ] **Step 2: Run the existing invite tests — establish the green baseline**

Run: `cargo test -p jaunder use_invite` (and the storage invite tests if present: `cargo test -p storage invite`)
Expected: PASS on the current code (both backends, via the `backends` template).

- [ ] **Step 3: Collapse `use_invite` to a single-statement claim**

Replace the body of `use_invite` (`storage/src/invites.rs:101-140`) with:

```rust
    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError> {
        let now = Utc::now();

        // Atomically claim the invite in one statement: the UPDATE succeeds only
        // when the invite exists, is unused, and has not expired. No prior read
        // is needed, so two concurrent requests cannot both succeed and the
        // SQLite read-then-write lock upgrade (ADR-0021) is avoided.
        let claimed = sqlx::query(
            "UPDATE invites SET used_at = $1, used_by = $2 \
             WHERE code = $3 AND used_at IS NULL AND expires_at > $4",
        )
        .bind(now)
        .bind(user_id)
        .bind(code)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|_| UseInviteError::NotFound)?;

        if claimed.rows_affected() > 0 {
            return Ok(());
        }

        // Zero rows affected — read the row to return the precise error.
        let row = sqlx::query_as::<_, crate::helpers::InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by \
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseInviteError::NotFound)?
        .ok_or(UseInviteError::NotFound)?;

        let record = crate::helpers::invite_record_from_row(row);
        if record.used_at.is_some() {
            return Err(UseInviteError::AlreadyUsed);
        }
        // Present and unused but the claim failed ⇒ expired.
        Err(UseInviteError::Expired)
    }
```

Notes: mirrors `use_email_verification` (`storage/src/email.rs:130`). `now` is bound twice (`$1` for the SET, `$4` for the `expires_at >` predicate). Original `expires_at <= now` (expired) maps to the WHERE excluding equality, so an exactly-`now` expiry falls to the disambiguation branch and returns `Expired` — same semantics. The `InviteRow` import already exists; remove the no-longer-used local `tx`.

- [ ] **Step 4: Run the invite tests — verify parity on both backends**

Run: `cargo test -p jaunder use_invite` (and `cargo test -p storage invite` if applicable)
Expected: PASS — same results as Step 2, on SQLite and Postgres.

- [ ] **Step 5: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: clippy + fmt clean (exit 0).

- [ ] **Step 6: Commit**

```bash
git add storage/src/invites.rs server/tests
git commit -m "fix(storage): collapse use_invite to a single-statement claim (ADR-0021, #18)"
```

---

## Task 4: File follow-up issues for the substantial audit findings

**Files:** none (issue tracker only). Uses the jaunder-issues skill.

**Interfaces:** produces three issue numbers, folded into the PR's findings table at ship.

- [ ] **Step 1: File an issue for `create_user_with_invite`**

Use the jaunder-issues skill. Title: `storage(sqlite): create_user_with_invite read-then-write transaction is SQLITE_BUSY-prone (ADR-0021)`. Body: cite `storage/src/sqlite/mod.rs:157`; sequence `SELECT invite → INSERT user RETURNING → UPDATE invite`; the second write consumes `user_id` from the first, so it is not a mechanical collapse — needs `BEGIN IMMEDIATE` or a redesign. Milestone 1; labels `test-infra`/`tooling` as appropriate. Reference ADR-0021 and #18.

- [ ] **Step 2: File an issue for `update_post`**

Title: `storage(sqlite): update_post read-then-write transaction is SQLITE_BUSY-prone (ADR-0021)`. Body: cite `storage/src/sqlite/posts.rs:31`; sequence `SELECT auth → INSERT revision → UPDATE post → replace_post_audiences`; multiple interdependent writes; needs `BEGIN IMMEDIATE`. Milestone 1; reference ADR-0021 and #18.

- [ ] **Step 3: File an issue for `tag_post`**

Title: `storage(sqlite): tag_post read-then-write transaction is SQLITE_BUSY-prone (ADR-0021)`. Body: cite `storage/src/sqlite/posts.rs:106`; sequence `SELECT exists → INSERT OR IGNORE tag → SELECT tag_id → INSERT post_tags`; interleaved read/write; needs `BEGIN IMMEDIATE` or a deterministic `INSERT … RETURNING` chain. Milestone 1; reference ADR-0021 and #18.

- [ ] **Step 4: Record the issue numbers**

Capture the three issue numbers; they fill the `issue-#NNN` dispositions in the findings table reproduced in the PR body at ship time. No commit (tracker-only task).

---

## Self-review

- **Spec coverage:** Phase 1 fix → Task 1; disposable-but-kept `#[ignore]` concurrency test, placed out-of-file under `server/tests/feed/` → Task 1. ADR-0021 → Task 2. Phase 2 audit → pre-computed findings table; trivial remediation (`use_invite`) → Task 3; substantial → Task 4 issues. Success criteria (single-statement claim, existing tests pass, ADR exists, every site dispositioned, `validate` green) all mapped. ADR-0019 wording fix is resolved as "no fix needed" (sessions is already compliant), recorded in the findings table and ADR-0021.
- **Placeholder scan:** all code steps carry full code; commands carry expected output. No TBD/TODO. Task 4 is intentionally tracker-only (its deliverable is issue numbers, not code).
- **Type consistency:** `claim_pending_batch` signature and `FeedEventRecord` mapping unchanged from the existing impl; `use_invite` reuses `InviteRow` / `invite_record_from_row` / `UseInviteError`; test uses real helper `Backend::Sqlite.setup()` and trait methods `enqueue` / `claim_pending_batch`.

## Final gate (before ship)

Run: `cargo xtask validate`
Expected: green — static + clippy + coverage + e2e (sqlite + postgres). This is the "green → may ship" signal.
