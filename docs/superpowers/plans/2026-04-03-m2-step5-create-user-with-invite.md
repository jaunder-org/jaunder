# M2 Step 5: Atomic Invite-and-User Creation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `create_user_with_invite` as a free function that atomically validates an invite code, creates a user, and marks the invite used — all within a single SQLite transaction.

**Architecture:** A free function in `server/src/storage/mod.rs` that takes a `&SqlitePool` directly, since it coordinates two tables in a single transaction that cannot be cleanly expressed through the existing single-table trait objects. The function duplicates the password-hashing approach from `SqliteUserStorage::create_user` and the invite-validation logic from `SqliteInviteStorage::use_invite`.

**Tech Stack:** Rust, sqlx (SQLite), argon2, chrono, thiserror, tokio::task::spawn_blocking

---

## Files

- **Modify:** `server/src/storage/mod.rs` — add `RegisterWithInviteError` enum and `create_user_with_invite` free function
- **Modify:** `server/tests/storage.rs` — add 5 integration tests

---

### Task 1: Define `RegisterWithInviteError` and implement `create_user_with_invite`

**Files:**
- Modify: `server/src/storage/mod.rs`

- [ ] **Step 1: Add `RegisterWithInviteError` to `storage/mod.rs`**

After the `UseInviteError` definition (around line 178), add:

```rust
/// Errors that can occur during atomic invite-and-user creation.
#[derive(Debug, Error)]
pub enum RegisterWithInviteError {
    #[error("invite code not found")]
    InviteNotFound,
    #[error("invite code has expired")]
    InviteExpired,
    #[error("invite code has already been used")]
    InviteAlreadyUsed,
    #[error("username is already taken")]
    UsernameTaken,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}
```

- [ ] **Step 2: Add required imports to `storage/mod.rs`**

The free function needs `SqlitePool`, `Utc`, and the argon2/spawn_blocking machinery. Add to the imports at the top of the file:

```rust
use sqlx::SqlitePool;
```

`chrono::Utc` is already imported via the existing `use chrono::{DateTime, Utc};` line (verify it includes `Utc`; if the line only has `DateTime`, update it).

- [ ] **Step 3: Add `create_user_with_invite` free function to `storage/mod.rs`**

After the `InviteStorage` trait definition and before the `AppState` struct, add:

```rust
/// Atomically creates a user and marks an invite code as used within a single
/// transaction. This spans two tables so it cannot be expressed through the
/// single-table trait objects.
///
/// Steps:
/// (a) SELECT the invite row; return `InviteNotFound`, `InviteAlreadyUsed`, or
///     `InviteExpired` as appropriate.
/// (b) Hash the password via `spawn_blocking`.
/// (c) INSERT the user row; map a unique-constraint violation to `UsernameTaken`.
/// (d) UPDATE the invite row setting `used_at` and `used_by`.
/// (e) COMMIT.
pub async fn create_user_with_invite(
    pool: &SqlitePool,
    username: &Username,
    password: &Password,
    display_name: Option<&str>,
    invite_code: &str,
) -> Result<i64, RegisterWithInviteError> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };

    let mut tx = pool.begin().await?;

    // (a) Validate invite
    let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
        "SELECT used_at, expires_at FROM invites WHERE code = ?",
    )
    .bind(invite_code)
    .fetch_optional(&mut *tx)
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

    // (b) Hash password outside the async executor
    let password_str = password.as_str().to_owned();
    let password_hash = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password_str.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e))))?
    .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e))))?;

    // (c) Insert user
    let result = sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (username, password_hash, display_name, created_at)
         VALUES (?, ?, ?, ?)
         RETURNING user_id",
    )
    .bind(username.as_str())
    .bind(&password_hash)
    .bind(display_name)
    .bind(now)
    .fetch_one(&mut *tx)
    .await;

    let user_id = match result {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(RegisterWithInviteError::UsernameTaken);
        }
        Err(e) => return Err(RegisterWithInviteError::Internal(e)),
    };

    // (d) Mark invite used
    sqlx::query("UPDATE invites SET used_at = ?, used_by = ? WHERE code = ?")
        .bind(now)
        .bind(user_id)
        .bind(invite_code)
        .execute(&mut *tx)
        .await?;

    // (e) Commit
    tx.commit().await?;

    Ok(user_id)
}
```

- [ ] **Step 4: Verify build succeeds**

Run: `cargo build`
Expected: compiles without errors or warnings

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: clean

---

### Task 2: Integration tests

**Files:**
- Modify: `server/tests/storage.rs`

- [ ] **Step 1: Add `RegisterWithInviteError` and `create_user_with_invite` to imports**

Extend the `use server::storage::{...}` block to include:

```rust
use server::storage::{
    create_user_with_invite, open_database, CreateUserError, DbConnectOptions, InviteStorage,
    ProfileUpdate, RegisterWithInviteError, SessionAuthError, SessionStorage, SqliteInviteStorage,
    SqliteSessionStorage, SqliteUserStorage, UseInviteError, UserAuthError, UserStorage,
};
```

- [ ] **Step 2: Write and run test: valid invite creates user and marks invite used**

```rust
// --- create_user_with_invite integration tests ---

#[tokio::test]
async fn create_user_with_invite_creates_user_and_marks_invite_used() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());
    let users = SqliteUserStorage::new(pool.clone());

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    let user_id = create_user_with_invite(
        &pool,
        &username("alice"),
        &password("password123"),
        Some("Alice"),
        &code,
    )
    .await
    .unwrap();

    // User was created
    let record = users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.username.as_str(), "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));

    // Invite was marked used
    let list = invites.list_invites().await.unwrap();
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}
```

Run: `cargo nextest run -E 'test(create_user_with_invite_creates_user_and_marks_invite_used)'`
Expected: PASS

- [ ] **Step 3: Write and run test: second call with same code returns InviteAlreadyUsed**

```rust
#[tokio::test]
async fn create_user_with_invite_second_call_returns_already_used() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    create_user_with_invite(
        &pool,
        &username("alice"),
        &password("password123"),
        None,
        &code,
    )
    .await
    .unwrap();

    let err = create_user_with_invite(
        &pool,
        &username("bob"),
        &password("password123"),
        None,
        &code,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));

    // bob was not inserted
    let users = SqliteUserStorage::new(pool.clone());
    assert!(users
        .get_user_by_username(&username("bob"))
        .await
        .unwrap()
        .is_none());
}
```

Run: `cargo nextest run -E 'test(create_user_with_invite_second_call_returns_already_used)'`
Expected: PASS

- [ ] **Step 4: Write and run test: expired invite returns InviteExpired, no user inserted**

```rust
#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = invites.create_invite(expires_at).await.unwrap();

    let err = create_user_with_invite(
        &pool,
        &username("alice"),
        &password("password123"),
        None,
        &code,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteExpired));

    let users = SqliteUserStorage::new(pool.clone());
    assert!(users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .is_none());
}
```

Run: `cargo nextest run -E 'test(create_user_with_invite_expired_returns_invite_expired)'`
Expected: PASS

- [ ] **Step 5: Write and run test: unknown invite code returns InviteNotFound, no user inserted**

```rust
#[tokio::test]
async fn create_user_with_invite_unknown_code_returns_not_found() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;

    let err = create_user_with_invite(
        &pool,
        &username("alice"),
        &password("password123"),
        None,
        "no-such-code",
    )
    .await
    .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteNotFound));

    let users = SqliteUserStorage::new(pool.clone());
    assert!(users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .is_none());
}
```

Run: `cargo nextest run -E 'test(create_user_with_invite_unknown_code_returns_not_found)'`
Expected: PASS

- [ ] **Step 6: Write and run test: duplicate username returns UsernameTaken, invite not marked used**

```rust
#[tokio::test]
async fn create_user_with_invite_duplicate_username_returns_username_taken() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());
    let users = SqliteUserStorage::new(pool.clone());

    // Create alice directly (without invite)
    users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    let err = create_user_with_invite(
        &pool,
        &username("alice"),
        &password("other_password"),
        None,
        &code,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::UsernameTaken));

    // Invite was NOT marked used
    let list = invites.list_invites().await.unwrap();
    assert!(list[0].used_at.is_none());
}
```

Run: `cargo nextest run -E 'test(create_user_with_invite_duplicate_username_returns_username_taken)'`
Expected: PASS

---

### Task 3: Full verification

- [ ] **Step 1: Run all tests**

Run: `cargo nextest run`
Expected: all tests pass (55 total)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: clean

- [ ] **Step 3: Run nix flake check**

Run: `nix flake check`
Expected: passes

- [ ] **Step 4: Stop and report for user review before committing**

Do NOT commit. Report results and wait for user approval.
