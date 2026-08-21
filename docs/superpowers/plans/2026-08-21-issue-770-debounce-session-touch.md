# Debounce Session Touch Writes — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `SessionStorage::authenticate` refresh `sessions.last_used_at`
only when the stored timestamp is older than 60 seconds, so fresh authenticated
requests stop being SQLite writers.

**Architecture:** Keep the public `SessionStorage` trait unchanged and preserve
ADR-0019's dialect split behind `SessionDialect::touch_and_load`. The generic
store computes one shared freshness cutoff; SQLite loads first and skips writes
for fresh rows, while Postgres uses a conditional update followed by a fallback
load when no update happens.

**Tech Stack:** Rust, `chrono`, `sqlx`, `rstest_reuse`, dual-backend storage
tests in the `jaunder` integration test binary.

**Spec:**
[`2026-08-21-issue-770-debounce-session-touch.md`](../specs/2026-08-21-issue-770-debounce-session-touch.md)

## Review Header

**Scope — in:** `SessionStorage::authenticate` semantics, the
`SessionDialect::touch_and_load` contract and both dialect implementations,
dual-backend storage tests, and docs for `last_used_at` bounded staleness.

**Scope — out:** schema changes, configurable freshness windows,
transport-specific touch policies, app-side SQLite write queues, e2e tests, and
an ADR.

**Tasks:**

1. Pin the public storage semantics and dual-backend tests for fresh and stale
   authentication.
2. Implement the shared cutoff and both dialect shapes.
3. Update durable docs and run the focused plus commit gates.

**Key risks/decisions:**

- SQLite fresh authentication must not issue even a conditional no-op `UPDATE`;
  it must read first and return fresh rows without entering a write path.
- SQLite stale authentication must not read and then write inside the same
  deferred transaction; the stale write is a separate conditional update.
- Tests must age rows directly rather than sleep.
- The returned `SessionRecord.last_used_at` is the persisted value, never a
  synthesized `now`.

## Global Constraints

- Freshness window is exactly **60 seconds**.
- `last_used_at` remains persisted in `sessions.last_used_at`; no schema or
  configuration change.
- The policy applies uniformly through `SessionStorage::authenticate`; callers
  do not gain transport-specific touch code.
- Storage tests that exercise backend behavior use `#[apply(backends)]`.
- No `#[allow(...)]` or `#[expect(...)]` lint suppression without explicit user
  approval.
- No `Co-Authored-By` trailer.

---

### Task 1: Specify Bounded-Stale Session Semantics in Tests

**Files:**

- Modify: `server/tests/storage/sessions.rs`

**Interfaces:**

- Consumes:
  - `storage::SessionStorage::authenticate(&self, raw_token: &RawToken) -> Result<SessionRecord, SessionAuthError>`
  - `storage::test_support::{Backend, CloseablePool, SeedUser, TestEnv, backends}`
  - `common::test_support::parse_session_label`
- Produces:
  - Two new dual-backend tests that later tasks must satisfy:
    - `fresh_authenticate_returns_the_persisted_last_used_at`
    - `stale_authenticate_refreshes_the_persisted_last_used_at`
  - Optional local helpers:
    - `async fn set_last_used_at(pool: &CloseablePool, token_hash: &TokenHash, last_used_at: DateTime<Utc>)`
    - `async fn load_last_used_at(pool: &CloseablePool, token_hash: &TokenHash) -> DateTime<Utc>`

- [x] **Step 1: Add the failing tests and raw-SQL helpers**

Add imports near the top of `server/tests/storage/sessions.rs`:

```rust
use chrono::{Duration, Utc};
use common::token::TokenHash;
use storage::test_support::{CloseablePool, TestEnv};
```

Keep the existing `authenticate_updates_last_used_at` test for the moment; it
will be rewritten or removed after the new tests pin the replacement semantics.

Add backend-dispatched helpers at the bottom of the file:

```rust
async fn set_last_used_at(
    pool: &CloseablePool,
    token_hash: &TokenHash,
    last_used_at: chrono::DateTime<Utc>,
) {
    match pool {
        CloseablePool::Sqlite(pool) => {
            sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
                .bind(last_used_at)
                .bind(token_hash)
                .execute(pool)
                .await
                .unwrap();
        }
        CloseablePool::Postgres(pool) => {
            sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
                .bind(last_used_at)
                .bind(token_hash)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn load_last_used_at(
    pool: &CloseablePool,
    token_hash: &TokenHash,
) -> chrono::DateTime<Utc> {
    match pool {
        CloseablePool::Sqlite(pool) => {
            sqlx::query_scalar("SELECT last_used_at FROM sessions WHERE token_hash = $1")
                .bind(token_hash)
                .fetch_one(pool)
                .await
                .unwrap()
        }
        CloseablePool::Postgres(pool) => {
            sqlx::query_scalar("SELECT last_used_at FROM sessions WHERE token_hash = $1")
                .bind(token_hash)
                .fetch_one(pool)
                .await
                .unwrap()
        }
    }
}
```

Add the fresh-path test:

```rust
#[apply(backends)]
#[tokio::test]
async fn fresh_authenticate_returns_the_persisted_last_used_at(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = SeedUser::new().seed(&state).await.user_id;
    let raw_token = state
        .sessions
        .create_session(user_id, &parse_session_label("test session"))
        .await
        .unwrap();

    let token_hash = host::token::hash(&raw_token).unwrap();
    let stored = load_last_used_at(base.pool(), &token_hash).await;

    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    let persisted_after_auth = load_last_used_at(base.pool(), &token_hash).await;

    assert_eq!(record.last_used_at, stored);
    assert_eq!(persisted_after_auth, stored);
}
```

Add the stale-path test:

```rust
#[apply(backends)]
#[tokio::test]
async fn stale_authenticate_refreshes_the_persisted_last_used_at(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = SeedUser::new().seed(&state).await.user_id;
    let raw_token = state
        .sessions
        .create_session(user_id, &parse_session_label("test session"))
        .await
        .unwrap();

    let token_hash = host::token::hash(&raw_token).unwrap();
    let stale = Utc::now() - Duration::seconds(120);
    set_last_used_at(base.pool(), &token_hash, stale).await;

    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    let persisted_after_auth = load_last_used_at(base.pool(), &token_hash).await;
    let freshness_cutoff_after_auth = Utc::now() - Duration::seconds(60);

    assert!(record.last_used_at > stale);
    assert_eq!(record.last_used_at, persisted_after_auth);
    assert!(persisted_after_auth >= freshness_cutoff_after_auth);
}
```

These tests intentionally use 120 seconds stale and a 60 second cutoff, so they
do not sleep and do not depend on sub-second timing.

- [x] **Step 2: Run the focused tests and verify they fail**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder --test integration fresh_authenticate_returns_the_persisted_last_used_at stale_authenticate_refreshes_the_persisted_last_used_at`

Expected: **FAIL**. The fresh test should fail because current authentication
updates `last_used_at` immediately. The stale test may pass or fail depending on
timestamp timing, but the run is still red because fresh authentication is not
yet debounced.

- [x] **Step 3: Commit the failing tests is not allowed**

Do not commit at this point. Continue to Task 2; the per-commit gate must only
see a green tree.

---

### Task 2: Implement Shared Cutoff and Dialect-Specific Touch Shapes

**Files:**

- Modify: `storage/src/sessions.rs`
- Modify: `storage/src/sqlite/sessions.rs`
- Modify: `storage/src/postgres/sessions.rs`
- Modify: `server/tests/storage/sessions.rs`

**Interfaces:**

- Consumes:
  - Tests from Task 1.
  - `crate::helpers::{SessionRow, session_record_from_row}`
- Produces:
  - `const SESSION_TOUCH_FRESHNESS_SECONDS: i64 = 60;`
  - `fn session_touch_cutoff(now: DateTime<Utc>) -> DateTime<Utc>`
  - Updated dialect contract:
    ```rust
    async fn touch_and_load(
        pool: &Pool<Self>,
        token_hash: &TokenHash,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> sqlx::Result<Option<SessionRow>>;
    ```

- [x] **Step 1: Add the shared freshness contract**

In `storage/src/sessions.rs`, add the constant and helper near the
`SessionDialect` section:

```rust
const SESSION_TOUCH_FRESHNESS_SECONDS: i64 = 60;

fn session_touch_cutoff(now: DateTime<Utc>) -> DateTime<Utc> {
    now - chrono::Duration::seconds(SESSION_TOUCH_FRESHNESS_SECONDS)
}
```

Update the `SessionRecord.last_used_at` field doc from:

```rust
/// When the session was last used to authenticate a request.
```

to:

```rust
/// When the session was last persisted as used to authenticate a request.
///
/// This is operator-facing metadata with bounded staleness: authentication may
/// skip updating it for up to 60 seconds while the stored value is fresh.
```

Update the `SessionStorage::authenticate` doc from “On success, updates...” to:

```rust
/// On success, refreshes `last_used_at` only when the stored value is at least
/// than the 60 second freshness window.
```

Update `SessionDialect::touch_and_load` docs to say it returns the joined
session row and touches only when `last_used_at < stale_before`.

In `authenticate`, compute:

```rust
let now = Utc::now();
let stale_before = session_touch_cutoff(now);

let row = DB::touch_and_load(&self.pool, &token_hash, now, stale_before)
    .await?
    .ok_or(SessionAuthError::SessionNotFound)?;
```

- [x] **Step 2: Implement SQLite fresh-read / stale-write**

Replace SQLite's unconditional update implementation with this structure in
`storage/src/sqlite/sessions.rs`:

```rust
let row = sqlx::query_as::<_, SessionRow>(
    "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
     FROM sessions s
     JOIN users u ON u.user_id = s.user_id
     WHERE s.token_hash = $1",
)
.bind(token_hash)
.fetch_optional(pool)
.await?;

let Some(row) = row else {
    return Ok(None);
};

if row.5 >= stale_before {
    return Ok(Some(row));
}

sqlx::query(
    "UPDATE sessions
     SET last_used_at = $1
     WHERE token_hash = $2 AND last_used_at < $3",
)
.bind(now)
.bind(token_hash)
.bind(stale_before)
.execute(pool)
.await?;

sqlx::query_as::<_, SessionRow>(
    "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
     FROM sessions s
     JOIN users u ON u.user_id = s.user_id
     WHERE s.token_hash = $1",
)
.bind(token_hash)
.fetch_optional(pool)
.await
```

Do not wrap the initial read and later update in one `pool.begin()` transaction.
The initial fresh path must stay read-only, and the stale write must be a
separate write-first conditional statement.

- [x] **Step 3: Implement Postgres conditional touch and fallback load**

Replace the Postgres CTE in `storage/src/postgres/sessions.rs` with a
conditional update first:

```sql
WITH updated AS (
    UPDATE sessions s
    SET last_used_at = $1
    WHERE s.token_hash = $2
      AND s.last_used_at < $3
    RETURNING s.token_hash, s.user_id, s.label, s.created_at, s.last_used_at
)
SELECT updated.token_hash, updated.user_id, u.username, updated.label,
       updated.created_at, updated.last_used_at
FROM updated
JOIN users u ON updated.user_id = u.user_id
```

Bind `$1 = now`, `$2 = token_hash`, `$3 = stale_before`.

If the update returns a row, return it. If the update returns `None`, run a
plain joined `SELECT` by `token_hash` and return that row. This preserves the
stale-race invariant: if a concurrent request already refreshed the session, the
conditional update does not write again and the fallback load observes the
current persisted timestamp.

- [x] **Step 4: Rewrite the old monotonic test**

Replace `authenticate_updates_last_used_at` in
`server/tests/storage/sessions.rs` with a name and assertion that matches the
new semantics, or remove it if the two Task 1 tests cover its full value.

Acceptable replacement:

```rust
#[apply(backends)]
#[tokio::test]
async fn authenticate_returns_session_record_for_valid_token(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let raw_token = create_session_for(state, user_id).await.token;
    let record = state.sessions.authenticate(&raw_token).await.unwrap();

    assert_eq!(record.user_id, user_id);
}
```

Do not keep the old `second.last_used_at >= first.last_used_at` assertion as
touch coverage; it no longer describes the required behavior.

- [x] **Step 5: Run the focused tests and verify they pass**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder --test integration sessions`

Expected: **PASS** for the session storage tests on both backends selected by
the existing `#[apply(backends)]` template.

- [x] **Step 6: Commit the tested implementation**

Run: `devtool run -- cargo xtask check`

If `cargo xtask check` reformats files, inspect and stage the mechanical fixes
before committing.

Stage the implementation, tests, and lifecycle docs/progress:

```bash
git add storage/src/sessions.rs storage/src/sqlite/sessions.rs storage/src/postgres/sessions.rs server/tests/storage/sessions.rs docs/superpowers/specs/2026-08-21-issue-770-debounce-session-touch.md docs/superpowers/plans/2026-08-21-issue-770-debounce-session-touch.md
git commit -m "fix(storage): debounce session touch writes (#770)"
```

---

### Task 3: Update Architecture Docs and Finish the Branch Gate

**Files:**

- Modify: `docs/ARCHITECTURE.md`
- Modify:
  `docs/superpowers/plans/2026-08-21-issue-770-debounce-session-touch.md`

**Interfaces:**

- Consumes:
  - Spec D1/D8 and AC1.
  - Implemented 60 second constant from Task 2.
- Produces:
  - Architecture wording that describes `last_used_at` as bounded-stale operator
    metadata.
  - A checked-off plan task before the docs commit gate.

- [x] **Step 1: Update the session architecture paragraph**

In `docs/ARCHITECTURE.md` near the session row description, replace the sentence
that currently describes sessions as:

```markdown
Sessions never expire; the `sessions` row is
`(token_hash, user_id, label, created_at, last_used_at)` ...
```

with wording equivalent to:

```markdown
Sessions never expire; the `sessions` row is
`(token_hash, user_id, label, created_at, last_used_at)`. `last_used_at` is
operator-facing metadata and is bounded-stale: authentication refreshes it only
when the stored value is more than 60 seconds old, so fresh authenticated
requests need not become database writers. ...
```

Keep the existing surrounding statements about labelled sessions,
`SessionLabel`, and app passwords intact.

- [x] **Step 2: Format docs**

Run:
`devtool run -- prettier -w docs/ARCHITECTURE.md docs/superpowers/plans/2026-08-21-issue-770-debounce-session-touch.md`

Expected: **PASS**.

- [x] **Step 3: Commit docs and plan progress**

Before committing, tick completed plan checkboxes for Tasks 1-3 as appropriate.
Then run:

`devtool run -- cargo xtask check`

If `cargo xtask check` reformats files, inspect and stage the mechanical fixes
before committing.

Stage docs:

```bash
git add docs/ARCHITECTURE.md docs/superpowers/plans/2026-08-21-issue-770-debounce-session-touch.md
git commit -m "docs(storage): record bounded-stale session usage metadata (#770)"
```

- [x] **Step 4: Run the branch-level gate**

Run: `devtool run -- cargo xtask validate --no-e2e`

Expected: **PASS** on the clean committed tree.

If it fails, inspect `.xtask/last-result.json` and the relevant
`.xtask/diagnostics/<check>/failure-excerpt.log` before changing code.

---

## Self-Review

- Spec coverage: AC1 is covered by Tasks 2 and 3; AC2-AC7 are covered by Tasks 1
  and 2; AC8 is covered by Task 3.
- Placeholder scan: no `TBD`, `TODO`, “similar to,” or unspecified tests remain
  in the task contracts.
- Type consistency: the planned dialect signature threads `now` and
  `stale_before` through the existing generic store and both dialect impls; the
  test helpers use the existing `CloseablePool` backend dispatch.
