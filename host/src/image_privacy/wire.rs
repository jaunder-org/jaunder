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
                _ => unreachable!("review new native variants against the pinned worker protocol"),
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
    use super::{Failure, PreparationError, io_failure, private_io};
    use exiftool_rs::Error as NativeError;
    use serde_json::{Value as JsonValue, json};
    use std::io;

    fn wire_value(failure: Failure) -> JsonValue {
        serde_json::to_value(failure).expect("wire failure serializes")
    }

    #[test]
    fn worker_errors_map_to_closed_pii_free_failures() {
        let private = "private /media/tmp/upload-secret";
        let cases = [
            (PreparationError::InvalidImage, json!("InvalidImage")),
            (PreparationError::Verification, json!("Verification")),
            (
                PreparationError::Io(io::Error::new(io::ErrorKind::PermissionDenied, private)),
                json!({"Io": {"kind": "PermissionDenied", "os": null}}),
            ),
            (
                PreparationError::Native(NativeError::Io(io::Error::new(
                    io::ErrorKind::NotFound,
                    private,
                ))),
                json!({"NativeIo": {"kind": "NotFound", "os": null}}),
            ),
            (
                PreparationError::Native(NativeError::UnsupportedFileType(private.into())),
                json!("Unsupported"),
            ),
            (
                PreparationError::Native(NativeError::InvalidData(private.into())),
                json!("InvalidData"),
            ),
            (
                PreparationError::Native(NativeError::TagNotFound(private.into())),
                json!("TagNotFound"),
            ),
            (
                PreparationError::Native(NativeError::InvalidTiffHeader),
                json!("InvalidTiffHeader"),
            ),
            (
                PreparationError::Native(NativeError::InvalidExif(private.into())),
                json!("InvalidExif"),
            ),
            (
                PreparationError::Native(NativeError::InvalidIptc(private.into())),
                json!("InvalidIptc"),
            ),
            (
                PreparationError::Native(NativeError::InvalidXmp(private.into())),
                json!("InvalidXmp"),
            ),
            (
                PreparationError::Native(NativeError::TruncatedData {
                    expected: 20,
                    actual: 12,
                }),
                json!({"TruncatedData": {"expected": 20, "actual": 12}}),
            ),
            (
                PreparationError::Native(NativeError::ConversionError(private.into())),
                json!("Conversion"),
            ),
            (PreparationError::Busy, json!("Protocol")),
            (PreparationError::Stopping, json!("Protocol")),
            (PreparationError::Deadline, json!("Protocol")),
            (PreparationError::Protocol, json!("Protocol")),
        ];

        for (error, expected) in cases {
            let actual = wire_value(Failure::from(error));
            assert_eq!(actual, expected);
            assert!(!actual.to_string().contains(private));
        }
    }

    #[test]
    fn private_io_walks_wrapped_sources_and_redacts_path_text() {
        #[derive(Debug)]
        struct Wrapped(io::Error);
        impl std::fmt::Display for Wrapped {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
        impl std::error::Error for Wrapped {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        assert_eq!(
            Wrapped(io::Error::other("wrapped detail")).to_string(),
            "wrapped detail"
        );
        let nested = io::Error::other(Wrapped(io::Error::from_raw_os_error(13)));
        let redacted = private_io(&nested);
        assert_eq!(redacted.raw_os_error(), Some(13));
        assert!(!redacted.to_string().contains("upload-secret"));

        let no_code = private_io(&io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private upload-secret path",
        ));
        assert_eq!(no_code.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            no_code.to_string(),
            "private image filesystem operation failed"
        );
    }

    #[test]
    fn wire_io_kinds_restore_without_os_codes_and_unknown_kinds_fail_closed() {
        let kinds = [
            ("NotFound", io::ErrorKind::NotFound),
            ("PermissionDenied", io::ErrorKind::PermissionDenied),
            ("AlreadyExists", io::ErrorKind::AlreadyExists),
            ("InvalidInput", io::ErrorKind::InvalidInput),
            ("InvalidData", io::ErrorKind::InvalidData),
            ("UnexpectedEof", io::ErrorKind::UnexpectedEof),
            ("WriteZero", io::ErrorKind::WriteZero),
            ("Interrupted", io::ErrorKind::Interrupted),
            ("OutOfMemory", io::ErrorKind::OutOfMemory),
            ("StorageFull", io::ErrorKind::StorageFull),
            ("TimedOut", io::ErrorKind::TimedOut),
            ("BrokenPipe", io::ErrorKind::BrokenPipe),
            ("ConnectionRefused", io::ErrorKind::ConnectionRefused),
            ("ConnectionReset", io::ErrorKind::ConnectionReset),
            ("ConnectionAborted", io::ErrorKind::ConnectionAborted),
            ("NotConnected", io::ErrorKind::NotConnected),
            ("AddrInUse", io::ErrorKind::AddrInUse),
            ("AddrNotAvailable", io::ErrorKind::AddrNotAvailable),
            ("WouldBlock", io::ErrorKind::WouldBlock),
            ("NotADirectory", io::ErrorKind::NotADirectory),
            ("IsADirectory", io::ErrorKind::IsADirectory),
            ("DirectoryNotEmpty", io::ErrorKind::DirectoryNotEmpty),
            ("ReadOnlyFilesystem", io::ErrorKind::ReadOnlyFilesystem),
            ("Unsupported", io::ErrorKind::Unsupported),
            ("Other", io::ErrorKind::Other),
        ];

        for (name, expected_kind) in kinds {
            let ordinary = Failure::Io {
                kind: name.into(),
                os: None,
            };
            assert!(
                matches!(
                    PreparationError::from(ordinary),
                    PreparationError::Io(error) if error.kind() == expected_kind
                ),
                "ordinary {name} must restore its I/O kind"
            );

            let native = Failure::NativeIo {
                kind: name.into(),
                os: None,
            };
            assert!(
                matches!(
                    PreparationError::from(native),
                    PreparationError::Native(NativeError::Io(error))
                        if error.kind() == expected_kind
                ),
                "native {name} must restore its I/O kind"
            );
        }

        for failure in [
            Failure::Io {
                kind: "FutureKind".into(),
                os: None,
            },
            Failure::NativeIo {
                kind: "FutureKind".into(),
                os: None,
            },
        ] {
            assert!(matches!(
                PreparationError::from(failure),
                PreparationError::Protocol
            ));
        }
    }

    #[test]
    fn worker_failures_restore_their_typed_causes() {
        let cases = [
            Failure::InvalidImage,
            Failure::Verification,
            Failure::Protocol,
            Failure::Unsupported,
            Failure::InvalidData,
            Failure::TagNotFound,
            Failure::InvalidTiffHeader,
            Failure::InvalidExif,
            Failure::InvalidIptc,
            Failure::InvalidXmp,
            Failure::TruncatedData {
                expected: 20,
                actual: 12,
            },
            Failure::Conversion,
        ];
        for failure in cases {
            let restored = PreparationError::from(failure);
            assert!(matches!(
                restored,
                PreparationError::InvalidImage
                    | PreparationError::Verification
                    | PreparationError::Protocol
                    | PreparationError::Native(_)
            ));
        }
        assert!(matches!(
            PreparationError::from(Failure::Io {
                kind: "PermissionDenied".into(),
                os: Some(13),
            }),
            PreparationError::Io(error) if error.raw_os_error() == Some(13)
        ));
        assert!(matches!(
            PreparationError::from(Failure::NativeIo {
                kind: "PermissionDenied".into(),
                os: Some(13),
            }),
            PreparationError::Native(NativeError::Io(error)) if error.raw_os_error() == Some(13)
        ));
    }

    #[test]
    fn io_failure_redacts_raw_private_detail() {
        let error = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private spool /media/tmp/upload-secret",
        );
        let failure = io_failure(&error, false);
        let serialized = wire_value(failure);

        assert_eq!(error.to_string(), "private spool /media/tmp/upload-secret");
        assert_eq!(
            serialized,
            json!({"Io": {"kind": "PermissionDenied", "os": null}})
        );
        assert!(!serialized.to_string().contains("upload-secret"));
    }
}
