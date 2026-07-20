use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use common::ids::UserId;
use common::media::{
    detect_content_type, media_path, media_url, ContentHash, ContentType, Filename, MaxFileSize,
    UserQuota,
};
use storage::{CreateMediaError, MediaRecord, MediaSource, MediaStorage, SiteConfigStorage};
use web::auth::AuthUser;

use axum::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
enum MediaError {
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Payload too large")]
    PayloadTooLarge,
    #[error("Insufficient storage")]
    InsufficientStorage,
    #[error("Internal server error: {0}")]
    Internal(String),
}

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

    /// Accepts a multipart upload, stores the file content-addressed under
    /// `<storage_path>/media/upload/`, deduplicates via hard-links, inserts a DB
    /// record, and returns an `UploadResponse`.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` on validation failures, I/O errors, or quota exceeded.
    ///
    /// # Panics
    ///
    /// Panics if the target path does not have a parent directory.
    pub async fn upload(
        &self,
        auth_user: &AuthUser,
        mut field: axum::extract::multipart::Field<'_>,
    ) -> anyhow::Result<crate::media::UploadResponse> {
        let (max_file_size, user_quota) = self.get_limits().await?;

        let filename = Self::validate_filename(field.file_name())?;
        let content_type = Self::get_content_type(field.content_type(), &filename)?;

        let tmp_path = self.create_temp_file().await?;

        let (sha256_hex, size_bytes) = self
            .stream_to_temp(&mut field, &tmp_path, max_file_size)
            .await?;

        let metadata = UploadMetadata {
            filename,
            content_type,
            sha256_hex,
            size_bytes,
        };

        self.finalize_upload(auth_user.user_id, metadata, &tmp_path, user_quota)
            .await
    }

    /// Validates a filename and returns a sanitized version.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` if the filename is empty after sanitization.
    pub fn validate_filename(file_name: Option<&str>) -> anyhow::Result<Filename> {
        let raw_name = file_name.unwrap_or("upload");
        // Door B: normalize the client's arbitrary name to a safe leaf, rejecting an
        // empty-after-sanitize result as a bad request.
        Filename::sanitized(raw_name)
            .map_err(|_| anyhow::anyhow!(MediaError::BadRequest("Invalid filename".to_owned())))
    }

    /// The single validating content-type door, shared by the multipart and atompub intake
    /// paths: a present client `Content-Type` is validated (a malformed one is a bad
    /// request), an absent one is detected from the filename.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` (`MediaError::BadRequest`) when `content_type` is present but
    /// not a valid `type/subtype` media type.
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

    #[must_use]
    pub fn map_error(err: &anyhow::Error) -> StatusCode {
        let media_err = err.downcast_ref::<MediaError>();
        host::metrics::media_upload(Self::upload_outcome(media_err));
        match media_err {
            Some(MediaError::BadRequest(_)) => StatusCode::BAD_REQUEST,
            Some(MediaError::PayloadTooLarge) => StatusCode::PAYLOAD_TOO_LARGE,
            Some(MediaError::InsufficientStorage) => StatusCode::INSUFFICIENT_STORAGE,
            Some(MediaError::Internal(_)) | None => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Maps a failed upload to its bounded `outcome` attribute for the
    /// `jaunder.media.uploads` metric. A non-`MediaError` (unexpected I/O, etc.)
    /// counts as `error`. Exhaustively tested so every arm's mapping is covered.
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

        if current_usage + size_bytes > user_quota.value() {
            anyhow::bail!(MediaError::InsufficientStorage);
        }
        Ok(())
    }

    /// Content-addresses the temp file at `target_path`, deduplicating against
    /// already-stored identical content. Returns `true` when the bytes were
    /// deduplicated (the target already existed, or an identical file was
    /// hard-linked) and `false` when this is a freshly stored file.
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
            size_bytes,
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

    /// Shared finalization for an upload whose bytes are already written to
    /// `tmp_path` with a known content hash and size: enforces quota, content-
    /// addresses the file (dedup via hard-link), records it in the DB, and builds
    /// the response. The temp file is consumed (moved, linked, or removed).
    async fn finalize_upload(
        &self,
        user_id: UserId,
        metadata: UploadMetadata,
        tmp_path: &Path,
        user_quota: UserQuota,
    ) -> anyhow::Result<crate::media::UploadResponse> {
        if let Err(e) = self
            .check_quota(user_id, metadata.size_bytes, user_quota)
            .await
        {
            let _ = fs::remove_file(tmp_path).await;
            return Err(e);
        }
        let relative_path = media_path("upload", &metadata.sha256_hex, &metadata.filename);
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
        Ok(crate::media::UploadResponse {
            sha256: metadata.sha256_hex,
            filename: metadata.filename,
            content_type: metadata.content_type,
            size_bytes: metadata.size_bytes,
            url,
        })
    }

    /// Uploads raw in-memory bytes (e.g. an `AtomPub` media POST), reusing the same
    /// content-addressing, dedup, quota, and DB-record path as multipart uploads.
    /// Returns the existing record's response when identical content was already
    /// stored (idempotent).
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` on an invalid filename, oversized payload, quota
    /// exhaustion, I/O failure, or DB error.
    pub async fn upload_bytes(
        &self,
        auth_user: &AuthUser,
        filename: &Filename,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<crate::media::UploadResponse> {
        let (max_file_size, user_quota) = self.get_limits().await?;
        // `filename` is already a validated `Filename` (the caller ran Door B on the
        // client's name), so there is no re-sanitize here.
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

        self.finalize_upload(auth_user.user_id, metadata, &tmp_path, user_quota)
            .await
    }

    async fn stream_to_temp(
        &self,
        field: &mut axum::extract::multipart::Field<'_>,
        tmp_path: &Path,
        max_file_size: MaxFileSize,
    ) -> anyhow::Result<(ContentHash, i64)> {
        let mut file = fs::File::create(tmp_path).await?;
        let mut hasher = Sha256::new();
        let mut bytes_written: i64 = 0;

        while let Some(chunk) = field.chunk().await? {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{media, migrated_sqlite_db, site_config, users};
    use common::test_support::{parse_content_hash, parse_content_type, parse_filename};
    use storage::MEDIA_MAX_FILE_SIZE_BYTES_KEY;
    use tempfile::TempDir;

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
            MediaManager::upload_outcome(Some(&MediaError::Internal("x".to_owned()))),
            UploadOutcome::Error
        ));
        assert!(matches!(
            MediaManager::upload_outcome(None),
            UploadOutcome::Error
        ));
    }

    #[test]
    fn get_content_type_validates_present_and_detects_absent() {
        // The single door (#495): a malformed present client `Content-Type` is a bad
        // request, a valid one is taken verbatim, and an absent one is detected.
        assert!(MediaManager::get_content_type(Some("garbage"), "x.png").is_err());
        assert_eq!(
            MediaManager::get_content_type(Some("image/png"), "x.bin").unwrap(),
            "image/png"
        );
        assert_eq!(
            MediaManager::get_content_type(None, "photo.jpg").unwrap(),
            "image/jpeg"
        );
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn register_in_db_maps_internal_create_error() {
        let mut media = storage::MockMediaStorage::new();
        media
            .expect_create_media()
            .times(1)
            .returning(|_| Err(CreateMediaError::Internal(sqlx::Error::PoolClosed)));
        let manager = MediaManager::new(
            Arc::new(media),
            Arc::new(storage::MockSiteConfigStorage::new()),
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
                100,
            )
            .await
            .unwrap_err();

        let media_err = err
            .downcast_ref::<MediaError>()
            .expect("internal create error maps to MediaError");
        assert!(matches!(media_err, MediaError::Internal(_)));
    }

    #[tokio::test]
    async fn test_first_file_in_dir() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        // Keep the DB out of `dir`, which the test scans for media files.
        let db = TempDir::new().unwrap();
        let (_, pool) = migrated_sqlite_db(db.path()).await;
        let storage_path = Arc::new(dir.to_path_buf());
        let manager = MediaManager::new(media(&pool), site_config(&pool), storage_path);

        // Empty dir
        assert_eq!(manager.first_file_in_dir(dir).await, None);

        // Dir with a subdir (should be ignored by is_file())
        let subdir = dir.join("subdir");
        fs::create_dir(&subdir).await.unwrap();
        assert_eq!(manager.first_file_in_dir(dir).await, None);

        // Dir with a file
        let file = dir.join("test.txt");
        fs::write(&file, "hello").await.unwrap();
        assert_eq!(manager.first_file_in_dir(dir).await, Some(file));
    }

    #[tokio::test]
    async fn test_handle_deduplication() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();
        let media_dir = dir.join("media");
        fs::create_dir(&media_dir).await.unwrap();
        let tmp_dir = media_dir.join("tmp");
        fs::create_dir(&tmp_dir).await.unwrap();

        // The DB is unused by dedup; keep it out of the scanned storage dir.
        let db = TempDir::new().unwrap();
        let (_, pool) = migrated_sqlite_db(db.path()).await;
        let storage_path = Arc::new(dir.to_path_buf());
        let manager = MediaManager::new(media(&pool), site_config(&pool), storage_path);

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
        // Verify it's a hard link by checking if they are the same file
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

    #[tokio::test]
    async fn test_validate_filename() {
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

    #[tokio::test]
    async fn test_map_error() {
        assert_eq!(
            MediaManager::map_error(&anyhow::anyhow!(MediaError::BadRequest("bad".to_owned()))),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            MediaManager::map_error(&anyhow::anyhow!(MediaError::PayloadTooLarge)),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            MediaManager::map_error(&anyhow::anyhow!(MediaError::InsufficientStorage)),
            StatusCode::INSUFFICIENT_STORAGE
        );
        assert_eq!(
            MediaManager::map_error(&anyhow::anyhow!(MediaError::Internal("error".to_owned()))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            MediaManager::map_error(&anyhow::anyhow!("unknown")),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn upload_bytes_is_content_addressed_and_idempotent() {
        let temp = TempDir::new().unwrap();
        let (_, pool) = migrated_sqlite_db(temp.path()).await;
        let user_id = users(&pool)
            .create_user(
                &"uploader".parse().unwrap(),
                &"password123".parse().unwrap(),
                None,
                false,
            )
            .await
            .unwrap();
        let storage_path = Arc::new(temp.path().to_path_buf());
        let manager = MediaManager::new(media(&pool), site_config(&pool), storage_path);

        let auth = web::auth::AuthUser {
            user_id,
            username: "uploader".parse().unwrap(),
            token_hash: common::token::TokenHash::from_digest(""),
        };

        // A tiny PNG signature + IHDR-ish bytes (content need not be a valid image).
        let bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02, 0x03,
        ];
        let expected_sha = format!("{:x}", Sha256::digest(bytes));

        let first = manager
            .upload_bytes(&auth, &parse_filename("pic.png"), "image/png", bytes)
            .await
            .unwrap();
        assert_eq!(first.sha256.as_ref(), expected_sha.as_str());
        assert_eq!(first.filename, "pic.png");
        assert_eq!(first.content_type, "image/png");
        assert_eq!(first.size_bytes, i64::try_from(bytes.len()).unwrap());

        // Identical re-upload must succeed and dedup to the same record.
        let second = manager
            .upload_bytes(&auth, &parse_filename("pic.png"), "image/png", bytes)
            .await
            .unwrap();
        assert_eq!(second.sha256, first.sha256);
        assert_eq!(second.url, first.url);
    }

    #[tokio::test]
    async fn upload_bytes_rejects_oversized_payload() {
        let temp = TempDir::new().unwrap();
        let (_, pool) = migrated_sqlite_db(temp.path()).await;
        let cfg = site_config(&pool);
        // Cap the per-file limit well below the payload size.
        cfg.set(MEDIA_MAX_FILE_SIZE_BYTES_KEY, "5").await.unwrap();
        let user_id = users(&pool)
            .create_user(
                &"uploader".parse().unwrap(),
                &"password123".parse().unwrap(),
                None,
                false,
            )
            .await
            .unwrap();
        let manager = MediaManager::new(media(&pool), cfg, Arc::new(temp.path().to_path_buf()));
        let auth = web::auth::AuthUser {
            user_id,
            username: "uploader".parse().unwrap(),
            token_hash: common::token::TokenHash::from_digest(""),
        };

        let err = manager
            .upload_bytes(
                &auth,
                &parse_filename("big.bin"),
                "application/octet-stream",
                &[0_u8; 11],
            )
            .await
            .unwrap_err();
        assert_eq!(MediaManager::map_error(&err), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
