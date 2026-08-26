//! Content-addressed media upload service: streams an upload to a hashed,
//! dedup'd on-disk path, enforces per-file and per-user limits, and records the
//! result. Relocated from `server` (#517) so a `web` `#[server]` fn can construct
//! it directly — its work is persistence and its deps are all `storage`'s.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use chrono::Utc;
use futures_util::{Stream, StreamExt, TryStreamExt, stream};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use common::ids::UserId;
use common::media::{
    ByteSize, ContentHash, ContentType, Filename, MaxFileSize, MediaRef, MediaSource,
    UploadedMedia, UserQuota, detect_content_type, media_path, media_url,
};

use crate::{CreateMediaError, MediaRecord, MediaStorage, SiteConfigStorage, TryDeleteOutcome};

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
    #[error("Internal server error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

// `UploadedMedia` is defined in `common::media`, not here — it is the `#[server]` fn's
// return type, which must be nameable on the wasm client build where `storage` is not
// compiled (`storage` is a `server`-gated `web` dep). `common` is ungated and reachable
// by storage + web (both targets) + server, so the manager returns it directly with no
// mapping layer.

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
    size_bytes: ByteSize,
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
        content_type: Option<ContentType>,
        stream: S,
    ) -> anyhow::Result<UploadedMedia>
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
        content_type: Option<ContentType>,
        stream: S,
    ) -> anyhow::Result<UploadedMedia>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let (max_file_size, user_quota) = self.get_limits().await?;

        let content_type = content_type.unwrap_or_else(|| detect_content_type(filename));

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

    // Upload content types have already been parsed at their HTTP boundary; absent
    // multipart content types are detected from the validated filename.

    /// Emits the single `media_upload` failure metric for a completed upload attempt.
    /// The success metrics are emitted in `finalize_upload`, so this fires only on
    /// the `Err` path — keeping emission to exactly once per upload.
    fn emit_failure_metric(result: &anyhow::Result<UploadedMedia>) {
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
        size_bytes: ByteSize,
        user_quota: UserQuota,
    ) -> anyhow::Result<()> {
        let current_usage = self.media.get_user_upload_usage(user_id).await?;
        if current_usage
            .value()
            .checked_add(size_bytes.value())
            .is_none_or(|total| total > user_quota.value())
        {
            anyhow::bail!(MediaError::InsufficientStorage);
        }
        Ok(())
    }

    fn finish_temp_cleanup<T>(primary: T, cleanup: io::Result<()>, context: &'static str) -> T {
        crate::helpers::preserve_after_secondary(
            primary,
            cleanup,
            host::error::ErrorKind::Internal,
            host::error::ErrorClass::Transient,
            context,
        )
    }

    /// Content-addresses the temp file at `target_path`, deduplicating against
    /// already-stored identical content. Returns `true` when the bytes were
    /// deduplicated (the target already existed, or an identical file was
    /// hard-linked) and `false` when this is a freshly stored file.
    async fn handle_deduplication(
        &self,
        tmp_path: &Path,
        target_path: &Path,
        hash_dir: &Path,
    ) -> anyhow::Result<bool> {
        if target_path.exists() {
            return Self::finish_temp_cleanup(
                Ok(true),
                fs::remove_file(tmp_path).await,
                "storage.media.dedup_temp_cleanup",
            );
        }

        // A new hash has no directory yet. Create it before enumeration so
        // `NotFound` is not confused with an expected empty directory; after
        // this point every `read_dir`/`next_entry` error is unexpected and must
        // propagate.
        fs::create_dir_all(hash_dir).await?;
        let existing_file = self.first_file_in_dir(hash_dir).await;
        Self::finish_deduplication_from_result(tmp_path, target_path, existing_file).await
    }

    async fn finish_deduplication_from_result(
        tmp_path: &Path,
        target_path: &Path,
        existing_file: io::Result<Option<PathBuf>>,
    ) -> anyhow::Result<bool> {
        Self::finish_deduplication(tmp_path, target_path, existing_file?).await
    }

    async fn finish_deduplication(
        tmp_path: &Path,
        target_path: &Path,
        existing_file: Option<PathBuf>,
    ) -> anyhow::Result<bool> {
        if let Some(existing) = existing_file {
            fs::hard_link(&existing, target_path).await?;
            Self::finish_temp_cleanup(
                Ok(true),
                fs::remove_file(tmp_path).await,
                "storage.media.dedup_temp_cleanup",
            )
        } else {
            fs::rename(tmp_path, target_path).await?;
            Ok(false)
        }
    }

    async fn register_in_db(
        &self,
        user_id: UserId,
        sha256_hex: &ContentHash,
        filename: &Filename,
        content_type: &ContentType,
        size_bytes: ByteSize,
    ) -> anyhow::Result<()> {
        let record = MediaRecord {
            user_id,
            sha256: sha256_hex.clone(),
            filename: filename.clone(),
            source: MediaSource::Upload,
            content_type: content_type.clone(),
            size_bytes,
            source_url: None,
            created_at: Utc::now(),
        };
        match self.media.create_media(&record).await {
            Ok(()) | Err(CreateMediaError::AlreadyExists) => Ok(()),
            Err(CreateMediaError::Internal(e)) => {
                tracing::error!(error = %e, "create_media failed");
                Err(anyhow::anyhow!(MediaError::Internal(Box::new(e))))
            }
        }
    }

    /// Shared finalization for an upload whose bytes are already written to
    /// `tmp_path` with a known content hash and size: enforces quota, content-
    /// addresses the file (dedup via hard-link), records it in the DB, and builds
    /// the response. The temp file is consumed (moved, linked, or removed). Emits
    /// the success `media_upload*` metrics.
    async fn finalize_upload(
        &self,
        user_id: UserId,
        metadata: UploadMetadata,
        tmp_path: &Path,
        user_quota: UserQuota,
    ) -> anyhow::Result<UploadedMedia> {
        if let Err(e) = self
            .check_quota(user_id, metadata.size_bytes, user_quota)
            .await
        {
            return Self::finish_temp_cleanup(
                Err(e),
                fs::remove_file(tmp_path).await,
                "storage.media.quota_temp_cleanup",
            );
        }
        let relative_path = media_path(
            &MediaSource::Upload,
            &metadata.sha256_hex,
            &metadata.filename,
        );
        let target_path = self.storage_path.join("media").join(&relative_path);
        // `target_path` is built by joining `media`/`relative_path` onto the storage
        // root, so it always ends in a filename component and has a parent; surface a
        // clear error rather than panicking if that invariant is ever violated.
        let hash_dir = target_path
            .parent()
            // cov:ignore-start — defensive: `target_path` always has a parent (see
            // above), so this error branch is unreachable in practice.
            .ok_or_else(|| {
                anyhow::anyhow!("media target path {} has no parent", target_path.display())
            })?
            // cov:ignore-stop
            .to_path_buf();
        let deduplicated = self
            .handle_deduplication(tmp_path, &target_path, &hash_dir)
            .await?;
        self.register_in_db(
            user_id,
            &metadata.sha256_hex,
            &metadata.filename,
            &metadata.content_type,
            metadata.size_bytes,
        )
        .await?;
        host::metrics::media_upload_bytes(metadata.size_bytes.value().unsigned_abs());
        host::metrics::media_upload(if deduplicated {
            host::metrics::UploadOutcome::Deduplicated
        } else {
            host::metrics::UploadOutcome::Stored
        });
        let url = media_url(
            &MediaSource::Upload,
            &metadata.sha256_hex,
            &metadata.filename,
        );
        Ok(UploadedMedia {
            sha256: metadata.sha256_hex,
            filename: metadata.filename,
            content_type: metadata.content_type,
            size_bytes: metadata.size_bytes,
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
        content_type: ContentType,
        bytes: &[u8],
    ) -> anyhow::Result<UploadedMedia> {
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
        content_type: ContentType,
        bytes: &[u8],
    ) -> anyhow::Result<UploadedMedia> {
        let (max_file_size, user_quota) = self.get_limits().await?;
        // `filename` and `content_type` were validated at their respective inbound
        // boundaries, so neither needs revalidation in the persistence seam.

        let size_bytes = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        if size_bytes > max_file_size.value() {
            anyhow::bail!(MediaError::PayloadTooLarge);
        }

        let size_bytes = ByteSize::try_from(size_bytes)?;

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

    /// Deletes a media row and reclaims the on-disk entry when no remaining row or
    /// live Post names the same canonical media address.
    ///
    /// # Errors
    ///
    /// Returns storage errors from the row decision or reclamation check, and I/O
    /// errors from removing a reclaimable file.
    pub async fn delete_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
        force: bool,
    ) -> anyhow::Result<TryDeleteOutcome> {
        let outcome = self.media.try_delete_media(user_id, media, force).await?;
        if outcome == TryDeleteOutcome::Deleted {
            Self::reclaim_deleted_media_file(
                self.media.as_ref(),
                self.storage_path.as_ref(),
                media,
            )
            .await?;
        }
        Ok(outcome)
    }

    /// Reclaims the file for an already-deleted media row when the canonical entry is
    /// no longer named by any remaining row or live Post.
    ///
    /// # Errors
    ///
    /// Returns storage errors from the reclaimability query and I/O errors from
    /// removing a reclaimable file.
    pub async fn reclaim_deleted_media_file(
        media_storage: &dyn MediaStorage,
        storage_path: &Path,
        media: &MediaRef,
    ) -> anyhow::Result<()> {
        if !media_storage.media_entry_is_reclaimable(media).await? {
            return Ok(());
        }

        let file_path = storage_path.join("media").join(media_path(
            &media.source,
            &media.sha256,
            &media.filename,
        ));
        match fs::remove_file(&file_path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(MediaError::Internal(Box::new(error)).into()),
        }
    }

    async fn stream_to_temp<S, E>(
        &self,
        mut stream: S,
        tmp_path: &Path,
        max_file_size: MaxFileSize,
    ) -> anyhow::Result<(ContentHash, ByteSize)>
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
        Ok((sha256_hex, ByteSize::try_from(bytes_written)?))
    }

    async fn directory_entries(dir: &Path) -> io::Result<impl Stream<Item = io::Result<PathBuf>>> {
        let read_dir = fs::read_dir(dir).await?;
        Ok(stream::try_unfold(read_dir, |mut read_dir| async move {
            read_dir
                .next_entry()
                .await
                .map(|entry| entry.map(|entry| (entry.path(), read_dir)))
        }))
    }

    async fn first_file_in_dir(&self, dir: &Path) -> io::Result<Option<PathBuf>> {
        Self::first_file_in_entries(Self::directory_entries(dir).await).await
    }

    async fn first_file_in_entries<S>(entries: io::Result<S>) -> io::Result<Option<PathBuf>>
    where
        S: Stream<Item = io::Result<PathBuf>>,
    {
        let entries = entries?;
        futures_util::pin_mut!(entries);
        while let Some(path) = entries.try_next().await? {
            if path.is_file() {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SiteConfigKey;
    use crate::test_support::{
        Backend, SeedUser, backends, create_post_via_service, media_row_exists,
    };
    use common::media::MediaRef;
    use common::test_support::{
        parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_post_body,
    };
    use rstest::*;
    use rstest_reuse::*;
    use tempfile::TempDir;

    /// A `MediaManager` whose storage handles are mocks with no expectations, over a
    /// bare `TempDir` root — for the pure filesystem paths (`first_file_in_dir`,
    /// `handle_deduplication`) that never touch the DB (ADR-0053 sidestep).
    fn mock_manager(storage_path: Arc<PathBuf>) -> MediaManager {
        MediaManager::new(
            Arc::new(crate::MockMediaStorage::new()),
            Arc::new(crate::MockSiteConfigStorage::new()),
            storage_path,
        )
    }

    fn upload_ref(response: &UploadedMedia) -> MediaRef {
        MediaRef {
            source: MediaSource::Upload,
            sha256: response.sha256.clone(),
            filename: response.filename.clone(),
        }
    }

    fn stored_path(root: &Path, media: &MediaRef) -> PathBuf {
        root.join("media")
            .join(media_path(&media.source, &media.sha256, &media.filename))
    }

    #[test]
    fn upload_outcome_maps_each_media_error() {
        use host::metrics::UploadOutcome;
        assert!(matches!(
            MediaManager::upload_outcome(Some(&MediaError::BadRequest("x".to_owned()))),
            UploadOutcome::Invalid
        ));
        assert!(matches!(
            MediaManager::upload_outcome(Some(&MediaError::PayloadTooLarge)),
            UploadOutcome::TooLarge
        ));
        assert!(matches!(
            MediaManager::upload_outcome(Some(&MediaError::InsufficientStorage)),
            UploadOutcome::QuotaExceeded
        ));
        assert!(matches!(
            MediaManager::upload_outcome(Some(&MediaError::Internal(Box::new(
                std::io::Error::other("x"),
            )))),
            UploadOutcome::Error
        ));
        assert!(matches!(
            MediaManager::upload_outcome(None),
            UploadOutcome::Error
        ));
    }

    #[test]
    fn typed_content_type_is_preserved_and_absent_is_detected_from_filename() {
        assert_eq!("image/png".parse::<ContentType>().unwrap(), "image/png");
        assert_eq!(
            detect_content_type(&parse_filename("photo.jpg")),
            "image/jpeg"
        );
    }

    #[test]
    fn validate_filename_sanitizes_or_rejects() {
        assert_eq!(
            MediaManager::validate_filename(Some("test.jpg")).unwrap(),
            "test.jpg"
        );
        assert_eq!(
            MediaManager::validate_filename(None::<&str>).unwrap(),
            "upload"
        );
        assert!(MediaManager::validate_filename(Some("")).is_err());
        assert!(MediaManager::validate_filename(Some("..")).is_err());
    }

    #[test]
    fn upload_metadata_carries_validated_byte_size() {
        let metadata = UploadMetadata {
            filename: parse_filename("file.png"),
            content_type: parse_content_type("image/png"),
            sha256_hex: parse_content_hash(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
            ),
            size_bytes: parse_byte_size("0"),
        };

        assert_eq!(metadata.size_bytes, parse_byte_size("0"));
        assert!(ByteSize::try_from(-1).is_err());
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn register_in_db_maps_internal_create_error() {
        let mut media = crate::MockMediaStorage::new();
        media
            .expect_create_media()
            .times(1)
            .returning(|_| Err(CreateMediaError::Internal(sqlx::Error::PoolClosed)));
        let manager = MediaManager::new(
            Arc::new(media),
            Arc::new(crate::MockSiteConfigStorage::new()),
            Arc::new(PathBuf::from("/tmp")),
        );

        let err = manager
            .register_in_db(
                UserId::from(1),
                &parse_content_hash(
                    "deadbeef00000000000000000000000000000000000000000000000000000000",
                ),
                &parse_filename("file.png"),
                &parse_content_type("image/png"),
                parse_byte_size("100"),
            )
            .await
            .unwrap_err();

        let media_err = err
            .downcast_ref::<MediaError>()
            .expect("internal create error maps to MediaError");
        assert!(matches!(media_err, MediaError::Internal(_)));
    }

    #[test]
    fn continuation_reporting_cleanup_failures_preserve_quota_and_dedup_results_and_report_once() {
        let cleanup_error = || {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "temp cleanup denied",
            ))
        };
        let quota = anyhow::anyhow!(MediaError::InsufficientStorage);
        let (result, trace) = crate::helpers::swallowed_test::capture(|| {
            MediaManager::finish_temp_cleanup(
                Err::<UploadedMedia, _>(quota),
                cleanup_error(),
                "storage.media.quota_temp_cleanup",
            )
        });
        assert!(matches!(
            result
                .expect_err("quota error must remain primary")
                .downcast_ref::<MediaError>(),
            Some(MediaError::InsufficientStorage)
        ));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.media.quota_temp_cleanup",
        );

        let (result, trace) = crate::helpers::swallowed_test::capture(|| {
            MediaManager::finish_temp_cleanup(
                Ok::<bool, anyhow::Error>(true),
                cleanup_error(),
                "storage.media.dedup_temp_cleanup",
            )
        });
        assert!(result.expect("deduplication remains successful"));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.media.dedup_temp_cleanup",
        );
    }

    // guard:no-backend — mock store; the DB is unused by the dir scan
    #[tokio::test]
    async fn first_file_in_dir_skips_subdirs_and_finds_a_file() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();
        let manager = mock_manager(Arc::new(dir.to_path_buf()));

        assert_eq!(manager.first_file_in_dir(dir).await.unwrap(), None);

        // Dir with a subdir (should be ignored by is_file())
        let subdir = dir.join("subdir");
        fs::create_dir(&subdir).await.unwrap();
        assert_eq!(manager.first_file_in_dir(dir).await.unwrap(), None);

        let file = dir.join("test.txt");
        fs::write(&file, "hello").await.unwrap();
        assert_eq!(manager.first_file_in_dir(dir).await.unwrap(), Some(file));
    }

    // guard:no-backend — mock store; dedup is a pure filesystem operation
    #[tokio::test]
    async fn handle_deduplication_removes_links_or_renames() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();
        let media_dir = dir.join("media");
        fs::create_dir(&media_dir).await.unwrap();
        let tmp_dir = media_dir.join("tmp");
        fs::create_dir(&tmp_dir).await.unwrap();
        let manager = mock_manager(Arc::new(dir.to_path_buf()));

        let tmp_path = tmp_dir.join("temp_file");
        fs::write(&tmp_path, "content").await.unwrap();

        let target_path = media_dir.join("target_file");
        let hash_dir = media_dir.join("hash_dir");

        // Scenario 1: Target exists (should remove tmp)
        fs::write(&target_path, "existing").await.unwrap();
        manager
            .handle_deduplication(&tmp_path, &target_path, &hash_dir)
            .await
            .unwrap();
        assert!(!tmp_path.exists());
        assert!(target_path.exists());

        // Scenario 2: Target does not exist, but existing file in hash_dir
        fs::create_dir(&hash_dir).await.unwrap();
        let existing_file = hash_dir.join("existing_file");
        fs::write(&existing_file, "existing").await.unwrap();

        let tmp_path2 = tmp_dir.join("temp_file2");
        fs::write(&tmp_path2, "content").await.unwrap();
        let target_path2 = media_dir.join("target_file2");

        manager
            .handle_deduplication(&tmp_path2, &target_path2, &hash_dir)
            .await
            .unwrap();

        assert!(!tmp_path2.exists());
        assert!(target_path2.exists());
        // The target is hard-linked to the existing file; matching length is the
        // observable proxy.
        let meta1 = fs::metadata(&existing_file).await.unwrap();
        let meta2 = fs::metadata(&target_path2).await.unwrap();
        assert_eq!(meta1.len(), meta2.len());

        // Scenario 3: Neither exists (should rename)
        let tmp_path3 = tmp_dir.join("temp_file3");
        fs::write(&tmp_path3, "content").await.unwrap();
        let target_path3 = media_dir.join("target_file3");
        let hash_dir3 = media_dir.join("hash_dir3");

        manager
            .handle_deduplication(&tmp_path3, &target_path3, &hash_dir3)
            .await
            .unwrap();

        assert!(!tmp_path3.exists());
        assert!(target_path3.exists());
    }

    // guard:no-backend — injected directory enumeration failure
    #[tokio::test]
    async fn dedup_initial_directory_read_failure_propagates_before_success() {
        let temp = TempDir::new().unwrap();
        let media_dir = temp.path().join("media");
        let tmp_dir = media_dir.join("tmp");
        fs::create_dir_all(&tmp_dir).await.unwrap();
        let tmp_path = tmp_dir.join("upload");
        fs::write(&tmp_path, b"payload").await.unwrap();
        let target_path = media_dir.join("hash").join("upload.png");
        let entries: std::io::Result<futures_util::stream::Empty<std::io::Result<PathBuf>>> =
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "initial directory read sentinel",
            ));

        let existing_file = MediaManager::first_file_in_entries(entries).await;
        let error =
            MediaManager::finish_deduplication_from_result(&tmp_path, &target_path, existing_file)
                .await
                .expect_err("dedup must not report success");

        let source = error
            .downcast_ref::<std::io::Error>()
            .expect("typed initial read error");
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(source.to_string(), "initial directory read sentinel");
        assert!(tmp_path.exists(), "failed probe must not consume upload");
        assert!(!target_path.exists(), "failed probe must not create target");
    }

    // guard:no-backend — injected later directory enumeration failure
    #[tokio::test]
    async fn dedup_later_next_entry_failure_propagates_before_success() {
        let temp = TempDir::new().unwrap();
        let media_dir = temp.path().join("media");
        let tmp_dir = media_dir.join("tmp");
        let hash_dir = media_dir.join("hash");
        fs::create_dir_all(&tmp_dir).await.unwrap();
        fs::create_dir_all(hash_dir.join("subdir")).await.unwrap();
        let tmp_path = tmp_dir.join("upload");
        fs::write(&tmp_path, b"payload").await.unwrap();
        let target_path = hash_dir.join("upload.png");
        let entries = Ok(futures_util::stream::iter([
            Ok(hash_dir.join("subdir")),
            Err(std::io::Error::other("later next-entry sentinel")),
        ]));

        let existing_file = MediaManager::first_file_in_entries(entries).await;
        let error =
            MediaManager::finish_deduplication_from_result(&tmp_path, &target_path, existing_file)
                .await
                .expect_err("dedup must not report success after partial enumeration");

        let source = error
            .downcast_ref::<std::io::Error>()
            .expect("typed next-entry error");
        assert_eq!(source.kind(), std::io::ErrorKind::Other);
        assert_eq!(source.to_string(), "later next-entry sentinel");
        assert!(tmp_path.exists(), "failed probe must not consume upload");
        assert!(!target_path.exists(), "failed probe must not create target");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upload_bytes_is_content_addressed_and_idempotent(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        // A tiny PNG signature + IHDR-ish bytes (content need not be a valid image).
        let bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02, 0x03,
        ];
        let expected_sha = format!("{:x}", Sha256::digest(bytes));

        let first = manager
            .upload_bytes(
                user_id,
                &parse_filename("pic.png"),
                "image/png".parse().unwrap(),
                bytes,
            )
            .await
            .unwrap();
        assert_eq!(first.sha256.as_ref(), expected_sha.as_str());
        assert_eq!(first.filename, "pic.png");
        assert_eq!(first.content_type, "image/png");
        assert_eq!(
            first.size_bytes,
            ByteSize::try_from(i64::try_from(bytes.len()).unwrap()).unwrap()
        );

        // Identical re-upload must succeed and dedup to the same record.
        let second = manager
            .upload_bytes(
                user_id,
                &parse_filename("pic.png"),
                "image/png".parse().unwrap(),
                bytes,
            )
            .await
            .unwrap();
        assert_eq!(second.sha256, first.sha256);
        assert_eq!(second.url, first.url);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upload_bytes_rejects_oversized_payload(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        // Cap the per-file limit well below the payload size.
        env.state
            .site_config
            .set(SiteConfigKey::MediaMaxFileSizeBytes, "5")
            .await
            .unwrap();
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let err = manager
            .upload_bytes(
                user_id,
                &parse_filename("big.bin"),
                "application/octet-stream".parse().unwrap(),
                &[0_u8; 11],
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<MediaError>(),
            Some(MediaError::PayloadTooLarge)
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upload_streams_to_a_content_addressed_path(#[case] backend: Backend) {
        // The generic `stream_to_temp` path (multipart in production) had no host-level
        // unit test before — it was e2e-only. Drive it with an in-memory chunk stream so
        // the byte-stream branch stays covered.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let chunks = [
            Bytes::from_static(&[0x89, 0x50, 0x4E, 0x47]),
            Bytes::from_static(&[0x0D, 0x0A, 0x1A, 0x0A]),
        ];
        let mut hasher = Sha256::new();
        for chunk in &chunks {
            hasher.update(chunk);
        }
        let expected = ContentHash::from_digest(hasher.finalize().into());

        let stream = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, std::io::Error>));
        let filename = parse_filename("s.png");
        let resp = manager
            .upload(
                user_id,
                &filename,
                Some("image/png".parse().unwrap()),
                stream,
            )
            .await
            .unwrap();

        assert_eq!(resp.sha256, expected);
        assert_eq!(resp.content_type, "image/png");
        assert_eq!(
            resp.url,
            media_url(&MediaSource::Upload, &expected, &filename)
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_reclaims_unreferenced_file_and_quota(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let uploaded = manager
            .upload_bytes(
                user_id,
                &parse_filename("photo.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let media = upload_ref(&uploaded);
        let file_path = stored_path(env.base.path(), &media);
        assert!(file_path.exists(), "upload stores the file before deletion");

        assert_eq!(
            manager.delete_media(user_id, &media, false).await.unwrap(),
            TryDeleteOutcome::Deleted
        );

        assert!(!media_row_exists(&env.state, user_id, &media).await);
        assert_eq!(
            env.state
                .media
                .get_user_upload_usage(user_id)
                .await
                .unwrap(),
            parse_byte_size("0")
        );
        assert!(!file_path.exists(), "unreferenced delete reclaims the path");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_surfaces_unexpected_file_reclaim_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let uploaded = manager
            .upload_bytes(
                user_id,
                &parse_filename("blocked.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let media = upload_ref(&uploaded);
        let file_path = stored_path(env.base.path(), &media);
        std::fs::remove_file(&file_path).unwrap();
        std::fs::create_dir(&file_path).unwrap();

        let error = manager
            .delete_media(user_id, &media, false)
            .await
            .unwrap_err();

        assert!(matches!(
            error.downcast_ref::<MediaError>(),
            Some(MediaError::Internal(_))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_force_refuses_rowless_referenced_file(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let uploaded = manager
            .upload_bytes(
                user_id,
                &parse_filename("photo.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let media = upload_ref(&uploaded);
        let file_path = stored_path(env.base.path(), &media);
        create_post_via_service(
            &env.state,
            user_id,
            parse_post_body(&format!("<img src=\"{}\">", uploaded.url)),
        )
        .await;

        assert_eq!(
            manager.delete_media(user_id, &media, true).await.unwrap(),
            TryDeleteOutcome::RefusedReferenced
        );
        assert!(media_row_exists(&env.state, user_id, &media).await);
        assert!(file_path.exists(), "refused delete leaves referenced bytes");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_keeps_shared_path_until_last_row_is_deleted(#[case] backend: Backend) {
        let env = backend.setup().await;
        let first_user = SeedUser::new().seed(&env.state).await.user_id;
        let second_user = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let first = manager
            .upload_bytes(
                first_user,
                &parse_filename("photo.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let second = manager
            .upload_bytes(
                second_user,
                &parse_filename("photo.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let media = upload_ref(&first);
        let file_path = stored_path(env.base.path(), &media);
        assert_eq!(upload_ref(&second), media);

        manager
            .delete_media(first_user, &media, false)
            .await
            .unwrap();

        assert!(!media_row_exists(&env.state, first_user, &media).await);
        assert!(media_row_exists(&env.state, second_user, &media).await);
        assert!(file_path.exists(), "remaining media row retains the file");

        manager
            .delete_media(second_user, &media, false)
            .await
            .unwrap();

        assert!(!file_path.exists(), "last row deletion reclaims the file");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_reclaims_one_hard_link_without_removing_another_filename(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.site_config.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let first = manager
            .upload_bytes(
                user_id,
                &parse_filename("first.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let second = manager
            .upload_bytes(
                user_id,
                &parse_filename("second.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let first_media = upload_ref(&first);
        let second_media = upload_ref(&second);
        let first_path = stored_path(env.base.path(), &first_media);
        let second_path = stored_path(env.base.path(), &second_media);

        manager
            .delete_media(user_id, &first_media, false)
            .await
            .unwrap();

        assert!(!first_path.exists(), "deleted filename path is reclaimed");
        assert!(
            second_path.exists(),
            "different filename entry for the same hash remains served"
        );
        assert!(media_row_exists(&env.state, user_id, &second_media).await);
    }
}
