# Plan — Media upload as a multipart `#[server]` fn, via relocating `MediaManager` to `storage` (#517)

Spec: `docs/superpowers/specs/2026-07-23-issue-517-media-upload-server-fn.md`.
This file is the **how**; read the spec for the **what/why**. Each task is one
commit that leaves the tree compiling and `cargo xtask check`-green; the
strangler ordering (old endpoint alive until Task 4) is the spine — do not
reorder.

## Review header

**Goal.** Replace the bespoke browser-`fetch` → axum-multipart-handler upload
glue with a multipart `#[server]` fn, unblocked by relocating `MediaManager`
from `server` to `storage` (where `web` can construct it directly, no DI seam).

**Scope — in.** Move `MediaManager`/`MediaError`/`upload_outcome` + helpers to
`storage` (and `UploadResponse` to `common::media`), decoupled from
`web`/`axum`; the metric funnel; the web `#[server]` fn; client switch; delete
the old handler/route + client glue; repoint AtomPub; rewrite two e2e tests;
dead-dep sweep (`web-sys` fetch features, `server` `uuid`); add `bytes`

- `uuid` to `storage`; enable `server_fn`'s `multipart` codec.

**Scope — out.** Content-addressing / dedupe / quota / on-disk layout
(byte-for-byte preserved); the serve/proxy routes; AtomPub protocol handling;
the browser `FormData`/`File` assembly in `component.rs`; any new upload
scenarios or `UploadResponse` DTO consolidation.

**Tasks.**

1. `refactor(storage)`: relocate `MediaManager` (+ `MediaError` pub,
   `UploadResponse`, `upload_outcome`, helpers, streaming/dedupe) to `storage`,
   decouple to `UserId` + generic byte stream, funnel metrics; repoint both
   server callers + AtomPub `HandlerError` conversion; keep `map_error` in
   `server/src/media.rs`; move streaming tests. **The big, atomic one.**
2. `feat(web)`: add the `upload_media` multipart `#[server]` fn + register it;
   enable `server_fn` `multipart`; wire `storage_path` via
   `leptos_axum::extract`. Old endpoint still present.
3. `refactor(web)`: switch the client to `upload_media`; delete `upload_file` /
   `extract_upload_url`; sweep dead `web-sys` fetch features.
4. `refactor(server)`: delete `upload_handler` + `POST /media/upload` route;
   drop `uuid` from `server`; rewrite the two direct-POST e2e tests.
5. Full gate: `cargo xtask validate` green, `git status --porcelain` empty.

**Key risks / decisions.**

- **Task 1 atomicity.** `upload`/`upload_bytes` change signature (`&AuthUser` →
  `UserId`) and `stream_to_temp` changes input type. Every caller (browser
  `upload_handler`, AtomPub `collection_post`, the `HandlerError`
  `From<anyhow::Error>` impl, and the moved tests) must update in the same
  commit or the tree won't compile. One large commit is unavoidable and correct
  here.
- **`storage_path` mechanism (decided).** Use
  `leptos_axum::extract::<Extension<Arc<PathBuf>>>()` inside the `#[server]` fn
  — `storage_path` is already layered as an axum `Extension`
  (`server/src/lib.rs:126`), so **no** change to the `/api` `additional_context`
  closure is needed. (The `provide_context` alternative would force the
  per-request closure to capture a `storage_path` clone it does not hold today.)
- **Metric funnel.** Success metrics already fire inside `finalize_upload`; the
  _failure_ metric currently fires in `map_error` at the boundary. Move failure
  emission **into** `MediaManager` (wrap `upload`/`upload_bytes` with an inner
  fn; emit `media_upload` on the `Err` path) so exactly one `media_upload*`
  fires per upload and `map_error` becomes a pure `MediaError → StatusCode` map.
- **ADR-0053 dual-backend hazard.** The moved DB-backed tests bind the whole
  `TestEnv` (`let TestEnv { state, base } = backend.setup().await;`) and run
  `#[apply(backends)]`; pure/dedup tests use `storage`'s `Mock*Storage` + a bare
  `TempDir`, no DB.
- **`storage` must not gain `web`/`server`/`axum`.** Enforced by an explicit
  grep in the Task 1 Verify block.
- **`server_fn` codec feature reaches wasm.** `MultipartData` is named in the fn
  signature compiled for **both** the client (wasm) and server builds, so the
  `multipart` feature must be enabled unconditionally, not gated to `web`'s
  `server` feature (see Task 2).

## Global Constraints

- **Fork point:** `wt-base-issue-517`. Review the whole branch with
  `git diff wt-base-issue-517..HEAD`.
- **Per-commit gate:** `cargo xtask check` (run via
  `devtool run -- cargo xtask check`). **Full gate:** `cargo xtask validate`.
- Commit subjects: `type(scope): subject (#517)`; **no** `Co-Authored-By`
  trailer.
- Storage tests are dual-backend (ADR-0053) — bind the whole `TestEnv`. Follow
  `CONTRIBUTING.md` (backend parity, coverage policy, verify ladder).
- No commit without explicit user approval; request review first.
- **Agentic workers:** dispatch tasks via `jaunder-dispatch`; execute the
  per-task loop via `jaunder-iterate`.

---

## Task 1 — Relocate `MediaManager` to `storage`; repoint server callers; keep `map_error` in `server`

The big one. Splits into: (1a) workspace/crate deps, (1b) the new `storage`
module, (1c) `map_error`'s new home in `server`, (1d) repoint the three server
call sites, (1e) move the tests.

### 1a. Dependencies

**Files:** `Cargo.toml` (workspace), `storage/Cargo.toml`.

Add to `[workspace.dependencies]` in the root `Cargo.toml` (alphabetical-ish,
near the existing entries):

```toml
bytes = "1"
uuid = { version = "1", features = ["v4"] }
```

Add to `storage/Cargo.toml` `[dependencies]`:

```toml
bytes.workspace = true
uuid.workspace = true
```

(`futures-util` is already a `storage` dep — used for the generic stream driver.
`host`, `sha2`, `tokio`, `chrono`, `thiserror`, `anyhow` are all already
present.)

### 1a-bis. Define `UploadResponse` in `common::media`

**Files:** `common`'s media module (the file backing `common::media` —
`common/src/media.rs` or `common/src/media/mod.rs`; read to confirm), where
`ByteSize`/`ContentHash`/ `ContentType`/`Filename` already live.

`UploadResponse` is the `#[server]` fn's **return type**, so it must be nameable
on the wasm client build — where `storage` is not compiled (`storage` is a
`server`-gated `web` dep). `common` is ungated and reachable by `storage` +
`web` (both targets) + `server`, so define it there (deriving
`Serialize + Deserialize` for the wire round-trip; it was `Serialize`-only in
`server`). Every field is already a `common` type:

```rust
/// The metadata returned on a successful media upload — the server-fn wire response
/// (#517), moved here from `server` so it is nameable on the wasm client. `storage`'s
/// `MediaManager` returns it directly; `web`'s `upload_media` fn returns it; AtomPub
/// serializes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResponse {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub content_type: ContentType,
    pub size_bytes: ByteSize,
    pub url: String,
}
```

(`common` already has `serde` — the sibling media types derive it. Re-export
from `common::media` so `common::media::UploadResponse` resolves, matching how
`web`/`storage` already import `common::media::{…}`.)

### 1b. New module `storage/src/media_manager.rs`

**Files:** new `storage/src/media_manager.rs`; `storage/src/lib.rs` (`mod` +
re-export).

In `storage/src/lib.rs`, add `mod media_manager;` (alphabetically, after
`mod media;`) and a re-export near the other `pub use`s:

```rust
pub use media_manager::{MediaError, MediaManager};
```

(`UploadResponse` is **not** re-exported from `storage` — it lives in
`common::media`, step 1a-bis.)

Create `storage/src/media_manager.rs`. It is the moved body of
`server/src/media_manager.rs` with four mechanical changes: (i) `MediaError`
made `pub`; (ii) `UploadResponse` moved in (deriving `Serialize + Deserialize`);
(iii) auth decoupled to `UserId`; (iv) `stream_to_temp` generalized to a byte
stream, and the metric funnel. `map_error` does **not** move here (it needs
`axum::http::StatusCode`).

Imports change from `crate::media::UploadResponse` / `web::auth::AuthUser` /
`axum::…::Field` to storage-local names. Full file:

```rust
//! Content-addressed media upload service: streams an upload to a hashed,
//! dedup'd on-disk path, enforces per-file and per-user limits, and records the
//! result. Relocated from `server` (#517) so a `web` `#[server]` fn can construct
//! it directly — its work is persistence and its deps are all `storage`'s.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use chrono::Utc;
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use common::ids::UserId;
use common::media::{
    detect_content_type, media_path, media_url, ByteSize, ContentHash, ContentType, Filename,
    MaxFileSize, MediaSource, UploadResponse, UserQuota,
};

use crate::{CreateMediaError, MediaRecord, MediaStorage, SiteConfigStorage};

/// A media upload failure with a bounded, client-mappable classification. `pub`
/// so the HTTP boundary in `server` can `downcast_ref` it to a `StatusCode`
/// (`server::media::map_error`).
#[derive(Debug, Error)]
pub enum MediaError {
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Payload too large")]
    PayloadTooLarge,
    #[error("Insufficient storage")]
    InsufficientStorage,
    #[error("Internal server error: {0}")]
    Internal(String),
}

// `UploadResponse` is defined in `common::media` (step 1a-bis), not here — it is the
// `#[server]` fn's return type, which must be nameable on the wasm client build where
// `storage` is not compiled (`storage` is a `server`-gated `web` dep). `common` is
// ungated and reachable by storage + web (both targets) + server, so the manager
// returns it directly with no mapping layer.

pub struct MediaManager {
    media: Arc<dyn MediaStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    storage_path: Arc<PathBuf>,
}

/// File metadata for upload finalization.
#[derive(Debug)]
struct UploadMetadata {
    filename: Filename,
    content_type: ContentType,
    sha256_hex: ContentHash,
    size_bytes: i64,
}

impl MediaManager {
    #[must_use]
    pub fn new(
        media: Arc<dyn MediaStorage>,
        site_config: Arc<dyn SiteConfigStorage>,
        storage_path: Arc<PathBuf>,
    ) -> Self {
        Self {
            media,
            site_config,
            storage_path,
        }
    }

    /// Streams a multipart upload to a content-addressed, dedup'd path and records
    /// it. `filename`/`content_type` are extracted by the caller off its multipart
    /// field (before the field is consumed as the byte stream); `stream` yields the
    /// file bytes. Emits exactly one `media_upload*` metric (success in
    /// `finalize_upload`, failure here).
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` on validation failure, quota exhaustion, or I/O error.
    pub async fn upload<S, E>(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: Option<&str>,
        stream: S,
    ) -> anyhow::Result<UploadResponse>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let result = self
            .upload_inner(user_id, filename, content_type, stream)
            .await;
        Self::emit_failure_metric(&result);
        result
    }

    async fn upload_inner<S, E>(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: Option<&str>,
        stream: S,
    ) -> anyhow::Result<UploadResponse>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let (max_file_size, user_quota) = self.get_limits().await?;

        let content_type = Self::get_content_type(content_type, filename)?;

        let tmp_path = self.create_temp_file().await?;
        let (sha256_hex, size_bytes) = self
            .stream_to_temp(stream, &tmp_path, max_file_size)
            .await?;

        let metadata = UploadMetadata {
            filename: filename.clone(),
            content_type,
            sha256_hex,
            size_bytes,
        };

        self.finalize_upload(user_id, metadata, &tmp_path, user_quota)
            .await
    }

    /// Validates a filename and returns a sanitized version. Callers on the
    /// multipart path run this on the field's `file_name()` before streaming.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` if the filename is empty after sanitization.
    pub fn validate_filename(file_name: Option<&str>) -> anyhow::Result<Filename> {
        let raw_name = file_name.unwrap_or("upload");
        Filename::sanitized(raw_name)
            .map_err(|_| anyhow::anyhow!(MediaError::BadRequest("Invalid filename".to_owned())))
    }

    /// The single validating content-type door: a present client `Content-Type` is
    /// validated (malformed → bad request), an absent one is detected from the name.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` (`MediaError::BadRequest`) when `content_type` is present
    /// but not a valid `type/subtype` media type.
    pub fn get_content_type(
        content_type: Option<&str>,
        filename: &str,
    ) -> anyhow::Result<ContentType> {
        match content_type {
            Some(c) => c.parse().map_err(|_| {
                anyhow::anyhow!(MediaError::BadRequest("Invalid content type".to_owned()))
            }),
            None => Ok(detect_content_type(filename)),
        }
    }

    /// Emits the single `media_upload` failure metric for a completed upload attempt.
    /// The success metrics are emitted in `finalize_upload`, so this fires only on
    /// the `Err` path — keeping emission to exactly once per upload.
    fn emit_failure_metric(result: &anyhow::Result<UploadResponse>) {
        if let Err(err) = result {
            host::metrics::media_upload(Self::upload_outcome(err.downcast_ref::<MediaError>()));
        }
    }

    /// Maps a failed upload to its bounded `outcome` attribute for the
    /// `jaunder.media.uploads` metric. A non-`MediaError` counts as `error`.
    fn upload_outcome(err: Option<&MediaError>) -> host::metrics::UploadOutcome {
        match err {
            Some(MediaError::BadRequest(_)) => host::metrics::UploadOutcome::Invalid,
            Some(MediaError::PayloadTooLarge) => host::metrics::UploadOutcome::TooLarge,
            Some(MediaError::InsufficientStorage) => host::metrics::UploadOutcome::QuotaExceeded,
            Some(MediaError::Internal(_)) | None => host::metrics::UploadOutcome::Error,
        }
    }

    async fn get_limits(&self) -> anyhow::Result<(MaxFileSize, UserQuota)> {
        let max_file_size = self.site_config.get_media_max_file_size().await?;
        let user_quota = self.site_config.get_media_user_quota().await?;
        Ok((max_file_size, user_quota))
    }

    async fn create_temp_file(&self) -> anyhow::Result<PathBuf> {
        let tmp_dir = self.storage_path.join("media").join("tmp");
        fs::create_dir_all(&tmp_dir).await?;
        let tmp_id = uuid::Uuid::new_v4();
        Ok(tmp_dir.join(tmp_id.to_string()))
    }

    async fn check_quota(
        &self,
        user_id: UserId,
        size_bytes: i64,
        user_quota: UserQuota,
    ) -> anyhow::Result<()> {
        let current_usage = self.media.get_user_upload_usage(user_id).await?;
        if current_usage.value() + size_bytes > user_quota.value() {
            anyhow::bail!(MediaError::InsufficientStorage);
        }
        Ok(())
    }

    async fn handle_deduplication(
        &self,
        tmp_path: &PathBuf,
        target_path: &PathBuf,
        hash_dir: &PathBuf,
    ) -> anyhow::Result<bool> {
        if target_path.exists() {
            let _ = fs::remove_file(tmp_path).await;
            Ok(true)
        } else {
            let existing_file = self.first_file_in_dir(hash_dir).await;
            fs::create_dir_all(hash_dir).await?;
            if let Some(existing) = existing_file {
                fs::hard_link(&existing, target_path).await?;
                let _ = fs::remove_file(tmp_path).await;
                Ok(true)
            } else {
                fs::rename(tmp_path, target_path).await?;
                Ok(false)
            }
        }
    }

    async fn register_in_db(
        &self,
        user_id: UserId,
        sha256_hex: &ContentHash,
        filename: &Filename,
        content_type: &ContentType,
        size_bytes: i64,
    ) -> anyhow::Result<()> {
        let record = MediaRecord {
            user_id,
            sha256: sha256_hex.clone(),
            filename: filename.clone(),
            source: MediaSource::Upload,
            content_type: content_type.clone(),
            size_bytes: ByteSize::try_from(size_bytes)?,
            source_url: None,
            created_at: Utc::now(),
        };
        match self.media.create_media(&record).await {
            Ok(()) | Err(CreateMediaError::AlreadyExists) => Ok(()),
            Err(CreateMediaError::Internal(e)) => {
                tracing::error!(error = %e, "create_media failed");
                Err(anyhow::anyhow!(MediaError::Internal(e.to_string())))
            }
        }
    }

    async fn finalize_upload(
        &self,
        user_id: UserId,
        metadata: UploadMetadata,
        tmp_path: &Path,
        user_quota: UserQuota,
    ) -> anyhow::Result<UploadResponse> {
        if let Err(e) = self
            .check_quota(user_id, metadata.size_bytes, user_quota)
            .await
        {
            let _ = fs::remove_file(tmp_path).await;
            return Err(e);
        }
        let relative_path = media_path("upload", &metadata.sha256_hex, &metadata.filename);
        let target_path = self.storage_path.join("media").join(&relative_path);
        let hash_dir = target_path
            .parent()
            // cov:ignore-start — defensive: `target_path` always has a parent.
            .ok_or_else(|| {
                anyhow::anyhow!("media target path {} has no parent", target_path.display())
            })?
            // cov:ignore-stop
            .to_path_buf();
        let deduplicated = self
            .handle_deduplication(&tmp_path.to_path_buf(), &target_path, &hash_dir)
            .await?;
        self.register_in_db(
            user_id,
            &metadata.sha256_hex,
            &metadata.filename,
            &metadata.content_type,
            metadata.size_bytes,
        )
        .await?;
        host::metrics::media_upload_bytes(u64::try_from(metadata.size_bytes).unwrap_or(0));
        host::metrics::media_upload(if deduplicated {
            host::metrics::UploadOutcome::Deduplicated
        } else {
            host::metrics::UploadOutcome::Stored
        });
        let url = media_url("upload", &metadata.sha256_hex, &metadata.filename);
        Ok(UploadResponse {
            sha256: metadata.sha256_hex,
            filename: metadata.filename,
            content_type: metadata.content_type,
            size_bytes: ByteSize::try_from(metadata.size_bytes)?,
            url,
        })
    }

    /// Uploads raw in-memory bytes (e.g. an `AtomPub` media POST), reusing the same
    /// content-addressing/dedup/quota/DB path. Emits exactly one `media_upload*`.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` on invalid filename, oversized payload, quota
    /// exhaustion, I/O failure, or DB error.
    pub async fn upload_bytes(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<UploadResponse> {
        let result = self
            .upload_bytes_inner(user_id, filename, content_type, bytes)
            .await;
        Self::emit_failure_metric(&result);
        result
    }

    async fn upload_bytes_inner(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<UploadResponse> {
        let (max_file_size, user_quota) = self.get_limits().await?;
        let content_type = Self::get_content_type(Some(content_type), filename)?;

        let size_bytes = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        if size_bytes > max_file_size.value() {
            anyhow::bail!(MediaError::PayloadTooLarge);
        }

        let sha256_hex = ContentHash::from_digest(Sha256::digest(bytes).into());
        let tmp_path = self.create_temp_file().await?;
        fs::write(&tmp_path, bytes).await?;

        let metadata = UploadMetadata {
            filename: filename.clone(),
            content_type,
            sha256_hex,
            size_bytes,
        };
        self.finalize_upload(user_id, metadata, &tmp_path, user_quota)
            .await
    }

    async fn stream_to_temp<S, E>(
        &self,
        mut stream: S,
        tmp_path: &Path,
        max_file_size: MaxFileSize,
    ) -> anyhow::Result<(ContentHash, i64)>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut file = fs::File::create(tmp_path).await?;
        let mut hasher = Sha256::new();
        let mut bytes_written: i64 = 0;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            bytes_written += i64::try_from(chunk.len()).unwrap_or(i64::MAX);
            if bytes_written > max_file_size.value() {
                anyhow::bail!(MediaError::PayloadTooLarge);
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }

        file.flush().await?;
        drop(file);

        let sha256_hex = ContentHash::from_digest(hasher.finalize().into());
        Ok((sha256_hex, bytes_written))
    }

    async fn first_file_in_dir(&self, dir: &Path) -> Option<PathBuf> {
        let mut read_dir = fs::read_dir(dir).await.ok()?;
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                return Some(path);
            }
        }
        None
    }
}
```

Notes:

- `stream.next()?` funnels `multer::Error` (which is
  `std::error::Error + Send + Sync`) into `anyhow` via the `E` bound, matching
  the spec.
- `upload` no longer validates the filename itself: the caller extracts
  `file_name()` and runs `MediaManager::validate_filename` before consuming the
  field as the stream (the field's borrow of `file_name()`/`content_type()` must
  end before it is moved into the stream). `validate_filename` remains `pub` for
  the callers.

### 1c. `map_error` in `server/src/media.rs`

**Files:** `server/src/media.rs`.

Remove the `#[derive(Debug, Serialize)] pub struct UploadResponse` block (moved
to `storage`). Add a free `map_error` fn (was `MediaManager::map_error`, now
downcasting the `pub storage::MediaError`, **metric emission removed** — it now
lives in `MediaManager`):

```rust
use axum::http::StatusCode;

/// Maps a media upload `anyhow::Error` to the client-facing HTTP status. The
/// upload metric is emitted inside `storage::MediaManager`, so this is a pure map.
#[must_use]
pub fn map_error(err: &anyhow::Error) -> StatusCode {
    match err.downcast_ref::<storage::MediaError>() {
        Some(storage::MediaError::BadRequest(_)) => StatusCode::BAD_REQUEST,
        Some(storage::MediaError::PayloadTooLarge) => StatusCode::PAYLOAD_TOO_LARGE,
        Some(storage::MediaError::InsufficientStorage) => StatusCode::INSUFFICIENT_STORAGE,
        Some(storage::MediaError::Internal(_)) | None => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
```

Move `test_map_error` into `server/src/media.rs`'s `#[cfg(test)] mod tests`,
referencing `storage::MediaError` (a plain `#[test]`, not `#[tokio::test]`):

```rust
#[test]
fn map_error_maps_each_media_error() {
    assert_eq!(
        map_error(&anyhow::anyhow!(storage::MediaError::BadRequest("bad".to_owned()))),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        map_error(&anyhow::anyhow!(storage::MediaError::PayloadTooLarge)),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        map_error(&anyhow::anyhow!(storage::MediaError::InsufficientStorage)),
        StatusCode::INSUFFICIENT_STORAGE
    );
    assert_eq!(
        map_error(&anyhow::anyhow!(storage::MediaError::Internal("error".to_owned()))),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        map_error(&anyhow::anyhow!("unknown")),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
```

Then delete `server/src/media_manager.rs` and remove `pub mod media_manager;`
from `server/src/lib.rs:15`.

### 1d. Repoint the three server call sites

**Files:** `server/src/media.rs`, `server/src/atompub/media.rs`,
`server/src/atompub/mod.rs`.

**(i) Browser `upload_handler`** (`server/src/media.rs:65`, still exists this
task). Extract the field metadata before consuming the field as the stream
(axum's `Field` `impl Stream<Item = Result<Bytes, MultipartError>>`):

```rust
let filename = crate::media::map_error /* unchanged import path */;
// ...
let Some(field) = multipart
    .next_field()
    .await
    .map_err(|_| StatusCode::BAD_REQUEST)?
else {
    return Err(StatusCode::BAD_REQUEST);
};

let filename = storage::MediaManager::validate_filename(field.file_name())
    .map_err(|e| map_error(&e))?;
let content_type = field.content_type().map(ToOwned::to_owned);

let manager = storage::MediaManager::new(media, site_config, storage_path);
let response = manager
    .upload(auth_user.user_id, &filename, content_type.as_deref(), field)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "upload failed");
        map_error(&e)
    })?;

Ok((StatusCode::CREATED, Json(response)).into_response())
```

Update the `use` block: drop `use crate::media_manager::…`; `UploadResponse` is
no longer named here (the handler builds `Json(response)` from
`storage::UploadResponse`). Keep the `serde::Serialize` import only if still
used elsewhere in the file (it is — `ServeParams` / `ProxyParams` derive
`Deserialize`; verify and trim `Serialize` if now unused to avoid an
unused-import warning).

**(ii) AtomPub `collection_post`** (`server/src/atompub/media.rs:88`):

```rust
let manager = storage::MediaManager::new(media.clone(), site_config.clone(), storage_path);
let upload = manager
    .upload_bytes(auth_user.user_id, &filename, &content_type, &body)
    .await?;
```

Update its imports: drop nothing new (it already imports `storage::…`); remove
the `crate::media_manager::MediaManager` path.

**(iii) `HandlerError` `From<anyhow::Error>`** (`server/src/atompub/mod.rs`) —
_not in the task brief's caller list, but real_: its body is
`HandlerError::Status(crate::media_manager::MediaManager::map_error(&err))`.
Repoint to the new free fn:

```rust
HandlerError::Status(crate::media::map_error(&err))
```

Its doc comment referencing `MediaManager::map_error` updates to
`media::map_error`. The existing `anyhow_error_maps_through_media_map_error`
test in that module keeps passing (it exercises the `From` impl).

### 1e. Move the streaming tests to `storage`

**Files:** `storage/src/media_manager.rs` (`#[cfg(test)] mod tests`).

Port the tests from the old `server/src/media_manager.rs`, dropping `AuthUser`
literals for a `UserId` and adopting `storage`'s fixtures. Split by DB need:

- **Pure / mock (no DB):** `upload_outcome_maps_each_media_error`,
  `get_content_type_validates_present_and_detects_absent`,
  `register_in_db_maps_internal_create_error` (uses `storage::MockMediaStorage`
  / `MockSiteConfigStorage`), `validate_filename` cases. These move verbatim
  (adjust paths: `MediaManager`/`MediaError` are `super::…`; `CreateMediaError`,
  `MockMediaStorage`, `MockSiteConfigStorage` are `crate::…`).
  `parse_content_hash`/`parse_filename`/ `parse_content_type` come from
  `common::test_support` (a `storage` dev-dep).
- **Dedup / dir-scan (no DB needed):** `test_first_file_in_dir`,
  `test_handle_deduplication` construct `MediaManager` with
  `Arc::new(storage::MockMediaStorage::new())` +
  `Arc::new(storage::MockSiteConfigStorage::new())` (no expectations) and a bare
  `TempDir` as `storage_path` — the DB is unused, so this sidesteps ADR-0053
  cleanly.
- **DB-backed uploads (dual-backend, bind whole `TestEnv`):** the two
  `upload_bytes` tests. Rewrite against `Backend::setup()`:

```rust
use crate::test_support::{seed_user, Backend};
use crate::MEDIA_MAX_FILE_SIZE_BYTES_KEY;
use common::test_support::parse_filename;
use rstest::rstest;
use rstest_reuse::apply;
use std::sync::Arc;

#[apply(backends)]
#[tokio::test]
async fn upload_bytes_is_content_addressed_and_idempotent(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user_id = seed_user(&env.state).await;
    let manager = MediaManager::new(
        env.state.media.clone(),
        env.state.site_config.clone(),
        Arc::new(env.base.path().to_path_buf()),
    );

    let bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02, 0x03];
    let expected_sha = format!("{:x}", sha2::Sha256::digest(bytes));

    let first = manager
        .upload_bytes(user_id, &parse_filename("pic.png"), "image/png", bytes)
        .await
        .unwrap();
    assert_eq!(first.sha256.as_ref(), expected_sha.as_str());

    let second = manager
        .upload_bytes(user_id, &parse_filename("pic.png"), "image/png", bytes)
        .await
        .unwrap();
    assert_eq!(second.sha256, first.sha256);
    assert_eq!(second.url, first.url);
}
```

The oversized test sets the limit via
`env.state.site_config.set(MEDIA_MAX_FILE_SIZE_BYTES_KEY, "5")` and asserts on
the downcast (`map_error` is gone from `storage`):

```rust
let err = manager
    .upload_bytes(user_id, &parse_filename("big.bin"), "application/octet-stream", &[0_u8; 11])
    .await
    .unwrap_err();
assert!(matches!(
    err.downcast_ref::<MediaError>(),
    Some(MediaError::PayloadTooLarge)
));
```

- **New: a streaming test** covering the generic `stream_to_temp` path (the old
  code had no `upload`-via-stream unit test; it was e2e-only). Add one so the
  generic path stays host-covered (AC8): build a `futures_util::stream::iter` of
  `Ok::<_, std::io::Error>(Bytes::from_static(...))` chunks, call
  `manager.upload(user_id, &parse_filename("s.png"), Some("image/png"), stream)`,
  assert the returned `sha256`/`url`. Use `#[apply(backends)]`.

**Making the mock-based tests compile in `storage` (one decision, do this
first).** `storage`'s `Mock*Storage` are gated
`#[cfg_attr(feature = "test-utils", mockall::automock)]`
(`storage/src/media.rs:59`, `site_config.rs:18`), and `mockall` is an _optional_
dep — **not** a storage dev-dep — so `storage`'s own `cfg(test)` build cannot
see `MockMediaStorage`/ `MockSiteConfigStorage`. Two of the moved tests need
them (`register_in_db_maps_internal_create_error` forces
`CreateMediaError::Internal`; the dedup tests want a no-DB handle). Resolve by
**re-gating the mocks to
`#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]` and adding
`mockall` as a `storage` dev-dependency**
(`storage/Cargo.toml [dev-dependencies] mockall.workspace = true`) — this makes
storage's own tests see the mocks without changing the downstream `test-utils`
surface. (`register_in_db`'s Internal path genuinely needs a mock; the DB-backed
alternative can't force it.) The dedup/dir-scan tests then keep using
`crate::MockMediaStorage::new()` (no DB) as written; the `upload_bytes`/`upload`
tests are DB-backed via `Backend::setup()` (above). `storage`'s `cfg(test)`
already has `tempfile`, `rstest`, `rstest_reuse`, `tokio` (`macros`,`rt`), and
`common/test-support`.

### Verify (Task 1)

```
devtool run -- cargo xtask check
```

Expected: PASS (host static + clippy + coverage). Then:

```
devtool run -- cargo nextest run -p storage media
```

Expected: PASS — the moved `media_manager` + existing `media` tests, both
backends.

Grep that `storage` took on no forbidden deps:

```
rg -n "\\b(web|axum)\\b" storage/src/media_manager.rs
rg -n "^web\\b|^axum\\b|\"axum\"|path = \"../web\"|path = \"../server\"" storage/Cargo.toml
```

Expected: **no** matches (FAIL of the grep = PASS of the invariant;
`bytes`/`futures_util` only).

Confirm the old file is gone:

```
test ! -e server/src/media_manager.rs
```

**Commit:**
`refactor(storage): relocate MediaManager from server, decouple to UserId + byte stream (#517)`

---

## Task 2 — Add the web multipart `#[server]` fn; register it; enable `multipart`; wire `storage_path`

Old endpoint (`POST /media/upload`) still present — this task only _adds_ the
server fn.

### 2a. Enable the `multipart` codec

**Files:** `web/Cargo.toml`.

`MultipartData` is named in the `upload_media` signature, which compiles for
**both** the wasm client and the server build, so the feature cannot be gated to
`server` only. Per the repo's workspace-dep convention, add `server_fn` to the
root `[workspace.dependencies]` (version-matched to leptos 0.8.2's transitive
`server_fn` — read `Cargo.lock` for the exact `server_fn` version to reuse the
vendor), then in `web/Cargo.toml`:

```toml
server_fn = { workspace = true, features = ["multipart"] }
```

Non-optional (needed on both builds); `multer` arrives transitively via the
feature and is server-only inside `server_fn`, so it should not bloat the wasm
bundle. No change to `web`'s `[features]` is required.

**Do this feasibility spike FIRST** (the review's top risk — the repo's first
multipart `#[server]` fn), before building out 2b, to confirm the codec compiles
clean on wasm:

```
devtool run -- cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings
```

Expected: PASS with just the dep added + a trivial
`use leptos::server_fn::codec::MultipartData;` in place. If `multer` or other
server-only types leak into the wasm build, stop and report before proceeding.

### 2b. The `upload_media` server fn

**Files:** `web/src/media/api.rs`, `web/src/media/mod.rs`.

The existing `#[cfg(feature = "server")]` block (`api.rs:13-19`) already imports
`require_auth`, `InternalError`, `Arc`, and
`storage::{MediaStorage, PostStorage, SiteConfigStorage}`. Add only the
genuinely-new server-side items to it — `MediaManager` (to the `storage::{…}`
set), `leptos_axum::extract`, and `std::path::PathBuf` — **not** `WebError`
(unused; `WebResult` is already imported ungated at `api.rs:25`) and **not** a
second `PostStorage`. Add the return type **ungated** (nameable on both targets)
next to the existing `common::media::{…}` import:

```rust
use common::media::UploadResponse;
```

Both client and server need the codec types in scope for the signature
(ungated):

```rust
use leptos::server_fn::codec::{MultipartData, MultipartFormData};
```

Add the fn, mirroring the sibling `#[server(endpoint = …)]` + `boundary!` style.
`boundary!` expands to `server_boundary($name, async move $body).await`, and
`server_boundary` takes a future yielding **`InternalResult<T>`** and returns
`WebResult<T>` (projecting `InternalError.kind()` → `WebError`). So the block
must yield `InternalResult<UploadResponse>`: `require_auth().await?` already
yields the right type (`ErrorKind::Auth` → `WebError::Unauthorized`, satisfying
AC6), and the `anyhow`/`MediaError` from `MediaManager::upload` is translated to
`InternalError` via a single `map_media_error` helper (below):

```rust
/// Streams a multipart file upload to storage and returns its stored URL/metadata.
/// The multipart `#[server]` fn replacing the old `POST /media/upload` glue (#517).
#[server(input = MultipartFormData, endpoint = "/upload_media")]
pub async fn upload_media(data: MultipartData) -> WebResult<UploadResponse> {
    boundary!("upload_media", {
        let auth = require_auth().await?;
        let media = expect_context::<Arc<dyn MediaStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();

        // `storage_path` is an axum `Extension` (server/src/lib.rs:126), not a leptos
        // context value, so pull it via the request extractor rather than expect_context.
        let axum::Extension(storage_path) =
            extract::<axum::Extension<Arc<PathBuf>>>()
                .await
                .map_err(|e| InternalError::server_message(format!("storage_path extract: {e}")))?;

        // `into_inner()` is `Some` on the server (the parsed multipart body).
        let mut multipart = data
            .into_inner()
            .ok_or_else(|| InternalError::validation("missing multipart body"))?;

        let field = multipart
            .next_field()
            .await
            .map_err(|e| InternalError::validation(format!("bad multipart: {e}")))?
            .ok_or_else(|| InternalError::validation("no file field"))?;

        let filename = MediaManager::validate_filename(field.file_name())
            .map_err(InternalError::validation_from_anyhow)?; // see note
        let content_type = field.content_type().map(ToOwned::to_owned);

        let manager = MediaManager::new(media, site_config, storage_path);
        manager
            .upload(auth.user_id, &filename, content_type.as_deref(), field)
            .await
            .map_err(map_media_error) // MediaError → InternalError
    })
}
```

Two helpers to settle during implementation (both `#[cfg(feature = "server")]`,
local to `api.rs`), because `boundary!` expects the block to yield
`InternalResult<T>`:

- `map_media_error(err: anyhow::Error) -> InternalError`: downcast
  `err.downcast_ref::<storage::MediaError>()` and map `BadRequest`→`validation`,
  `PayloadTooLarge`/`InsufficientStorage`→`validation` (or a dedicated
  `InternalError::conflict`/`server_message` — pick per the desired `WebError`
  variant; the spec only requires "mapping `MediaError → WebError`", so
  `BadRequest`/too-large/quota → `WebError::Validation`, `Internal`/unknown →
  `WebError::Server`). Since `require_auth`'s rejection already yields
  `Unauthorized`, AC6 is satisfied by the `require_auth().await?` line
  regardless of this mapping.
- The `validate_filename` anyhow → `InternalError` conversion is the same
  `MediaError` downcast; fold both into one `map_media_error` and apply it to
  the `validate_filename` result too (drop the placeholder
  `validation_from_anyhow`).

`multer::Multipart::next_field()` yields `multer::Field`, which
`impl Stream<Item = Result<Bytes, multer::Error>>` — matching `upload`'s
`E: Error + Send + Sync` bound. The `Filename`/`content_type` borrows of `field`
end before `field` is moved into `upload`.

`mod.rs`: extend the re-export to include the fn and its generated types:

```rust
pub use api::{
    delete_media, list_my_media, media_usage, upload_media, DeleteMedia, DeleteMediaResult,
    ListMyMedia, MediaItem, MediaUsage, MediaUsageData, UploadMedia,
};
```

`UploadResponse` is `common::media::UploadResponse` (step 1a-bis) — **ungated**,
so it is nameable on both the wasm client stub and the server body with no
mapping layer. Import it ungated in `api.rs`
(`use common::media::UploadResponse;`, near the existing `common::media::{…}`
import) and reference it bare in the return type (`WebResult<UploadResponse>`).
`storage::MediaManager::upload` already returns this exact type, so the server
body returns it directly.

### 2c. Register the server fn

**Files:** `server/tests/helpers/mod.rs`.

Add to `ensure_server_fns_registered()` (near the other media entries, ~line
72):

```rust
server_fn::axum::register_explicit::<web::media::UploadMedia>();
```

### Verify (Task 2)

```
devtool run -- cargo xtask check
```

Expected: PASS (incl. the `server-fn-registrar` guard now that `UploadMedia` is
listed).

Wasm clippy (the wasm build compiles the client stub + `MultipartData`
signature):

```
devtool run -- cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings
```

Expected: PASS.

**Commit:** `feat(web): add upload_media multipart server fn (#517)`

---

## Task 3 — Switch the client to the server fn; delete client glue

Old route still exists → e2e still green (the UI now uses the server fn, but the
direct-POST e2e tests still hit the live `/media/upload`).

**Files:** `web/src/media/component.rs`, `web/src/media/api.rs`,
`web/src/media/mod.rs`, `web/Cargo.toml`.

**`component.rs` `on_file_change`:** replace the `upload_file(form_data)` call
with the server fn. The server fn takes `MultipartData`, which is
`From<web_sys::FormData>` on the client:

```rust
spawn_local(async move {
    let result = upload_media(form_data.into()).await;
    uploading.set(false);
    match result {
        Ok(resp) => {
            let url = resp.url;
            if let Some(cb) = on_uploaded {
                cb.run(url.clone());
            }
            if show_result {
                last_media_url.set(Some(url));
                upload_error.set(None);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if let Some(cb) = on_error {
                cb.run(msg.clone());
            }
            if show_result {
                upload_error.set(Some(msg));
            }
        }
    }
});
```

Add `use super::upload_media;` to the `component.rs` imports. Update the
`MediaUpload` doc comment (it says "via a JS `fetch`") to describe the server-fn
call.

**Delete** `upload_file` (`api.rs`/`component.rs` — it is in
`component.rs:144-188`) and `extract_upload_url` + its `#[cfg(test)] mod tests`
(`api.rs:71-78, 196-239`), and the `extract_upload_url` re-export (`mod.rs:22`).
Drop the `crap:allow` (it was on `upload_file`).

**Sweep dead `web-sys` features** (`web/Cargo.toml:32-45`). First confirm none
are used elsewhere in `web`:

```
rg -n "web_sys::(Request|RequestInit|RequestMode|Response)\\b|fetch_with_request|\\.fetch\\b" web/src
```

Expected: **no** matches after the deletions. Then remove `"Request"`,
`"RequestInit"`, `"RequestMode"`, `"Response"` from the `web-sys` feature list
(keep `Window`, `Document`, `Element`, `Location`, `File`, `FileList`,
`FormData`, `HtmlInputElement`).

If `wasm-bindgen-futures` / `JsFuture` are now unused in `web`, check and trim:

```
rg -n "JsFuture|wasm_bindgen_futures" web/src
```

(likely still used elsewhere — only trim the `Cargo.toml` dep if the grep is
empty).

### Verify (Task 3)

```
devtool run -- cargo xtask check
devtool run -- cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings
```

Expected: PASS (no dead-import / dead-code warnings; coverage clean without the
removed `extract_upload_url` tests, since the code they covered is gone).

**Commit:**
`refactor(web): call upload_media server fn, delete fetch glue (#517)`

---

## Task 4 — Delete the bespoke handler + route; drop `uuid`; rewrite e2e

**Files:** `server/src/media.rs`, `server/Cargo.toml`,
`end2end/tests/media.spec.ts`.

**`server/src/media.rs`:** delete `upload_handler` (`:64-87`) and the
`.route("/media/upload", post(upload_handler))` line (`:31`) from `router()`.
Remove the now-unused imports: `axum::extract::Multipart`, `axum::Json`,
`axum::routing::post` (keep `get`), and `AuthUser` **only if** no other handler
in the file uses it (`proxy_handler` uses `AuthUser` — keep it). `map_error`
stays (Task 1c) — its live caller is the AtomPub `HandlerError`
`From<anyhow::Error>` impl. Trim any leftover `serde::Serialize` import if
`UploadResponse`'s removal (Task 1) left it unused.

**`server/Cargo.toml`:** remove `uuid = { version = "1", features = ["v4"] }`
(`:55`) — it left with `MediaManager`. Confirm no other `server` source uses
`uuid`:

```
rg -n "\\buuid\\b" server/src
```

Expected: no matches.

**`end2end/tests/media.spec.ts`:** the `/media/upload` route is gone, so the two
`page.request.post(BASE_URL + "/media/upload", …)` tests must change:

- **"authenticated user can upload and access media"** (`:5-34`): drive the
  server-fn endpoint instead. The server fn is at `POST /api/upload_media` with
  a multipart body (session cookie in the page jar). Assert the server-fn
  success response (HTTP 200 and a JSON body carrying
  `url`/`sha256`/`filename`), then GET `json.url` and assert the served 200 +
  immutable cache header. Sketch:

```ts
const fileContent = Buffer.from("fake image content for testing");
const response = await page.request.post(BASE_URL + "/api/upload_media", {
  multipart: {
    file: {
      name: "test-image.jpg",
      mimeType: "image/jpeg",
      buffer: fileContent,
    },
  },
});
expect(response.status()).toBe(200);
const json = await response.json();
expect(json.url).toContain("/media/upload/");
expect(json.filename).toBe("test-image.jpg");
const serveResponse = await page.request.get(BASE_URL + json.url);
expect(serveResponse.status()).toBe(200);
expect(serveResponse.headers()["cache-control"]).toBe(
  "public, max-age=31536000, immutable",
);
```

(Confirm the server-fn success wire shape during implementation — leptos
`#[server]` JSON-encodes `Ok(UploadResponse)` as the bare struct with the
default codec; if the envelope differs, assert `response.ok()` + parse
accordingly. Prefer, if simpler and more robust, the **UI-driven** variant used
by the other passing tests: `setInputFiles` on the hidden file input and assert
the readonly URL input — see the existing "upload widget…shows URL" tests, which
already exercise the full server-fn path once Task 3 lands.)

- **"unauthenticated upload returns 401"** (`:36-47`): POST `/api/upload_media`
  with **no** session. `require_auth()` rejects → the server fn returns a
  serialized `WebError::Unauthorized` (AC6: not necessarily a bare 401). Assert
  the rejection: either the HTTP status the leptos server-fn error path uses for
  an app error, or a response body containing `"unauthorized"`. Recommended
  robust assertion:

```ts
const response = await page.request.post(BASE_URL + "/api/upload_media", {
  multipart: {
    file: {
      name: "test.jpg",
      mimeType: "image/jpeg",
      buffer: Buffer.from("data"),
    },
  },
});
expect(response.ok()).toBeFalsy();
const body = await response.text();
expect(body).toContain("unauthorized");
```

(During implementation, confirm the exact status/body the server-fn auth-error
path emits by running the spec locally — see Verify — and tighten the assertion
to match.)

The four UI-driven tests (`:49-104`) are unchanged; after Task 3 they already
exercise the new server-fn path and are the primary behavioral coverage (AC7).

### Verify (Task 4)

```
devtool run -- cargo xtask check
```

Expected: PASS. The e2e runs under `cargo xtask validate` (Task 5); to iterate
on the rewritten spec locally without the full matrix:

```
cargo xtask e2e-local media.spec.ts
```

Expected: PASS (both rewritten tests + the four UI tests). Confirm
`/media/upload` is gone:

```
rg -n "media/upload\"" server/src/media.rs
```

Expected: no `post`/route match (the serve route `"/media/{source}/…"` remains).

**Commit:**
`refactor(server): delete bespoke media upload handler and route (#517)`

---

## Task 5 — Full gate

**Files:** none (verification only).

```
devtool run -- cargo xtask validate
```

Expected: PASS — static + clippy + coverage + the full e2e matrix
(`{sqlite,postgres}×{chromium,firefox}`), incl. `wasm-clippy`,
`server-fn-registrar`, and `rendered-html-from-trusted` guards. Coverage clean:
no new `cov:ignore` / `crap:allow`, no regression (the moved streaming tests +
the new stream test keep `MediaManager` covered in `storage`).

```
git status --porcelain
```

Expected: empty (xtask auto-fmt may have touched files during earlier tasks — if
so, they were folded into the introducing commit; nothing should remain here).

**Commit:** none (gate only), unless a fmt tail must be folded back.

---

## Self-review — spec AC → task map

| AC      | Requirement                                                                                                                                                                                                                                             | Satisfied by                                                                      |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| **AC1** | `MediaManager` in `storage`, decoupled; `server/src/media_manager.rs` gone; `UserId`; `pub MediaError`; `UploadResponse` in `common::media`; no `web`/`server`/`axum` dep                                                                               | Task 1 (1a-bis common DTO; 1b delete + module; 1a deps; Verify grep)              |
| **AC2** | Streaming + `max_file_size` mid-stream + content-address + dedupe + quota preserved; generic `Stream<Item = Result<Bytes,_>>`                                                                                                                           | Task 1b (`stream_to_temp`/`finalize_upload` moved byte-for-byte; new stream test) |
| **AC3** | Multipart `#[server]` fn constructs `storage::MediaManager` incl. `storage_path`, returns `UploadResponse`, maps `MediaError → WebError`; no `web_sys::Request/Response/fetch` in `web/src/media`; `upload_file`/`extract_upload_url`/`crap:allow` gone | Task 2 (fn) + Task 3 (client switch + deletions + web-sys sweep)                  |
| **AC4** | `upload_handler` + `POST /media/upload` deleted; serve/proxy unchanged; `map_error` remains in `server`                                                                                                                                                 | Task 1c (`map_error` home) + Task 4 (handler/route delete)                        |
| **AC5** | AtomPub constructs `storage::MediaManager`, `upload_bytes(user_id, …)`; behavior unchanged                                                                                                                                                              | Task 1d(ii) + 1d(iii)                                                             |
| **AC6** | `#[server]` fn `require_auth()`; unauth rejected as serialized `WebError::Unauthorized`; e2e asserts it                                                                                                                                                 | Task 2b (`require_auth().await?`) + Task 4 (unauth e2e rewrite)                   |
| **AC7** | End-to-end behavior identical; media e2e (incl. two rewritten tests) passes; metric emitted exactly once in `MediaManager`, `map_error` no longer emits                                                                                                 | Task 1b (metric funnel) + Task 3 (UI path) + Task 4 (e2e) + Task 5 (matrix)       |
| **AC8** | `cargo xtask validate` green (e2e matrix, `wasm-clippy`, registrar + trusted-html guards); coverage clean, no new `cov:ignore`/`crap:allow`                                                                                                             | Task 5 (full gate); registrar entry in Task 2c                                    |
