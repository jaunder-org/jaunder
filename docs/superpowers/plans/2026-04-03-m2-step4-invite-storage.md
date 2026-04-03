# M2 Step 4: InviteStorage and Invites Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `InviteStorage` trait, `SqliteInviteStorage` implementation, and the `invites` migration, with full integration test coverage.

**Architecture:** Follows the same pattern as `SessionStorage`: define a migration, a record type and error type in `storage/mod.rs`, a trait, and a concrete SQLite implementation in `storage/sqlite.rs`. Tests live in `tests/storage.rs` using a shared pool helper.

**Tech Stack:** Rust, sqlx (SQLite), chrono, thiserror, async_trait, `generate_token()` from `auth.rs`

---

## Files

- **Create:** `server/migrations/0004_create_invites.sql`
- **Modify:** `server/src/storage/mod.rs` — add `InviteRecord`, `UseInviteError`, `InviteStorage` trait; update `pub use sqlite::{...}` to include `SqliteInviteStorage`
- **Modify:** `server/src/storage/sqlite.rs` — add `SqliteInviteStorage` struct and `InviteStorage` impl
- **Modify:** `server/tests/storage.rs` — add `invite_storage_triple` helper and five integration tests

---

### Task 1: Create invites migration

**Files:**
- Create: `server/migrations/0004_create_invites.sql`

- [ ] **Step 1: Write the migration**

```sql
CREATE TABLE invites (
    code       TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at    TEXT,
    used_by    INTEGER REFERENCES users(user_id) ON DELETE SET NULL
);
```

- [ ] **Step 2: Verify build succeeds**

Run: `cargo build`
Expected: compiles without errors

---

### Task 2: Define `InviteRecord`, `UseInviteError`, and `InviteStorage` trait

**Files:**
- Modify: `server/src/storage/mod.rs`

- [ ] **Step 1: Add `InviteRecord` and `UseInviteError` to `storage/mod.rs`**

After the `SessionAuthError` definition (around line 157), add:

```rust
/// An invite code record returned by [`InviteStorage`] queries.
#[derive(Clone, Debug)]
pub struct InviteRecord {
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by: Option<i64>,
}

/// Errors that can occur when consuming an invite code.
#[derive(Debug, Error)]
pub enum UseInviteError {
    #[error("invite code not found")]
    NotFound,
    #[error("invite code has expired")]
    Expired,
    #[error("invite code has already been used")]
    AlreadyUsed,
}
```

- [ ] **Step 2: Add `InviteStorage` trait to `storage/mod.rs`**

After the `SessionStorage` trait definition (around line 169), add:

```rust
/// Async operations on the `invites` table.
#[async_trait]
pub trait InviteStorage: Send + Sync {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String>;

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError>;

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}
```

- [ ] **Step 3: Update the `pub use sqlite::{...}` line in `storage/mod.rs`**

Change:
```rust
pub use sqlite::{SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage};
```
To:
```rust
pub use sqlite::{
    SqliteInviteStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};
```

- [ ] **Step 4: Verify build succeeds (expected to fail until impl is added)**

Run: `cargo build`
Expected: fails with "cannot find struct `SqliteInviteStorage`" — that's expected at this point.

---

### Task 3: Implement `SqliteInviteStorage`

**Files:**
- Modify: `server/src/storage/sqlite.rs`

- [ ] **Step 1: Add `InviteStorage` to the imports in `sqlite.rs`**

Change the `use super::{...}` block at the top to include the new types:

```rust
use super::{
    CreateUserError, InviteRecord, InviteStorage, ProfileUpdate, SessionAuthError, SessionRecord,
    SessionStorage, SiteConfigStorage, UseInviteError, UserAuthError, UserRecord, UserStorage,
};
```

- [ ] **Step 2: Add the `SqliteInviteStorage` struct and implementation at the end of `sqlite.rs`**

```rust
// ---------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------

type InviteRow = (
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<i64>,
);

fn invite_record_from_row(
    (code, created_at, expires_at, used_at, used_by): InviteRow,
) -> InviteRecord {
    InviteRecord {
        code,
        created_at,
        expires_at,
        used_at,
        used_by,
    }
}

/// SQLite-backed [`InviteStorage`].
pub struct SqliteInviteStorage {
    pool: SqlitePool,
}

impl SqliteInviteStorage {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteStorage for SqliteInviteStorage {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String> {
        let code = crate::auth::generate_token();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO invites (code, created_at, expires_at) VALUES (?, ?, ?)",
        )
        .bind(&code)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(code)
    }

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        let row = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by
             FROM invites WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UseInviteError::NotFound)?
        .ok_or(UseInviteError::NotFound)?;

        let record = invite_record_from_row(row);

        if record.used_at.is_some() {
            return Err(UseInviteError::AlreadyUsed);
        }

        let now = Utc::now();
        if record.expires_at <= now {
            return Err(UseInviteError::Expired);
        }

        sqlx::query(
            "UPDATE invites SET used_at = ?, used_by = ? WHERE code = ?",
        )
        .bind(now)
        .bind(user_id)
        .bind(code)
        .execute(&mut *tx)
        .await
        .map_err(|_| UseInviteError::NotFound)?;

        tx.commit().await.map_err(|_| UseInviteError::NotFound)?;

        Ok(())
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let rows = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by FROM invites",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(invite_record_from_row).collect())
    }
}
```

- [ ] **Step 3: Verify build succeeds**

Run: `cargo build`
Expected: compiles without errors or warnings

- [ ] **Step 4: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings or errors

---

### Task 4: Integration tests

**Files:**
- Modify: `server/tests/storage.rs`

- [ ] **Step 1: Add imports and helper to `tests/storage.rs`**

Add to the `use` block at the top:

```rust
use server::storage::{InviteStorage, SqliteInviteStorage, UseInviteError};
```

Add the `invite_storage_triple` helper after the `storage_pair` function:

```rust
async fn invite_storage_triple(
    base: &TempDir,
) -> (SqliteUserStorage, SqliteSessionStorage, SqliteInviteStorage) {
    let pool = open_pool(base).await;
    (
        SqliteUserStorage::new(pool.clone()),
        SqliteSessionStorage::new(pool.clone()),
        SqliteInviteStorage::new(pool),
    )
}
```

- [ ] **Step 2: Write and run test: create_invite returns a code; list_invites includes it**

Add to `tests/storage.rs`:

```rust
// --- InviteStorage integration tests ---

#[tokio::test]
async fn create_invite_and_list_invites_includes_it() {
    let base = TempDir::new().unwrap();
    let (_, _, invites) = invite_storage_triple(&base).await;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    let list = invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].code, code);
    assert!(list[0].used_at.is_none());
}
```

Run: `cargo nextest run -E 'test(create_invite_and_list_invites_includes_it)'`
Expected: PASS

- [ ] **Step 3: Write and run test: use_invite with valid code marks it used**

```rust
#[tokio::test]
async fn use_invite_with_valid_code_marks_it_used() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    invites.use_invite(&code, user_id).await.unwrap();

    let list = invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}
```

Run: `cargo nextest run -E 'test(use_invite_with_valid_code_marks_it_used)'`
Expected: PASS

- [ ] **Step 4: Write and run test: use_invite with unknown code returns NotFound**

```rust
#[tokio::test]
async fn use_invite_with_unknown_code_returns_not_found() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("bob"), &password("password123"), None)
        .await
        .unwrap();

    let err = invites.use_invite("no-such-code", user_id).await.unwrap_err();
    assert!(matches!(err, UseInviteError::NotFound));
}
```

Run: `cargo nextest run -E 'test(use_invite_with_unknown_code_returns_not_found)'`
Expected: PASS

- [ ] **Step 5: Write and run test: use_invite with expired code returns Expired**

```rust
#[tokio::test]
async fn use_invite_with_expired_code_returns_expired() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("carol"), &password("password123"), None)
        .await
        .unwrap();

    // expires_at in the past
    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = invites.create_invite(expires_at).await.unwrap();

    let err = invites.use_invite(&code, user_id).await.unwrap_err();
    assert!(matches!(err, UseInviteError::Expired));
}
```

Run: `cargo nextest run -E 'test(use_invite_with_expired_code_returns_expired)'`
Expected: PASS

- [ ] **Step 6: Write and run test: use_invite on already-used code returns AlreadyUsed**

```rust
#[tokio::test]
async fn use_invite_on_already_used_code_returns_already_used() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("dave"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    invites.use_invite(&code, user_id).await.unwrap();

    let err = invites.use_invite(&code, user_id).await.unwrap_err();
    assert!(matches!(err, UseInviteError::AlreadyUsed));
}
```

Run: `cargo nextest run -E 'test(use_invite_on_already_used_code_returns_already_used)'`
Expected: PASS

---

### Task 5: Full verification and commit

- [ ] **Step 1: Run all tests**

Run: `cargo nextest run`
Expected: all tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: clean

- [ ] **Step 3: Run nix flake check**

Run: `nix flake check`
Expected: passes

- [ ] **Step 4: Request user review**

Stop here and ask the user to review before committing.
After user approval, commit:

```bash
git add server/migrations/0004_create_invites.sql \
        server/src/storage/mod.rs \
        server/src/storage/sqlite.rs \
        server/tests/storage.rs
git commit -m "M2.4.1-M2.4.10: InviteStorage, invites migration, and integration tests"
```
