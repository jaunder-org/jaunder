//! Private, conventional metadata editing in a killable same-executable child.

pub mod native;
mod native_kernel;
mod owner_registry;
pub use owner_registry::{OwnerFailure, PanicFailure, PanicKind};
mod supervision;
pub mod worker;
pub mod worker_protocol;
pub mod worker_request;

use common::media::{ContentType, Filename, MaxFileSize};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::sync::{Semaphore, oneshot};

const PROCESSING_TIMEOUT: Duration = Duration::from_secs(30);
const ACTIVE_JOBS: usize = 2;

// Cancellation during queue wait has no returning error boundary. Explicitly
// close the spool and report cleanup failures rather than relying on tempfile's
// best-effort Drop implementation.
struct QueuedSpool(Option<NamedTempFile>);
impl QueuedSpool {
    fn file(&self) -> &NamedTempFile {
        match &self.0 {
            Some(file) => file,
            None => unreachable!("queued spool ownership already transferred"),
        }
    }
    fn take(&mut self) -> NamedTempFile {
        match self.0.take() {
            Some(file) => file,
            None => unreachable!("queued spool transfers once"),
        }
    }
    fn fail(&mut self, primary: ImageSanitizerError) -> ImageSanitizerError {
        match self.take().close() {
            Ok(()) => primary,
            Err(source) => ImageSanitizerError::Cleanup {
                primary: Box::new(primary),
                source,
            },
        }
    }
}
impl Drop for QueuedSpool {
    fn drop(&mut self) {
        if let Some(file) = self.0.take()
            && let Err(error) = file.close()
        {
            report_private_failure(&error, "private image spool cleanup");
        }
    }
}
fn report_private_failure(error: &(dyn std::error::Error + 'static), context: &'static str) {
    crate::error::report_swallowed(
        crate::error::ErrorKind::Internal,
        crate::error::ErrorClass::Bug,
        context,
        crate::error::SwallowedSource::Error(error),
    );
}

struct CancelJob(Option<oneshot::Sender<()>>);
impl Drop for CancelJob {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

/// A private, sanitized image ready for `MediaManager` to hash and place.
#[derive(Debug)]
pub struct SanitizedImage {
    file: NamedTempFile,
    content_type: ContentType,
}

impl SanitizedImage {
    /// The owned private file containing the edited bytes.
    #[must_use]
    pub fn file(&self) -> &NamedTempFile {
        &self.file
    }

    /// The byte-detected type for the edited covered image.
    #[must_use]
    pub fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    /// Transfers private-file ownership to the `MediaManager` finalization seam.
    #[must_use]
    pub fn into_file(self) -> NamedTempFile {
        self.file
    }
}

/// Result of accepting an upload into the sanitizer boundary.
#[derive(Debug)]
pub enum Sanitization {
    /// A covered raster was edited and has a canonical, byte-detected MIME type.
    Image(SanitizedImage),
    /// SVG and non-image data retain their private input bytes and caller MIME behavior.
    Passthrough(NamedTempFile),
}

/// Errors kept private to the host boundary; callers map them to their established upload errors.
#[derive(Debug, Error)]
pub enum ImageSanitizerError {
    /// The supplied executable is not a usable absolute runtime path.
    #[error("image metadata runtime is unavailable")]
    RuntimeUnavailable(#[source] std::io::Error),
    /// A claimed or byte-detected raster cannot safely cross this boundary unchanged.
    #[error("invalid image upload")]
    InvalidImage,
    /// The configured Media maximum bounds both the received and edited bytes.
    #[error("image upload exceeds the configured file-size limit")]
    FileTooLarge,
    /// The editor did not complete before the whole processing deadline.
    #[error("image processing timed out")]
    TimedOut,
    /// The requesting future was canceled before processing completed.
    #[error("image processing canceled")]
    Canceled,
    /// The process or private-file operation failed.
    #[error("image processing failed")]
    Processing(#[source] std::io::Error),
    /// Metadata-free, typed worker failure projection.
    #[error("native image worker failed")]
    Worker(#[source] worker_protocol::Failure),
    /// Framing or schema failure.
    #[error("invalid native image worker reply")]
    Protocol(#[source] worker_protocol::ProtocolError),
    /// Reaping was not confirmed; resources remain retained.
    #[error("image worker reap failed")]
    Reap(#[source] std::io::Error),
    /// Preserve cleanup failure and the original processing failure.
    #[error("private image spool cleanup failed after {primary}")]
    Cleanup {
        primary: Box<ImageSanitizerError>,
        #[source]
        source: std::io::Error,
    },
    /// The explicitly shared capacity was closed unexpectedly.
    #[error("image processing capacity is closed")]
    CapacityClosed(#[source] tokio::sync::AcquireError),
    /// The resource-owning thread terminated without a result.
    #[error("image supervisor terminated")]
    Supervisor(#[source] oneshot::error::RecvError),
    /// PII-free supervisor lifecycle source projection.
    #[error("image supervisor ownership failed")]
    Owner(#[source] OwnerFailure),
    /// Valid JSON with the wrong protocol shape is an infrastructure failure.
    #[error("invalid image worker response")]
    InvalidWorkerResponse,
    /// Preserve the actual exit classification without logging private metadata.
    #[error("image worker exited unsuccessfully ({0})")]
    WorkerExited(std::process::ExitStatus),
}

/// Explicitly injected deployed executable and shared processing capacity.
#[derive(Clone)]
pub struct ImageSanitizer {
    executable: Arc<PathBuf>,
    active_jobs: Arc<Semaphore>,
    owners: Arc<owner_registry::Owners>,
    processing_timeout: Duration,
}
impl ImageSanitizer {
    /// Validate an explicitly supplied absolute executable, without PATH lookup.
    /// # Errors
    /// Returns a typed runtime error for missing or unusable executables.
    pub fn new(executable: PathBuf) -> Result<Self, ImageSanitizerError> {
        if !executable.is_absolute() {
            return Err(ImageSanitizerError::RuntimeUnavailable(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "worker executable must be absolute",
                ),
            ));
        }
        let metadata =
            std::fs::metadata(&executable).map_err(ImageSanitizerError::RuntimeUnavailable)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(ImageSanitizerError::RuntimeUnavailable(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "worker must name an executable file",
                ),
            ));
        }
        Ok(Self {
            executable: Arc::new(executable),
            active_jobs: Arc::new(Semaphore::new(ACTIVE_JOBS)),
            owners: Arc::new(owner_registry::Owners::new()),
            processing_timeout: PROCESSING_TIMEOUT,
        })
    }

    /// Accept private bytes; the single deadline includes capacity wait and replies.
    /// Native editing runs only in the child. A runtime-independent owner retains
    /// the permit and spool until that child is reaped, including runtime teardown.
    /// # Errors
    /// Returns typed validation, processing, protocol, and lifecycle failures.
    pub async fn sanitize(
        &self,
        input: NamedTempFile,
        filename: &Filename,
        claimed_content_type: Option<&ContentType>,
        max_file_size: MaxFileSize,
    ) -> Result<Sanitization, ImageSanitizerError> {
        let deadline = Instant::now() + self.processing_timeout;
        let mut input = QueuedSpool(Some(input));
        let limit = max_file_size.value().unsigned_abs();
        let metadata = input
            .file()
            .as_file()
            .metadata()
            .map_err(|error| input.fail(ImageSanitizerError::Processing(error)))?;
        if metadata.len() > limit {
            return Err(input.fail(ImageSanitizerError::FileTooLarge));
        }
        let permit = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.active_jobs.clone().acquire_owned(),
        )
        .await
        .map_err(|_| input.fail(ImageSanitizerError::TimedOut))?
        .map_err(|error| input.fail(ImageSanitizerError::CapacityClosed(error)))?;
        let (cancel_sender, cancel_receiver) = oneshot::channel();
        let _cancel = CancelJob(Some(cancel_sender));
        let (sender, receiver) = oneshot::channel();
        let executable = self.executable.clone();
        let claim = claims_raster(claimed_content_type, filename);
        self.owners
            .spawn(move |done| {
                let reaped = Arc::new(AtomicBool::new(true));
                let permit_owner = owner_registry::PermitOwner::new(permit, done, reaped.clone());
                let result = capture_owner_failure(|| {
                    supervision::run(
                        &executable,
                        input.take(),
                        claim,
                        limit,
                        deadline,
                        cancel_receiver,
                        &reaped,
                    )
                });
                // Publish completion only after releasing confirmed-reaped capacity,
                // so a resumed caller cannot observe an unsettled resource guard.
                drop(permit_owner);
                // A dropped receiver means the request/runtime is gone. The child is
                // already reaped; explicitly close any completed private output.
                publish_completion(sender, result);
            })
            .map_err(ImageSanitizerError::Owner)?;
        let (input, reply) = receiver.await.map_err(ImageSanitizerError::Supervisor)??;
        Self::finish(input, reply, limit, deadline)
    }

    fn finish(
        input: NamedTempFile,
        reply: worker_protocol::Reply,
        limit: u64,
        deadline: Instant,
    ) -> Result<Sanitization, ImageSanitizerError> {
        let result = (|| {
            use std::os::unix::fs::MetadataExt;
            let actual = input
                .as_file()
                .metadata()
                .map_err(ImageSanitizerError::Processing)?;
            let path_metadata =
                std::fs::symlink_metadata(input.path()).map_err(ImageSanitizerError::Processing)?;
            if !path_metadata.is_file()
                || path_metadata.dev() != actual.dev()
                || path_metadata.ino() != actual.ino()
            {
                return Err(ImageSanitizerError::InvalidWorkerResponse);
            }
            if actual.len() > limit {
                return Err(ImageSanitizerError::FileTooLarge);
            }
            if actual.permissions().mode() & 0o7777 != 0o600 {
                return Err(ImageSanitizerError::InvalidWorkerResponse);
            }
            match reply {
                worker_protocol::Reply::Image { mime, size } => {
                    if size != actual.len() {
                        return Err(ImageSanitizerError::InvalidWorkerResponse);
                    }
                    let mime =
                        covered_mime(&mime).ok_or(ImageSanitizerError::InvalidWorkerResponse)?;
                    Ok(Some(mime))
                }
                worker_protocol::Reply::Passthrough { size } => {
                    if size != actual.len() {
                        return Err(ImageSanitizerError::InvalidWorkerResponse);
                    }
                    Ok(None)
                }
                worker_protocol::Reply::Failed(worker_protocol::Failure::InvalidImage) => {
                    Err(ImageSanitizerError::InvalidImage)
                }
                worker_protocol::Reply::Failed(worker_protocol::Failure::FileTooLarge) => {
                    Err(ImageSanitizerError::FileTooLarge)
                }
                worker_protocol::Reply::Failed(error) => Err(ImageSanitizerError::Worker(error)),
            }
        })();
        // Reject late success without discarding a typed processing failure.
        let result = match result {
            Ok(_) if Instant::now() >= deadline => Err(ImageSanitizerError::TimedOut),
            result => result,
        };
        match result {
            Ok(Some(content_type)) => Ok(Sanitization::Image(SanitizedImage {
                file: input,
                content_type,
            })),
            Ok(None) => Ok(Sanitization::Passthrough(input)),
            Err(primary) => match input.close() {
                Ok(()) => Err(primary),
                Err(source) => Err(ImageSanitizerError::Cleanup {
                    primary: Box::new(primary),
                    source,
                }),
            },
        }
    }

    /// Probe the deployed worker through the ordinary ownership boundary.
    /// # Errors
    /// Returns the typed startup processing failure.
    pub async fn check_runtime(&self) -> Result<(), ImageSanitizerError> {
        let probe = include_bytes!("image_sanitizer_fixtures/png-original.png");
        let input = NamedTempFile::new().map_err(ImageSanitizerError::Processing)?;
        std::fs::write(input.path(), probe).map_err(ImageSanitizerError::Processing)?;
        let name =
            Filename::sanitized("probe.png").map_err(|_| ImageSanitizerError::InvalidImage)?;
        match self
            .sanitize(input, &name, None, MaxFileSize::default())
            .await?
        {
            Sanitization::Image(image)
                if image.content_type().as_ref() == "image/png" && {
                    use sha2::Digest;
                    sha2::Sha256::digest(
                        std::fs::read(image.file().path())
                            .map_err(ImageSanitizerError::Processing)?,
                    )
                    .as_slice()
                        == [
                            0xfd, 0x0e, 0x88, 0x13, 0x6f, 0xda, 0x66, 0x66, 0x1f, 0x07, 0xab, 0x7f,
                            0x26, 0x9c, 0x9f, 0x6a, 0x28, 0x72, 0x6f, 0x43, 0x4c, 0xc9, 0x17, 0x21,
                            0xee, 0x7f, 0xd6, 0x08, 0xfd, 0x48, 0x16, 0x20,
                        ]
                } =>
            {
                Ok(())
            }
            _ => Err(ImageSanitizerError::InvalidWorkerResponse),
        }
    }
}

type OwnerResult = Result<(NamedTempFile, worker_protocol::Reply), ImageSanitizerError>;

fn capture_owner_failure(work: impl FnOnce() -> OwnerResult) -> OwnerResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(result) => result,
        Err(payload) => Err(ImageSanitizerError::Owner(OwnerFailure::Panic(
            payload.as_ref().into(),
        ))),
    }
}

fn publish_completion(sender: oneshot::Sender<OwnerResult>, result: OwnerResult) {
    // The caller settles its permit/reap guard before publication. Failed
    // delivery cannot leave a confirmed-reaped private output on disk.
    match sender.send(result) {
        Err(Ok((input, _))) => {
            if let Err(error) = input.close() {
                report_private_failure(&error, "abandoned image spool cleanup");
            }
        }
        Err(Err(error)) if !matches!(error, ImageSanitizerError::Canceled) => {
            report_private_failure(&error, "abandoned image owner failure");
        }
        _ => {}
    }
}

fn claims_raster(content_type: Option<&ContentType>, filename: &Filename) -> bool {
    let decoded = filename.decoded();
    let extension = Path::new(decoded.as_ref())
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "apng"
            | "gif"
            | "webp"
            | "heic"
            | "heif"
            | "avif"
            | "avifs"
            | "bmp"
            | "tif"
            | "tiff"
            | "ico"
            | "jxl"
            | "jp2"
    ) {
        return true;
    }
    content_type.is_some_and(|value| {
        let essence = value
            .as_ref()
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        essence.starts_with("image/") && essence != "image/svg+xml"
    })
}

fn covered_mime(mime: &str) -> Option<ContentType> {
    match mime {
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/heic" | "image/heif" => {
            mime.parse().ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::process::Stdio;

    #[test]
    fn captured_owner_panic_retains_safe_payload_classification() {
        // Owner unwinding must retain a typed source without reflecting its
        // arbitrary numeric payload in the returning error envelope.
        let error = capture_owner_failure(|| std::panic::panic_any(404_u16))
            .expect_err("intentional owner panic");
        assert!(matches!(
            &error,
            ImageSanitizerError::Owner(OwnerFailure::Panic(PanicFailure {
                kind: PanicKind::Other
            }))
        ));
        assert!(!format!("{error:?} {error}").contains("404"));
    }

    #[test]
    fn abandoned_completion_removes_output_or_retains_real_unlink_failure() {
        // A dropped request must not strand completed private bytes. A real
        // failed unlink remains observable, not falsely treated as removal.
        let input = NamedTempFile::new().expect("private spool");
        let path = input.path().to_owned();
        let (sender, receiver) = oneshot::channel();
        drop(receiver);
        publish_completion(
            sender,
            Ok((input, worker_protocol::Reply::Passthrough { size: 0 })),
        );
        assert!(!path.exists(), "undelivered output must be removed");

        let directory = tempfile::tempdir().expect("isolated private directory");
        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        let path = input.path().to_owned();
        std::fs::remove_file(&path).expect("replace private pathname");
        std::fs::create_dir(&path).expect("non-file unlink failure");
        let (sender, receiver) = oneshot::channel();
        drop(receiver);
        publish_completion(
            sender,
            Ok((input, worker_protocol::Reply::Passthrough { size: 0 })),
        );
        assert!(path.is_dir(), "failed unlink remains observable");
    }

    #[test]
    fn completion_delivery_preserves_typed_failure_and_closed_failure_is_safe() {
        // Connected requests retain their concrete error. Disconnection must
        // not unwind for either typed failure or ordinary cancellation.
        let (sender, receiver) = oneshot::channel();
        publish_completion(sender, Err(ImageSanitizerError::InvalidImage));
        assert!(matches!(
            receiver.blocking_recv().expect("completion delivered"),
            Err(ImageSanitizerError::InvalidImage)
        ));
        for error in [
            ImageSanitizerError::InvalidImage,
            ImageSanitizerError::Canceled,
        ] {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let (sender, receiver) = oneshot::channel();
                    drop(receiver);
                    publish_completion(sender, Err(error));
                }))
                .is_ok(),
                "closed error delivery must not unwind"
            );
        }
    }

    #[test]
    fn completion_rejects_mismatched_size_and_unknown_image_identity() {
        // A worker reply cannot certify a different byte count or an identity
        // outside the covered raster set, even after successful child exit.
        for reply in [
            worker_protocol::Reply::Image {
                mime: "image/png".to_owned(),
                size: 1,
            },
            worker_protocol::Reply::Passthrough { size: 1 },
            worker_protocol::Reply::Image {
                mime: "image/private-invalid".to_owned(),
                size: 0,
            },
        ] {
            let input = NamedTempFile::new().expect("private spool");
            let path = input.path().to_owned();
            assert!(matches!(
                ImageSanitizer::finish(input, reply, 1024, Instant::now() + PROCESSING_TIMEOUT),
                Err(ImageSanitizerError::InvalidWorkerResponse)
            ));
            assert!(
                !path.exists(),
                "rejected completion must remove private bytes"
            );
        }
        let input = NamedTempFile::new().expect("private spool");
        let path = input.path().to_owned();
        assert!(matches!(
            ImageSanitizer::finish(
                input,
                worker_protocol::Reply::Failed(worker_protocol::Failure::FileTooLarge),
                1024,
                Instant::now() + PROCESSING_TIMEOUT
            ),
            Err(ImageSanitizerError::FileTooLarge)
        ));
        assert!(
            !path.exists(),
            "typed size rejection must remove private bytes"
        );
    }

    #[test]
    fn completion_rejects_exposed_or_replaced_private_spool() {
        // The supervisor must validate mode and pathname/inode correspondence,
        // not accept a size-consistent response for exposed or substituted data.
        let input = NamedTempFile::new().expect("private spool");
        let path = input.path().to_owned();
        input
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o644))
            .expect("inject exposed spool");
        assert!(matches!(
            ImageSanitizer::finish(
                input,
                worker_protocol::Reply::Passthrough { size: 0 },
                1024,
                Instant::now() + PROCESSING_TIMEOUT
            ),
            Err(ImageSanitizerError::InvalidWorkerResponse)
        ));
        assert!(!path.exists(), "exposed spool must be removed");

        let input = NamedTempFile::new().expect("private spool");
        let path = input.path().to_owned();
        std::fs::remove_file(&path).expect("replace private pathname");
        std::fs::write(&path, b"different inode").expect("replacement bytes");
        assert!(matches!(
            ImageSanitizer::finish(
                input,
                worker_protocol::Reply::Passthrough { size: 0 },
                1024,
                Instant::now() + PROCESSING_TIMEOUT
            ),
            Err(ImageSanitizerError::InvalidWorkerResponse)
        ));
        assert!(!path.exists(), "substituted pathname must be removed");
    }

    #[test]
    fn completion_cleanup_failure_preserves_primary_rejection() {
        // Failed unlink must retain its concrete I/O source and the primary
        // validation error, never become successful completion or hide cleanup.
        let directory = tempfile::tempdir().expect("isolated private directory");
        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        let path = input.path().to_owned();
        std::fs::remove_file(&path).expect("replace private pathname");
        std::fs::create_dir(&path).expect("non-file unlink failure");
        let error = ImageSanitizer::finish(
            input,
            worker_protocol::Reply::Passthrough { size: 0 },
            1024,
            Instant::now() + PROCESSING_TIMEOUT,
        )
        .expect_err("non-file completion");
        assert!(
            matches!(error, ImageSanitizerError::Cleanup { primary, source } if matches!(*primary, ImageSanitizerError::InvalidWorkerResponse) && source.kind() == std::io::ErrorKind::IsADirectory)
        );
        assert!(
            path.is_dir(),
            "cleanup failure cannot claim successful unlink"
        );
    }

    #[test]
    fn queued_spool_cleanup_failure_preserves_primary_and_failed_unlink_state() {
        // Explicit queue rejection retains its primary typed error. Neither
        // explicit rejection nor cancellation Drop can pretend a failed unlink
        // removed the obstructing pathname; private failure reporting is separate.
        let directory = tempfile::tempdir().expect("isolated private directory");
        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        let path = input.path().to_owned();
        std::fs::remove_file(&path).expect("replace private pathname");
        std::fs::create_dir(&path).expect("non-file unlink failure");
        let mut queued = QueuedSpool(Some(input));
        assert!(
            matches!(queued.fail(ImageSanitizerError::InvalidImage), ImageSanitizerError::Cleanup { primary, source } if matches!(*primary, ImageSanitizerError::InvalidImage) && source.kind() == std::io::ErrorKind::IsADirectory)
        );
        assert!(
            queued.0.is_none(),
            "explicit rejection transfers ownership once"
        );
        assert!(path.is_dir(), "failed unlink remains observable");

        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        let path = input.path().to_owned();
        std::fs::remove_file(&path).expect("replace private pathname");
        std::fs::create_dir(&path).expect("non-file unlink failure");
        drop(QueuedSpool(Some(input)));
        assert!(
            path.is_dir(),
            "Drop cannot falsely report successful unlink"
        );
    }

    #[test]
    fn expired_deadline_preserves_typed_failure_and_rejects_success() {
        use worker_protocol::{Failure, MetadataFailure, Reply};
        let deadline = Instant::now();
        let input = NamedTempFile::new().expect("private spool");
        let path = input.path().to_owned();
        let error = ImageSanitizer::finish(
            input,
            Reply::Failed(Failure::Metadata(MetadataFailure::InvalidXmp)),
            1024,
            deadline,
        )
        .expect_err("late worker failure");
        assert!(matches!(
            error,
            ImageSanitizerError::Worker(Failure::Metadata(MetadataFailure::InvalidXmp))
        ));
        assert!(
            !path.exists(),
            "failed output remains private and is removed"
        );
        let input = NamedTempFile::new().expect("private spool");
        let path = input.path().to_owned();
        assert!(matches!(
            ImageSanitizer::finish(input, Reply::Passthrough { size: 0 }, 1024, deadline),
            Err(ImageSanitizerError::TimedOut)
        ));
        assert!(!path.exists(), "late success cannot survive the deadline");
    }

    fn editor_runtime() -> PathBuf {
        std::env::var_os("JAUNDER_IMAGE_WORKER_BIN")
            .map(PathBuf::from)
            .expect("real worker fixture prepared by test-local")
    }
    fn test_filename() -> Filename {
        Filename::sanitized("fixture.bin").expect("name")
    }
    fn max_file_size() -> MaxFileSize {
        "1048576".parse().expect("limit")
    }
    fn image_variant(result: Sanitization) -> Option<SanitizedImage> {
        match result {
            Sanitization::Image(image) => Some(image),
            Sanitization::Passthrough(_) => None,
        }
    }
    fn test_tool(name: &str) -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").expect("test tool PATH"))
            .map(|directory| directory.join(name))
            .find(|path| path.is_file())
            .expect("pinned independent observer")
    }
    fn metadata_record(bytes: &[u8]) -> Result<serde_json::Map<String, Value>, serde_json::Error> {
        let mut records: Vec<serde_json::Map<String, Value>> = serde_json::from_slice(bytes)?;
        assert_eq!(records.len(), 1, "observer record count");
        Ok(records.remove(0))
    }
    struct ObserverOutput(std::process::Output);
    impl ObserverOutput {
        fn successful(self) -> Result<Vec<u8>, std::io::Error> {
            if self.0.status.success() {
                Ok(self.0.stdout)
            } else {
                Err(std::io::Error::other("independent observer failed"))
            }
        }
    }
    // Observers own a separate test-only executable, never the native adapter.
    async fn observe_tags(
        arguments: &[&str],
        path: &Path,
    ) -> Result<ObserverOutput, std::io::Error> {
        tokio::process::Command::new(test_tool("exiftool"))
            .args(["-config", ""])
            .args(arguments)
            .arg(path)
            .stdin(Stdio::null())
            .output()
            .await
            .map(ObserverOutput)
    }
    fn script(contents: &str) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("script root");
        let path = directory.path().join("tool.sh");
        std::fs::write(&path, contents).expect("script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).expect("executable");
        directory
    }
    #[tokio::test]
    async fn readiness_rejects_a_successful_worker_that_keeps_original_probe_bytes() {
        let size = include_bytes!("image_sanitizer_fixtures/png-original.png").len();
        let tool = script(&format!(
            "#!/bin/sh\nprintf 'JAUNDER-IMAGE-1\\0{{\"Image\":{{\"mime\":\"image/png\",\"size\":{size}}}}}'\n"
        ));
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("test worker");
        assert!(matches!(
            sanitizer.check_runtime().await,
            Err(ImageSanitizerError::InvalidWorkerResponse)
        ));
    }

    #[tokio::test]
    async fn image_assertion_helper_rejects_real_svg_passthrough_and_drops_private_input() {
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("native worker");
        let input = NamedTempFile::new().expect("private SVG input");
        let path = input.path().to_owned();
        std::fs::write(&path, b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>").expect("SVG bytes");
        let result = sanitizer
            .sanitize(input, &test_filename(), None, max_file_size())
            .await
            .expect("excluded SVG remains passthrough");
        assert!(
            image_variant(result).is_none(),
            "passthrough cannot satisfy an image assertion"
        );
        assert!(
            !path.exists(),
            "rejected test outcome releases private input"
        );
    }

    #[tokio::test]
    async fn independent_observer_failure_is_not_accepted_as_success() {
        let directory = tempfile::tempdir().expect("observer test root");
        let output = observe_tags(&["-j"], &directory.path().join("missing-image.png"))
            .await
            .expect("independent observer launches");
        assert!(
            !output.0.status.success(),
            "missing input must fail independently"
        );
        assert!(matches!(output.successful(), Err(error)
            if error.kind() == std::io::ErrorKind::Other
            && error.to_string() == "independent observer failed"));
    }

    #[tokio::test]
    async fn pid_observation_rejects_unexpected_filesystem_failure() {
        let directory = tempfile::tempdir().expect("PID observation root");
        std::fs::create_dir(directory.path().join("pid")).expect("non-file PID obstruction");
        let failure = tokio::spawn(async move { tool_pid(directory.path()).await })
            .await
            .expect_err("unexpected filesystem failure must not become a retry");
        assert!(failure.is_panic(), "PID observation must fail closed");
        let payload = failure.into_panic();
        let message = payload
            .downcast_ref::<String>()
            .expect("formatted PID observation panic");
        assert!(
            message.starts_with("PID observation failed:"),
            "unexpected filesystem errors must fail immediately, not time out: {message}"
        );
    }

    fn blocked_tool() -> tempfile::TempDir {
        let directory = script("#!/bin/sh\nexit 99\n");
        std::fs::write(
            directory.path().join("tool.sh"),
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec '{}' 30\n",
                directory.path().join("pid").display(),
                test_tool("sleep").display()
            ),
        )
        .expect("blocked tool");
        directory
    }
    async fn tool_pid(directory: &Path) -> rustix::process::Pid {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match tokio::fs::read_to_string(directory.join("pid")).await {
                    Ok(value) if !value.is_empty() => {
                        return rustix::process::Pid::from_raw(value.parse().expect("PID"))
                            .expect("positive PID");
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("PID observation failed: {error}"),
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("child starts")
    }
    fn spawn_sanitization(
        sanitizer: ImageSanitizer,
        input: NamedTempFile,
    ) -> tokio::task::JoinHandle<Result<Sanitization, ImageSanitizerError>> {
        tokio::spawn(async move {
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await
        })
    }
    #[tokio::test]
    async fn timeout_reaps_child_and_cleans_private_input() {
        let tool = blocked_tool();
        let mut sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        sanitizer.processing_timeout = Duration::from_secs(1);
        let input = tempfile::NamedTempFile::new().expect("input");
        let path = input.path().to_owned();
        let job = spawn_sanitization(sanitizer, input);
        let pid = tool_pid(tool.path()).await;
        assert!(matches!(
            job.await.expect("job"),
            Err(ImageSanitizerError::TimedOut)
        ));
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
        assert!(
            !path.exists(),
            "private input must be cleaned after reaping"
        );
    }
    #[tokio::test]
    async fn cancellation_reaps_before_cleanup_and_releases_capacity() {
        let tool = blocked_tool();
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let active = sanitizer.active_jobs.clone();
        let input = tempfile::NamedTempFile::new().expect("input");
        let path = input.path().to_owned();
        let job = spawn_sanitization(sanitizer, input);
        let pid = tool_pid(tool.path()).await;
        assert!(path.exists());
        assert_eq!(active.available_permits(), ACTIVE_JOBS - 1);
        job.abort();
        assert!(job.await.expect_err("request canceled").is_cancelled());
        tokio::time::timeout(Duration::from_secs(5), async {
            while path.exists() || active.available_permits() != ACTIVE_JOBS {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("job owner must finish cleanup");
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
    }
    #[test]
    fn startup_rejects_missing_and_relative_runtimes() {
        assert!(ImageSanitizer::new(PathBuf::from("exiftool")).is_err());
        assert!(ImageSanitizer::new(PathBuf::from("/definitely/missing/exiftool")).is_err());
    }
    #[test]
    fn startup_rejects_directories_and_non_executable_files() {
        let directory = tempfile::tempdir().expect("runtime directory");
        assert!(matches!(
            ImageSanitizer::new(directory.path().to_owned()),
            Err(ImageSanitizerError::RuntimeUnavailable(_))
        ));
        let file = NamedTempFile::new_in(directory.path()).expect("non-executable file");
        assert!(matches!(
            ImageSanitizer::new(file.path().to_owned()),
            Err(ImageSanitizerError::RuntimeUnavailable(_))
        ));
    }
    #[tokio::test]
    async fn closed_capacity_is_a_typed_failure_and_cleans_input() {
        let tool = script("#!/bin/sh\nexit 99\n");
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        sanitizer.active_jobs.close();
        let input = NamedTempFile::new().expect("private input");
        let path = input.path().to_owned();
        assert!(matches!(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await,
            Err(ImageSanitizerError::CapacityClosed(_))
        ));
        assert!(!path.exists());
    }
    #[tokio::test]
    async fn current_upload_limit_is_not_cached_by_the_service() {
        let tool = script(
            "#!/bin/sh\nprintf '[{\"SourceFile\":\"private\",\"MIMEType\":\"text/plain\"}]'\n",
        );
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        std::fs::write(input.path(), b"ordinary text").expect("input bytes");
        let smaller = "4".parse().expect("limit");
        assert!(matches!(
            sanitizer
                .sanitize(input, &test_filename(), None, smaller)
                .await,
            Err(ImageSanitizerError::FileTooLarge)
        ));
    }

    #[cfg(target_os = "linux")]
    async fn stopped_real_worker(
        sanitizer: ImageSanitizer,
        directory: &Path,
    ) -> (
        tokio::task::JoinHandle<Result<Sanitization, ImageSanitizerError>>,
        PathBuf,
        rustix::process::Pid,
    ) {
        let input = NamedTempFile::new().expect("real private input");
        // Reading/copying this bounded sparse attachment gives the observer time
        // to suspend the real executable, without changing its worker behavior.
        input
            .as_file()
            .set_len(64 * 1024 * 1024)
            .expect("large bounded input");
        let path = input.path().to_owned();
        let job = tokio::spawn(async move {
            sanitizer
                .sanitize(
                    input,
                    &test_filename(),
                    None,
                    "134217728".parse().expect("limit"),
                )
                .await
        });
        let pid = tool_pid(directory).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let exe = std::fs::read_link(format!("/proc/{}/exe", pid.as_raw_nonzero()))
                    .expect("live worker executable");
                if exe == editor_runtime() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("wrapper must exec the actual worker");
        rustix::process::kill_process(pid, rustix::process::Signal::STOP)
            .expect("stop actual native worker");
        (job, path, pid)
    }

    #[cfg(target_os = "linux")]
    fn real_worker_wrapper() -> tempfile::TempDir {
        let directory = script("#!/bin/sh\nexit 99\n");
        std::fs::write(
            directory.path().join("tool.sh"),
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec '{}' \"$@\"\n",
                directory.path().join("pid").display(),
                editor_runtime().display()
            ),
        )
        .expect("real exec observer");
        directory
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn real_executable_timeout_and_cancellation_reap_before_capacity_and_spool_release() {
        for timeout in [false, true] {
            let directory = real_worker_wrapper();
            let mut sanitizer =
                ImageSanitizer::new(directory.path().join("tool.sh")).expect("real worker wrapper");
            if timeout {
                sanitizer.processing_timeout = Duration::from_secs(1);
            }
            let capacity = sanitizer.active_jobs.clone();
            let (job, path, pid) = stopped_real_worker(sanitizer, directory.path()).await;
            assert!(path.exists());
            assert_eq!(capacity.available_permits(), 1);
            if timeout {
                assert!(matches!(
                    job.await.expect("supervisor result"),
                    Err(ImageSanitizerError::TimedOut)
                ));
            } else {
                job.abort();
                assert!(job.await.expect_err("caller canceled").is_cancelled());
            }
            tokio::time::timeout(Duration::from_secs(5), async {
                while path.exists() || capacity.available_permits() != 2 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("confirmed native reap and cleanup");
            assert_eq!(
                rustix::process::test_kill_process(pid),
                Err(rustix::io::Errno::SRCH)
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn real_executable_crash_is_typed_and_reaped_before_cleanup() {
        use std::os::unix::process::ExitStatusExt;
        let directory = real_worker_wrapper();
        let sanitizer = ImageSanitizer::new(directory.path().join("tool.sh")).expect("real worker");
        let capacity = sanitizer.active_jobs.clone();
        let (job, path, pid) = stopped_real_worker(sanitizer, directory.path()).await;
        rustix::process::kill_process(pid, rustix::process::Signal::KILL)
            .expect("crash real worker");
        assert!(
            matches!(job.await.expect("owner result"), Err(ImageSanitizerError::WorkerExited(status)) if status.signal() == Some(9))
        );
        assert!(!path.exists());
        assert_eq!(capacity.available_permits(), 2);
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_executable_is_reaped_after_runtime_teardown() {
        let directory = real_worker_wrapper();
        let sanitizer = ImageSanitizer::new(directory.path().join("tool.sh")).expect("real worker");
        let capacity = sanitizer.active_jobs.clone();
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (_job, path, pid) = runtime.block_on(stopped_real_worker(sanitizer, directory.path()));
        assert!(path.exists());
        drop(runtime);
        assert!(
            !path.exists(),
            "runtime teardown must join final adapter owners"
        );
        assert_eq!(capacity.available_permits(), 2);
        if let Some(receipt) = std::env::var_os("JAUNDER_OWNER_EXIT_RECEIPT") {
            std::fs::write(receipt, serde_json::to_vec(&serde_json::json!({"pid": pid.as_raw_nonzero().get(), "spool": path, "capacity": 2})).expect("exit receipt")).expect("write exit receipt");
        }
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn normal_process_exit_joins_real_worker_owners() {
        let directory = tempfile::tempdir().expect("normal-exit proof");
        let receipt = directory.path().join("receipt.json");
        let output =
            std::process::Command::new(std::env::current_exe().expect("test host executable"))
                .args([
                    "--exact",
                    "image_sanitizer::tests::real_executable_is_reaped_after_runtime_teardown",
                    "--nocapture",
                ])
                .env("JAUNDER_OWNER_EXIT_RECEIPT", &receipt)
                .output()
                .expect("normal-return subprocess");
        assert!(
            output.status.success(),
            "subprocess teardown: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(receipt).expect("normal-exit receipt"))
                .expect("receipt schema");
        let pid = rustix::process::Pid::from_raw(
            i32::try_from(record["pid"].as_i64().expect("PID")).expect("PID fits platform"),
        )
        .expect("positive PID");
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
        assert!(!Path::new(record["spool"].as_str().expect("spool path")).exists());
        assert_eq!(record["capacity"], 2);
    }

    #[tokio::test]
    async fn clones_share_exactly_two_active_jobs_and_queued_cancellation_starts_no_child() {
        let directory = script("#!/bin/sh\nexit 99\n");
        std::fs::write(
            directory.path().join("tool.sh"),
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}/pid-'\"$$\"\nexec '{}' 30\n",
                directory.path().display(),
                test_tool("sleep").display()
            ),
        )
        .expect("two-job observer");
        let sanitizer = ImageSanitizer::new(directory.path().join("tool.sh")).expect("worker");
        let capacity = sanitizer.active_jobs.clone();
        let mut jobs = Vec::new();
        let mut paths = Vec::new();
        for _ in 0..2 {
            let input = NamedTempFile::new().expect("input");
            paths.push(input.path().to_owned());
            jobs.push(spawn_sanitization(sanitizer.clone(), input));
        }
        let pids = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let markers: Vec<_> = std::fs::read_dir(directory.path())
                    .expect("markers")
                    .map(|entry| entry.expect("entry").path())
                    .filter(|path| {
                        path.file_name()
                            .expect("marker name")
                            .to_string_lossy()
                            .starts_with("pid-")
                    })
                    .collect();
                if markers.len() == 2 {
                    return markers
                        .iter()
                        .map(|path| {
                            rustix::process::Pid::from_raw(
                                path.file_name()
                                    .expect("name")
                                    .to_string_lossy()
                                    .trim_start_matches("pid-")
                                    .parse()
                                    .expect("PID"),
                            )
                            .expect("positive PID")
                        })
                        .collect::<Vec<_>>();
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("two active children");
        assert_eq!(capacity.available_permits(), 0);
        let queued_input = NamedTempFile::new().expect("queued input");
        paths.push(queued_input.path().to_owned());
        jobs.push(spawn_sanitization(sanitizer, queued_input));
        tokio::task::yield_now().await;
        // Cancel the queued caller before reaping active children frees capacity.
        jobs.reverse();
        for job in jobs {
            job.abort();
            assert!(job.await.expect_err("canceled caller").is_cancelled());
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while paths.iter().any(|path| path.exists()) || capacity.available_permits() != 2 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("reap all active children and close queued spool");
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("markers")
                .count(),
            3,
            "one script and exactly two PID markers"
        );
        for pid in pids {
            assert_eq!(
                rustix::process::test_kill_process(pid),
                Err(rustix::io::Errno::SRCH)
            );
        }
    }

    #[test]
    fn runtime_teardown_cancels_owner_without_releasing_a_live_child() {
        let directory = blocked_tool();
        let sanitizer = ImageSanitizer::new(directory.path().join("tool.sh")).expect("worker");
        let capacity = sanitizer.active_jobs.clone();
        let input = NamedTempFile::new().expect("private spool");
        let path = input.path().to_owned();
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let pid = runtime.block_on(async {
            let _job = spawn_sanitization(sanitizer, input);
            tool_pid(directory.path()).await
        });
        assert!(path.exists());
        drop(runtime);
        let deadline = Instant::now() + Duration::from_secs(5);
        while path.exists() || capacity.available_permits() != 2 {
            assert!(
                Instant::now() < deadline,
                "runtime-independent owner must clean up"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
    }

    #[tokio::test]
    async fn queue_wait_uses_the_whole_deadline() {
        let tool = script("#!/bin/sh\nexit 99\n");
        let mut sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("adapter");
        sanitizer.processing_timeout = Duration::from_millis(25);
        let _held = sanitizer
            .active_jobs
            .acquire_many(2)
            .await
            .expect("capacity");
        let input = NamedTempFile::new().expect("input");
        let path = input.path().to_owned();
        assert!(matches!(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await,
            Err(ImageSanitizerError::TimedOut)
        ));
        assert!(!path.exists());
    }
    #[tokio::test]
    async fn malformed_or_failed_replies_cannot_become_passthrough() {
        for (payload, exit) in [
            ("[]", 0),
            ("not json", 0),
            ("{\"Image\":{\"mime\":\"image/unknown\",\"size\":0}}", 0),
            ("{\"Passthrough\":{\"size\":1}}", 0),
            ("{\"Passthrough\":{\"size\":0}}", 17),
        ] {
            let tool = script(&format!(
                "#!/bin/sh\nprintf 'JAUNDER-IMAGE-1\\000%s' '{payload}'\nexit {exit}\n"
            ));
            let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("adapter");
            let input = NamedTempFile::new().expect("input");
            let path = input.path().to_owned();
            let result = sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await;
            assert!(result.is_err(), "{result:?}");
            assert!(!path.exists());
            assert_eq!(sanitizer.active_jobs.available_permits(), 2);
        }
    }
    #[tokio::test]
    async fn oversized_response_is_reaped_and_cleans_private_spool() {
        let tool = script(&format!(
            "#!{}\nimport sys\nsys.stdout.write('x' * {})\n",
            test_tool("python3").display(),
            worker_protocol::MAX_REPLY_BYTES + 1
        ));
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("adapter");
        let input = NamedTempFile::new().expect("input");
        let path = input.path().to_owned();
        assert!(matches!(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await,
            Err(ImageSanitizerError::Protocol(
                worker_protocol::ProtocolError::TooLarge
            ))
        ));
        assert!(!path.exists());
    }
    #[tokio::test]
    async fn real_worker_preserves_attachments_and_rejects_claimed_raster() {
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("real worker");
        for (name, bytes, rejected) in [
            ("note.txt", b"ordinary".as_slice(), false),
            ("photo.JPG", b"ordinary".as_slice(), true),
            (
                "vector.svg",
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".as_slice(),
                false,
            ),
            (
                "misleading.JPG",
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".as_slice(),
                false,
            ),
        ] {
            let input = NamedTempFile::new().expect("input");
            std::fs::write(input.path(), bytes).expect("bytes");
            let result = sanitizer
                .sanitize(
                    input,
                    &Filename::sanitized(name).expect("name"),
                    None,
                    max_file_size(),
                )
                .await;
            if rejected {
                assert!(matches!(result, Err(ImageSanitizerError::InvalidImage)));
            } else {
                let Sanitization::Passthrough(file) = result.expect("passthrough") else {
                    panic!("attachment converted")
                };
                assert_eq!(std::fs::read(file.path()).expect("bytes"), bytes);
            }
        }
    }
    #[tokio::test]
    async fn edited_file_handle_and_path_name_the_same_bytes() {
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("pinned runtime");
        let input = tempfile::NamedTempFile::new().expect("private input");
        std::fs::write(
            input.path(),
            include_bytes!("image_sanitizer_fixtures/png-original.png"),
        )
        .expect("copy fixture");
        let edited = image_variant(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await
                .expect("edit"),
        )
        .expect("PNG must be edited");
        let mut handle = edited.file().as_file().try_clone().expect("clone handle");
        let mut handle_bytes = Vec::new();
        std::io::Read::read_to_end(&mut handle, &mut handle_bytes).expect("read handle");
        assert!(
            handle_bytes == std::fs::read(edited.file().path()).expect("read path"),
            "the returned handle must not retain the unedited inode",
        );
    }

    #[tokio::test]
    async fn representative_hdr_signal_survives_private_comment_removal() {
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        std::fs::write(
            input.path(),
            include_bytes!("image_sanitizer_fixtures/hdr-signal.png"),
        )
        .expect("owned HDR-signaling fixture");
        let arguments = [
            "-j",
            "-n",
            "-G1",
            "-ColorPrimaries",
            "-TransferCharacteristics",
            "-MatrixCoefficients",
            "-VideoFullRangeFlag",
            "-Comment",
        ];
        let before = observe_tags(&arguments, input.path())
            .await
            .expect("independent metadata inspection")
            .successful()
            .expect("tool success");
        let before = metadata_record(&before).expect("metadata");
        assert!(before.contains_key("PNG:Comment"));
        let edited = image_variant(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await
                .expect("metadata removal"),
        )
        .expect("HDR-signaling PNG remains an image");
        let after = observe_tags(&arguments, edited.file().path())
            .await
            .expect("independent output inspection")
            .successful()
            .expect("tool success");
        let after = metadata_record(&after).expect("metadata");
        assert!(!after.contains_key("PNG:Comment"));
        for (key, value) in [
            ("PNG-cICP:ColorPrimaries", 9),
            ("PNG-cICP:TransferCharacteristics", 16),
            ("PNG-cICP:MatrixCoefficients", 0),
            ("PNG-cICP:VideoFullRangeFlag", 1),
        ] {
            assert_eq!(before.get(key), Some(&serde_json::json!(value)));
            assert_eq!(after.get(key), before.get(key), "rendering signal {key}");
        }
    }

    struct FixtureObservation {
        private_fields: usize,
        rendering_metadata: serde_json::Map<String, Value>,
        decoded: Value,
    }

    async fn observe_fixture(path: &Path) -> FixtureObservation {
        let response = observe_tags(
            &[
                "-j",
                "-n",
                "-G1",
                "-Comment",
                "-Artist",
                "-Make",
                "-Model",
                "-GPSLatitude",
                "-Orientation",
                "-ImageWidth",
                "-ImageHeight",
                "-FrameCount",
                "-AnimationIterations",
                "-Duration",
                "-NumFrames",
                "-NumPlays",
            ],
            path,
        )
        .await
        .expect("independent tag read")
        .successful()
        .expect("metadata reader");
        let mut metadata = metadata_record(&response).expect("fixture metadata");
        metadata.remove("SourceFile");
        let private_fields = metadata
            .keys()
            .filter(|key| {
                ["Comment", "Artist", "Make", "Model", "GPSLatitude"]
                    .iter()
                    .any(|tag| key.ends_with(&format!(":{tag}")))
            })
            .count();
        metadata.retain(|key, _| {
            !["Comment", "Artist", "Make", "Model", "GPSLatitude"]
                .iter()
                .any(|tag| key.ends_with(&format!(":{tag}")))
        });
        let python = std::env::var_os("JAUNDER_IMAGE_PYTHON")
            .map_or_else(|| test_tool("python3"), PathBuf::from);
        let magick = std::env::var_os("JAUNDER_IMAGE_MAGICK")
            .map_or_else(|| test_tool("magick"), PathBuf::from);
        let decoded = std::process::Command::new(python)
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/image_sanitizer_fixtures/inspect_rendering.py"
            ))
            .arg(path)
            .arg(magick)
            .arg(test_tool("exiftool"))
            .output()
            .expect("conventional decoder");
        let decoding_diagnostic = String::from_utf8_lossy(&decoded.stderr);
        assert!(
            decoded.status.success(),
            "owned fixture decoding: {decoding_diagnostic}"
        );
        FixtureObservation {
            private_fields,
            rendering_metadata: metadata,
            decoded: serde_json::from_slice(&decoded.stdout).expect("decoder observation"),
        }
    }

    struct OwnedFormatCase {
        name: &'static str,
        bytes: &'static [u8],
        mime: &'static str,
        animated: bool,
        non_opaque_alpha: bool,
    }

    async fn assert_owned_format_with_stable_output(case: OwnedFormatCase) {
        let name = case.name;
        let expected_mime = case.mime;
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        std::fs::write(input.path(), case.bytes).expect("fixture");
        let before = observe_fixture(input.path()).await;
        let frames = before.decoded["frames"].as_array();
        if case.animated {
            assert!(
                frames.is_some_and(|frames| frames.len() > 1),
                "animated fixture {name} must expose its frames"
            );
        }
        if case.non_opaque_alpha {
            assert!(
                frames.is_some_and(|frames| frames.iter().any(|frame| {
                    frame["alpha_extrema"][0]
                        .as_u64()
                        .is_some_and(|alpha| alpha < 255)
                })),
                "owned fixture {name} must exercise non-opaque alpha"
            );
        }
        assert!(
            before.private_fields > 0,
            "case {name} must have planted private fields"
        );
        let first = image_variant(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await
                .expect("owned raster sanitization"),
        )
        .expect("owned raster must be edited");
        assert_eq!(first.content_type().as_ref(), expected_mime);
        let after = observe_fixture(first.file().path()).await;
        assert_eq!(
            after.private_fields, 0,
            "case {name} removes planted fields"
        );
        assert_eq!(
            before.rendering_metadata, after.rendering_metadata,
            "case {name} rendering tags"
        );
        assert_eq!(
            before.decoded, after.decoded,
            "case {name} pixels, frames, timing, transparency and intact ICC"
        );
        let edited = std::fs::read(first.file().path()).expect("edited bytes");
        let second = image_variant(
            sanitizer
                .sanitize(first.into_file(), &test_filename(), None, max_file_size())
                .await
                .expect("reupload"),
        )
        .expect("edited raster must stay covered");
        assert!(
            edited == std::fs::read(second.file().path()).expect("second bytes"),
            "unstable {expected_mime}"
        );
    }

    // Each format owns the normal whole-test budget; the complete observer and
    // reupload proof remains shared rather than bundling nine jobs into one case.
    macro_rules! owned_format_cases {
        ($($name:ident => {
            fixture: $fixture:literal,
            mime: $mime:literal,
            animated: $animated:literal,
            non_opaque_alpha: $alpha:literal
        }),+ $(,)?) => {
            const _: [&str; 9] = [$(stringify!($name)),+];
            $(#[tokio::test]
            async fn $name() {
                assert_owned_format_with_stable_output(OwnedFormatCase {
                    name: stringify!($name),
                    bytes: include_bytes!($fixture),
                    mime: $mime,
                    animated: $animated,
                    non_opaque_alpha: $alpha,
                }).await;
            })+
        };
    }
    owned_format_cases! {
        actual_editor_covers_owned_formats_with_stable_output_jpeg => {
            fixture: "image_sanitizer_fixtures/jpeg-original.jpg", mime: "image/jpeg",
            animated: false, non_opaque_alpha: false
        },
        actual_editor_covers_owned_formats_with_stable_output_png => {
            fixture: "image_sanitizer_fixtures/png-original.png", mime: "image/png",
            animated: false, non_opaque_alpha: true
        },
        actual_editor_covers_owned_formats_with_stable_output_apng_first => {
            fixture: "image_sanitizer_fixtures/apng-first-original.png", mime: "image/png",
            animated: true, non_opaque_alpha: false
        },
        actual_editor_covers_owned_formats_with_stable_output_apng_default => {
            fixture: "image_sanitizer_fixtures/apng-default-original.png", mime: "image/png",
            animated: true, non_opaque_alpha: false
        },
        actual_editor_covers_owned_formats_with_stable_output_gif_static => {
            fixture: "image_sanitizer_fixtures/gif-static-original.gif", mime: "image/gif",
            animated: false, non_opaque_alpha: false
        },
        actual_editor_covers_owned_formats_with_stable_output_gif_animation => {
            fixture: "image_sanitizer_fixtures/gif-animation-original.gif", mime: "image/gif",
            animated: true, non_opaque_alpha: false
        },
        actual_editor_covers_owned_formats_with_stable_output_webp_static => {
            fixture: "image_sanitizer_fixtures/webp-static-original.webp", mime: "image/webp",
            animated: false, non_opaque_alpha: false
        },
        actual_editor_covers_owned_formats_with_stable_output_webp_animation => {
            fixture: "image_sanitizer_fixtures/webp-animation-original.webp", mime: "image/webp",
            animated: true, non_opaque_alpha: false
        },
        actual_editor_covers_owned_formats_with_stable_output_heic => {
            fixture: "image_sanitizer_fixtures/heic-original.heic", mime: "image/heic",
            animated: false, non_opaque_alpha: false
        },
    }

    #[tokio::test]
    async fn edited_output_stays_in_private_input_spool() {
        let spool = tempfile::tempdir().expect("private spool");
        let input = NamedTempFile::new_in(spool.path()).expect("private input");
        std::fs::write(
            input.path(),
            include_bytes!("image_sanitizer_fixtures/png-original.png"),
        )
        .expect("owned fixture");
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("runtime");
        let image = image_variant(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await
                .expect("edited image"),
        )
        .expect("covered image");
        assert_eq!(
            image.file().path().parent(),
            Some(spool.path()),
            "final placement must not depend on system temporary mount topology"
        );
        drop(image);
        assert!(
            std::fs::read_dir(spool.path())
                .expect("spool")
                .next()
                .is_none()
        );
    }

    #[tokio::test]
    async fn actual_editor_removes_metadata_and_is_byte_idempotent() {
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("pinned runtime");
        sanitizer
            .check_runtime()
            .await
            .expect("startup runtime probe");
        let input = tempfile::NamedTempFile::new().expect("private input");
        std::fs::write(
            input.path(),
            include_bytes!("image_sanitizer_fixtures/png-original.png"),
        )
        .expect("copy owned fixture");

        let edited = image_variant(
            sanitizer
                .sanitize(
                    input,
                    &test_filename(),
                    Some(&"image/jpeg".parse().expect("valid mime")),
                    max_file_size(),
                )
                .await
                .expect("metadata removal"),
        )
        .expect("a PNG must be sanitized");
        assert_eq!(edited.content_type(), "image/png");
        let second_input = tempfile::NamedTempFile::new().expect("private input");
        std::fs::copy(edited.file().path(), second_input.path()).expect("copy edited input");
        let first = std::fs::read(edited.file().path()).expect("read edited bytes");
        let second = image_variant(
            sanitizer
                .sanitize(second_input, &test_filename(), None, max_file_size())
                .await
                .expect("idempotent metadata removal"),
        )
        .expect("an edited PNG remains covered");
        assert_eq!(
            first,
            std::fs::read(second.file().path()).expect("read second bytes")
        );
    }
}
