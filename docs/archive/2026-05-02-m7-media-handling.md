# M7: Media Handling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add user media upload with content-addressed local serving, plus a caching system for remote media attachments with per-user policy control.

**Architecture:** Content-addressed filesystem storage (SHA256) with hard-link deduplication. Database tracks ownership/metadata. Upload and cached media share a common storage layout under `<storage_path>/media/`. Per-user cache policy stored in a new `user_config` table.

**Tech Stack:** Rust, Axum (multipart upload handler + file serving), SQLx (SQLite + Postgres), SHA256 (via `sha2` crate), Tokio filesystem I/O, Leptos server functions for management UI.

---

## Scope Decomposition

This milestone is large. The plan is divided into sub-plans that build on each other:

1. **Preparatory refactor** — Split `common/src/storage.rs` into module directory
2. **Migrations & storage traits** — `media` table, `user_config` table, traits
3. **Pure media utilities** — `common/src/media.rs` (path computation, filename sanitization, URL derivation)
4. **Upload handler** — Axum multipart handler with streaming SHA256, temp files, quota enforcement
5. **Serving handler** — Static file serving with caching headers, ETag, Content-Disposition
6. **Storage implementations** — SQLite and Postgres `MediaStorage` + `UserConfigStorage`
7. **Management server functions** — `list_my_media`, `delete_media`, `media_usage`
8. **Management UI** — Leptos page component
9. **Lazy proxy & cache infrastructure** — Endpoint + download logic (stub for future M9/M17)

---

## Task 1: Preparatory Refactor — Split `common/src/storage.rs` into Module Directory

**Files:**
- Delete: `common/src/storage.rs`
- Create: `common/src/storage/mod.rs`
- Create: `common/src/storage/site_config.rs`
- Create: `common/src/storage/users.rs`
- Create: `common/src/storage/sessions.rs`
- Create: `common/src/storage/invites.rs`
- Create: `common/src/storage/atomic.rs`
- Create: `common/src/storage/email.rs`
- Create: `common/src/storage/password.rs`
- Create: `common/src/storage/posts.rs`
- Create: `common/src/storage/app_state.rs`

This is a pure refactor. All downstream imports via `common::storage::*` must continue to work unchanged.

- [ ] **Step 1: Create `common/src/storage/` directory and `site_config.rs`**

Move the `SiteConfigStorage` trait and backup key constants:

```rust
// common/src/storage/site_config.rs
use async_trait::async_trait;

/// Async operations on the `site_config` key-value table.
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for `key`, or `None` if the key is not set.
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>>;

    /// Inserts or replaces the value for `key`.
    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()>;
}

pub const BACKUP_DESTINATION_PATH_KEY: &str = "backup.destination_path";
pub const BACKUP_SCHEDULE_KEY: &str = "backup.schedule";
pub const BACKUP_RETENTION_COUNT_KEY: &str = "backup.retention_count";
pub const BACKUP_MODE_KEY: &str = "backup.mode";
```

- [ ] **Step 2: Create `common/src/storage/users.rs`**

Move `UserRecord`, `CreateUserError`, `UserAuthError`, `ProfileUpdate`, `UserStorage`:

```rust
// common/src/storage/users.rs
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use thiserror::Error;

use crate::password::Password;
use crate::username::Username;

#[derive(Clone, Debug)]
pub struct UserRecord {
    pub user_id: i64,
    pub username: Username,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_authenticated_at: Option<DateTime<Utc>>,
    pub email: Option<EmailAddress>,
    pub email_verified: bool,
    pub is_operator: bool,
}

#[derive(Debug, Error)]
pub enum CreateUserError {
    #[error("username is already taken")]
    UsernameTaken,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

#[derive(Debug, Error)]
pub enum UserAuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("internal error: {0}")]
    Internal(String),
}

pub struct ProfileUpdate<'a> {
    pub display_name: Option<&'a str>,
    pub bio: Option<&'a str>,
}

#[async_trait]
pub trait UserStorage: Send + Sync {
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
    ) -> Result<i64, CreateUserError>;

    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError>;

    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>>;

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>>;

    async fn update_profile(&self, user_id: i64, update: &ProfileUpdate<'_>) -> sqlx::Result<()>;

    async fn set_email(
        &self,
        user_id: i64,
        email: Option<&EmailAddress>,
        verified: bool,
    ) -> sqlx::Result<()>;

    async fn set_password(&self, user_id: i64, new_password: &Password) -> sqlx::Result<()>;
}
```

- [ ] **Step 3: Create `common/src/storage/sessions.rs`**

Move `SessionRecord`, `SessionAuthError`, `SessionStorage`:

```rust
// common/src/storage/sessions.rs
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::username::Username;

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub token_hash: String,
    pub user_id: i64,
    pub username: Username,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum SessionAuthError {
    #[error("invalid token")]
    InvalidToken,
    #[error("session not found")]
    SessionNotFound,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn create_session(&self, user_id: i64, label: Option<&str>) -> sqlx::Result<String>;

    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError>;

    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()>;

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>>;
}
```

- [ ] **Step 4: Create `common/src/storage/invites.rs`**

Move `InviteRecord`, `UseInviteError`, `InviteStorage`:

```rust
// common/src/storage/invites.rs
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct InviteRecord {
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by: Option<i64>,
}

#[derive(Debug, Error)]
pub enum UseInviteError {
    #[error("invite code not found")]
    NotFound,
    #[error("invite code has expired")]
    Expired,
    #[error("invite code has already been used")]
    AlreadyUsed,
}

#[async_trait]
pub trait InviteStorage: Send + Sync {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String>;

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError>;

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}
```

- [ ] **Step 5: Create `common/src/storage/atomic.rs`**

Move `RegisterWithInviteError`, `ConfirmPasswordResetError`, `AtomicOps`:

```rust
// common/src/storage/atomic.rs
use async_trait::async_trait;
use thiserror::Error;

use crate::password::Password;
use crate::username::Username;

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

#[derive(Debug, Error)]
pub enum ConfirmPasswordResetError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

#[async_trait]
pub trait AtomicOps: Send + Sync {
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError>;

    async fn confirm_password_reset(
        &self,
        raw_token: &str,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError>;
}
```

- [ ] **Step 6: Create `common/src/storage/email.rs`**

Move `UseEmailVerificationError`, `EmailVerificationStorage`:

```rust
// common/src/storage/email.rs
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UseEmailVerificationError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

#[async_trait]
pub trait EmailVerificationStorage: Send + Sync {
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &str,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String>;

    async fn use_email_verification(
        &self,
        raw_token: &str,
    ) -> Result<(i64, String), UseEmailVerificationError>;
}
```

- [ ] **Step 7: Create `common/src/storage/password.rs`**

Move `UsePasswordResetError`, `PasswordResetStorage`:

```rust
// common/src/storage/password.rs
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UsePasswordResetError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

#[async_trait]
pub trait PasswordResetStorage: Send + Sync {
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String>;

    async fn use_password_reset(&self, raw_token: &str) -> Result<i64, UsePasswordResetError>;
}
```

- [ ] **Step 8: Create `common/src/storage/posts.rs`**

Move all post-related types (`PostFormat`, `InvalidPostFormat`, `PostRecord`, `PostRevisionRecord`, `CreatePostError`, `UpdatePostError`, `PostCursor`, `CreatePostInput`, `UpdatePostInput`, `TagRecord`, `PostTag`, `TaggingError`, `ListByTagError`, `PostStorage`). This is the largest file — copy the full Posts section (lines 322–548 of the original `storage.rs`) plus the needed imports.

- [ ] **Step 9: Create `common/src/storage/app_state.rs`**

```rust
// common/src/storage/app_state.rs
use std::sync::Arc;

use crate::mailer::MailSender;

use super::{
    AtomicOps, EmailVerificationStorage, InviteStorage, PasswordResetStorage, PostStorage,
    SessionStorage, SiteConfigStorage, UserStorage,
};

pub struct AppState {
    pub site_config: Arc<dyn SiteConfigStorage>,
    pub users: Arc<dyn UserStorage>,
    pub sessions: Arc<dyn SessionStorage>,
    pub invites: Arc<dyn InviteStorage>,
    pub atomic: Arc<dyn AtomicOps>,
    pub email_verifications: Arc<dyn EmailVerificationStorage>,
    pub password_resets: Arc<dyn PasswordResetStorage>,
    pub posts: Arc<dyn PostStorage>,
    pub mailer: Arc<dyn MailSender>,
}
```

- [ ] **Step 10: Create `common/src/storage/mod.rs` with re-exports**

```rust
// common/src/storage/mod.rs
mod app_state;
mod atomic;
mod email;
mod invites;
mod password;
mod posts;
mod sessions;
mod site_config;
mod users;

pub use app_state::*;
pub use atomic::*;
pub use email::*;
pub use invites::*;
pub use password::*;
pub use posts::*;
pub use sessions::*;
pub use site_config::*;
pub use users::*;
```

- [ ] **Step 11: Move tests from old `storage.rs` into appropriate sub-modules**

Place the unit tests from the original `storage.rs` (lines 574–666) into the sub-module they test. For example, `PostFormat` tests go in `posts.rs`, `TaggingError` tests go in `posts.rs`, `ListByTagError` tests go in `posts.rs`.

- [ ] **Step 12: Verify the refactor compiles and all tests pass**

Run: `scripts/verify`
Expected: All existing tests pass, no changes to downstream imports needed.

- [ ] **Step 13: Commit the refactor**

```bash
git add common/src/storage/ common/src/storage.rs
git commit -m "M7.1: refactor common/src/storage.rs into module directory

Split the 667-line storage.rs into focused sub-modules:
site_config, users, sessions, invites, atomic, email, password, posts, app_state.

All re-exports from mod.rs ensure downstream imports unchanged.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 2: Database Migrations — `media` and `user_config` Tables

**Files:**
- Create: `server/migrations/sqlite/0012_create_media.sql`
- Create: `server/migrations/postgres/0012_create_media.sql`
- Create: `server/migrations/sqlite/0013_create_user_config.sql`
- Create: `server/migrations/postgres/0013_create_user_config.sql`

- [ ] **Step 1: Create SQLite media migration**

```sql
-- server/migrations/sqlite/0012_create_media.sql
CREATE TABLE IF NOT EXISTS media (
    user_id INTEGER NOT NULL REFERENCES users(user_id),
    sha256 TEXT NOT NULL,
    filename TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('upload', 'cached')),
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    source_url TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (user_id, sha256, filename, source)
);
```

- [ ] **Step 2: Create Postgres media migration**

```sql
-- server/migrations/postgres/0012_create_media.sql
CREATE TABLE IF NOT EXISTS media (
    user_id BIGINT NOT NULL REFERENCES users(user_id),
    sha256 TEXT NOT NULL,
    filename TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('upload', 'cached')),
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    source_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, sha256, filename, source)
);
```

- [ ] **Step 3: Create SQLite user_config migration**

```sql
-- server/migrations/sqlite/0013_create_user_config.sql
CREATE TABLE IF NOT EXISTS user_config (
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (user_id, key)
);
```

- [ ] **Step 4: Create Postgres user_config migration**

```sql
-- server/migrations/postgres/0013_create_user_config.sql
CREATE TABLE IF NOT EXISTS user_config (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (user_id, key)
);
```

- [ ] **Step 5: Verify migrations run cleanly**

Run: `cargo nextest run` (integration tests run migrations against in-memory SQLite)
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add server/migrations/
git commit -m "M7.2: add media and user_config table migrations

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 3: Storage Traits — `MediaStorage` and `UserConfigStorage`

**Files:**
- Create: `common/src/storage/media.rs`
- Create: `common/src/storage/user_config.rs`
- Modify: `common/src/storage/mod.rs` (add modules + re-exports)
- Modify: `common/src/storage/app_state.rs` (add new fields)

- [ ] **Step 1: Create `common/src/storage/media.rs`**

```rust
// common/src/storage/media.rs
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// Source of a media record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaSource {
    Upload,
    Cached,
}

impl MediaSource {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaSource::Upload => "upload",
            MediaSource::Cached => "cached",
        }
    }
}

impl std::fmt::Display for MediaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MediaSource {
    type Err = InvalidMediaSource;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "upload" => Ok(MediaSource::Upload),
            "cached" => Ok(MediaSource::Cached),
            _ => Err(InvalidMediaSource),
        }
    }
}

#[derive(Debug, Error)]
#[error("media source must be \"upload\" or \"cached\"")]
pub struct InvalidMediaSource;

/// A media record from the database.
#[derive(Clone, Debug)]
pub struct MediaRecord {
    pub user_id: i64,
    pub sha256: String,
    pub filename: String,
    pub source: MediaSource,
    pub content_type: String,
    pub size_bytes: i64,
    pub source_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Errors that can occur when inserting a media record.
#[derive(Debug, Error)]
pub enum CreateMediaError {
    #[error("media already exists")]
    AlreadyExists,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when deleting a media record.
#[derive(Debug, Error)]
pub enum DeleteMediaError {
    #[error("media not found")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Async operations on the `media` table.
#[async_trait]
pub trait MediaStorage: Send + Sync {
    /// Insert a new media record.
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError>;

    /// Get a specific media record by its natural key.
    async fn get_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>>;

    /// List media for a user, ordered by created_at descending.
    /// Returns up to `limit` records, offset by `offset`.
    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
        limit: u32,
        offset: u32,
    ) -> sqlx::Result<Vec<MediaRecord>>;

    /// Delete a media record by its natural key.
    async fn delete_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError>;

    /// Get total bytes used by a user (sum of size_bytes for source='upload').
    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64>;

    /// Find a media record by sha256 and source (any user) to check for existing content on disk.
    async fn find_by_hash(
        &self,
        sha256: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>>;
}

// Site config keys for media quotas
pub const MEDIA_MAX_FILE_SIZE_BYTES_KEY: &str = "media.max_file_size_bytes";
pub const MEDIA_USER_QUOTA_BYTES_KEY: &str = "media.user_quota_bytes";
pub const MEDIA_CACHE_POLICY_DEFAULT_KEY: &str = "media.cache_policy_default";

/// Default max file size: 50 MB.
pub const DEFAULT_MAX_FILE_SIZE_BYTES: i64 = 52_428_800;
/// Default per-user quota: 1 GB.
pub const DEFAULT_USER_QUOTA_BYTES: i64 = 1_073_741_824;
```

- [ ] **Step 2: Create `common/src/storage/user_config.rs`**

```rust
// common/src/storage/user_config.rs
use async_trait::async_trait;

/// Async operations on the `user_config` key-value table.
#[async_trait]
pub trait UserConfigStorage: Send + Sync {
    /// Returns the value for `key` for the given user, or `None` if not set.
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>>;

    /// Inserts or replaces the value for `key` for the given user.
    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()>;

    /// Deletes a key for the given user (reverts to site default).
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()>;
}

/// User config key for media cache policy.
pub const USER_MEDIA_CACHE_POLICY_KEY: &str = "media.cache_policy";
```

- [ ] **Step 3: Update `common/src/storage/mod.rs` to include new modules**

Add after the existing module declarations:

```rust
mod media;
mod user_config;

pub use media::*;
pub use user_config::*;
```

- [ ] **Step 4: Update `common/src/storage/app_state.rs` to include new storage handles**

Add two new fields to `AppState`:

```rust
pub media: Arc<dyn MediaStorage>,
pub user_config: Arc<dyn UserConfigStorage>,
```

- [ ] **Step 5: Fix all compile errors from new `AppState` fields**

Update `make_app_state` in `server/src/storage/sqlite.rs` and `make_postgres_app_state` in `server/src/storage/mod.rs`. Temporarily use a placeholder/stub implementation (create a `todo!()` struct or an in-memory stub) to allow compilation while the real implementations come in Task 6. Better yet, implement the SQLite and Postgres versions now as empty structs that compile but have `todo!()` bodies — this unblocks the rest.

Actually, the best approach: create the implementations first in Task 6, then update AppState. Instead, reorder: add the fields with `todo!()` placeholder implementations so the codebase compiles:

```rust
// Temporary stub in server/src/storage/sqlite.rs
pub struct SqliteMediaStorage { pool: SqlitePool }
impl SqliteMediaStorage {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}
// (Full implementation comes in Task 6)

pub struct SqliteUserConfigStorage { pool: SqlitePool }
impl SqliteUserConfigStorage {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}
```

Wire both into `make_app_state`:
```rust
media: Arc::new(SqliteMediaStorage::new(pool.clone())),
user_config: Arc::new(SqliteUserConfigStorage::new(pool.clone())),
```

And equivalently for Postgres.

- [ ] **Step 6: Update `server/src/storage/mod.rs` re-exports**

Add to the `pub use common::storage::{...}` block:
```rust
CreateMediaError, DeleteMediaError, InvalidMediaSource, MediaRecord, MediaSource, MediaStorage,
UserConfigStorage, MEDIA_MAX_FILE_SIZE_BYTES_KEY, MEDIA_USER_QUOTA_BYTES_KEY,
MEDIA_CACHE_POLICY_DEFAULT_KEY, DEFAULT_MAX_FILE_SIZE_BYTES, DEFAULT_USER_QUOTA_BYTES,
USER_MEDIA_CACHE_POLICY_KEY,
```

Add to the `pub use sqlite::{...}` block:
```rust
SqliteMediaStorage, SqliteUserConfigStorage,
```

Add to the `pub use postgres::{...}` block:
```rust
PostgresMediaStorage, PostgresUserConfigStorage,
```

- [ ] **Step 7: Verify it compiles**

Run: `cargo build`
Expected: Compiles (trait methods may be `todo!()`)

- [ ] **Step 8: Commit**

```bash
git add common/src/storage/ server/src/storage/
git commit -m "M7.3: add MediaStorage and UserConfigStorage traits, update AppState

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 4: Pure Media Utilities — `common/src/media.rs`

**Files:**
- Create: `common/src/media.rs`
- Modify: `common/src/lib.rs` (add `pub mod media;`)

- [ ] **Step 1: Write tests for filename sanitization**

```rust
// common/src/media.rs (tests section at bottom)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_components() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("foo/bar/baz.txt"), "baz.txt");
        assert_eq!(sanitize_filename("C:\\Users\\file.txt"), "file.txt");
    }

    #[test]
    fn sanitize_replaces_unsafe_chars() {
        assert_eq!(sanitize_filename("file\0name.txt"), "file_name.txt");
        assert_eq!(sanitize_filename("file/name.txt"), "name.txt");
        assert_eq!(sanitize_filename("a\\b"), "b");
    }

    #[test]
    fn sanitize_rejects_empty() {
        assert!(sanitize_filename("").is_empty());
        assert!(sanitize_filename("..").is_empty());
        assert!(sanitize_filename(".").is_empty());
    }

    #[test]
    fn media_path_computation() {
        let path = media_path("upload", "a3f2deadbeef1234abcd", "photo.jpg");
        assert_eq!(path, "upload/a3f2/a3f2deadbeef1234abcd/photo.jpg");
    }

    #[test]
    fn media_url_computation() {
        let url = media_url("upload", "a3f2deadbeef1234abcd", "photo.jpg");
        assert_eq!(url, "/media/upload/a3f2/a3f2deadbeef1234abcd/photo.jpg");
    }

    #[test]
    fn content_disposition_inline_for_images() {
        assert!(should_inline("image/jpeg"));
        assert!(should_inline("image/png"));
        assert!(should_inline("image/gif"));
        assert!(should_inline("image/webp"));
        assert!(should_inline("image/svg+xml"));
    }

    #[test]
    fn content_disposition_inline_for_media() {
        assert!(should_inline("audio/mpeg"));
        assert!(should_inline("video/mp4"));
        assert!(should_inline("application/pdf"));
    }

    #[test]
    fn content_disposition_attachment_for_others() {
        assert!(!should_inline("application/zip"));
        assert!(!should_inline("text/plain"));
        assert!(!should_inline("application/octet-stream"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p common -E 'test(media)'`
Expected: FAIL (module doesn't exist)

- [ ] **Step 3: Implement the media utility functions**

```rust
// common/src/media.rs

use std::path::Path;

/// Sanitize a filename for safe use in filesystem paths and URLs.
///
/// Returns an empty string if the filename is invalid after sanitization.
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    // Strip path components: take only the final component
    let name = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Replace null bytes with underscores
    let name = name.replace('\0', "_");

    // Reject . and ..
    if name == "." || name == ".." || name.is_empty() {
        return String::new();
    }

    name
}

/// Compute the relative filesystem path for a media file.
///
/// Returns `"<source>/<4-hex-prefix>/<full-sha256>/<filename>"`.
#[must_use]
pub fn media_path(source: &str, sha256: &str, filename: &str) -> String {
    let prefix = &sha256[..4];
    format!("{source}/{prefix}/{sha256}/{filename}")
}

/// Compute the URL path for serving a media file.
///
/// Returns `"/media/<source>/<4-hex-prefix>/<full-sha256>/<filename>"`.
#[must_use]
pub fn media_url(source: &str, sha256: &str, filename: &str) -> String {
    let prefix = &sha256[..4];
    format!("/media/{source}/{prefix}/{sha256}/{filename}")
}

/// Returns `true` if the content type should be served with `Content-Disposition: inline`.
#[must_use]
pub fn should_inline(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg"
            | "image/png"
            | "image/gif"
            | "image/webp"
            | "image/svg+xml"
            | "audio/mpeg"
            | "audio/ogg"
            | "audio/flac"
            | "audio/wav"
            | "video/mp4"
            | "video/webm"
            | "application/pdf"
    )
}

/// Detect content type from filename extension.
///
/// Falls back to `"application/octet-stream"` for unknown extensions.
#[must_use]
pub fn detect_content_type(filename: &str) -> &'static str {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    // ... (tests from Step 1)
}
```

- [ ] **Step 4: Add `pub mod media;` to `common/src/lib.rs`**

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p common -E 'test(media)'`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add common/src/media.rs common/src/lib.rs
git commit -m "M7.4: add pure media utility functions (path, URL, sanitization)

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 5: Storage Implementations — SQLite and Postgres

**Files:**
- Modify: `server/src/storage/sqlite.rs` (add `MediaStorage` + `UserConfigStorage` impls)
- Modify: `server/src/storage/postgres.rs` (add `MediaStorage` + `UserConfigStorage` impls)

- [ ] **Step 1: Write integration test for MediaStorage**

Create `server/tests/storage_media.rs`:

```rust
// server/tests/storage_media.rs
use chrono::Utc;
use common::storage::{
    CreateMediaError, DeleteMediaError, MediaRecord, MediaSource, MediaStorage,
    UserConfigStorage, USER_MEDIA_CACHE_POLICY_KEY,
};
use jaunder::storage::{open_database, DbConnectOptions};

async fn setup() -> std::sync::Arc<common::storage::AppState> {
    let opts: DbConnectOptions = "sqlite::memory:".parse().unwrap();
    open_database(&opts).await.unwrap()
}

#[tokio::test]
async fn create_and_get_media() {
    let state = setup().await;
    let record = MediaRecord {
        user_id: 1, // will need a real user — create one first
        sha256: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234".to_string(),
        filename: "test.jpg".to_string(),
        source: MediaSource::Upload,
        content_type: "image/jpeg".to_string(),
        size_bytes: 12345,
        source_url: None,
        created_at: Utc::now(),
    };
    // First, create a user for the FK
    let password: common::password::Password = "testpass123".parse().unwrap();
    let username: common::username::Username = "testuser".parse().unwrap();
    let user_id = state.users.create_user(&username, &password, None, false).await.unwrap();

    let record = MediaRecord { user_id, ..record };
    state.media.create_media(&record).await.unwrap();

    let fetched = state.media.get_media(
        user_id,
        &record.sha256,
        &record.filename,
        &MediaSource::Upload,
    ).await.unwrap();
    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.content_type, "image/jpeg");
    assert_eq!(fetched.size_bytes, 12345);
}

#[tokio::test]
async fn duplicate_media_returns_already_exists() {
    let state = setup().await;
    let password: common::password::Password = "testpass123".parse().unwrap();
    let username: common::username::Username = "testuser".parse().unwrap();
    let user_id = state.users.create_user(&username, &password, None, false).await.unwrap();

    let record = MediaRecord {
        user_id,
        sha256: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234".to_string(),
        filename: "test.jpg".to_string(),
        source: MediaSource::Upload,
        content_type: "image/jpeg".to_string(),
        size_bytes: 12345,
        source_url: None,
        created_at: Utc::now(),
    };
    state.media.create_media(&record).await.unwrap();
    let err = state.media.create_media(&record).await.unwrap_err();
    assert!(matches!(err, CreateMediaError::AlreadyExists));
}

#[tokio::test]
async fn delete_media_removes_record() {
    let state = setup().await;
    let password: common::password::Password = "testpass123".parse().unwrap();
    let username: common::username::Username = "testuser".parse().unwrap();
    let user_id = state.users.create_user(&username, &password, None, false).await.unwrap();

    let record = MediaRecord {
        user_id,
        sha256: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234".to_string(),
        filename: "test.jpg".to_string(),
        source: MediaSource::Upload,
        content_type: "image/jpeg".to_string(),
        size_bytes: 12345,
        source_url: None,
        created_at: Utc::now(),
    };
    state.media.create_media(&record).await.unwrap();
    state.media.delete_media(user_id, &record.sha256, &record.filename, &MediaSource::Upload).await.unwrap();

    let fetched = state.media.get_media(user_id, &record.sha256, &record.filename, &MediaSource::Upload).await.unwrap();
    assert!(fetched.is_none());
}

#[tokio::test]
async fn delete_nonexistent_media_returns_not_found() {
    let state = setup().await;
    let password: common::password::Password = "testpass123".parse().unwrap();
    let username: common::username::Username = "testuser".parse().unwrap();
    let user_id = state.users.create_user(&username, &password, None, false).await.unwrap();

    let err = state.media.delete_media(user_id, "nonexistent", "file.txt", &MediaSource::Upload).await.unwrap_err();
    assert!(matches!(err, DeleteMediaError::NotFound));
}

#[tokio::test]
async fn get_user_upload_usage() {
    let state = setup().await;
    let password: common::password::Password = "testpass123".parse().unwrap();
    let username: common::username::Username = "testuser".parse().unwrap();
    let user_id = state.users.create_user(&username, &password, None, false).await.unwrap();

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(usage, 0);

    let record = MediaRecord {
        user_id,
        sha256: "aaaa1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234".to_string(),
        filename: "a.jpg".to_string(),
        source: MediaSource::Upload,
        content_type: "image/jpeg".to_string(),
        size_bytes: 1000,
        source_url: None,
        created_at: Utc::now(),
    };
    state.media.create_media(&record).await.unwrap();

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(usage, 1000);
}

#[tokio::test]
async fn user_config_get_set_delete() {
    let state = setup().await;
    let password: common::password::Password = "testpass123".parse().unwrap();
    let username: common::username::Username = "testuser".parse().unwrap();
    let user_id = state.users.create_user(&username, &password, None, false).await.unwrap();

    // Initially unset
    let val = state.user_config.get(user_id, USER_MEDIA_CACHE_POLICY_KEY).await.unwrap();
    assert!(val.is_none());

    // Set
    state.user_config.set(user_id, USER_MEDIA_CACHE_POLICY_KEY, "eager").await.unwrap();
    let val = state.user_config.get(user_id, USER_MEDIA_CACHE_POLICY_KEY).await.unwrap();
    assert_eq!(val.as_deref(), Some("eager"));

    // Overwrite
    state.user_config.set(user_id, USER_MEDIA_CACHE_POLICY_KEY, "lazy").await.unwrap();
    let val = state.user_config.get(user_id, USER_MEDIA_CACHE_POLICY_KEY).await.unwrap();
    assert_eq!(val.as_deref(), Some("lazy"));

    // Delete
    state.user_config.delete(user_id, USER_MEDIA_CACHE_POLICY_KEY).await.unwrap();
    let val = state.user_config.get(user_id, USER_MEDIA_CACHE_POLICY_KEY).await.unwrap();
    assert!(val.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -E 'test(storage_media)' -E 'test(user_config)'`
Expected: FAIL (trait methods are `todo!()`)

- [ ] **Step 3: Implement `SqliteMediaStorage`**

```rust
// In server/src/storage/sqlite.rs — add after existing implementations

pub struct SqliteMediaStorage {
    pool: SqlitePool,
}

impl SqliteMediaStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MediaStorage for SqliteMediaStorage {
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError> {
        let result = sqlx::query(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        )
        .bind(record.user_id)
        .bind(&record.sha256)
        .bind(&record.filename)
        .bind(record.source.as_str())
        .bind(&record.content_type)
        .bind(record.size_bytes)
        .bind(&record.source_url)
        .bind(record.created_at)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) if e.as_database_error().is_some_and(|de| de.is_unique_violation()) => {
                Err(CreateMediaError::AlreadyExists)
            }
            Err(e) => Err(CreateMediaError::Internal(e)),
        }
    }

    async fn get_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row: Option<(i64, String, String, String, String, i64, Option<String>, DateTime<Utc>)> =
            sqlx::query_as(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4"
            )
            .bind(user_id)
            .bind(sha256)
            .bind(filename)
            .bind(source.as_str())
            .fetch_optional(&self.pool)
            .await?;

        row.map(|r| build_media_record(r)).transpose()
    }

    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
        limit: u32,
        offset: u32,
    ) -> sqlx::Result<Vec<MediaRecord>> {
        let rows: Vec<(i64, String, String, String, String, i64, Option<String>, DateTime<Utc>)> =
            match source {
                Some(s) => {
                    sqlx::query_as(
                        "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                         FROM media WHERE user_id = $1 AND source = $2
                         ORDER BY created_at DESC LIMIT $3 OFFSET $4"
                    )
                    .bind(user_id)
                    .bind(s.as_str())
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(&self.pool)
                    .await?
                }
                None => {
                    sqlx::query_as(
                        "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                         FROM media WHERE user_id = $1
                         ORDER BY created_at DESC LIMIT $2 OFFSET $3"
                    )
                    .bind(user_id)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(&self.pool)
                    .await?
                }
            };

        rows.into_iter().map(build_media_record).collect()
    }

    async fn delete_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError> {
        let result = sqlx::query(
            "DELETE FROM media WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4"
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .execute(&self.pool)
        .await
        .map_err(DeleteMediaError::Internal)?;

        if result.rows_affected() == 0 {
            return Err(DeleteMediaError::NotFound);
        }
        Ok(())
    }

    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM media WHERE user_id = $1 AND source = 'upload'"
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    async fn find_by_hash(
        &self,
        sha256: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row: Option<(i64, String, String, String, String, i64, Option<String>, DateTime<Utc>)> =
            sqlx::query_as(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media WHERE sha256 = $1 AND source = $2 LIMIT 1"
            )
            .bind(sha256)
            .bind(source.as_str())
            .fetch_optional(&self.pool)
            .await?;

        row.map(build_media_record).transpose()
    }
}
```

Add the helper function in `server/src/storage/mod.rs`:

```rust
pub(super) type MediaRow = (i64, String, String, String, String, i64, Option<String>, DateTime<Utc>);

pub(super) fn build_media_record(
    (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at): MediaRow,
) -> sqlx::Result<MediaRecord> {
    let source: MediaSource = source
        .parse()
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    Ok(MediaRecord {
        user_id,
        sha256,
        filename,
        source,
        content_type,
        size_bytes,
        source_url,
        created_at,
    })
}
```

- [ ] **Step 4: Implement `SqliteUserConfigStorage`**

```rust
pub struct SqliteUserConfigStorage {
    pool: SqlitePool,
}

impl SqliteUserConfigStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserConfigStorage for SqliteUserConfigStorage {
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT value FROM user_config WHERE user_id = $1 AND key = $2"
        )
        .bind(user_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(v,)| v))
    }

    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO user_config (user_id, key, value) VALUES ($1, $2, $3)
             ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value"
        )
        .bind(user_id)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM user_config WHERE user_id = $1 AND key = $2")
            .bind(user_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
```

- [ ] **Step 5: Implement Postgres versions**

Mirror the SQLite implementations in `server/src/storage/postgres.rs` with the same struct names prefixed with `Postgres` instead of `Sqlite`. The SQL is identical since both use `$1` parameter syntax via sqlx.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo nextest run -E 'test(storage_media)' -E 'test(user_config)'`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add server/src/storage/ server/tests/storage_media.rs
git commit -m "M7.5: implement MediaStorage and UserConfigStorage for SQLite and Postgres

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 6: Upload Handler

**Files:**
- Create: `server/src/media.rs`
- Modify: `server/src/lib.rs` (add `pub mod media;`, add route)
- Modify: `server/Cargo.toml` (add `tokio` fs features if needed, `mime_guess` or similar)

- [ ] **Step 1: Write integration test for upload endpoint**

Create `server/tests/media_upload.rs`:

```rust
// server/tests/media_upload.rs
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;
use common::storage::MediaSource;

mod helpers; // if you have test helpers, or inline setup

async fn setup_app() -> (axum::Router, i64, String) {
    // Set up in-memory DB, create a user, get a session token
    let opts: jaunder::storage::DbConnectOptions = "sqlite::memory:".parse().unwrap();
    let state = jaunder::storage::open_database(&opts).await.unwrap();

    let password: common::password::Password = "testpass123".parse().unwrap();
    let username: common::username::Username = "testuser".parse().unwrap();
    let user_id = state.users.create_user(&username, &password, None, false).await.unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();

    let router = jaunder::create_router(
        leptos::config::LeptosOptions::default(),
        state,
        false,
    );
    (router, user_id, token)
}

#[tokio::test]
async fn upload_file_succeeds() {
    let (app, _user_id, token) = setup_app().await;

    let boundary = "----boundary";
    let body = format!(
        "------boundary\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"test.jpg\"\r\n\
         Content-Type: image/jpeg\r\n\
         \r\n\
         fake jpeg content\r\n\
         ------boundary--\r\n"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
                .header(header::COOKIE, format!("session={token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn upload_requires_auth() {
    let (app, _, _) = setup_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(header::CONTENT_TYPE, "multipart/form-data; boundary=----b")
                .body(Body::from("------b--\r\n"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Implement the upload handler**

```rust
// server/src/media.rs
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    Extension,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use common::media::{media_url, sanitize_filename};
use common::storage::{
    AppState, CreateMediaError, MediaRecord, MediaSource, MediaStorage, SiteConfigStorage,
    DEFAULT_MAX_FILE_SIZE_BYTES, DEFAULT_USER_QUOTA_BYTES, MEDIA_MAX_FILE_SIZE_BYTES_KEY,
    MEDIA_USER_QUOTA_BYTES_KEY,
};
use web::auth::AuthUser;

#[derive(Serialize)]
pub struct UploadResponse {
    pub sha256: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub url: String,
}

pub async fn upload_handler(
    auth: AuthUser,
    Extension(state): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, StatusCode> {
    let max_file_size = get_max_file_size(&*state.site_config).await;
    let user_quota = get_user_quota(&*state.site_config).await;

    let field = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .ok_or(StatusCode::BAD_REQUEST)?;

    let original_filename = field
        .file_name()
        .unwrap_or("upload")
        .to_owned();
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();

    let filename = sanitize_filename(&original_filename);
    if filename.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Get storage path from environment or default
    let storage_path = std::env::var("JAUNDER_STORAGE_PATH").unwrap_or_else(|_| "./data".to_owned());
    let media_base = PathBuf::from(&storage_path).join("media");
    let tmp_dir = media_base.join("tmp");
    fs::create_dir_all(&tmp_dir).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Stream to temp file, compute SHA256 incrementally
    let tmp_path = tmp_dir.join(format!("{}", uuid::Uuid::new_v4()));
    let mut file = fs::File::create(&tmp_path).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut hasher = Sha256::new();
    let mut size: i64 = 0;

    // Stream chunks from the multipart field
    let mut stream = field;
    loop {
        match stream.chunk().await {
            Ok(Some(chunk)) => {
                size += chunk.len() as i64;
                if size > max_file_size {
                    drop(file);
                    let _ = fs::remove_file(&tmp_path).await;
                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
                hasher.update(&chunk);
                file.write_all(&chunk).await.map_err(|_| {
                    // Clean up on write error
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            }
            Ok(None) => break,
            Err(_) => {
                drop(file);
                let _ = fs::remove_file(&tmp_path).await;
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }
    file.flush().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(file);

    // Check user quota
    let current_usage = state.media.get_user_upload_usage(auth.user_id).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if current_usage + size > user_quota {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }

    let sha256_hex = format!("{:x}", hasher.finalize());
    let prefix = &sha256_hex[..4];
    let target_dir = media_base.join("upload").join(prefix).join(&sha256_hex);
    let target_path = target_dir.join(&filename);

    // Create target directory structure
    fs::create_dir_all(&target_dir).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if target_path.exists() {
        // File already exists with same content — no filesystem work needed
        let _ = fs::remove_file(&tmp_path).await;
    } else if let Ok(mut entries) = fs::read_dir(&target_dir).await {
        // Check if any file already exists in the hash dir (same content, different name)
        let mut existing_file: Option<PathBuf> = None;
        while let Ok(Some(entry)) = entries.next_entry().await {
            existing_file = Some(entry.path());
            break;
        }
        if let Some(existing) = existing_file {
            // Hard-link from existing file (same content)
            fs::hard_link(&existing, &target_path).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let _ = fs::remove_file(&tmp_path).await;
        } else {
            // Empty dir (shouldn't happen), move temp file in
            fs::rename(&tmp_path, &target_path).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
    } else {
        // Move temp file to target
        fs::rename(&tmp_path, &target_path).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // Insert DB record (file-first ordering: filesystem succeeded, now record it)
    let record = MediaRecord {
        user_id: auth.user_id,
        sha256: sha256_hex.clone(),
        filename: filename.clone(),
        source: MediaSource::Upload,
        content_type: content_type.clone(),
        size_bytes: size,
        source_url: None,
        created_at: chrono::Utc::now(),
    };

    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(CreateMediaError::Internal(_)) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    }

    let url = media_url("upload", &sha256_hex, &filename);
    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            sha256: sha256_hex,
            filename,
            content_type,
            size_bytes: size,
            url,
        }),
    ))
}

async fn get_max_file_size(site_config: &dyn SiteConfigStorage) -> i64 {
    site_config
        .get(MEDIA_MAX_FILE_SIZE_BYTES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_FILE_SIZE_BYTES)
}

async fn get_user_quota(site_config: &dyn SiteConfigStorage) -> i64 {
    site_config
        .get(MEDIA_USER_QUOTA_BYTES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_USER_QUOTA_BYTES)
}
```

- [ ] **Step 3: Add route to `server/src/lib.rs`**

In `create_router`, before the `.nest_service("/style", serve_assets)` line, add:

```rust
.route("/media/upload", axum::routing::post(crate::media::upload_handler))
```

- [ ] **Step 4: Add `pub mod media;` to `server/src/lib.rs`**

- [ ] **Step 5: Add dependencies to `server/Cargo.toml`**

Add `uuid` (for temp file naming):
```toml
uuid = { version = "1", features = ["v4"] }
```

The `sha2` crate is already a workspace dependency.

- [ ] **Step 6: Run tests**

Run: `cargo nextest run -E 'test(media_upload)'`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add server/src/media.rs server/src/lib.rs server/Cargo.toml server/tests/media_upload.rs
git commit -m "M7.6: add media upload handler with streaming SHA256 and quota enforcement

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 7: Media Serving Handler

**Files:**
- Modify: `server/src/media.rs` (add `serve_handler`)
- Modify: `server/src/lib.rs` (add serving route)

- [ ] **Step 1: Write integration test for media serving**

Add to `server/tests/media_upload.rs` (or create `server/tests/media_serve.rs`):

```rust
#[tokio::test]
async fn serve_uploaded_file() {
    // Upload a file first, then GET the returned URL
    let (app, _user_id, token) = setup_app().await;
    // ... (upload a file, parse the response JSON to get url)
    // Then GET the url without auth
    let response = app
        .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("cache-control").unwrap(),
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn serve_nonexistent_returns_404() {
    let (app, _, _) = setup_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/media/upload/aaaa/aaaa1234.../nonexistent.jpg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Implement the serving handler**

```rust
// In server/src/media.rs

use axum::extract::Path as AxumPath;
use axum::http::{header, HeaderMap, HeaderValue};
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use axum::body::Body;

pub async fn serve_handler(
    AxumPath((source, prefix, hash, filename)): AxumPath<(String, String, String, String)>,
    headers: HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    // Validate source
    if source != "upload" && source != "cached" {
        return Err(StatusCode::NOT_FOUND);
    }

    // Validate prefix matches hash
    if hash.len() < 4 || &hash[..4] != prefix {
        return Err(StatusCode::NOT_FOUND);
    }

    let storage_path = std::env::var("JAUNDER_STORAGE_PATH").unwrap_or_else(|_| "./data".to_owned());
    let file_path = PathBuf::from(&storage_path)
        .join("media")
        .join(&source)
        .join(&prefix)
        .join(&hash)
        .join(&filename);

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // ETag support — check If-None-Match
    if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH) {
        if let Ok(etag_val) = if_none_match.to_str() {
            let etag_clean = etag_val.trim_matches('"');
            if etag_clean == hash {
                return Ok(StatusCode::NOT_MODIFIED.into_response());
            }
        }
    }

    // Determine content type from DB or fallback to extension
    let content_type = {
        let source_enum: MediaSource = source.parse().map_err(|_| StatusCode::NOT_FOUND)?;
        // Try DB lookup (any user with this hash+filename+source)
        match state.media.find_by_hash(&hash, &source_enum).await {
            Ok(Some(record)) => record.content_type,
            _ => common::media::detect_content_type(&filename).to_owned(),
        }
    };

    let disposition = if common::media::should_inline(&content_type) {
        "inline".to_owned()
    } else {
        format!("attachment; filename=\"{filename}\"")
    };

    let file = File::open(&file_path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    response_headers.insert(header::ETAG, HeaderValue::from_str(&format!("\"{hash}\"")).unwrap_or(HeaderValue::from_static("")));
    response_headers.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).unwrap_or(HeaderValue::from_static("attachment")));

    Ok((response_headers, body).into_response())
}
```

- [ ] **Step 3: Add serving route to `server/src/lib.rs`**

```rust
.route("/media/{source}/{prefix}/{hash}/{filename}", axum::routing::get(crate::media::serve_handler))
```

- [ ] **Step 4: Add `tokio-util` dependency to `server/Cargo.toml`**

```toml
tokio-util = { version = "0.7", features = ["io"] }
```

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -E 'test(media_serve)' -E 'test(serve)'`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add server/src/media.rs server/src/lib.rs server/Cargo.toml server/tests/
git commit -m "M7.7: add media serving handler with caching headers and ETag support

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 8: Management Server Functions

**Files:**
- Create: `web/src/media.rs`
- Modify: `web/src/lib.rs` (add `pub mod media;`)

- [ ] **Step 1: Implement server functions**

```rust
// web/src/media.rs
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
use crate::error::WebResult;
#[cfg(feature = "ssr")]
use crate::error::{InternalError, InternalResult};
#[cfg(feature = "ssr")]
use common::storage::{
    AppState, MediaSource, MediaStorage, SiteConfigStorage,
    DEFAULT_MAX_FILE_SIZE_BYTES, DEFAULT_USER_QUOTA_BYTES,
    MEDIA_MAX_FILE_SIZE_BYTES_KEY, MEDIA_USER_QUOTA_BYTES_KEY,
};
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub sha256: String,
    pub filename: String,
    pub source: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub url: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaUsage {
    pub used_bytes: i64,
    pub quota_bytes: i64,
    pub max_file_size_bytes: i64,
}

#[server]
pub async fn list_my_media(
    source: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> WebResult<Vec<MediaItem>> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let source_filter = source
        .as_deref()
        .map(|s| s.parse::<MediaSource>())
        .transpose()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let records = state
        .media
        .list_media(
            auth.user_id,
            source_filter.as_ref(),
            limit.unwrap_or(50),
            offset.unwrap_or(0),
        )
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let items = records
        .into_iter()
        .map(|r| {
            let url = common::media::media_url(r.source.as_str(), &r.sha256, &r.filename);
            MediaItem {
                sha256: r.sha256,
                filename: r.filename,
                source: r.source.to_string(),
                content_type: r.content_type,
                size_bytes: r.size_bytes,
                url,
                created_at: r.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(items)
}

#[server]
pub async fn media_usage() -> WebResult<MediaUsage> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let used_bytes = state
        .media
        .get_user_upload_usage(auth.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let quota_bytes = state
        .site_config
        .get(MEDIA_USER_QUOTA_BYTES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_USER_QUOTA_BYTES);

    let max_file_size_bytes = state
        .site_config
        .get(MEDIA_MAX_FILE_SIZE_BYTES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_FILE_SIZE_BYTES);

    Ok(MediaUsage {
        used_bytes,
        quota_bytes,
        max_file_size_bytes,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMediaResult {
    pub deleted: bool,
    pub referenced_in_posts: Vec<i64>,
}

#[server]
pub async fn delete_media(
    sha256: String,
    filename: String,
    source: String,
    force: Option<bool>,
) -> WebResult<DeleteMediaResult> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let source_enum: MediaSource = source
        .parse()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Scan user's posts for references to this media URL
    let url = common::media::media_url(source_enum.as_str(), &sha256, &filename);
    let posts = state.posts.list_published_by_user(
        &auth.username, None, 1000
    ).await.map_err(|e| ServerFnError::new(e.to_string()))?;
    let drafts = state.posts.list_drafts_by_user(
        auth.user_id, None, 1000
    ).await.map_err(|e| ServerFnError::new(e.to_string()))?;

    let referenced_in: Vec<i64> = posts.iter().chain(drafts.iter())
        .filter(|p| p.body.contains(&url) || p.rendered_html.contains(&url))
        .map(|p| p.post_id)
        .collect();

    if !referenced_in.is_empty() && !force.unwrap_or(false) {
        return Ok(DeleteMediaResult {
            deleted: false,
            referenced_in_posts: referenced_in,
        });
    }

    state
        .media
        .delete_media(auth.user_id, &sha256, &filename, &source_enum)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Leave file on disk — orphan cleanup is out of scope (future GC command)

    Ok(DeleteMediaResult {
        deleted: true,
        referenced_in_posts: referenced_in,
    })
}
```

- [ ] **Step 2: Add `pub mod media;` to `web/src/lib.rs`**

- [ ] **Step 3: Write integration tests for server functions**

Create `server/tests/media_server_fns.rs` testing the server functions via HTTP (similar to how other server function tests are done in the project).

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -E 'test(media)'`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add web/src/media.rs web/src/lib.rs server/tests/
git commit -m "M7.8: add media management server functions (list, delete, usage)

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 9: Media Management UI Page

**Files:**
- Create: `web/src/pages/media.rs`
- Modify: `web/src/pages/mod.rs` (add `pub mod media;`)
- Modify: `web/src/pages/mod.rs` or routing (add route for `/media` page)

- [ ] **Step 1: Create the media management page component**

```rust
// web/src/pages/media.rs
use leptos::prelude::*;
use crate::media::{list_my_media, media_usage, delete_media, MediaItem, MediaUsage};

#[component]
pub fn MediaPage() -> impl IntoView {
    let usage = Resource::new(|| (), |_| media_usage());
    let media_list = Resource::new(|| (), |_| list_my_media(None, None, None));

    view! {
        <h1>"Media"</h1>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || usage.get().map(|result| match result {
                Ok(u) => view! {
                    <div class="media-usage">
                        <p>{format!("Storage: {} / {} ({:.1}%)",
                            format_bytes(u.used_bytes),
                            format_bytes(u.quota_bytes),
                            (u.used_bytes as f64 / u.quota_bytes as f64) * 100.0
                        )}</p>
                        <p>{format!("Max file size: {}", format_bytes(u.max_file_size_bytes))}</p>
                    </div>
                }.into_any(),
                Err(_) => view! { <p class="error">"Failed to load usage"</p> }.into_any(),
            })}
        </Suspense>
        <Suspense fallback=|| view! { <p>"Loading media..."</p> }>
            {move || media_list.get().map(|result| match result {
                Ok(items) => view! {
                    <MediaList items=items/>
                }.into_any(),
                Err(_) => view! { <p class="error">"Failed to load media"</p> }.into_any(),
            })}
        </Suspense>
    }
}

#[component]
fn MediaList(items: Vec<MediaItem>) -> impl IntoView {
    if items.is_empty() {
        return view! { <p>"No media uploaded yet."</p> }.into_any();
    }
    view! {
        <table>
            <thead>
                <tr>
                    <th>"Filename"</th>
                    <th>"Type"</th>
                    <th>"Size"</th>
                    <th>"Source"</th>
                    <th>"Actions"</th>
                </tr>
            </thead>
            <tbody>
                {items.into_iter().map(|item| view! {
                    <MediaRow item=item/>
                }).collect_view()}
            </tbody>
        </table>
    }.into_any()
}

#[component]
fn MediaRow(item: MediaItem) -> impl IntoView {
    let delete_action = ServerAction::<DeleteMedia>::new();
    let sha256 = item.sha256.clone();
    let filename = item.filename.clone();
    let source = item.source.clone();

    view! {
        <tr>
            <td><a href={item.url.clone()} target="_blank">{&item.filename}</a></td>
            <td>{&item.content_type}</td>
            <td>{format_bytes(item.size_bytes)}</td>
            <td>{&item.source}</td>
            <td>
                <ActionForm action=delete_action>
                    <input type="hidden" name="sha256" value={sha256}/>
                    <input type="hidden" name="filename" value={filename}/>
                    <input type="hidden" name="source" value={source}/>
                    <button type="submit">"Delete"</button>
                </ActionForm>
            </td>
        </tr>
    }
}

fn format_bytes(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1_073_741_824 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    }
}
```

- [ ] **Step 2: Add route to the app**

Add the `/media` route to the application's routing in `web/src/pages/mod.rs` (following existing patterns for authenticated pages).

- [ ] **Step 3: Run full verify**

Run: `scripts/verify`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/media.rs web/src/pages/mod.rs
git commit -m "M7.9: add media management UI page

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 10: Lazy Proxy Endpoint (Stub for Future Use)

**Files:**
- Modify: `server/src/media.rs` (add `proxy_handler`)
- Modify: `server/src/lib.rs` (add route)

This endpoint is called by future milestones (M9, M17). We implement the handler now so the infrastructure is in place, but it won't be actively called until those milestones land.

- [ ] **Step 1: Write integration test**

```rust
#[tokio::test]
async fn proxy_requires_auth() {
    let (app, _, _) = setup_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/media/proxy?url=http%3A%2F%2Fexample.com%2Fimg.jpg&user_id=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Implement the proxy handler**

```rust
// In server/src/media.rs

use axum::extract::Query;

#[derive(serde::Deserialize)]
pub struct ProxyParams {
    pub url: String,
    pub user_id: i64,
}

pub async fn proxy_handler(
    auth: AuthUser,
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<ProxyParams>,
) -> Result<impl IntoResponse, StatusCode> {
    // User must match the user_id parameter
    if auth.user_id != params.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // Check if already cached — look up by source_url
    // For now, redirect to the remote URL (full caching implementation comes with M9/M17)
    // This gives us the endpoint structure; the download-and-cache logic will be filled in later.

    // TODO(M9/M17): Implement actual fetch, cache, and redirect to local URL
    // For now, pass through to remote URL
    Ok((
        StatusCode::TEMPORARY_REDIRECT,
        [(header::LOCATION, params.url)],
    ))
}
```

- [ ] **Step 3: Add route**

```rust
.route("/media/proxy", axum::routing::get(crate::media::proxy_handler))
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -E 'test(proxy)'`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add server/src/media.rs server/src/lib.rs server/tests/
git commit -m "M7.10: add lazy proxy endpoint stub for future cache integration

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 11: End-to-End Tests

**Files:**
- Create: `end2end/tests/media.spec.ts`

- [ ] **Step 1: Write e2e test for media upload and serving**

```typescript
// end2end/tests/media.spec.ts
import { test, expect } from "@playwright/test";
import { goto, register, login } from "./helpers";

test.describe("Media upload and serving", () => {
    test("authenticated user can upload and access media", async ({ page }) => {
        const username = await register(page);

        // Upload via the API directly
        const fileContent = Buffer.from("fake image content for testing");
        const response = await page.request.post(`${BASE_URL}/media/upload`, {
            multipart: {
                file: {
                    name: "test-image.jpg",
                    mimeType: "image/jpeg",
                    buffer: fileContent,
                },
            },
        });
        expect(response.status()).toBe(201);

        const json = await response.json();
        expect(json.sha256).toBeTruthy();
        expect(json.filename).toBe("test-image.jpg");
        expect(json.url).toContain("/media/upload/");

        // Access the served file (public, no auth needed)
        const serveResponse = await page.request.get(`${BASE_URL}${json.url}`);
        expect(serveResponse.status()).toBe(200);
        expect(serveResponse.headers()["cache-control"]).toBe(
            "public, max-age=31536000, immutable"
        );
    });

    test("unauthenticated upload returns 401", async ({ page }) => {
        const response = await page.request.post(`${BASE_URL}/media/upload`, {
            multipart: {
                file: {
                    name: "test.jpg",
                    mimeType: "image/jpeg",
                    buffer: Buffer.from("data"),
                },
            },
        });
        expect(response.status()).toBe(401);
    });
});
```

- [ ] **Step 2: Run e2e tests**

Run: `nix flake check` (or the e2e subset)
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add end2end/tests/media.spec.ts
git commit -m "M7.11: add end-to-end tests for media upload and serving

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 12: Update `init_storage` for Media Subdirectories

**Files:**
- Modify: `server/src/storage/mod.rs` (update `init_storage`)

- [ ] **Step 1: Update `init_storage` to create media subdirectories**

```rust
pub fn init_storage(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)?;
    std::fs::create_dir_all(path.join("media"))?;
    std::fs::create_dir_all(path.join("media/upload"))?;
    std::fs::create_dir_all(path.join("media/cached"))?;
    std::fs::create_dir_all(path.join("media/tmp"))?;
    std::fs::create_dir_all(path.join("backups"))?;
    Ok(())
}
```

- [ ] **Step 2: Update the existing test**

The `new_path_created_with_subdirs` test should also assert the new directories exist.

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -E 'test(init_storage)'`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add server/src/storage/mod.rs
git commit -m "M7.12: create media subdirectories (upload, cached, tmp) on init

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Task 13: Final Verification

- [ ] **Step 1: Run full test suite**

Run: `scripts/verify`
Expected: All tests pass.

- [ ] **Step 2: Run e2e tests**

Run: `nix flake check`
Expected: PASS

- [ ] **Step 3: Check off M7 milestone items in `docs/milestones/M7.md`**

Update the milestone document to mark completed items.
