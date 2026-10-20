//! Bounded worker status with typed, privacy-safe causes. Raw library text may
//! contain metadata, so only its variant and non-personal numeric details cross.
use std::io;

use exiftool_rs::Error as NativeError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PreparationError {
    #[error("media preparation is busy")]
    Busy,
    #[error("media preparation is stopping")]
    Stopping,
    #[error("image processing deadline exceeded")]
    Deadline,
    #[error("image-labelled input is not a recognized image")]
    InvalidImage,
    #[error("image metadata verification failed")]
    Verification,
    #[error("invalid image worker response")]
    Protocol,
    #[error("image worker exited: {0}")]
    WorkerExit(std::process::ExitStatus),
    #[error("image processing I/O failed")]
    Io(#[from] io::Error),
    #[error("native metadata processing failed")]
    Native(#[source] NativeError),
    #[error("image job owner failed")]
    Owner(#[from] tokio::task::JoinError),
    #[error("worker exit acknowledgement failed after an operation failure")]
    Reap {
        #[source]
        primary: Box<PreparationError>,
        reap: io::Error,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Status {
    pub version: u8,
    pub result: Result<Option<String>, Failure>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Failure {
    InvalidImage,
    Verification,
    Io { kind: String, os: Option<i32> },
    NativeIo { kind: String, os: Option<i32> },
    Unsupported,
    InvalidData,
    TagNotFound,
    InvalidTiffHeader,
    InvalidExif,
    InvalidIptc,
    InvalidXmp,
    TruncatedData { expected: usize, actual: usize },
    Conversion,
    Protocol,
}

/// Temporary-file errors may wrap private paths. Retain the underlying I/O
/// kind and OS code without rendering that path-bearing wrapper at a boundary.
pub(super) fn private_io(error: &io::Error) -> io::Error {
    let mut source: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(code) = source
            .downcast_ref::<io::Error>()
            .and_then(io::Error::raw_os_error)
        {
            return io::Error::from_raw_os_error(code);
        }
        match source.source() {
            Some(next) => source = next,
            None => break,
        }
    }
    io::Error::new(error.kind(), "private image filesystem operation failed")
}

fn io_failure(error: &io::Error, native: bool) -> Failure {
    let error = private_io(error);
    let kind = format!("{:?}", error.kind());
    let os = error.raw_os_error();
    if native {
        Failure::NativeIo { kind, os }
    } else {
        Failure::Io { kind, os }
    }
}

fn restore_io(kind: &str, os: Option<i32>) -> Result<io::Error, PreparationError> {
    if let Some(code) = os {
        return Ok(io::Error::from_raw_os_error(code));
    }
    let kind = match kind {
        "NotFound" => io::ErrorKind::NotFound,
        "PermissionDenied" => io::ErrorKind::PermissionDenied,
        "AlreadyExists" => io::ErrorKind::AlreadyExists,
        "InvalidInput" => io::ErrorKind::InvalidInput,
        "InvalidData" => io::ErrorKind::InvalidData,
        "UnexpectedEof" => io::ErrorKind::UnexpectedEof,
        "WriteZero" => io::ErrorKind::WriteZero,
        "Interrupted" => io::ErrorKind::Interrupted,
        "OutOfMemory" => io::ErrorKind::OutOfMemory,
        "StorageFull" => io::ErrorKind::StorageFull,
        "TimedOut" => io::ErrorKind::TimedOut,
        "BrokenPipe" => io::ErrorKind::BrokenPipe,
        "ConnectionRefused" => io::ErrorKind::ConnectionRefused,
        "ConnectionReset" => io::ErrorKind::ConnectionReset,
        "ConnectionAborted" => io::ErrorKind::ConnectionAborted,
        "NotConnected" => io::ErrorKind::NotConnected,
        "AddrInUse" => io::ErrorKind::AddrInUse,
        "AddrNotAvailable" => io::ErrorKind::AddrNotAvailable,
        "WouldBlock" => io::ErrorKind::WouldBlock,
        "NotADirectory" => io::ErrorKind::NotADirectory,
        "IsADirectory" => io::ErrorKind::IsADirectory,
        "DirectoryNotEmpty" => io::ErrorKind::DirectoryNotEmpty,
        "ReadOnlyFilesystem" => io::ErrorKind::ReadOnlyFilesystem,
        "Unsupported" => io::ErrorKind::Unsupported,
        "Other" => io::ErrorKind::Other,
        _ => return Err(PreparationError::Protocol),
    };
    Ok(io::Error::new(kind, "private image I/O failure"))
}

impl From<PreparationError> for Failure {
    fn from(error: PreparationError) -> Self {
        match error {
            PreparationError::InvalidImage => Self::InvalidImage,
            PreparationError::Verification => Self::Verification,
            PreparationError::Io(error) => io_failure(&error, false),
            PreparationError::Native(error) => match error {
                NativeError::Io(error) => io_failure(&error, true),
                NativeError::UnsupportedFileType(_) => Self::Unsupported,
                NativeError::InvalidData(_) => Self::InvalidData,
                NativeError::TagNotFound(_) => Self::TagNotFound,
                NativeError::InvalidTiffHeader => Self::InvalidTiffHeader,
                NativeError::InvalidExif(_) => Self::InvalidExif,
                NativeError::InvalidIptc(_) => Self::InvalidIptc,
                NativeError::InvalidXmp(_) => Self::InvalidXmp,
                NativeError::TruncatedData { expected, actual } => {
                    Self::TruncatedData { expected, actual }
                }
                NativeError::ConversionError(_) => Self::Conversion,
                _ => Self::Protocol,
            },
            _ => Self::Protocol,
        }
    }
}

impl From<Failure> for PreparationError {
    fn from(error: Failure) -> Self {
        const DETAIL: &str = "private image metadata processing";
        let native = match error {
            Failure::InvalidImage => return Self::InvalidImage,
            Failure::Verification => return Self::Verification,
            Failure::Protocol => return Self::Protocol,
            Failure::Io { kind, os } => {
                return match restore_io(&kind, os) {
                    Ok(error) => Self::Io(error),
                    Err(error) => error,
                };
            }
            Failure::NativeIo { kind, os } => match restore_io(&kind, os) {
                Ok(error) => NativeError::Io(error),
                Err(error) => return error,
            },
            Failure::Unsupported => NativeError::UnsupportedFileType(DETAIL.into()),
            Failure::InvalidData => NativeError::InvalidData(DETAIL.into()),
            Failure::TagNotFound => NativeError::TagNotFound(DETAIL.into()),
            Failure::InvalidTiffHeader => NativeError::InvalidTiffHeader,
            Failure::InvalidExif => NativeError::InvalidExif(DETAIL.into()),
            Failure::InvalidIptc => NativeError::InvalidIptc(DETAIL.into()),
            Failure::InvalidXmp => NativeError::InvalidXmp(DETAIL.into()),
            Failure::TruncatedData { expected, actual } => {
                NativeError::TruncatedData { expected, actual }
            }
            Failure::Conversion => NativeError::ConversionError(DETAIL.into()),
        };
        Self::Native(native)
    }
}

#[cfg(test)]
mod tests {
    use super::{Failure, io_failure};
    use std::io;

    #[test]
    fn io_failure_borrows_and_redacts_private_error_detail() {
        let error = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private spool /media/tmp/upload-secret",
        );
        let failure = io_failure(&error, false);

        assert_eq!(error.to_string(), "private spool /media/tmp/upload-secret");
        match failure {
            Failure::Io { kind, os } => {
                assert_eq!(kind, "PermissionDenied");
                assert_eq!(os, None);
            }
            _ => panic!("ordinary I/O must retain its worker failure kind"),
        }
    }
}
