//! Private-spool operations for the same-executable metadata worker.
//!
//! Native CPU work belongs in the killable child, not a timed-out blocking
//! task in the server. The request supervisor retains spool ownership until
//! that child has been reaped; this module only borrows its private path.

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use thiserror::Error;

use super::native::{self, EditedBytes, NativeImageError};

/// Completion details; image bytes remain in the request-owned private spool.
#[derive(Debug, PartialEq, Eq)]
pub enum Completion {
    Image { mime: &'static str, size: u64 },
    Passthrough { size: u64 },
}

/// Concrete sources remain available inside the worker before safe projection.
#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("private image spool operation failed")]
    Io(#[from] std::io::Error),
    #[error("native image processing failed")]
    Native(#[from] NativeImageError),
    #[error("image worker requires a private regular spool")]
    InvalidSpool,
    #[error("invalid image worker size limit")]
    InvalidLimit,
}

/// Processes only a request-owned private spool in the isolated child.
///
/// # Errors
/// Returns the underlying private-file or native editing failure.
pub fn process(
    path: &Path,
    claims_raster: bool,
    max_bytes: usize,
) -> Result<Completion, WorkerError> {
    let read_limit = (max_bytes as u64)
        .checked_add(1)
        .ok_or(WorkerError::InvalidLimit)?;
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let mut file = std::fs::File::from(descriptor);
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o7777 != 0o600 {
        return Err(WorkerError::InvalidSpool);
    }
    if metadata.len() > max_bytes as u64 {
        return Err(NativeImageError::FileTooLarge.into());
    }
    let mut input = Vec::new();
    (&mut file).take(read_limit).read_to_end(&mut input)?;
    match native::sanitize(&input, claims_raster, max_bytes)? {
        EditedBytes::Passthrough(bytes) => Ok(Completion::Passthrough {
            size: bytes.len() as u64,
        }),
        EditedBytes::Image { bytes, mime } => {
            // Keep the validated private inode: the supervisor owns this spool,
            // and public placement/permissions belong to MediaManager afterward.
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&bytes)?;
            file.set_len(bytes.len() as u64)?;
            Ok(Completion::Image {
                mime,
                size: bytes.len() as u64,
            })
        }
    }
}

/// Runs the isolated worker with a dedicated response descriptor.
///
/// Redirect before calling any native library: their logging, warnings and
/// panic hook may contain metadata. Only our safe protocol reaches the parent.
///
/// # Errors
/// Returns concrete channel setup, encoding or write failures.
pub fn execute(request: &super::worker_request::WorkerRequest) -> Result<(), WorkerExecutionError> {
    let descriptor = rustix::io::dup(std::io::stdout()).map_err(std::io::Error::from)?;
    let mut response = std::fs::File::from(descriptor);
    let null = std::fs::OpenOptions::new().write(true).open("/dev/null")?;
    rustix::stdio::dup2_stdout(&null).map_err(std::io::Error::from)?;
    rustix::stdio::dup2_stderr(&null).map_err(std::io::Error::from)?;
    let reply = match process(&request.path, request.claims_raster, request.max_bytes) {
        Ok(completion) => super::worker_protocol::Reply::from(completion),
        Err(error) => super::worker_protocol::Reply::Failed((&error).into()),
    };
    response.write_all(&super::worker_protocol::encode(&reply)?)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum WorkerExecutionError {
    #[error("worker response channel failed")]
    Io(#[from] std::io::Error),
    #[error("worker response encoding failed")]
    Protocol(#[from] super::worker_protocol::ProtocolError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    use std::os::unix::fs::MetadataExt;

    fn spool(bytes: &[u8]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().expect("owned private spool");
        std::fs::write(file.path(), bytes).expect("owned input");
        file
    }

    #[test]
    fn passthrough_and_rejection_do_not_modify_private_input() {
        let original = b"ordinary non-raster attachment";
        let input = spool(original);
        assert_eq!(
            process(input.path(), false, 1024).expect("non-raster passthrough"),
            Completion::Passthrough {
                size: original.len() as u64
            }
        );
        assert!(matches!(
            process(input.path(), true, 1024),
            Err(WorkerError::Native(NativeImageError::InvalidImage(_)))
        ));
        assert_eq!(
            std::fs::read(input.path()).expect("retained private input"),
            original
        );
    }

    #[test]
    fn size_and_permission_rejections_leave_original_spool_untouched() {
        let original = include_bytes!("../image_sanitizer_fixtures/png-original.png");
        let input = spool(original);
        assert!(matches!(
            process(input.path(), false, original.len() - 1),
            Err(WorkerError::Native(NativeImageError::FileTooLarge))
        ));
        std::fs::set_permissions(input.path(), std::fs::Permissions::from_mode(0o644))
            .expect("non-private permission control");
        assert!(matches!(
            process(input.path(), false, 16_777_216),
            Err(WorkerError::InvalidSpool)
        ));
        assert_eq!(
            std::fs::read(input.path()).expect("untouched input"),
            original
        );
        assert_eq!(
            input
                .as_file()
                .metadata()
                .expect("input permissions")
                .permissions()
                .mode()
                & 0o777,
            0o644,
            "the worker must not chmod existing files"
        );
    }

    #[test]
    fn missing_and_symlink_spools_retain_concrete_io_failures() {
        use std::error::Error;
        let directory = tempfile::tempdir().expect("private root");
        let missing = directory.path().join("missing");
        let error = process(&missing, false, 1024).expect_err("missing spool");
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .expect("concrete I/O source")
                .kind(),
            std::io::ErrorKind::NotFound
        );
        let input = spool(b"private attachment");
        std::os::unix::fs::symlink(input.path(), &missing).expect("symlink control");
        let error = process(&missing, false, 1024).expect_err("symlink refused");
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .expect("concrete symlink failure")
                .raw_os_error(),
            Some(rustix::io::Errno::LOOP.raw_os_error())
        );
        assert_eq!(
            std::fs::read(input.path()).expect("untouched target"),
            b"private attachment"
        );
    }

    #[test]
    fn private_spool_contains_native_sanitized_bytes_not_originals() {
        let input = tempfile::NamedTempFile::new().expect("owned private spool");
        let original = include_bytes!("../image_sanitizer_fixtures/png-original.png");
        std::fs::write(input.path(), original).expect("owned PNG");
        let inode = input.as_file().metadata().expect("input inode").ino();
        let result = process(input.path(), false, 16_777_216).expect("native worker");
        let edited = std::fs::read(input.path()).expect("edited private bytes");
        assert_eq!(
            result,
            Completion::Image {
                mime: "image/png",
                size: edited.len() as u64
            }
        );
        assert_ne!(edited, original);
        let metadata = input.as_file().metadata().expect("edited inode");
        assert_eq!(metadata.ino(), inode);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(metadata.len(), edited.len() as u64);
        assert_eq!(
            Sha256::digest(&edited)
                .iter()
                .fold(String::new(), |mut digest, byte| {
                    write!(digest, "{byte:02x}").expect("formatting a digest into a String");
                    digest
                }),
            "fd0e88136fda66661f07ab7f269c9f6a28726f434cc91721ee7fd608fd481620"
        );
    }
}
