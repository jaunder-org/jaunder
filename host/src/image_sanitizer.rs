//! Private, conventional metadata editing for covered raster uploads.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use common::media::{ContentType, Filename, MaxFileSize};
use serde_json::Value;
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::{Semaphore, oneshot};
use tokio::time::{Instant, timeout_at};

/// Absolute runtime path embedded by the package builder for composition roots.
/// Development roots can instead receive `JAUNDER_EXIFTOOL` at runtime.
pub const PACKAGED_EXIFTOOL: Option<&str> = option_env!("JAUNDER_EXIFTOOL");

const PROCESSING_TIMEOUT: Duration = Duration::from_secs(30);
const ACTIVE_JOBS: usize = 2;
const MAX_TOOL_RESPONSE: u64 = 128 * 1024;

// Dropping the request asks the job owner to reap its child before dropping
// private files. The job, not the canceled caller, owns those resources.
struct CancelJob(Option<oneshot::Sender<()>>);

impl Drop for CancelJob {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            // A completed job has already closed its receiver; no work remains
            // to cancel in that expected case.
            let _ = sender.send(());
        }
    }
}

async fn read_tool_response(reader: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_TOOL_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_TOOL_RESPONSE {
        return Err(std::io::Error::other(
            "metadata response exceeds processing limit",
        ));
    }
    Ok(bytes)
}
const PRIVATE_TAGS: &[&str] = &[
    "-GPS:all",
    "-EXIF:Make",
    "-EXIF:Model",
    "-EXIF:Artist",
    "-EXIF:DateTimeOriginal",
    "-EXIF:CreateDate",
    "-EXIF:ModifyDate",
    "-EXIF:SerialNumber",
    "-EXIF:BodySerialNumber",
    "-EXIF:LensSerialNumber",
    "-EXIF:CameraOwnerName",
    "-EXIF:ImageUniqueID",
    "-XMP:all",
    "-IPTC:all",
    "-Comment",
    "-Description",
    "-ThumbnailImage",
    "-PreviewImage",
];

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
    /// The resource-owning processing task failed unexpectedly.
    #[error("image processing task failed")]
    Task(#[source] tokio::task::JoinError),
    /// The process or private-file operation failed.
    #[error("image processing failed")]
    Processing(#[source] std::io::Error),
    /// The runtime emitted an unusable response; it is never treated as a non-image result.
    #[error("image processing failed")]
    ToolResponse(#[source] serde_json::Error),
    /// Valid JSON with the wrong protocol shape is an infrastructure failure.
    #[error("invalid metadata runtime response")]
    InvalidToolResponse,
    /// Preserve the actual exit classification without logging private metadata.
    #[error("metadata runtime exited unsuccessfully ({0})")]
    ToolExited(std::process::ExitStatus),
}

/// Established-tool image metadata editor, injected with its runtime path and size policy.
#[derive(Clone)]
pub struct ImageSanitizer {
    executable: Arc<PathBuf>,
    active_jobs: Arc<Semaphore>,
    processing_timeout: Duration,
}

impl ImageSanitizer {
    /// Checks the explicitly supplied, absolute `ExifTool` executable path.
    ///
    /// # Errors
    ///
    /// Returns an error when `executable` is relative, missing, or not a regular file.
    pub fn new(executable: PathBuf) -> Result<Self, ImageSanitizerError> {
        if !executable.is_absolute() {
            return Err(ImageSanitizerError::RuntimeUnavailable(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "runtime path must be absolute",
                ),
            ));
        }
        let metadata =
            std::fs::metadata(&executable).map_err(ImageSanitizerError::RuntimeUnavailable)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(ImageSanitizerError::RuntimeUnavailable(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "runtime path must name an executable file",
                ),
            ));
        }
        Ok(Self {
            executable: Arc::new(executable),
            active_jobs: Arc::new(Semaphore::new(ACTIVE_JOBS)),
            processing_timeout: PROCESSING_TIMEOUT,
        })
    }

    /// Edits a private owned input without ever exposing an original-byte fallback.
    ///
    /// `Passthrough` is reserved for SVG and data which neither claims nor detects as a
    /// raster image. A caller claiming an image cannot use failed detection to bypass this
    /// boundary.
    ///
    /// # Errors
    ///
    /// Returns an ordinary invalid-image rejection for malformed/unknown raster claims and
    /// typed infrastructure errors for runtime, I/O, timeout, or tool-response failures.
    pub async fn sanitize(
        &self,
        input: NamedTempFile,
        filename: &Filename,
        claimed_content_type: Option<&ContentType>,
        max_file_size: MaxFileSize,
    ) -> Result<Sanitization, ImageSanitizerError> {
        let (sender, receiver) = oneshot::channel();
        let _cancel = CancelJob(Some(sender));
        let sanitizer = self.clone();
        let claim = claims_raster(claimed_content_type, filename);
        tokio::spawn(async move {
            sanitizer
                .sanitize_owned(input, claim, max_file_size, receiver)
                .await
        })
        .await
        .map_err(ImageSanitizerError::Task)?
    }

    /// Exercises the editor with an owned, already-clean PNG before serving.
    /// Uses the same bounded job ownership and cancellation path as uploads.
    ///
    /// # Errors
    /// Returns the typed runtime/processing failure when the editor is unusable.
    pub async fn check_runtime(&self) -> Result<(), ImageSanitizerError> {
        let probe = include_bytes!("image_sanitizer_fixtures/png-sanitized.png");
        let file = NamedTempFile::new().map_err(ImageSanitizerError::Processing)?;
        std::fs::write(file.path(), probe).map_err(ImageSanitizerError::Processing)?;
        let name =
            Filename::sanitized("runtime-probe.png").or(Err(ImageSanitizerError::InvalidImage))?;
        match self
            .sanitize(file, &name, None, MaxFileSize::default())
            .await?
        {
            Sanitization::Image(image) => {
                let actual =
                    std::fs::read(image.file().path()).map_err(ImageSanitizerError::Processing)?;
                if image.content_type().as_ref() != "image/png" || actual != probe.as_slice() {
                    return Err(ImageSanitizerError::InvalidToolResponse);
                }
                Ok(())
            }
            Sanitization::Passthrough(_) => Err(ImageSanitizerError::InvalidToolResponse),
        }
    }

    async fn sanitize_owned(
        &self,
        input: NamedTempFile,
        claims_raster: bool,
        max_file_size: MaxFileSize,
        mut canceled: oneshot::Receiver<()>,
    ) -> Result<Sanitization, ImageSanitizerError> {
        Self::ensure_within_limit(input.path(), max_file_size)?;
        let _permit = tokio::select! {
            permit = self.active_jobs.acquire() => permit.map_err(|_| {
                ImageSanitizerError::Processing(std::io::Error::other("sanitizer shut down"))
            })?,
            _ = &mut canceled => return Err(ImageSanitizerError::Canceled),
        };
        let deadline = Instant::now() + self.processing_timeout;
        let detected = self.detect(input.path(), deadline, &mut canceled).await?;
        match detected.as_deref() {
            Some("image/svg+xml") => Ok(Sanitization::Passthrough(input)),
            Some(mime) if covered_mime(mime).is_some() => {
                self.edit(input, mime, max_file_size, deadline, &mut canceled)
                    .await
            }
            Some(mime) if mime.starts_with("image/") => Err(ImageSanitizerError::InvalidImage),
            _ if claims_raster => Err(ImageSanitizerError::InvalidImage),
            _ => Ok(Sanitization::Passthrough(input)),
        }
    }

    async fn edit(
        &self,
        input: NamedTempFile,
        detected_mime: &str,
        max_file_size: MaxFileSize,
        deadline: Instant,
        canceled: &mut oneshot::Receiver<()>,
    ) -> Result<Sanitization, ImageSanitizerError> {
        let spool = input.path().parent().ok_or_else(|| {
            ImageSanitizerError::Processing(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "private input has no spool directory",
            ))
        })?;
        let output = NamedTempFile::new_in(spool).map_err(ImageSanitizerError::Processing)?;
        std::fs::copy(input.path(), output.path()).map_err(ImageSanitizerError::Processing)?;
        self.run(
            &[
                "-overwrite_original",
                "-all=",
                "--icc_profile:all",
                "-tagsFromFile",
                "@",
                "-Orientation",
            ],
            output.path(),
            deadline,
            canceled,
        )
        .await?
        .successful()?;
        Self::ensure_within_limit(output.path(), max_file_size)?;

        let post_edit_mime = self.detect(output.path(), deadline, canceled).await?;
        if post_edit_mime.as_deref() != Some(detected_mime) {
            return Err(ImageSanitizerError::InvalidImage);
        }
        self.verify_clean(output.path(), deadline, canceled).await?;
        let content_type = covered_mime(detected_mime).ok_or(ImageSanitizerError::InvalidImage)?;
        // The editor replaces the private path. Reopen that path so consumers
        // cannot read the unedited inode through NamedTempFile's original handle.
        let edited_file =
            std::fs::File::open(output.path()).map_err(ImageSanitizerError::Processing)?;
        let output = NamedTempFile::from_parts(edited_file, output.into_temp_path());
        Ok(Sanitization::Image(SanitizedImage {
            file: output,
            content_type,
        }))
    }

    fn ensure_within_limit(
        path: &Path,
        max_file_size: MaxFileSize,
    ) -> Result<(), ImageSanitizerError> {
        let size = std::fs::metadata(path)
            .map_err(ImageSanitizerError::Processing)?
            .len();
        if size > max_file_size.value().unsigned_abs() {
            return Err(ImageSanitizerError::FileTooLarge);
        }
        Ok(())
    }

    async fn detect(
        &self,
        path: &Path,
        deadline: Instant,
        canceled: &mut oneshot::Receiver<()>,
    ) -> Result<Option<String>, ImageSanitizerError> {
        let response = self
            .run(&["-j", "-MIMEType"], path, deadline, canceled)
            .await?;
        let record = metadata_record(&response.bytes).map_err(|error| {
            if response.status.success() {
                error
            } else {
                ImageSanitizerError::ToolExited(response.status)
            }
        })?;
        if let Some(error) = record.get("Error") {
            // ExifTool explicitly identifies unknown/empty generic attachments.
            // Other runtime failures must never turn into successful passthrough.
            return match error.as_str() {
                Some("Unknown file type" | "File is empty") => Ok(None),
                _ if !response.status.success() => {
                    Err(ImageSanitizerError::ToolExited(response.status))
                }
                _ => Err(ImageSanitizerError::InvalidToolResponse),
            };
        }
        if !response.status.success() {
            return Err(ImageSanitizerError::ToolExited(response.status));
        }
        let mime = record
            .get("MIMEType")
            .and_then(Value::as_str)
            .ok_or(ImageSanitizerError::InvalidToolResponse)?;
        mime.parse::<ContentType>()
            .map_err(|_| ImageSanitizerError::InvalidToolResponse)?;
        // APNG uses the PNG container and the shared PNG serving type.
        Ok(Some(
            if mime == "image/apng" {
                "image/png"
            } else {
                mime
            }
            .to_owned(),
        ))
    }

    async fn verify_clean(
        &self,
        path: &Path,
        deadline: Instant,
        canceled: &mut oneshot::Receiver<()>,
    ) -> Result<(), ImageSanitizerError> {
        let mut arguments = vec!["-j", "-n", "-G1", "-Orientation"];
        arguments.extend_from_slice(PRIVATE_TAGS);
        let response = self
            .run(&arguments, path, deadline, canceled)
            .await?
            .successful()?;
        let metadata = metadata_record(&response)?;
        if metadata.keys().any(|key| {
            !matches!(
                key.as_str(),
                "SourceFile" | "IFD0:Orientation" | "XMP-tiff:Orientation" | "XMP-x:XMPToolkit"
            )
        }) {
            return Err(ImageSanitizerError::InvalidImage);
        }
        Ok(())
    }

    async fn run(
        &self,
        arguments: &[&str],
        path: &Path,
        deadline: Instant,
        canceled: &mut oneshot::Receiver<()>,
    ) -> Result<ToolOutput, ImageSanitizerError> {
        let mut command = Command::new(self.executable.as_ref());
        command
            .args(["-config", ""])
            .args(arguments)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(ImageSanitizerError::Processing)?;
        let Some(stdout) = child.stdout.take() else {
            unreachable!("a successfully spawned command with piped stdout owns that pipe");
        };
        let work = async { tokio::try_join!(child.wait(), read_tool_response(stdout)) };
        let completed = tokio::select! {
            result = timeout_at(deadline, work) => Some(result),
            _ = &mut *canceled => None,
        };
        let failure = match completed {
            Some(Ok(Ok((status, bytes)))) => return Ok(ToolOutput { status, bytes }),
            Some(Ok(Err(error))) => ImageSanitizerError::Processing(error),
            Some(Err(_)) => ImageSanitizerError::TimedOut,
            None => ImageSanitizerError::Canceled,
        };
        child
            .start_kill()
            .map_err(ImageSanitizerError::Processing)?;
        child
            .wait()
            .await
            .map_err(ImageSanitizerError::Processing)?;
        Err(failure)
    }
}

struct ToolOutput {
    status: std::process::ExitStatus,
    bytes: Vec<u8>,
}

impl ToolOutput {
    fn successful(self) -> Result<Vec<u8>, ImageSanitizerError> {
        if self.status.success() {
            Ok(self.bytes)
        } else {
            Err(ImageSanitizerError::ToolExited(self.status))
        }
    }
}

fn metadata_record(bytes: &[u8]) -> Result<serde_json::Map<String, Value>, ImageSanitizerError> {
    let records: Vec<serde_json::Map<String, Value>> =
        serde_json::from_slice(bytes).map_err(ImageSanitizerError::ToolResponse)?;
    let [record]: [serde_json::Map<String, Value>; 1] = records
        .try_into()
        .map_err(|_| ImageSanitizerError::InvalidToolResponse)?;
    if !matches!(record.get("SourceFile"), Some(Value::String(_))) {
        return Err(ImageSanitizerError::InvalidToolResponse);
    }
    Ok(record)
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

    fn editor_runtime() -> PathBuf {
        std::env::var_os("JAUNDER_EXIFTOOL").map_or_else(|| test_tool("exiftool"), PathBuf::from)
    }

    fn test_filename() -> Filename {
        Filename::sanitized("fixture.bin").expect("fixture name")
    }

    fn max_file_size() -> MaxFileSize {
        "1048576".parse().expect("valid maximum")
    }

    fn image_variant(result: Sanitization) -> Option<SanitizedImage> {
        match result {
            Sanitization::Image(image) => Some(image),
            Sanitization::Passthrough(_) => None,
        }
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
    async fn passthrough_is_not_an_image_variant() {
        let tool = script(
            "#!/bin/sh\nprintf '[{\"SourceFile\":\"private\",\"MIMEType\":\"image/svg+xml\"}]'\n",
        );
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = NamedTempFile::new().expect("private input");
        assert!(
            image_variant(
                sanitizer
                    .sanitize(input, &test_filename(), None, max_file_size())
                    .await
                    .expect("SVG passthrough")
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn cancellation_while_waiting_for_capacity_cleans_input_without_starting_tool() {
        let tool = script("#!/bin/sh\nexit 99\n");
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let held = sanitizer
            .active_jobs
            .acquire_many(u32::try_from(ACTIVE_JOBS).expect("bounded capacity fits u32"))
            .await
            .expect("hold all capacity");
        let input = NamedTempFile::new().expect("private input");
        let path = input.path().to_owned();
        let (sender, receiver) = oneshot::channel();
        sender.send(()).expect("cancel owned job");
        assert!(matches!(
            sanitizer
                .sanitize_owned(input, false, max_file_size(), receiver)
                .await,
            Err(ImageSanitizerError::Canceled)
        ));
        assert!(!path.exists());
        drop(held);
        assert_eq!(sanitizer.active_jobs.available_permits(), ACTIVE_JOBS);
    }

    #[tokio::test]
    async fn an_invalid_private_spool_path_is_a_typed_error_before_editor_execution() {
        let tool = script("#!/bin/sh\nexit 99\n");
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let real_input = NamedTempFile::new().expect("owned file handle");
        // A synthetic path exercises the fallible from_parts boundary. It must
        // never be opened, written, or removed: cleanup is disabled before use.
        let mut path = tempfile::TempPath::try_from_path("/").expect("synthetic parentless path");
        path.disable_cleanup(true);
        let synthetic = NamedTempFile::from_parts(
            real_input.as_file().try_clone().expect("clone handle"),
            path,
        );
        let (_sender, mut canceled) = oneshot::channel();
        assert!(matches!(
            sanitizer.edit(synthetic, "image/png", max_file_size(), Instant::now() + PROCESSING_TIMEOUT, &mut canceled).await,
            Err(ImageSanitizerError::Processing(error)) if error.kind() == std::io::ErrorKind::InvalidInput
        ));
    }

    fn script(contents: &str) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("script directory");
        let path = directory.path().join("tool.sh");
        std::fs::write(&path, contents).expect("write script");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).expect("make script executable");
        directory
    }

    fn test_tool(name: &str) -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").expect("tool PATH"))
            .map(|directory| directory.join(name))
            .find(|path| path.is_file())
            .expect("pinned test executable")
    }

    fn blocked_tool() -> tempfile::TempDir {
        let sleep = test_tool("sleep");
        let directory = tempfile::tempdir().expect("tool workspace");
        let path = directory.path().join("tool.sh");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nexec '{}' 30\n",
                directory.path().join("pid").display(),
                sleep.display(),
            ),
        )
        .expect("write controlled tool");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("executable tool");
        directory
    }

    async fn tool_pid(directory: &Path) -> rustix::process::Pid {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match tokio::fs::read_to_string(directory.join("pid")).await {
                    Ok(value) if !value.is_empty() => {
                        return rustix::process::Pid::from_raw(value.parse().expect("tool PID"))
                            .expect("positive PID");
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("cannot observe tool PID: {error}"),
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("tool must start")
    }

    #[tokio::test]
    async fn pid_observer_waits_for_complete_bytes_and_reports_unreadable_markers() {
        for unreadable in [false, true] {
            let directory = tempfile::tempdir().expect("PID marker workspace");
            let marker = directory.path().join("pid");
            if unreadable {
                std::fs::create_dir(&marker).expect("unreadable directory marker");
            } else {
                std::fs::write(&marker, "").expect("pending empty marker");
            }
            let path = directory.path().to_owned();
            let observation = tokio::spawn(async move { tool_pid(&path).await });
            if unreadable {
                assert!(
                    observation
                        .await
                        .expect_err("unreadable marker must fail the fixture")
                        .is_panic()
                );
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                std::fs::write(&marker, std::process::id().to_string()).expect("complete marker");
                assert_eq!(
                    observation.await.expect("observed PID"),
                    rustix::process::getpid()
                );
            }
        }
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
    async fn processing_deadline_is_shared_across_editor_commands() {
        let tool = script(&format!(
            "#!{}\nimport time\ntime.sleep(0.4)\nprint('[{{\"SourceFile\":\"private\",\"MIMEType\":\"image/png\"}}]')\n",
            test_tool("python3").display(),
        ));
        let mut sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        sanitizer.processing_timeout = Duration::from_millis(700);
        let input = tempfile::NamedTempFile::new().expect("input");
        let path = input.path().to_owned();
        let result = sanitizer
            .sanitize(input, &test_filename(), None, max_file_size())
            .await;
        assert!(
            matches!(result, Err(ImageSanitizerError::TimedOut)),
            "{result:?}"
        );
        assert!(!path.exists());
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
    async fn oversized_tool_response_is_reaped_without_releasing_private_bytes() {
        let tool = script(&format!(
            "#!{}\nimport sys\nsys.stdout.write('x' * {})\nsys.stdout.flush()\n",
            test_tool("python3").display(),
            MAX_TOOL_RESPONSE + 1,
        ));
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = NamedTempFile::new_in(tool.path()).expect("private input");
        let path = input.path().to_owned();
        let result = sanitizer
            .sanitize(input, &test_filename(), None, max_file_size())
            .await;
        assert!(
            matches!(result, Err(ImageSanitizerError::Processing(_))),
            "{result:?}"
        );
        assert!(!path.exists());
        assert_eq!(sanitizer.active_jobs.available_permits(), ACTIVE_JOBS);
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
            Err(ImageSanitizerError::Processing(_))
        ));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn detection_protocol_and_exit_failures_cannot_become_passthrough() {
        for (response, exit, expected_exit) in [
            (
                r#"[{"SourceFile":"private","Error":"read failed"}]"#,
                17,
                true,
            ),
            (
                r#"[{"SourceFile":"private","Error":"read failed"}]"#,
                0,
                false,
            ),
            (
                r#"[{"SourceFile":"private","MIMEType":"image/png"}]"#,
                17,
                true,
            ),
            (r#"[{"MIMEType":"image/png"}]"#, 0, false),
            (r#"[{"SourceFile":false,"MIMEType":"image/png"}]"#, 0, false),
        ] {
            let tool = script(&format!(
                "#!/bin/sh\nprintf '%s' '{response}'\nexit {exit}\n"
            ));
            let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
            let input = NamedTempFile::new().expect("private input");
            let path = input.path().to_owned();
            let result = sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await;
            if expected_exit {
                assert!(
                    matches!(result, Err(ImageSanitizerError::ToolExited(_))),
                    "{result:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(ImageSanitizerError::InvalidToolResponse)),
                    "{result:?}"
                );
            }
            assert!(!path.exists());
        }
    }

    #[tokio::test]
    async fn startup_probe_rejects_mime_drift_and_non_image_detection() {
        for mime in ["image/jpeg", "image/svg+xml"] {
            let tool = script(&format!(
                "#!/bin/sh\ncase \" $* \" in *' -G1 '*) printf '[{{\"SourceFile\":\"private\"}}]';; *) printf '[{{\"SourceFile\":\"private\",\"MIMEType\":\"{mime}\"}}]';; esac\n"
            ));
            let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
            assert!(matches!(
                sanitizer.check_runtime().await,
                Err(ImageSanitizerError::InvalidToolResponse)
            ));
        }
    }

    #[tokio::test]
    async fn changed_mime_after_edit_is_rejected_and_cleans_spool() {
        let tool = script(&format!(
            "#!{}\nimport json,sys\nfrom pathlib import Path\np=Path(sys.argv[-1])\nif '-overwrite_original' in sys.argv:\n p.write_bytes(b'edited')\nelse:\n print(json.dumps([{{'SourceFile':str(p),'MIMEType':'image/jpeg' if p.read_bytes()==b'edited' else 'image/png'}}]))\n",
            test_tool("python3").display(),
        ));
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let spool = tempfile::tempdir().expect("spool");
        let input = NamedTempFile::new_in(spool.path()).expect("input");
        std::fs::write(input.path(), b"original").expect("original bytes");
        assert!(matches!(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await,
            Err(ImageSanitizerError::InvalidImage)
        ));
        assert!(
            std::fs::read_dir(spool.path())
                .expect("spool")
                .next()
                .is_none()
        );
        assert_eq!(sanitizer.active_jobs.available_permits(), ACTIVE_JOBS);
    }

    #[tokio::test]
    async fn invalid_tool_json_is_an_infrastructure_failure() {
        let tool = script("#!/bin/sh\nprintf 'not json'\n");
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        let result = sanitizer
            .sanitize(input, &test_filename(), None, max_file_size())
            .await;
        assert!(
            matches!(result, Err(ImageSanitizerError::ToolResponse(_))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn non_image_detection_cannot_override_a_raster_claim() {
        let tool = script(
            "#!/bin/sh\nprintf '[{\"SourceFile\":\"private\",\"MIMEType\":\"application/pdf\"}]'\n",
        );
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        let claim = "IMAGE/PNG; charset=binary".parse().expect("MIME");
        assert!(matches!(
            sanitizer
                .sanitize(input, &test_filename(), Some(&claim), max_file_size())
                .await,
            Err(ImageSanitizerError::InvalidImage)
        ));
    }

    #[tokio::test]
    async fn startup_probe_rejects_an_executable_but_unusable_runtime() {
        let tool = script("#!/bin/sh\nexit 64\n");
        let sanitizer =
            ImageSanitizer::new(tool.path().join("tool.sh")).expect("executable exists");
        assert!(matches!(
            sanitizer.check_runtime().await,
            Err(ImageSanitizerError::ToolExited(_))
        ));
    }

    #[tokio::test]
    async fn malformed_raster_filename_cannot_bypass_detection() {
        let tool = script(
            "#!/bin/sh\nprintf '[{\"SourceFile\":\"private\",\"Error\":\"Unknown file type\"}]'\n",
        );
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        let name = Filename::sanitized("photo.JPG").expect("name");
        assert!(matches!(
            sanitizer
                .sanitize(input, &name, None, max_file_size())
                .await,
            Err(ImageSanitizerError::InvalidImage)
        ));
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

    #[tokio::test]
    async fn empty_json_is_not_a_successful_non_image_detection() {
        let tool = script("#!/bin/sh\nprintf '[]'\n");
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        assert!(
            sanitizer
                .sanitize(input, &test_filename(), None, max_file_size())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn image_claim_with_undetected_bytes_is_rejected() {
        let tool = script(
            "#!/bin/sh\nprintf '[{\"SourceFile\":\"private\",\"Error\":\"Unknown file type\"}]'\n",
        );
        let sanitizer = ImageSanitizer::new(tool.path().join("tool.sh")).expect("runtime");
        let input = tempfile::NamedTempFile::new().expect("input");
        let claimed = "image/png".parse().expect("valid mime");
        let result = sanitizer
            .sanitize(input, &test_filename(), Some(&claimed), max_file_size())
            .await;
        assert!(
            matches!(result, Err(ImageSanitizerError::InvalidImage)),
            "{result:?}"
        );
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
        let (_sender, mut canceled) = oneshot::channel();
        let before = sanitizer
            .run(
                &arguments,
                input.path(),
                Instant::now() + PROCESSING_TIMEOUT,
                &mut canceled,
            )
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
        let after = sanitizer
            .run(
                &arguments,
                edited.file().path(),
                Instant::now() + PROCESSING_TIMEOUT,
                &mut canceled,
            )
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

    async fn observe_fixture(sanitizer: &ImageSanitizer, path: &Path) -> FixtureObservation {
        let (_sender, mut canceled) = oneshot::channel();
        let response = sanitizer
            .run(
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
                Instant::now() + PROCESSING_TIMEOUT,
                &mut canceled,
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
            .arg(sanitizer.executable.as_path())
            .output()
            .expect("conventional decoder");
        assert!(
            decoded.status.success(),
            "owned fixture decoding: {}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        FixtureObservation {
            private_fields,
            rendering_metadata: metadata,
            decoded: serde_json::from_slice(&decoded.stdout).expect("decoder observation"),
        }
    }

    const OWNED_FORMAT_CASES: &[(&[u8], &str)] = &[
        (
            include_bytes!("image_sanitizer_fixtures/jpeg-original.jpg"),
            "image/jpeg",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/png-original.png"),
            "image/png",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/apng-first-original.png"),
            "image/png",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/apng-default-original.png"),
            "image/png",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/gif-static-original.gif"),
            "image/gif",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/gif-animation-original.gif"),
            "image/gif",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/webp-static-original.webp"),
            "image/webp",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/webp-animation-original.webp"),
            "image/webp",
        ),
        (
            include_bytes!("image_sanitizer_fixtures/heic-original.heic"),
            "image/heic",
        ),
    ];
    #[tokio::test]
    async fn actual_editor_covers_owned_formats_with_stable_output() {
        let sanitizer = ImageSanitizer::new(editor_runtime()).expect("runtime");
        let mut observed_transparency = false;
        for (case_index, (bytes, expected_mime)) in OWNED_FORMAT_CASES.iter().enumerate() {
            let input = tempfile::NamedTempFile::new().expect("input");
            std::fs::write(input.path(), bytes).expect("fixture");
            let before = observe_fixture(&sanitizer, input.path()).await;
            if let Some(frames) = before.decoded["frames"].as_array() {
                if matches!(case_index, 2 | 3 | 5 | 7) {
                    assert!(
                        frames.len() > 1,
                        "animated fixture {case_index} must expose its frames"
                    );
                }
                observed_transparency |= frames.iter().any(|frame| {
                    frame["alpha_extrema"][0]
                        .as_u64()
                        .is_some_and(|alpha| alpha < 255)
                });
            }
            assert!(
                before.private_fields > 0,
                "case {case_index} must have planted private fields"
            );
            let first = image_variant(
                sanitizer
                    .sanitize(input, &test_filename(), None, max_file_size())
                    .await
                    .expect("owned raster sanitization"),
            )
            .expect("owned raster must be edited");
            assert_eq!(first.content_type().as_ref(), *expected_mime);
            let after = observe_fixture(&sanitizer, first.file().path()).await;
            assert_eq!(
                after.private_fields, 0,
                "case {case_index} removes planted fields"
            );
            assert_eq!(
                before.rendering_metadata, after.rendering_metadata,
                "case {case_index} rendering tags"
            );
            assert_eq!(
                before.decoded, after.decoded,
                "case {case_index} pixels, frames, timing, transparency and intact ICC"
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
        assert!(
            observed_transparency,
            "owned corpus must exercise non-opaque alpha"
        );
    }

    #[tokio::test]
    async fn expanded_edited_output_exceeds_current_limit_and_cleans_spool() {
        let python = test_tool("python3");
        let tool = script(&format!(
            "#!{}\nimport json,sys\nfrom pathlib import Path\np=Path(sys.argv[-1])\nif '-overwrite_original' in sys.argv:\n p.write_bytes(b'x'*64)\nelse:\n print(json.dumps([{{'SourceFile':str(p),'MIMEType':'image/png'}}]))\n",
            python.display()
        ));
        let sanitizer =
            ImageSanitizer::new(tool.path().join("tool.sh")).expect("controlled editor");
        let spool = tempfile::tempdir().expect("private spool");
        let input = NamedTempFile::new_in(spool.path()).expect("input");
        std::fs::write(input.path(), b"input").expect("small received bytes");
        let result = sanitizer
            .sanitize(input, &test_filename(), None, "8".parse().unwrap())
            .await;
        assert!(matches!(result, Err(ImageSanitizerError::FileTooLarge)));
        assert!(std::fs::read_dir(spool.path()).unwrap().next().is_none());
        assert_eq!(sanitizer.active_jobs.available_permits(), ACTIVE_JOBS);
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
