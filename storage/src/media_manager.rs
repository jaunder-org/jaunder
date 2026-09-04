//! Root-injected media operation service. It streams uploads to content-addressed,
//! deduplicated paths, enforces file and user limits, and owns the evidence-bearing
//! deletion sequence.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt, stream};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::InstanceId;
use common::ids::{PostId, UserId};
use common::media::{
    self, ByteSize, ContentHash, ContentType, Filename, MaxFileSize, MediaRef, MediaSource,
    UploadedMedia, UserQuota,
};
use common::time::UtcInstant;
use host::metrics::{self, UploadOutcome};

use crate::media_ownership::resolve_media_reference_ownership;
use crate::posts::media::{
    MediaReferenceEvidence, MediaReferenceSnapshot, PersistedMediaReference,
};
use crate::{
    CreateMediaError, MediaContentLocks, MediaDeleteMode, MediaRecord,
    MediaReferenceOwnershipResolver, MediaStorage, PostStorage, SiteConfigStorage,
    TryDeleteOutcome, WriteScope, WriteScopeError,
};
use common::MutationOutcome;

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
    #[error("media uploads are disabled")]
    UploadsDisabled,
    #[error("Internal server error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Failure to establish the clean temporary upload directory required at startup.
#[derive(Debug, Error)]
#[error(
    "failed {operation} media temporary directory {}: {source}",
    path.display()
)]
pub struct MediaTemporaryDirectoryError {
    /// The filesystem operation that failed.
    pub operation: &'static str,
    /// The temporary directory that could not be prepared.
    pub path: PathBuf,
    #[source]
    source: io::Error,
}

impl MediaTemporaryDirectoryError {
    fn new(operation: &'static str, path: PathBuf, source: io::Error) -> Self {
        Self {
            operation,
            path,
            source,
        }
    }
}

// `UploadedMedia` is defined in `common::media`, not here — it is the `#[server]` fn's
// return type, which must be nameable on the wasm client build where `storage` is not
// compiled (`storage` is a `server`-gated `web` dep). `common` is ungated and reachable
// by storage + web (both targets) + server, so the manager returns it directly with no
// mapping layer.

/// The manager's completed upload result, including whether its media record was
/// already present for the user. Protocol adapters use this to select their
/// creation versus idempotent-reupload response without a pre-admission lookup.
#[derive(Debug)]
pub struct ManagedUpload {
    /// Metadata and URL for the stored media.
    pub media: UploadedMedia,
    /// Whether the user's exact media record existed before this upload.
    pub already_existed: bool,
}

/// Result of one evidence-bearing media deletion attempt.
#[derive(Debug)]
pub struct MediaDeletionResult {
    outcome: MutationOutcome<TryDeleteOutcome>,
    references: MediaReferenceSnapshot,
    evidence: MediaReferenceEvidence,
}

impl MediaDeletionResult {
    fn new(
        outcome: MutationOutcome<TryDeleteOutcome>,
        references: MediaReferenceSnapshot,
        evidence: MediaReferenceEvidence,
    ) -> Self {
        Self {
            outcome,
            references,
            evidence,
        }
    }

    /// Borrow the guarded storage outcome.
    #[must_use]
    pub fn outcome(&self) -> &MutationOutcome<TryDeleteOutcome> {
        &self.outcome
    }

    /// Consume the evidence wrapper and return the guarded storage outcome.
    #[must_use]
    pub fn into_outcome(self) -> MutationOutcome<TryDeleteOutcome> {
        self.outcome
    }

    /// Return owner-scoped Posts that explain a retained-reference refusal.
    #[must_use]
    pub fn referenced_post_ids(&self, user_id: UserId) -> Vec<PostId> {
        if !matches!(self.outcome.value(), TryDeleteOutcome::RefusedReferenced) {
            return Vec::new();
        }
        self.references
            .references()
            .iter()
            .filter(|reference| {
                reference.owner_id() == Some(user_id) && !self.evidence.proves_foreign(reference)
            })
            .map(PersistedMediaReference::post_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

pub struct MediaManager {
    media: Arc<dyn MediaStorage>,
    posts: Arc<dyn PostStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    write_scope: WriteScope,
    storage_path: Arc<PathBuf>,
    content_locks: Arc<MediaContentLocks>,
    instance_id: InstanceId,
    ownership_resolver: Arc<dyn MediaReferenceOwnershipResolver>,
}

/// File metadata for upload finalization.
#[derive(Debug)]
struct UploadMetadata {
    filename: Filename,
    content_type: ContentType,
    sha256_hex: ContentHash,
    size_bytes: ByteSize,
}

/// How finalization placed the target file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetDisposition {
    ExistingTarget,
    CreatedHardLink,
    FreshRename,
}

impl TargetDisposition {
    fn is_deduplicated(self) -> bool {
        !matches!(self, Self::FreshRename)
    }

    fn was_created_by_upload(self) -> bool {
        !matches!(self, Self::ExistingTarget)
    }
}

impl MediaManager {
    #[must_use]
    pub fn new(
        media: Arc<dyn MediaStorage>,
        posts: Arc<dyn PostStorage>,
        site_config: Arc<dyn SiteConfigStorage>,
        write_scope: WriteScope,
        content_locks: Arc<MediaContentLocks>,
        instance_id: InstanceId,
        ownership_resolver: Arc<dyn MediaReferenceOwnershipResolver>,
    ) -> Self {
        Self {
            media,
            posts,
            site_config,
            write_scope,
            storage_path: Arc::clone(content_locks.storage_path()),
            content_locks,
            instance_id,
            ownership_resolver,
        }
    }

    /// Removes crash-orphaned upload artifacts and recreates `media/tmp` empty.
    ///
    /// This is deliberately confined to the transient directory. `remove_dir_all`
    /// removes a symlink itself rather than following it, so finalized media paths
    /// cannot be reached through a stale temporary-directory entry.
    ///
    /// # Errors
    ///
    /// Returns a typed error when removing stale artifacts or creating the usable
    /// replacement directory fails.
    pub async fn prepare_temporary_upload_directory(
        storage_path: &Path,
    ) -> Result<(), MediaTemporaryDirectoryError> {
        let tmp_dir = storage_path.join("media").join("tmp");
        match fs::remove_dir_all(&tmp_dir).await {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(MediaTemporaryDirectoryError::new(
                    "removing", tmp_dir, source,
                ));
            }
        }
        fs::create_dir_all(&tmp_dir)
            .await
            .map_err(|source| MediaTemporaryDirectoryError::new("creating", tmp_dir, source))
    }

    /// Streams a multipart upload to a content-addressed, dedup'd path and records
    /// it. `filename`/`content_type` are extracted by the caller off its multipart
    /// field (before the field is consumed as the byte stream); `stream` yields the
    /// file bytes. Emits exactly one `media_upload*` metric (success in
    /// `finalize_upload`, failure here).
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` when uploads are disabled, on validation failure, quota
    /// exhaustion, or I/O error.
    pub async fn upload<S, E>(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: Option<ContentType>,
        stream: S,
    ) -> anyhow::Result<MutationOutcome<UploadedMedia>>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let result = self
            .upload_inner(user_id, filename, content_type, stream)
            .await
            .map(|outcome| outcome.map(|upload| upload.media));
        Self::emit_failure_metric(&result);
        result
    }

    async fn upload_inner<S, E>(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: Option<ContentType>,
        stream: S,
    ) -> anyhow::Result<MutationOutcome<ManagedUpload>>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.ensure_uploads_enabled().await?;
        let (max_file_size, user_quota) = self.get_limits().await?;

        let content_type = content_type.unwrap_or_else(|| media::detect_content_type(filename));

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
    fn emit_failure_metric<T>(result: &anyhow::Result<MutationOutcome<T>>) {
        if let Err(err) = result {
            metrics::media_upload(Self::upload_outcome(err.downcast_ref::<MediaError>()));
        }
    }

    /// Maps a failed upload to its bounded `outcome` attribute for the
    /// `jaunder.media.uploads` metric. A non-`MediaError` counts as `error`.
    fn upload_outcome(err: Option<&MediaError>) -> UploadOutcome {
        match err {
            Some(MediaError::BadRequest(_)) => UploadOutcome::Invalid,
            Some(MediaError::PayloadTooLarge) => UploadOutcome::TooLarge,
            Some(MediaError::InsufficientStorage) => UploadOutcome::QuotaExceeded,
            Some(MediaError::UploadsDisabled) => UploadOutcome::Disabled,
            Some(MediaError::Internal(_)) | None => UploadOutcome::Error,
        }
    }

    /// Captures the media upload capability once at attempt entry. This is
    /// intentionally separate from limits so an admitted upload cannot be revoked by a
    /// later settings change.
    async fn ensure_uploads_enabled(&self) -> anyhow::Result<()> {
        if self.site_config.get_media_uploads_enabled().await? {
            Ok(())
        } else {
            anyhow::bail!(MediaError::UploadsDisabled);
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

    fn scope_error(error: WriteScopeError<anyhow::Error>) -> anyhow::Error {
        match error {
            WriteScopeError::Operation(error) => error,
            WriteScopeError::Begin(error) => {
                anyhow::anyhow!(MediaError::Internal(Box::new(error)))
            }
        }
    }

    /// Content-addresses the temp file at `target_path`, distinguishing a
    /// pre-existing target from one created by this upload.
    async fn handle_deduplication(
        tmp_path: &Path,
        target_path: &Path,
        hash_dir: &Path,
    ) -> anyhow::Result<TargetDisposition> {
        if target_path.exists() {
            return Self::finish_temp_cleanup(
                Ok(TargetDisposition::ExistingTarget),
                fs::remove_file(tmp_path).await,
                "storage.media.dedup_temp_cleanup",
            );
        }

        // A new hash has no directory yet. Create it before enumeration so
        // `NotFound` is not confused with an expected empty directory; after
        // this point every `read_dir`/`next_entry` error is unexpected and must
        // propagate.
        fs::create_dir_all(hash_dir).await?;
        let existing_file = Self::first_file_in_dir(hash_dir).await;
        Self::finish_deduplication_from_result(tmp_path, target_path, existing_file).await
    }

    async fn finish_deduplication_from_result(
        tmp_path: &Path,
        target_path: &Path,
        existing_file: io::Result<Option<PathBuf>>,
    ) -> anyhow::Result<TargetDisposition> {
        Self::finish_deduplication(tmp_path, target_path, existing_file?).await
    }

    async fn finish_deduplication(
        tmp_path: &Path,
        target_path: &Path,
        existing_file: Option<PathBuf>,
    ) -> anyhow::Result<TargetDisposition> {
        if let Some(existing) = existing_file {
            fs::hard_link(&existing, target_path).await?;
            Self::finish_temp_cleanup(
                Ok(TargetDisposition::CreatedHardLink),
                fs::remove_file(tmp_path).await,
                "storage.media.dedup_temp_cleanup",
            )
        } else {
            fs::rename(tmp_path, target_path).await?;
            Ok(TargetDisposition::FreshRename)
        }
    }

    /// Places an upload and records it under the media content lock.
    ///
    /// Filesystem placement and cleanup happen outside the database write
    /// scope. The cross-process content lock prevents another uploader from
    /// adopting a newly placed target before a failed registration removes it.
    async fn place_and_register(
        &self,
        record: MediaRecord,
        media_ref: MediaRef,
        tmp_path: PathBuf,
        target_path: PathBuf,
        hash_dir: PathBuf,
    ) -> anyhow::Result<(TargetDisposition, MutationOutcome<bool>)> {
        let _content_lock = match self.content_locks.acquire_one(&media_ref.sha256).await {
            Ok(content_lock) => content_lock,
            Err(error) => {
                return Self::finish_temp_cleanup(
                    Err(error.into()),
                    fs::remove_file(&tmp_path).await,
                    "storage.media.content_lock_temp_cleanup",
                );
            }
        };
        let disposition = match Self::handle_deduplication(&tmp_path, &target_path, &hash_dir).await
        {
            Ok(disposition) => disposition,
            Err(error) => {
                return Self::finish_temp_cleanup(
                    Err(error),
                    fs::remove_file(&tmp_path).await,
                    "storage.media.placement_temp_cleanup",
                );
            }
        };

        let media = Arc::clone(&self.media);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    media
                        .lock_media_reference(transaction, &media_ref)
                        .await
                        .map_err(|error| anyhow::anyhow!(MediaError::Internal(Box::new(error))))?;
                    match media.create_media(transaction, &record).await {
                        Ok(()) => Ok(false),
                        Err(CreateMediaError::AlreadyExists) => Ok(true),
                        Err(CreateMediaError::Internal(error)) => {
                            tracing::error!(error = %error, "create_media failed");
                            Err(anyhow::anyhow!(MediaError::Internal(Box::new(error))))
                        }
                    }
                })
            })
            .await;

        match outcome {
            Ok(outcome) => Ok((disposition, outcome)),
            Err(error) if disposition.was_created_by_upload() => Self::finish_temp_cleanup(
                Err(Self::scope_error(error)),
                fs::remove_file(&target_path).await,
                "storage.media.create_target_cleanup",
            ),
            Err(error) => Err(Self::scope_error(error)),
        }
    }

    /// Shared finalization for an upload whose bytes are already written to
    /// `tmp_path` with a known content hash and size: enforces quota, then uses
    /// the cross-process content lock to serialize filesystem placement with a
    /// short database registration scope. Streaming, hashing, placement, and
    /// cleanup remain outside the database scope.
    async fn finalize_upload(
        &self,
        user_id: UserId,
        metadata: UploadMetadata,
        tmp_path: &Path,
        user_quota: UserQuota,
    ) -> anyhow::Result<MutationOutcome<ManagedUpload>> {
        if let Err(error) = self
            .check_quota(user_id, metadata.size_bytes, user_quota)
            .await
        {
            return Self::finish_temp_cleanup(
                Err(error),
                fs::remove_file(tmp_path).await,
                "storage.media.quota_temp_cleanup",
            );
        }
        let relative_path = media::path(
            &MediaSource::Upload,
            &metadata.sha256_hex,
            &metadata.filename,
        );
        let target_path = self.storage_path.join("media").join(&relative_path);
        let hash_dir = target_path
            .parent()
            .unwrap_or_else(|| {
                unreachable!("media target path is constructed beneath storage root")
            })
            .to_path_buf();
        let media_ref = MediaRef {
            source: MediaSource::Upload,
            sha256: metadata.sha256_hex.clone(),
            filename: metadata.filename.clone(),
        };
        let record = MediaRecord {
            user_id,
            sha256: metadata.sha256_hex.clone(),
            filename: metadata.filename.clone(),
            source: MediaSource::Upload,
            content_type: metadata.content_type.clone(),
            size_bytes: metadata.size_bytes,
            source_url: None,
            created_at: UtcInstant::now(),
        };
        let (target_disposition, outcome) = self
            .place_and_register(
                record,
                media_ref,
                tmp_path.to_path_buf(),
                target_path,
                hash_dir,
            )
            .await?;
        metrics::media_upload_bytes(metadata.size_bytes.value().unsigned_abs());
        metrics::media_upload(if target_disposition.is_deduplicated() {
            UploadOutcome::Deduplicated
        } else {
            UploadOutcome::Stored
        });
        let url = media::url(
            &MediaSource::Upload,
            &metadata.sha256_hex,
            &metadata.filename,
        );
        let response = UploadedMedia {
            sha256: metadata.sha256_hex,
            filename: metadata.filename,
            content_type: metadata.content_type,
            size_bytes: metadata.size_bytes,
            url,
        };
        Ok(outcome.map(|already_existed| ManagedUpload {
            media: response,
            already_existed,
        }))
    }

    /// Uploads raw in-memory bytes (e.g. an `AtomPub` media POST), reusing the shared
    /// upload pipeline.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` when uploads are disabled, on invalid filename, oversized
    /// payload, quota exhaustion, I/O failure, or DB error.
    pub async fn upload_bytes(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: ContentType,
        bytes: &[u8],
    ) -> anyhow::Result<MutationOutcome<UploadedMedia>> {
        self.upload_bytes_with_disposition(user_id, filename, content_type, bytes)
            .await
            .map(|outcome| outcome.map(|upload| upload.media))
    }

    /// Uploads raw bytes and retains the manager-owned idempotency disposition.
    ///
    /// # Errors
    ///
    /// Returns `anyhow::Error` when uploads are disabled, on invalid filename, oversized
    /// payload, quota exhaustion, I/O failure, or DB error.
    pub async fn upload_bytes_with_disposition(
        &self,
        user_id: UserId,
        filename: &Filename,
        content_type: ContentType,
        bytes: &[u8],
    ) -> anyhow::Result<MutationOutcome<ManagedUpload>> {
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
    ) -> anyhow::Result<MutationOutcome<ManagedUpload>> {
        self.ensure_uploads_enabled().await?;
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

    /// Deletes a media row and reclaims the on-disk entry when no remaining row
    /// or live Post names the same canonical media address.
    ///
    /// Ownership resolution uses one global reference snapshot and completes
    /// before the content lock serializes the guarded storage decision with upload
    /// placement. Confirmed reclamation removes the file only after the database
    /// scope completes.
    /// # Errors
    ///
    /// Returns identity/reference reads, write-scope acquisition, or operation
    /// errors. Reclaim failures are reported diagnostically without changing a
    /// confirmed database outcome.
    pub async fn delete_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
        force: bool,
    ) -> anyhow::Result<MediaDeletionResult> {
        let identity = self.site_config.get_identity().await?;
        let references = self.posts.list_media_references(media).await?;
        let evidence = resolve_media_reference_ownership(
            self.ownership_resolver.as_ref(),
            references.references(),
            &self.instance_id,
            identity.base_url.as_ref(),
        )
        .await;
        let mode = if force {
            MediaDeleteMode::FORCED
        } else {
            MediaDeleteMode::GUARDED
        };
        let _content_lock = self.content_locks.acquire_one(&media.sha256).await?;
        let storage = Arc::clone(&self.media);
        let media_for_write = media.clone();
        let instance_for_write = self.instance_id.clone();
        let evidence_for_write = evidence.clone();
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    storage
                        .try_delete_media(
                            transaction,
                            user_id,
                            &media_for_write,
                            &instance_for_write,
                            &evidence_for_write,
                            mode,
                        )
                        .await
                        .map_err(anyhow::Error::from)
                })
            })
            .await
            .map_err(Self::scope_error)?;
        let outcome = if matches!(
            outcome,
            MutationOutcome::Confirmed(TryDeleteOutcome::Deleted)
        ) {
            let storage = Arc::clone(&self.media);
            let media_for_reclaim = media.clone();
            let instance_for_reclaim = self.instance_id.clone();
            let evidence_for_reclaim = evidence.clone();
            let reclaimability = self
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        Self::deleted_media_file_is_reclaimable(
                            storage.as_ref(),
                            transaction,
                            &media_for_reclaim,
                            &instance_for_reclaim,
                            &evidence_for_reclaim,
                        )
                        .await
                    })
                })
                .await;
            let reclaim =
                Self::reclaim_file_after_scope(self.storage_path.as_ref(), media, reclaimability)
                    .await;
            Self::finish_reclaim(outcome, reclaim)
        } else {
            outcome
        };
        Ok(MediaDeletionResult::new(outcome, references, evidence))
    }

    fn reclaimable_from_scope(
        reclaimability: Result<MutationOutcome<bool>, WriteScopeError<anyhow::Error>>,
    ) -> anyhow::Result<bool> {
        match reclaimability {
            Ok(MutationOutcome::Confirmed(reclaimable)) => Ok(reclaimable),
            Ok(MutationOutcome::CommitIndeterminate(_)) => Ok(false),
            Err(error) => Err(Self::scope_error(error)),
        }
    }

    async fn reclaim_file_after_scope(
        storage_path: &Path,
        media: &MediaRef,
        reclaimability: Result<MutationOutcome<bool>, WriteScopeError<anyhow::Error>>,
    ) -> anyhow::Result<()> {
        if Self::reclaimable_from_scope(reclaimability)? {
            Self::remove_media_file(storage_path, media).await
        } else {
            Ok(())
        }
    }

    fn finish_reclaim<T>(primary: T, reclaim: anyhow::Result<()>) -> T {
        if let Err(error) = reclaim {
            host::error::report_swallowed(
                host::error::ErrorKind::Internal,
                host::error::ErrorClass::Transient,
                "storage.media.reclaim_failure",
                host::error::SwallowedSource::Error(error.as_ref()),
            );
        }
        primary
    }

    /// Checks whether an already-deleted media row's canonical entry is no
    /// longer named by any remaining row or live Post.
    async fn deleted_media_file_is_reclaimable(
        media_storage: &dyn MediaStorage,
        transaction: &mut crate::WriteTransaction,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> anyhow::Result<bool> {
        media_storage
            .lock_media_reference(transaction, media)
            .await?;
        media_storage
            .media_entry_is_reclaimable(transaction, media, current_instance_id, evidence)
            .await
            .map_err(anyhow::Error::from)
    }

    /// Removes a confirmed-reclaimable canonical media file after its database
    /// scope has completed.
    async fn remove_media_file(storage_path: &Path, media: &MediaRef) -> anyhow::Result<()> {
        let file_path = storage_path.join("media").join(media::path(
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

    async fn first_file_in_dir(dir: &Path) -> io::Result<Option<PathBuf>> {
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
    use std::fs as std_fs;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::task::Poll;

    use super::*;

    use crate::posts::media::{
        MediaReferenceSnapshot, PersistedMediaReference, ProvenForeignReference,
    };
    use crate::test_support::{
        Backend, SeedUser, backends, create_post_via_service, media_row_exists, media_url_for,
    };
    use crate::{ForeignEvidenceSink, MediaReferenceOwnershipResolver};
    use common::ids::PostId;
    use common::media::{MaxFileSize, MediaRef, UserQuota};
    use common::test_support::{
        parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_post_body,
    };
    use rstest::*;
    use rstest_reuse::*;
    use tempfile::TempDir;

    struct NoForeignResolver;

    #[async_trait::async_trait]
    impl MediaReferenceOwnershipResolver for NoForeignResolver {
        async fn resolve(
            &self,
            _references: &[PersistedMediaReference],
            _instance_id: &InstanceId,
            _base_url: Option<&common::tagged_url::BaseUrl>,
            foreign: ForeignEvidenceSink,
        ) -> MediaReferenceEvidence {
            foreign.finish()
        }
    }

    fn no_posts() -> Arc<dyn PostStorage> {
        Arc::new(crate::MockPostStorage::new())
    }

    fn test_instance_id() -> InstanceId {
        "123e4567-e89b-12d3-a456-426614174000"
            .parse()
            .expect("canonical instance ID")
    }

    fn no_foreign_resolver() -> Arc<dyn MediaReferenceOwnershipResolver> {
        Arc::new(NoForeignResolver)
    }

    struct FirstForeignResolver;

    #[async_trait::async_trait]
    impl MediaReferenceOwnershipResolver for FirstForeignResolver {
        async fn resolve(
            &self,
            references: &[PersistedMediaReference],
            _instance_id: &InstanceId,
            _base_url: Option<&common::tagged_url::BaseUrl>,
            mut foreign: ForeignEvidenceSink,
        ) -> MediaReferenceEvidence {
            foreign.prove_foreign(references[0].clone());
            foreign.finish()
        }
    }

    struct BlockingResolver {
        started: tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        completed: tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    #[async_trait::async_trait]
    impl MediaReferenceOwnershipResolver for BlockingResolver {
        async fn resolve(
            &self,
            _references: &[PersistedMediaReference],
            _instance_id: &InstanceId,
            _base_url: Option<&common::tagged_url::BaseUrl>,
            foreign: ForeignEvidenceSink,
        ) -> MediaReferenceEvidence {
            self.started
                .lock()
                .await
                .take()
                .expect("resolver starts once")
                .send(())
                .expect("test observes resolver start");
            self.release
                .lock()
                .await
                .take()
                .expect("resolver completes once")
                .await
                .expect("test releases resolver");
            let evidence = foreign.finish();
            self.completed
                .lock()
                .await
                .take()
                .expect("resolver completes once")
                .send(())
                .expect("test observes resolver completion");
            evidence
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_resolves_one_snapshot_before_acquiring_the_content_lock(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let posts = env.state.posts.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
        let resolver = Arc::new(BlockingResolver {
            started: tokio::sync::Mutex::new(Some(started_tx)),
            release: tokio::sync::Mutex::new(Some(release_rx)),
            completed: tokio::sync::Mutex::new(Some(completed_tx)),
        });
        let content_locks = Arc::new(MediaContentLocks::new(Arc::new(
            env.base.path().to_path_buf(),
        )));
        let manager = Arc::new(MediaManager::new(
            env.state.media.clone(),
            posts,
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::clone(&content_locks),
            env.base.instance_id().clone(),
            resolver,
        ));
        let uploaded = manager
            .upload_bytes(
                user_id,
                &parse_filename("ordered.jpg"),
                "image/jpeg".parse().unwrap(),
                b"ordered",
            )
            .await
            .unwrap();
        let media = upload_ref(&uploaded);
        let held_lock = content_locks.acquire_one(&media.sha256).await.unwrap();
        let delete_manager = Arc::clone(&manager);
        let delete_media = media.clone();
        let deletion = tokio::spawn(async move {
            delete_manager
                .delete_media(user_id, &delete_media, false)
                .await
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), started_rx)
            .await
            .expect("ownership resolution starts while the content lock is held")
            .expect("resolver reports start");
        release_tx.send(()).expect("release resolver");
        tokio::time::timeout(std::time::Duration::from_secs(1), completed_rx)
            .await
            .expect("ownership resolution completes while the content lock is held")
            .expect("resolver reports completion");
        drop(held_lock);
        let result = deletion.await.expect("deletion task joins").unwrap();

        assert!(matches!(
            result.outcome(),
            MutationOutcome::Confirmed(TryDeleteOutcome::Deleted)
        ));
    }

    // guard:no-backend — mock storages expose snapshot and evidence forwarding at the manager seam
    #[tokio::test]
    async fn delete_media_reuses_one_snapshot_and_evidence_for_guard_and_reclaim() {
        let parsed =
            common::media::parse_media_url(&media_url_for("evidence.jpg")).expect("media parses");
        let media_ref = parsed.media().clone();
        let user_id = UserId::from(7);
        let reference = PersistedMediaReference::new(
            PostId::from(11),
            media_ref.clone(),
            parsed.kind(),
            parsed.reference_form().clone(),
        )
        .with_owner(user_id);
        let snapshot = MediaReferenceSnapshot::new(vec![reference.clone()], false);

        let expected_media_for_snapshot = media_ref.clone();
        let mut posts = crate::MockPostStorage::new();
        posts
            .expect_list_media_references()
            .times(1)
            .return_once(move |actual_media| {
                assert_eq!(actual_media, &expected_media_for_snapshot);
                Ok(snapshot)
            });

        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config.expect_get_identity().times(1).return_once(|| {
            Ok(common::site::SiteIdentity {
                title: common::site::SiteTitle::default(),
                base_url: None,
            })
        });

        let instance_id = test_instance_id();
        let expected_guard_media = media_ref.clone();
        let expected_guard_instance = instance_id.clone();
        let expected_guard_reference = reference.clone();
        let expected_reclaim_media = media_ref.clone();
        let expected_reclaim_instance = instance_id.clone();
        let expected_reclaim_reference = reference;
        let mut media = crate::MockMediaStorage::new();
        media.expect_try_delete_media().times(1).returning(
            move |_, actual_user, actual_media, actual_instance, evidence, mode| {
                assert_eq!(actual_user, user_id);
                assert_eq!(actual_media, &expected_guard_media);
                assert_eq!(actual_instance, &expected_guard_instance);
                assert!(evidence.proves_foreign(&expected_guard_reference));
                assert_eq!(mode, MediaDeleteMode::GUARDED);
                Ok(TryDeleteOutcome::Deleted)
            },
        );
        media
            .expect_lock_media_reference()
            .times(1)
            .returning(|_, _| Ok(()));
        media
            .expect_media_entry_is_reclaimable()
            .times(1)
            .returning(move |_, actual_media, actual_instance, evidence| {
                assert_eq!(actual_media, &expected_reclaim_media);
                assert_eq!(actual_instance, &expected_reclaim_instance);
                assert!(evidence.proves_foreign(&expected_reclaim_reference));
                Ok(false)
            });

        let temp = TempDir::new().unwrap();
        let manager = MediaManager::new(
            Arc::new(media),
            Arc::new(posts),
            Arc::new(site_config),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            instance_id,
            Arc::new(FirstForeignResolver),
        );

        let result = manager
            .delete_media(user_id, &media_ref, false)
            .await
            .unwrap();

        assert!(matches!(
            result.outcome(),
            MutationOutcome::Confirmed(TryDeleteOutcome::Deleted)
        ));
    }

    fn upload_ref(response: &MutationOutcome<UploadedMedia>) -> MediaRef {
        let response = response.value();
        MediaRef {
            source: MediaSource::Upload,
            sha256: response.sha256.clone(),
            filename: response.filename.clone(),
        }
    }

    fn stored_path(root: &Path, media: &MediaRef) -> PathBuf {
        root.join("media")
            .join(media::path(&media.source, &media.sha256, &media.filename))
    }

    // guard:no-backend — filesystem-only temporary upload cleanup.
    #[tokio::test]
    async fn prepare_temporary_upload_directory_creates_an_absent_directory() {
        let temp = TempDir::new().expect("temp dir");
        let tmp_dir = temp.path().join("media").join("tmp");

        MediaManager::prepare_temporary_upload_directory(temp.path())
            .await
            .expect("create absent temporary directory");

        assert!(
            tmp_dir.is_dir(),
            "temporary upload directory must be created"
        );
        assert!(
            std_fs::read_dir(tmp_dir)
                .expect("read temporary directory")
                .next()
                .is_none(),
            "new temporary upload directory must be empty"
        );
    }

    // guard:no-backend — filesystem-only temporary upload cleanup.
    #[tokio::test]
    async fn prepare_temporary_upload_directory_retains_an_empty_directory() {
        let temp = TempDir::new().expect("temp dir");
        let tmp_dir = temp.path().join("media").join("tmp");
        fs::create_dir_all(&tmp_dir)
            .await
            .expect("create temporary directory");

        MediaManager::prepare_temporary_upload_directory(temp.path())
            .await
            .expect("refresh empty temporary directory");

        assert!(
            tmp_dir.is_dir(),
            "temporary upload directory must remain usable"
        );
        assert!(
            std_fs::read_dir(tmp_dir)
                .expect("read temporary directory")
                .next()
                .is_none(),
            "temporary upload directory must remain empty"
        );
    }

    // guard:no-backend — filesystem-only temporary upload cleanup.
    #[tokio::test]
    async fn prepare_temporary_upload_directory_removes_populated_artifacts_only() {
        let temp = TempDir::new().expect("temp dir");
        let tmp_dir = temp.path().join("media").join("tmp");
        let finalized = temp.path().join("media").join("upload").join("finalized");
        fs::create_dir_all(&tmp_dir)
            .await
            .expect("create temporary directory");
        fs::write(tmp_dir.join("stale-upload"), b"stale")
            .await
            .expect("write stale artifact");
        fs::create_dir_all(finalized.parent().expect("finalized parent"))
            .await
            .expect("create finalized directory");
        fs::write(&finalized, b"durable")
            .await
            .expect("write finalized media");

        MediaManager::prepare_temporary_upload_directory(temp.path())
            .await
            .expect("clear temporary artifacts");

        assert!(
            std_fs::read_dir(&tmp_dir)
                .expect("read temporary directory")
                .next()
                .is_none(),
            "temporary artifacts must be removed"
        );
        assert_eq!(
            fs::read(finalized).await.expect("read finalized media"),
            b"durable",
            "cleanup must not reach finalized media"
        );
    }

    // guard:no-backend — filesystem-only temporary upload cleanup.
    #[tokio::test]
    async fn prepare_temporary_upload_directory_removes_nested_artifacts() {
        let temp = TempDir::new().expect("temp dir");
        let tmp_dir = temp.path().join("media").join("tmp");
        let nested = tmp_dir.join("interrupted").join("stream").join("upload");
        fs::create_dir_all(nested.parent().expect("nested parent"))
            .await
            .expect("create nested temporary directory");
        fs::write(&nested, b"stale")
            .await
            .expect("write nested temporary artifact");

        MediaManager::prepare_temporary_upload_directory(temp.path())
            .await
            .expect("clear nested temporary artifacts");

        assert!(
            std_fs::read_dir(tmp_dir)
                .expect("read temporary directory")
                .next()
                .is_none(),
            "nested temporary artifacts must be removed"
        );
    }

    // guard:no-backend — filesystem-only temporary upload cleanup.
    #[tokio::test]
    async fn prepare_temporary_upload_directory_returns_typed_cleanup_failure() {
        let temp = TempDir::new().expect("temp dir");
        let tmp_dir = temp.path().join("media").join("tmp");
        fs::create_dir_all(tmp_dir.parent().expect("temporary parent"))
            .await
            .expect("create media directory");
        fs::write(&tmp_dir, b"not a directory")
            .await
            .expect("block temporary directory removal");

        let error = MediaManager::prepare_temporary_upload_directory(temp.path())
            .await
            .expect_err("non-directory temporary path must fail preparation");

        assert_eq!(error.operation, "removing");
        assert_eq!(error.path, tmp_dir);
        assert_eq!(error.source.kind(), io::ErrorKind::NotADirectory);
    }

    #[test]
    fn deletion_result_reports_only_owned_nonforeign_posts_on_refusal() {
        let instance_id: InstanceId = "123e4567-e89b-12d3-a456-426614174000"
            .parse()
            .expect("canonical instance ID");
        let parsed =
            common::media::parse_media_url(&media_url_for("photo.jpg")).expect("media form parses");
        let owner_id = UserId::from(7);
        let foreign = PersistedMediaReference::new(
            PostId::from(3),
            parsed.media().clone(),
            parsed.kind(),
            parsed.reference_form().clone(),
        )
        .with_owner(owner_id);
        let references = MediaReferenceSnapshot::new(
            vec![
                PersistedMediaReference::new(
                    PostId::from(2),
                    parsed.media().clone(),
                    parsed.kind(),
                    parsed.reference_form().clone(),
                )
                .with_owner(owner_id),
                PersistedMediaReference::new(
                    PostId::from(1),
                    parsed.media().clone(),
                    parsed.kind(),
                    parsed.reference_form().clone(),
                )
                .with_owner(owner_id),
                PersistedMediaReference::new(
                    PostId::from(1),
                    parsed.media().clone(),
                    parsed.kind(),
                    parsed.reference_form().clone(),
                )
                .with_owner(owner_id),
                PersistedMediaReference::new(
                    PostId::from(4),
                    parsed.media().clone(),
                    parsed.kind(),
                    parsed.reference_form().clone(),
                )
                .with_owner(UserId::from(8)),
                foreign.clone(),
            ],
            false,
        );
        let mut evidence = MediaReferenceEvidence::new(instance_id.clone());
        assert!(evidence.insert(ProvenForeignReference::new(foreign, instance_id)));
        let result = MediaDeletionResult::new(
            MutationOutcome::Confirmed(TryDeleteOutcome::RefusedReferenced),
            references,
            evidence,
        );

        assert_eq!(
            result.referenced_post_ids(owner_id),
            vec![PostId::from(1), PostId::from(2)]
        );
        assert!(matches!(
            result.outcome(),
            MutationOutcome::Confirmed(TryDeleteOutcome::RefusedReferenced)
        ));
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
            MediaManager::upload_outcome(Some(&MediaError::UploadsDisabled)),
            UploadOutcome::Disabled
        ));
        assert!(matches!(
            MediaManager::upload_outcome(Some(&MediaError::Internal(Box::new(io::Error::other(
                "x",
            ))))),
            UploadOutcome::Error
        ));
        assert!(matches!(
            MediaManager::upload_outcome(None),
            UploadOutcome::Error
        ));
    }

    // guard:no-backend — a disabled capability must reject before polling the supplied stream
    // or calling any media storage method.
    #[tokio::test]
    async fn disabled_stream_upload_does_not_poll_or_start_downstream_work() {
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config
            .expect_get_media_uploads_enabled()
            .times(1)
            .return_once(|| Ok(false));
        let temp = TempDir::new().unwrap();
        let manager = MediaManager::new(
            Arc::new(crate::MockMediaStorage::new()),
            no_posts(),
            Arc::new(site_config),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );
        let polls = Arc::new(AtomicUsize::new(0));
        let stream_polls = Arc::clone(&polls);
        let stream = stream::poll_fn(move |_| {
            stream_polls.fetch_add(1, Ordering::Relaxed);
            Poll::Ready(None::<Result<Bytes, io::Error>>)
        });

        let err = manager
            .upload(
                UserId::from(1),
                &parse_filename("blocked.png"),
                Some(parse_content_type("image/png")),
                stream,
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err.downcast_ref::<MediaError>(),
            Some(MediaError::UploadsDisabled)
        ));
        assert_eq!(err.to_string(), "media uploads are disabled");
        assert_eq!(polls.load(Ordering::Relaxed), 0);
    }

    // guard:no-backend — byte uploads share the same entry policy and do no downstream work.
    #[tokio::test]
    async fn disabled_byte_upload_does_not_start_downstream_work() {
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config
            .expect_get_media_uploads_enabled()
            .times(1)
            .return_once(|| Ok(false));
        let temp = TempDir::new().unwrap();
        let manager = MediaManager::new(
            Arc::new(crate::MockMediaStorage::new()),
            no_posts(),
            Arc::new(site_config),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );

        let err = manager
            .upload_bytes(
                UserId::from(1),
                &parse_filename("blocked.png"),
                parse_content_type("image/png"),
                b"must not be hashed or written",
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err.downcast_ref::<MediaError>(),
            Some(MediaError::UploadsDisabled)
        ));
        assert!(
            !temp.path().join("media").join("tmp").exists(),
            "disabled upload must not create a temporary directory"
        );
    }

    // guard:no-backend — a capability read failure must not be converted into a policy denial.
    #[tokio::test]
    async fn upload_propagates_capability_storage_failure_without_downstream_work() {
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config
            .expect_get_media_uploads_enabled()
            .times(1)
            .return_once(|| Err(sqlx::Error::PoolTimedOut));
        let temp = TempDir::new().unwrap();
        let manager = MediaManager::new(
            Arc::new(crate::MockMediaStorage::new()),
            no_posts(),
            Arc::new(site_config),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );

        let err = manager
            .upload_bytes(
                UserId::from(1),
                &parse_filename("unreadable-config.png"),
                parse_content_type("image/png"),
                b"must not be written",
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        ));
        assert!(
            !temp.path().join("media").join("tmp").exists(),
            "storage failure must stop before temporary-file work"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn admitted_upload_reads_capability_once(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let enabled = Arc::new(AtomicBool::new(true));
        let policy_at_entry = Arc::clone(&enabled);
        let mut site_config = crate::MockSiteConfigStorage::new();
        site_config
            .expect_get_media_uploads_enabled()
            .times(1)
            .return_once(move || Ok(policy_at_entry.swap(false, Ordering::SeqCst)));
        site_config
            .expect_get_media_max_file_size()
            .times(1)
            .return_once(|| Ok(MaxFileSize::default()));
        site_config
            .expect_get_media_user_quota()
            .times(1)
            .return_once(|| Ok(UserQuota::default()));
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            Arc::new(site_config),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
        );

        manager
            .upload_bytes(
                user_id,
                &parse_filename("admitted.png"),
                parse_content_type("image/png"),
                b"admitted",
            )
            .await
            .unwrap();
        assert!(
            !enabled.load(Ordering::SeqCst),
            "the setting changes after admission without revoking this upload"
        );
    }
    #[test]
    fn typed_content_type_is_preserved_and_absent_is_detected_from_filename() {
        assert_eq!("image/png".parse::<ContentType>().unwrap(), "image/png");
        assert_eq!(
            media::detect_content_type(&parse_filename("photo.jpg")),
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

    // guard:no-backend — exercises the pure reclaim mapper before filesystem work
    #[tokio::test]
    async fn reclaim_file_after_scope_returns_scope_begin_failure() {
        let temp = TempDir::new().unwrap();
        let media = MediaRef {
            source: MediaSource::Upload,
            sha256: parse_content_hash(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
            ),
            filename: parse_filename("unreclaimed.png"),
        };

        let error = MediaManager::reclaim_file_after_scope(
            temp.path(),
            &media,
            Err(WriteScopeError::Begin(sqlx::Error::PoolClosed)),
        )
        .await
        .expect_err("scope begin failure must be returned");

        assert!(matches!(
            error
                .downcast_ref::<MediaError>()
                .and_then(|error| match error {
                    MediaError::Internal(source) => source.downcast_ref::<sqlx::Error>(),
                    _ => None,
                }),
            Some(sqlx::Error::PoolClosed)
        ));
    }

    #[test]
    fn indeterminate_reclaimability_does_not_reclaim() {
        assert!(
            !MediaManager::reclaimable_from_scope(Ok(MutationOutcome::CommitIndeterminate(true)))
                .expect("indeterminate scope is conservatively non-reclaimable")
        );
    }

    // guard:no-backend — placement fails before the database scope starts.
    #[tokio::test]
    async fn placement_failure_removes_temp_file_and_retains_hash_collision() {
        let temp = TempDir::new().unwrap();
        let collision = temp.path().join("hash-dir");
        fs::write(&collision, b"not-a-directory").await.unwrap();
        let tmp_path = temp.path().join("upload.tmp");
        fs::write(&tmp_path, b"png-ish").await.unwrap();
        let filename = parse_filename("placement-failed.png");
        let sha256 =
            parse_content_hash("deadbeef00000000000000000000000000000000000000000000000000000000");
        let media_ref = MediaRef {
            source: MediaSource::Upload,
            sha256: sha256.clone(),
            filename: filename.clone(),
        };
        let record = MediaRecord {
            user_id: UserId::from(1),
            sha256,
            filename,
            source: MediaSource::Upload,
            content_type: parse_content_type("image/png"),
            size_bytes: parse_byte_size("7"),
            source_url: None,
            created_at: UtcInstant::now(),
        };
        let manager = MediaManager::new(
            Arc::new(crate::MockMediaStorage::new()),
            no_posts(),
            Arc::new(crate::MockSiteConfigStorage::new()),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );

        assert!(
            manager
                .place_and_register(
                    record,
                    media_ref,
                    tmp_path.clone(),
                    collision.join("target"),
                    collision.clone(),
                )
                .await
                .is_err()
        );
        assert!(
            !tmp_path.exists(),
            "a placement failure must remove the prepared temp file"
        );
        assert_eq!(
            fs::read(&collision).await.unwrap(),
            b"not-a-directory",
            "cleanup must retain the pre-existing hash-path collision"
        );
    }

    // guard:low-level-db — a closed pool makes the scope fail after deduplication.
    #[tokio::test]
    async fn existing_target_scope_failure_retains_target_and_removes_temp_file() {
        let temp = TempDir::new().unwrap();
        let target_path = temp.path().join("existing-target");
        fs::write(&target_path, b"already-stored").await.unwrap();
        let tmp_path = temp.path().join("upload.tmp");
        fs::write(&tmp_path, b"png-ish").await.unwrap();
        let filename = parse_filename("existing-target.png");
        let sha256 =
            parse_content_hash("deadbeef00000000000000000000000000000000000000000000000000000000");
        let media_ref = MediaRef {
            source: MediaSource::Upload,
            sha256: sha256.clone(),
            filename: filename.clone(),
        };
        let record = MediaRecord {
            user_id: UserId::from(1),
            sha256,
            filename,
            source: MediaSource::Upload,
            content_type: parse_content_type("image/png"),
            size_bytes: parse_byte_size("7"),
            source_url: None,
            created_at: UtcInstant::now(),
        };
        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let write_scope = WriteScope::sqlite(pool.clone());
        pool.close().await;
        let manager = MediaManager::new(
            Arc::new(crate::MockMediaStorage::new()),
            no_posts(),
            Arc::new(crate::MockSiteConfigStorage::new()),
            write_scope,
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );

        let error = manager
            .place_and_register(
                record,
                media_ref,
                tmp_path.clone(),
                target_path.clone(),
                temp.path().join("unused-hash-dir"),
            )
            .await
            .expect_err("scope begin failure must be returned");
        assert!(matches!(
            error.downcast_ref::<MediaError>(),
            Some(MediaError::Internal(_))
        ));
        assert!(
            !tmp_path.exists(),
            "deduplicating to a pre-existing target must remove the temp file"
        );
        assert_eq!(
            fs::read(&target_path).await.unwrap(),
            b"already-stored",
            "scope failure must retain the pre-existing target"
        );
    }

    // guard:no-backend — mock scope rejects the DB operation after target preparation
    #[tokio::test]
    async fn upload_operation_failure_removes_only_newly_created_target() {
        let temp = TempDir::new().unwrap();
        let mut media = crate::MockMediaStorage::new();
        media
            .expect_get_user_upload_usage()
            .times(1)
            .returning(|_| Ok(parse_byte_size("0")));
        media
            .expect_lock_media_reference()
            .times(1)
            .returning(|_, _| Ok(()));
        media
            .expect_create_media()
            .times(1)
            .returning(|_, _| Err(CreateMediaError::Internal(sqlx::Error::PoolClosed)));
        let manager = MediaManager::new(
            Arc::new(media),
            no_posts(),
            Arc::new(crate::MockSiteConfigStorage::new()),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );
        let metadata = UploadMetadata {
            filename: parse_filename("failed.png"),
            content_type: parse_content_type("image/png"),
            sha256_hex: parse_content_hash(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
            ),
            size_bytes: parse_byte_size("7"),
        };
        let tmp_path = temp.path().join("upload.tmp");
        fs::write(&tmp_path, b"png-ish").await.unwrap();
        let target_path = temp.path().join("media").join(media::path(
            &MediaSource::Upload,
            &metadata.sha256_hex,
            &metadata.filename,
        ));
        assert!(
            manager
                .finalize_upload(
                    UserId::from(1),
                    metadata,
                    &tmp_path,
                    UserQuota::try_from(100_i64).unwrap()
                )
                .await
                .is_err()
        );
        assert!(
            !target_path.exists(),
            "a rolled-back create must remove only its newly created target"
        );
    }

    // guard:low-level-db — a deliberately closed SQLite pool exercises scope acquisition failure.
    #[tokio::test]
    async fn upload_begin_failure_removes_temp_file() {
        let temp = TempDir::new().unwrap();
        let mut media = crate::MockMediaStorage::new();
        media
            .expect_get_user_upload_usage()
            .times(1)
            .returning(|_| Ok(parse_byte_size("0")));
        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let write_scope = WriteScope::sqlite(pool.clone());
        pool.close().await;
        let manager = MediaManager::new(
            Arc::new(media),
            no_posts(),
            Arc::new(crate::MockSiteConfigStorage::new()),
            write_scope,
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );
        let metadata = UploadMetadata {
            filename: parse_filename("begin-failed.png"),
            content_type: parse_content_type("image/png"),
            sha256_hex: parse_content_hash(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
            ),
            size_bytes: parse_byte_size("7"),
        };
        let tmp_path = temp.path().join("upload.tmp");
        fs::write(&tmp_path, b"png-ish").await.unwrap();

        assert!(
            manager
                .finalize_upload(
                    UserId::from(1),
                    metadata,
                    &tmp_path,
                    UserQuota::try_from(100_i64).unwrap(),
                )
                .await
                .is_err()
        );
        assert!(
            !tmp_path.exists(),
            "a scope begin failure must remove the prepared temp file"
        );
    }

    // guard:no-backend — filesystem lock acquisition fails before the database scope.
    #[tokio::test]
    async fn upload_content_lock_failure_removes_temp_file() {
        let temp = TempDir::new().unwrap();
        let media_path = temp.path().join("media");
        fs::write(&media_path, b"not-a-directory").await.unwrap();
        let mut media = crate::MockMediaStorage::new();
        media
            .expect_get_user_upload_usage()
            .times(1)
            .returning(|_| Ok(parse_byte_size("0")));
        let manager = MediaManager::new(
            Arc::new(media),
            no_posts(),
            Arc::new(crate::MockSiteConfigStorage::new()),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );
        let metadata = UploadMetadata {
            filename: parse_filename("content-lock-failed.png"),
            content_type: parse_content_type("image/png"),
            sha256_hex: parse_content_hash(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
            ),
            size_bytes: parse_byte_size("7"),
        };
        let tmp_path = temp.path().join("upload.tmp");
        fs::write(&tmp_path, b"png-ish").await.unwrap();

        assert!(
            manager
                .finalize_upload(
                    UserId::from(1),
                    metadata,
                    &tmp_path,
                    UserQuota::try_from(100_i64).unwrap(),
                )
                .await
                .is_err()
        );
        assert!(
            !tmp_path.exists(),
            "an OS content-lock failure must remove the prepared temp file"
        );
        assert_eq!(
            fs::read(media_path).await.unwrap(),
            b"not-a-directory",
            "cleanup must retain the path that caused lock acquisition to fail"
        );
    }

    // guard:no-backend — a database identity-lock failure removes the target placed before the scope.
    #[tokio::test]
    async fn upload_lock_failure_removes_temp_file() {
        let temp = TempDir::new().unwrap();
        let mut media = crate::MockMediaStorage::new();
        media
            .expect_get_user_upload_usage()
            .times(1)
            .returning(|_| Ok(parse_byte_size("0")));
        media
            .expect_lock_media_reference()
            .times(1)
            .returning(|_, _| Err(sqlx::Error::PoolClosed));
        let manager = MediaManager::new(
            Arc::new(media),
            no_posts(),
            Arc::new(crate::MockSiteConfigStorage::new()),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );
        let metadata = UploadMetadata {
            filename: parse_filename("lock-failed.png"),
            content_type: parse_content_type("image/png"),
            sha256_hex: parse_content_hash(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
            ),
            size_bytes: parse_byte_size("7"),
        };
        let tmp_path = temp.path().join("upload.tmp");
        fs::write(&tmp_path, b"png-ish").await.unwrap();

        assert!(
            manager
                .finalize_upload(
                    UserId::from(1),
                    metadata,
                    &tmp_path,
                    UserQuota::try_from(100_i64).unwrap(),
                )
                .await
                .is_err()
        );
        assert!(
            !tmp_path.exists(),
            "an identity-lock failure must remove the prepared temp file"
        );
    }

    // guard:no-backend — mock scope rejects the DB operation after hard-link deduplication
    #[tokio::test]
    async fn upload_operation_failure_removes_hard_link_created_by_this_upload() {
        let temp = TempDir::new().unwrap();
        let mut media = crate::MockMediaStorage::new();
        media
            .expect_get_user_upload_usage()
            .times(1)
            .returning(|_| Ok(parse_byte_size("0")));
        media
            .expect_lock_media_reference()
            .times(1)
            .returning(|_, _| Ok(()));
        media
            .expect_create_media()
            .times(1)
            .returning(|_, _| Err(CreateMediaError::Internal(sqlx::Error::PoolClosed)));
        let manager = MediaManager::new(
            Arc::new(media),
            no_posts(),
            Arc::new(crate::MockSiteConfigStorage::new()),
            WriteScope::mock(),
            Arc::new(MediaContentLocks::new(Arc::new(temp.path().to_path_buf()))),
            test_instance_id(),
            no_foreign_resolver(),
        );
        let metadata = UploadMetadata {
            filename: parse_filename("failed-link.png"),
            content_type: parse_content_type("image/png"),
            sha256_hex: parse_content_hash(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
            ),
            size_bytes: parse_byte_size("7"),
        };
        let tmp_path = temp.path().join("upload.tmp");
        fs::write(&tmp_path, b"png-ish").await.unwrap();
        let target_path = temp.path().join("media").join(media::path(
            &MediaSource::Upload,
            &metadata.sha256_hex,
            &metadata.filename,
        ));
        let source_path = target_path.with_file_name("already-stored.png");
        fs::create_dir_all(source_path.parent().unwrap())
            .await
            .unwrap();
        fs::write(&source_path, b"png-ish").await.unwrap();
        assert!(
            manager
                .finalize_upload(
                    UserId::from(1),
                    metadata,
                    &tmp_path,
                    UserQuota::try_from(100_i64).unwrap()
                )
                .await
                .is_err()
        );
        assert!(
            !target_path.exists(),
            "a rolled-back create must remove its new hard link"
        );
        assert!(
            source_path.exists(),
            "cleanup must retain the pre-existing deduplication source"
        );
    }

    #[test]
    fn continuation_reporting_cleanup_failures_preserve_quota_and_dedup_results_and_report_once() {
        let cleanup_error = || {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
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

        let (outcome, trace) = crate::helpers::swallowed_test::capture(|| {
            MediaManager::finish_reclaim(
                MutationOutcome::Confirmed(TryDeleteOutcome::Deleted),
                cleanup_error().map_err(anyhow::Error::from),
            )
        });
        assert_eq!(
            outcome,
            MutationOutcome::Confirmed(TryDeleteOutcome::Deleted)
        );
        crate::helpers::swallowed_test::assert_one_report(&trace, "storage.media.reclaim_failure");
    }

    // guard:no-backend — mock store; the DB is unused by the dir scan
    #[tokio::test]
    async fn first_file_in_dir_skips_subdirs_and_finds_a_file() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        assert_eq!(MediaManager::first_file_in_dir(dir).await.unwrap(), None);

        // Dir with a subdir (should be ignored by is_file())
        let subdir = dir.join("subdir");
        fs::create_dir(&subdir).await.unwrap();
        assert_eq!(MediaManager::first_file_in_dir(dir).await.unwrap(), None);

        let file = dir.join("test.txt");
        fs::write(&file, "hello").await.unwrap();
        assert_eq!(
            MediaManager::first_file_in_dir(dir).await.unwrap(),
            Some(file)
        );
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

        let tmp_path = tmp_dir.join("temp_file");
        fs::write(&tmp_path, "content").await.unwrap();

        let target_path = media_dir.join("target_file");
        let hash_dir = media_dir.join("hash_dir");

        // Scenario 1: Target exists (should remove tmp)
        fs::write(&target_path, "existing").await.unwrap();
        MediaManager::handle_deduplication(&tmp_path, &target_path, &hash_dir)
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

        MediaManager::handle_deduplication(&tmp_path2, &target_path2, &hash_dir)
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

        MediaManager::handle_deduplication(&tmp_path3, &target_path3, &hash_dir3)
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
        let entries: io::Result<futures_util::stream::Empty<io::Result<PathBuf>>> =
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "initial directory read sentinel",
            ));

        let existing_file = MediaManager::first_file_in_entries(entries).await;
        let error =
            MediaManager::finish_deduplication_from_result(&tmp_path, &target_path, existing_file)
                .await
                .expect_err("dedup must not report success");

        let source = error
            .downcast_ref::<io::Error>()
            .expect("typed initial read error");
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
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
            Err(io::Error::other("later next-entry sentinel")),
        ]));

        let existing_file = MediaManager::first_file_in_entries(entries).await;
        let error =
            MediaManager::finish_deduplication_from_result(&tmp_path, &target_path, existing_file)
                .await
                .expect_err("dedup must not report success after partial enumeration");

        let source = error
            .downcast_ref::<io::Error>()
            .expect("typed next-entry error");
        assert_eq!(source.kind(), io::ErrorKind::Other);
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
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
        );

        // A tiny PNG signature + IHDR-ish bytes (content need not be a valid image).
        let bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02, 0x03,
        ];
        let expected_sha = format!("{:x}", Sha256::digest(bytes));

        let first = manager
            .upload_bytes_with_disposition(
                user_id,
                &parse_filename("pic.png"),
                "image/png".parse().unwrap(),
                bytes,
            )
            .await
            .unwrap();
        assert!(!first.value().already_existed);
        assert_eq!(first.value().media.sha256.as_ref(), expected_sha.as_str());
        assert_eq!(first.value().media.filename, "pic.png");
        assert_eq!(first.value().media.content_type, "image/png");
        assert_eq!(
            first.value().media.size_bytes,
            ByteSize::try_from(i64::try_from(bytes.len()).unwrap()).unwrap()
        );

        // Identical re-upload must succeed and dedup to the same record.
        let second = manager
            .upload_bytes_with_disposition(
                user_id,
                &parse_filename("pic.png"),
                "image/png".parse().unwrap(),
                bytes,
            )
            .await
            .unwrap();
        assert!(second.value().already_existed);
        assert_eq!(second.value().media.sha256, first.value().media.sha256);
        assert_eq!(second.value().media.url, first.value().media.url);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upload_bytes_retains_new_file_when_commit_is_indeterminate(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state
                .write_scope
                .with_commit_acknowledgement_loss_after_commit_for_test(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
        );

        let outcome = manager
            .upload_bytes(
                user_id,
                &parse_filename("indeterminate.png"),
                "image/png".parse().unwrap(),
                b"png-ish",
            )
            .await
            .unwrap();
        let media = upload_ref(&outcome);
        assert!(matches!(outcome, MutationOutcome::CommitIndeterminate(_)));
        assert!(
            stored_path(env.base.path(), &media).exists(),
            "an indeterminate commit must retain the newly prepared target"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upload_bytes_rejects_oversized_payload(#[case] backend: Backend) {
        // Cap the per-file limit well below the payload size.
        let env = backend
            .setup()
            .media_limits("5".parse().unwrap(), UserQuota::default())
            .await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
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
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
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

        let stream = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, io::Error>));
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

        assert_eq!(resp.value().sha256, expected);
        assert_eq!(resp.value().content_type, "image/png");
        assert_eq!(
            resp.value().url,
            media::url(&MediaSource::Upload, &expected, &filename)
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_reclaims_unreferenced_file_and_quota(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
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
            manager
                .delete_media(user_id, &media, false)
                .await
                .unwrap()
                .into_outcome(),
            MutationOutcome::Confirmed(TryDeleteOutcome::Deleted)
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
    async fn delete_media_retains_file_when_commit_is_indeterminate(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let confirmed_manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
        );
        let uploaded = confirmed_manager
            .upload_bytes(
                user_id,
                &parse_filename("indeterminate.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let media = upload_ref(&uploaded);
        let file_path = stored_path(env.base.path(), &media);
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state
                .write_scope
                .with_commit_acknowledgement_loss_after_commit_for_test(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
        );

        let outcome = manager
            .delete_media(user_id, &media, false)
            .await
            .unwrap()
            .into_outcome();
        assert_eq!(
            outcome,
            MutationOutcome::CommitIndeterminate(TryDeleteOutcome::Deleted)
        );
        assert!(
            file_path.exists(),
            "an indeterminate delete must not reclaim bytes"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_operation_failure_retains_file(#[case] backend: Backend) {
        let env = backend.setup().await;
        let owner = SeedUser::new().seed(&env.state).await.user_id;
        let other_user = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
        );
        let uploaded = manager
            .upload_bytes(
                owner,
                &parse_filename("retain-on-error.jpg"),
                "image/jpeg".parse().unwrap(),
                b"jpeg-ish",
            )
            .await
            .unwrap();
        let media = upload_ref(&uploaded);
        let file_path = stored_path(env.base.path(), &media);

        assert!(
            manager
                .delete_media(other_user, &media, false)
                .await
                .is_err()
        );
        assert!(
            file_path.exists(),
            "a failed delete must retain the media bytes"
        );
        assert!(media_row_exists(&env.state, owner, &media).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_reclaim_failure_preserves_confirmed_outcome(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
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
        std_fs::remove_file(&file_path).unwrap();
        std_fs::create_dir(&file_path).unwrap();

        let outcome = manager
            .delete_media(user_id, &media, false)
            .await
            .unwrap()
            .into_outcome();

        assert_eq!(
            outcome,
            MutationOutcome::Confirmed(TryDeleteOutcome::Deleted)
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_force_can_break_owner_retained_history(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
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
            parse_post_body(&format!("<img src=\"{}\">", uploaded.value().url)),
        )
        .await;

        assert_eq!(
            manager
                .delete_media(user_id, &media, true)
                .await
                .unwrap()
                .into_outcome(),
            MutationOutcome::Confirmed(TryDeleteOutcome::Deleted)
        );
        assert!(!media_row_exists(&env.state, user_id, &media).await);
        assert!(
            file_path.exists(),
            "reclamation remains conservative while retained history names the bytes"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_keeps_shared_path_until_last_row_is_deleted(#[case] backend: Backend) {
        let env = backend.setup().await;
        let first_user = SeedUser::new().seed(&env.state).await.user_id;
        let second_user = SeedUser::new().seed(&env.state).await.user_id;
        let manager = MediaManager::new(
            env.state.media.clone(),
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
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
            env.state.posts.clone(),
            env.state.site_config.clone(),
            env.state.write_scope.clone(),
            Arc::new(MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            no_foreign_resolver(),
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
