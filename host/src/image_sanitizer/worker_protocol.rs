//! Bounded, metadata-free worker replies and typed remote failure causes.
//!
//! A process boundary cannot transfer borrowed native error objects. Project
//! their concrete variant, nested I/O classification and structural counts,
//! never library messages that may quote an image's private fields. Original
//! errors remain intact in the worker until this projection is made.

use super::{
    native::NativeImageError,
    worker::{Completion, WorkerError},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_REPLY_BYTES: usize = 128 * 1024;
pub const MAGIC: &[u8] = b"JAUNDER-IMAGE-1\0";

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("image worker response exceeds its bound")]
    TooLarge,
    #[error("invalid image worker response framing")]
    Framing,
    #[error("invalid image worker JSON")]
    Json(#[source] serde_json::Error),
    #[error("invalid image worker response shape")]
    Shape,
}

/// Encode only our typed, metadata-free reply on the worker's dedicated pipe.
/// # Errors
/// Returns a typed encoding or response-bound failure.
pub fn encode(reply: &Reply) -> Result<Vec<u8>, ProtocolError> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend(serde_json::to_vec(reply).map_err(ProtocolError::Json)?);
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    Ok(bytes)
}

/// Reject corruption without reflecting private response values in diagnostics.
/// # Errors
/// Returns typed framing, JSON, or closed-schema rejection.
pub fn decode(bytes: &[u8]) -> Result<Reply, ProtocolError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    let payload = bytes.strip_prefix(MAGIC).ok_or(ProtocolError::Framing)?;
    let _: serde_json::Value = serde_json::from_slice(payload).map_err(ProtocolError::Json)?;
    // Wrong schema is explicit protocol rejection; serde's schema messages can
    // quote arbitrary values and therefore cannot become diagnostic sources.
    // Decode the original map, not Value's normalized map: duplicate fields
    // are invalid too and must not silently overwrite an earlier value.
    let reply: Reply = serde_json::from_slice(payload).map_err(|_| ProtocolError::Shape)?;
    match &reply {
        Reply::Image { mime, .. } if super::covered_mime(mime).is_none() => {
            return Err(ProtocolError::Shape);
        }
        Reply::Failed(Failure::Gif { kind, io })
            if matches!(kind, GifFailure::Io) != io.is_some() =>
        {
            return Err(ProtocolError::Shape);
        }
        Reply::Failed(Failure::Metadata(MetadataFailure::TruncatedData { expected, actual }))
            if actual > expected =>
        {
            return Err(ProtocolError::Shape);
        }
        _ => {}
    }
    Ok(reply)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Reply {
    Image { mime: String, size: u64 },
    Passthrough { size: u64 },
    Failed(Failure),
}

impl From<Completion> for Reply {
    fn from(value: Completion) -> Self {
        match value {
            Completion::Image { mime, size } => Self::Image {
                mime: mime.to_owned(),
                size,
            },
            Completion::Passthrough { size } => Self::Passthrough { size },
        }
    }
}

#[derive(Debug, Error, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[error("remote I/O failure ({kind}, OS code {os_code:?})")]
pub struct IoFailure {
    // The protocol is private to the same compiled executable, so preserve the
    // concrete enum discriminant without carrying any caller-provided string.
    pub kind: IoKind,
    pub os_code: Option<i32>,
}
impl From<&std::io::Error> for IoFailure {
    fn from(error: &std::io::Error) -> Self {
        Self {
            kind: error.kind().into(),
            os_code: error.raw_os_error(),
        }
    }
}

#[derive(Debug, Error, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum MetadataFailure {
    #[error("native metadata I/O failed")]
    Io(#[source] IoFailure),
    #[error("unsupported native metadata file type")]
    UnsupportedFileType,
    #[error("invalid native metadata data")]
    InvalidData,
    #[error("native metadata tag missing")]
    TagNotFound,
    #[error("invalid TIFF header")]
    InvalidTiffHeader,
    #[error("invalid EXIF")]
    InvalidExif,
    #[error("invalid IPTC")]
    InvalidIptc,
    #[error("invalid XMP")]
    InvalidXmp,
    #[error("truncated metadata ({actual} of {expected} bytes)")]
    TruncatedData { expected: usize, actual: usize },
    #[error("native metadata conversion failed")]
    ConversionError,
}
impl From<&exiftool_rs::Error> for MetadataFailure {
    fn from(error: &exiftool_rs::Error) -> Self {
        use exiftool_rs::Error as E;
        match error {
            E::Io(e) => Self::Io(e.into()),
            E::UnsupportedFileType(_) => Self::UnsupportedFileType,
            E::InvalidData(_) => Self::InvalidData,
            E::TagNotFound(_) => Self::TagNotFound,
            E::InvalidTiffHeader => Self::InvalidTiffHeader,
            E::InvalidExif(_) => Self::InvalidExif,
            E::InvalidIptc(_) => Self::InvalidIptc,
            E::InvalidXmp(_) => Self::InvalidXmp,
            E::TruncatedData { expected, actual } => Self::TruncatedData {
                expected: *expected,
                actual: *actual,
            },
            E::ConversionError(_) => Self::ConversionError,
        }
    }
}

#[derive(Debug, Error, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Failure {
    #[error("private spool I/O failed")]
    Io(#[source] IoFailure),
    #[error("native metadata processing failed")]
    Metadata(#[source] MetadataFailure),
    #[error("native metadata I/O failed")]
    NativeIo(#[source] IoFailure),
    #[error("invalid image")]
    InvalidImage,
    #[error("image exceeds configured size limit")]
    FileTooLarge,
    #[error("invalid private spool")]
    InvalidSpool,
    #[error("invalid worker limit")]
    InvalidLimit,
    #[error("invalid image container ({0:?})")]
    Container(#[source] ContainerFailure),
    #[error("native GIF processing failed ({kind})")]
    Gif {
        kind: GifFailure,
        #[source]
        io: Option<IoFailure>,
    },
    #[error("invalid rendering integer")]
    Number(#[source] NumberFailure),
    #[error("image extent does not fit platform")]
    Integer,
    #[error("truncated fixed-width image field")]
    Field,
}

#[derive(Debug, Error, Serialize, Deserialize)]
pub enum ContainerFailure {
    #[error("wrong container signature")]
    WrongSignature,
    #[error("bad container CRC")]
    BadCrc,
    #[error("truncated container")]
    Truncated,
}

#[derive(Debug, Error, Serialize, Deserialize)]
pub enum NumberFailure {
    #[error("empty rendering integer")]
    Empty,
    #[error("invalid rendering digit")]
    InvalidDigit,
    #[error("positive integer overflow")]
    PosOverflow,
    #[error("negative integer overflow")]
    NegOverflow,
    #[error("zero rendering integer")]
    Zero,
    #[error("unknown rendering integer failure")]
    Unknown,
}
impl From<&std::num::IntErrorKind> for NumberFailure {
    fn from(kind: &std::num::IntErrorKind) -> Self {
        use std::num::IntErrorKind as K;
        match kind {
            K::Empty => Self::Empty,
            K::InvalidDigit => Self::InvalidDigit,
            K::PosOverflow => Self::PosOverflow,
            K::NegOverflow => Self::NegOverflow,
            K::Zero => Self::Zero,
            _ => Self::Unknown,
        }
    }
}

macro_rules! io_kinds {
    ($($kind:ident),+ $(,)?) => {
        #[derive(Debug, Serialize, Deserialize)]
        pub enum IoKind { $($kind),+, Unknown }
        impl From<std::io::ErrorKind> for IoKind {
            fn from(kind: std::io::ErrorKind) -> Self {
                // ErrorKind's Debug spelling is enum-owned, not an I/O message.
                // This also projects platform kinds whose constructors are unstable.
                match format!("{kind:?}").as_str() { $(stringify!($kind) => Self::$kind,)+ _ => Self::Unknown }
            }
        }
        impl std::fmt::Display for IoKind {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
        }
    }
}
io_kinds!(
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    HostUnreachable,
    NetworkUnreachable,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    NetworkDown,
    BrokenPipe,
    AlreadyExists,
    WouldBlock,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    ReadOnlyFilesystem,
    FilesystemLoop,
    StaleNetworkFileHandle,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    StorageFull,
    NotSeekable,
    QuotaExceeded,
    FileTooLarge,
    ResourceBusy,
    ExecutableFileBusy,
    Deadlock,
    CrossesDevices,
    TooManyLinks,
    InvalidFilename,
    ArgumentListTooLong,
    Interrupted,
    Unsupported,
    UnexpectedEof,
    OutOfMemory,
    Other
);

#[derive(Debug, Serialize, Deserialize)]
pub enum GifFailure {
    OutOfMemory,
    MemoryLimit,
    DecoderNotFound,
    EndCodeNotFound,
    UnexpectedEof,
    Lzw,
    Format,
    Io,
    Unknown,
}
impl std::fmt::Display for GifFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl From<&WorkerError> for Failure {
    fn from(error: &WorkerError) -> Self {
        match error {
            WorkerError::Io(e) => Self::Io(e.into()),
            WorkerError::InvalidSpool => Self::InvalidSpool,
            WorkerError::InvalidLimit => Self::InvalidLimit,
            WorkerError::Native(e) => match e {
                NativeImageError::FileTooLarge => Self::FileTooLarge,
                NativeImageError::InvalidImage(_) => Self::InvalidImage,
                NativeImageError::Metadata(e) => Self::Metadata(e.into()),
                NativeImageError::Io(e) => Self::NativeIo(e.into()),
                NativeImageError::Container(e) => Self::Container(match e {
                    img_parts::Error::WrongSignature => ContainerFailure::WrongSignature,
                    img_parts::Error::BadCRC => ContainerFailure::BadCrc,
                    img_parts::Error::Truncated => ContainerFailure::Truncated,
                }),
                NativeImageError::Number(e) => Self::Number(e.kind().into()),
                NativeImageError::Integer(_) => Self::Integer,
                NativeImageError::Field(_) => Self::Field,
                NativeImageError::Gif(e) => {
                    use gif::DecodingError as E;
                    let kind = match e {
                        E::OutOfMemory => GifFailure::OutOfMemory,
                        E::MemoryLimit => GifFailure::MemoryLimit,
                        E::DecoderNotFound => GifFailure::DecoderNotFound,
                        E::EndCodeNotFound => GifFailure::EndCodeNotFound,
                        E::UnexpectedEof => GifFailure::UnexpectedEof,
                        E::LzwError(_) => GifFailure::Lzw,
                        E::Format(_) => GifFailure::Format,
                        E::Io(_) => GifFailure::Io,
                        _ => GifFailure::Unknown,
                    };
                    let io = if let E::Io(e) = e {
                        Some(e.into())
                    } else {
                        None
                    };
                    Self::Gif { kind, io }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    #[test]
    fn malformed_frames_schema_and_failure_sources_are_rejected_without_values() {
        for payload in [
            r#"{"Image":{"mime":"owner@example.test","size":0}}"#,
            r#"{"Passthrough":{"size":0,"metadata":"owner@example.test"}}"#,
            r#"{"Failed":{"Io":{"kind":"owner@example.test","os_code":null}}}"#,
            r#"{"Failed":{"Gif":{"kind":"Io","io":null}}}"#,
            r#"{"Failed":{"Metadata":{"TruncatedData":{"actual":2,"expected":1}}}}"#,
            r#"{"Image":{"mime":"image/png","size":-1}}"#,
            r#"{"Image":{"mime":"image/png","size":0,"size":1}}"#,
            r#"{"Passthrough":{"size":0},"Passthrough":{"size":1}}"#,
            "{}",
            "[]",
        ] {
            let bytes = [MAGIC, payload.as_bytes()].concat();
            let error = decode(&bytes).expect_err("closed reply schema");
            assert!(!format!("{error:?} {error}").contains("owner@example.test"));
        }
        assert!(matches!(decode(b"{}"), Err(ProtocolError::Framing)));
        assert!(matches!(
            decode(&vec![0; MAX_REPLY_BYTES + 1]),
            Err(ProtocolError::TooLarge)
        ));
        assert!(matches!(
            decode(&[MAGIC, b"not json"].concat()),
            Err(ProtocolError::Json(_))
        ));
        let reply = Reply::Image {
            mime: "image/png".into(),
            size: 42,
        };
        assert!(matches!(
            decode(&encode(&reply).expect("encode")).expect("round trip"),
            Reply::Image { size: 42, .. }
        ));
    }

    #[test]
    fn metadata_failure_round_trips_preserve_variants_counts_and_not_private_messages() {
        // Every native metadata variant must cross the process boundary with its
        // identity intact, but never with the library's private diagnostic text.
        use exiftool_rs::Error as E;
        let marker = "owner@example.test GPS-private-value";
        let cases = [
            (
                E::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    marker,
                )),
                serde_json::json!({"Io": {"kind": "PermissionDenied", "os_code": null}}),
            ),
            (
                E::UnsupportedFileType(marker.into()),
                serde_json::json!("UnsupportedFileType"),
            ),
            (
                E::InvalidData(marker.into()),
                serde_json::json!("InvalidData"),
            ),
            (
                E::TagNotFound(marker.into()),
                serde_json::json!("TagNotFound"),
            ),
            (E::InvalidTiffHeader, serde_json::json!("InvalidTiffHeader")),
            (
                E::InvalidExif(marker.into()),
                serde_json::json!("InvalidExif"),
            ),
            (
                E::InvalidIptc(marker.into()),
                serde_json::json!("InvalidIptc"),
            ),
            (
                E::InvalidXmp(marker.into()),
                serde_json::json!("InvalidXmp"),
            ),
            (
                E::TruncatedData {
                    expected: 7,
                    actual: 3,
                },
                serde_json::json!({"TruncatedData": {"expected": 7, "actual": 3}}),
            ),
            (
                E::ConversionError(marker.into()),
                serde_json::json!("ConversionError"),
            ),
        ];
        for (error, expected) in cases {
            let error = WorkerError::Native(NativeImageError::Metadata(error));
            let reply = Reply::Failed(Failure::from(&error));
            let bytes = encode(&reply).expect("bounded metadata projection");
            assert!(!String::from_utf8_lossy(&bytes).contains(marker));
            assert!(!format!("{reply:?}").contains(marker));
            let decoded = decode(&bytes).expect("valid projected metadata failure");
            assert_eq!(
                serde_json::to_value(decoded).expect("projected variant"),
                serde_json::json!({"Failed": {"Metadata": expected}}),
            );
        }
    }

    #[test]
    fn worker_completions_round_trip_mime_size_and_passthrough_identity() {
        // A successful worker cannot change the byte-accounting facts when its
        // completion is projected into the supervisor's private reply schema.
        for (completion, expected) in [
            (
                Completion::Image {
                    mime: "image/png",
                    size: 42,
                },
                serde_json::json!({"Image": {"mime": "image/png", "size": 42}}),
            ),
            (
                Completion::Passthrough { size: 17 },
                serde_json::json!({"Passthrough": {"size": 17}}),
            ),
        ] {
            let bytes = encode(&Reply::from(completion)).expect("completion encoding");
            let decoded = decode(&bytes).expect("completion decoding");
            assert_eq!(
                serde_json::to_value(decoded).expect("completion identity"),
                expected
            );
        }
    }

    #[test]
    fn projection_preserves_nested_io_and_metadata_variants_without_private_messages() {
        let marker = "owner@example.test GPS-private-value";
        let errors = [
            WorkerError::Native(NativeImageError::Metadata(exiftool_rs::Error::InvalidXmp(
                marker.into(),
            ))),
            WorkerError::Native(NativeImageError::Metadata(exiftool_rs::Error::Io(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, marker),
            ))),
        ];
        let projected = Failure::from(&errors[1]);
        assert!(
            projected
                .source()
                .expect("metadata source")
                .source()
                .expect("I/O source")
                .downcast_ref::<IoFailure>()
                .is_some()
        );
        for error in &errors {
            let reply = Reply::Failed(Failure::from(error));
            let bytes = serde_json::to_vec(&reply).expect("fixed reply");
            assert!(bytes.len() < MAX_REPLY_BYTES);
            assert!(!String::from_utf8(bytes).expect("JSON").contains(marker));
            assert!(!format!("{reply:?}").contains(marker));
        }
    }
}
