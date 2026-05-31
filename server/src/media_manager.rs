use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use common::media::{detect_content_type, media_path, media_url, sanitize_filename};
use storage::{
    AppState, CreateMediaError, MediaRecord, MediaSource, DEFAULT_MAX_FILE_SIZE_BYTES,
    DEFAULT_USER_QUOTA_BYTES, MEDIA_MAX_FILE_SIZE_BYTES_KEY, MEDIA_USER_QUOTA_BYTES_KEY,
};
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
    state: Arc<AppState>,
    storage_path: Arc<PathBuf>,
}

/// File metadata for upload finalization.
#[derive(Debug)]
struct UploadMetadata {
    filename: String,
    content_type: String,
    sha256_hex: String,
    size_bytes: i64,
}

impl MediaManager {
    #[must_use]
    pub fn new(state: Arc<AppState>, storage_path: Arc<PathBuf>) -> Self {
        Self {
            state,
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
        let content_type = Self::get_content_type(field.content_type(), &filename);

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
    #[allow(clippy::needless_pass_by_value)]
    pub fn validate_filename(file_name: Option<impl AsRef<str>>) -> anyhow::Result<String> {
        let raw_name = file_name
            .as_ref()
            .map_or("upload", std::convert::AsRef::as_ref);
        let name = sanitize_filename(raw_name);
        if name.is_empty() {
            anyhow::bail!(MediaError::BadRequest("Invalid filename".to_owned()));
        }
        Ok(name)
    }

    pub fn get_content_type(content_type: Option<&str>, filename: &str) -> String {
        content_type.map_or_else(|| detect_content_type(filename).to_owned(), str::to_owned)
    }

    #[must_use]
    pub fn map_error(err: &anyhow::Error) -> StatusCode {
        if let Some(media_err) = err.downcast_ref::<MediaError>() {
            match media_err {
                MediaError::BadRequest(_) => StatusCode::BAD_REQUEST,
                MediaError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
                MediaError::InsufficientStorage => StatusCode::INSUFFICIENT_STORAGE,
                MediaError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            }
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }

    async fn get_limits(&self) -> anyhow::Result<(i64, i64)> {
        let max_file_size = self
            .state
            .site_config
            .get_int(MEDIA_MAX_FILE_SIZE_BYTES_KEY, DEFAULT_MAX_FILE_SIZE_BYTES)
            .await;

        let user_quota = self
            .state
            .site_config
            .get_int(MEDIA_USER_QUOTA_BYTES_KEY, DEFAULT_USER_QUOTA_BYTES)
            .await;
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
        user_id: i64,
        size_bytes: i64,
        user_quota: i64,
    ) -> anyhow::Result<()> {
        let current_usage = self.state.media.get_user_upload_usage(user_id).await?;

        if current_usage + size_bytes > user_quota {
            anyhow::bail!(MediaError::InsufficientStorage);
        }
        Ok(())
    }

    async fn handle_deduplication(
        &self,
        tmp_path: &PathBuf,
        target_path: &PathBuf,
        hash_dir: &PathBuf,
    ) -> anyhow::Result<()> {
        if target_path.exists() {
            let _ = fs::remove_file(tmp_path).await;
        } else {
            let existing_file = self.first_file_in_dir(hash_dir).await;

            fs::create_dir_all(hash_dir).await?;

            if let Some(existing) = existing_file {
                fs::hard_link(&existing, target_path).await?;
                let _ = fs::remove_file(tmp_path).await;
            } else {
                fs::rename(tmp_path, target_path).await?;
            }
        }
        Ok(())
    }

    async fn register_in_db(
        &self,
        user_id: i64,
        sha256_hex: &str,
        filename: &str,
        content_type: &str,
        size_bytes: i64,
    ) -> anyhow::Result<()> {
        let record = MediaRecord {
            user_id,
            sha256: sha256_hex.to_owned(),
            filename: filename.to_owned(),
            source: MediaSource::Upload,
            content_type: content_type.to_owned(),
            size_bytes,
            source_url: None,
            created_at: Utc::now(),
        };
        match self.state.media.create_media(&record).await {
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
    ///
    /// # Panics
    ///
    /// Panics if the content-addressed target path has no parent directory.
    async fn finalize_upload(
        &self,
        user_id: i64,
        metadata: UploadMetadata,
        tmp_path: &Path,
        user_quota: i64,
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
        let hash_dir = target_path
            .parent()
            .expect("target_path should have parent")
            .to_path_buf();
        self.handle_deduplication(&tmp_path.to_path_buf(), &target_path, &hash_dir)
            .await?;
        self.register_in_db(
            user_id,
            &metadata.sha256_hex,
            &metadata.filename,
            &metadata.content_type,
            metadata.size_bytes,
        )
        .await?;
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
    ///
    /// # Panics
    ///
    /// Panics if the content-addressed target path has no parent directory.
    pub async fn upload_bytes(
        &self,
        auth_user: &AuthUser,
        filename: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<crate::media::UploadResponse> {
        let (max_file_size, user_quota) = self.get_limits().await?;
        let filename = Self::validate_filename(Some(filename))?;
        let content_type = Self::get_content_type(Some(content_type), &filename);

        let size_bytes = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        if size_bytes > max_file_size {
            anyhow::bail!(MediaError::PayloadTooLarge);
        }

        let digest = Sha256::digest(bytes);
        let sha256_hex = format!("{digest:x}");

        let tmp_path = self.create_temp_file().await?;
        fs::write(&tmp_path, bytes).await?;

        let metadata = UploadMetadata {
            filename,
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
        max_file_size: i64,
    ) -> anyhow::Result<(String, i64)> {
        let mut file = fs::File::create(tmp_path).await?;
        let mut hasher = Sha256::new();
        let mut bytes_written: i64 = 0;

        while let Some(chunk) = field.chunk().await? {
            bytes_written += i64::try_from(chunk.len()).unwrap_or(i64::MAX);
            if bytes_written > max_file_size {
                anyhow::bail!(MediaError::PayloadTooLarge);
            }

            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }

        file.flush().await?;
        drop(file);

        let digest = hasher.finalize();
        let sha256_hex = format!("{digest:x}");
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
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_first_file_in_dir() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let state = storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap();
        let storage_path = Arc::new(dir.to_path_buf());
        let manager = MediaManager::new(state, storage_path);

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

        let state = storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap();
        let storage_path = Arc::new(dir.to_path_buf());
        let manager = MediaManager::new(state, storage_path);

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
        let state = storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap();
        let user_id = state
            .users
            .create_user(
                &"uploader".parse().unwrap(),
                &"password123".parse().unwrap(),
                None,
                false,
            )
            .await
            .unwrap();
        let storage_path = Arc::new(temp.path().to_path_buf());
        let manager = MediaManager::new(state, storage_path);

        let auth = web::auth::AuthUser {
            user_id,
            username: "uploader".parse().unwrap(),
            token_hash: String::new(),
        };

        // A tiny PNG signature + IHDR-ish bytes (content need not be a valid image).
        let bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02, 0x03,
        ];
        let expected_sha = format!("{:x}", Sha256::digest(bytes));

        let first = manager
            .upload_bytes(&auth, "pic.png", "image/png", bytes)
            .await
            .unwrap();
        assert_eq!(first.sha256, expected_sha);
        assert_eq!(first.filename, "pic.png");
        assert_eq!(first.content_type, "image/png");
        assert_eq!(first.size_bytes, i64::try_from(bytes.len()).unwrap());

        // Identical re-upload must succeed and dedup to the same record.
        let second = manager
            .upload_bytes(&auth, "pic.png", "image/png", bytes)
            .await
            .unwrap();
        assert_eq!(second.sha256, first.sha256);
        assert_eq!(second.url, first.url);
    }

    #[tokio::test]
    async fn upload_bytes_rejects_oversized_payload() {
        let temp = TempDir::new().unwrap();
        let state = storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap();
        // Cap the per-file limit well below the payload size.
        state
            .site_config
            .set(MEDIA_MAX_FILE_SIZE_BYTES_KEY, "5")
            .await
            .unwrap();
        let user_id = state
            .users
            .create_user(
                &"uploader".parse().unwrap(),
                &"password123".parse().unwrap(),
                None,
                false,
            )
            .await
            .unwrap();
        let manager = MediaManager::new(state, Arc::new(temp.path().to_path_buf()));
        let auth = web::auth::AuthUser {
            user_id,
            username: "uploader".parse().unwrap(),
            token_hash: String::new(),
        };

        let err = manager
            .upload_bytes(&auth, "big.bin", "application/octet-stream", &[0_u8; 11])
            .await
            .unwrap_err();
        assert_eq!(MediaManager::map_error(&err), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
