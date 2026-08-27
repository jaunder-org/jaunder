//! The **`JAUNDER_CAPTURE_DIR` contract** (issue #227, ADR-0057). The e2e harness sets one
//! directory; each capture stream writes a well-known filename within it. This module is
//! the single source of the dir-var name and the per-stream filenames, so `server` (which
//! writes the streams) and `test-support` (which resets/queries them) agree without
//! restating any path.

use std::ffi::OsString;
use std::path::PathBuf;

/// The single env var naming the e2e capture directory. Unset in production ⇒ every
/// capture stream is inert.
pub const DIR_ENV: &str = "JAUNDER_CAPTURE_DIR";

/// A capture stream. The mapping from stream to on-disk filename lives here and nowhere
/// else — TypeScript readers and the flake reference streams/paths through this crate
/// (via `test-support capture-path`) rather than restating the filenames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Mail,
    WebSub,
    Diag,
}

/// A directory supplied for capture output could not be made ready.
#[derive(Debug)]
pub enum CaptureDirectoryError {
    /// A configured directory must name a location.
    Empty,
    /// The inherited directory cannot be converted without exposing its bytes.
    NonUnicode,
    /// The configured directory could not be prepared for stream writers.
    CreateDirectory {
        /// The filesystem failure returned while creating the directory.
        source: std::io::Error,
    },
}

impl std::fmt::Display for CaptureDirectoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("capture directory is empty"),
            Self::NonUnicode => formatter.write_str("capture directory is not valid Unicode"),
            Self::CreateDirectory { .. } => {
                formatter.write_str("could not create capture directory")
            }
        }
    }
}

impl std::error::Error for CaptureDirectoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateDirectory { source } => Some(source),
            Self::Empty | Self::NonUnicode => None,
        }
    }
}

/// A prepared, nonempty directory used for capture output.
///
/// Construction creates the directory once at the executable boundary, so stream writers
/// receive only ordinary leaf paths and never perform deferred configuration I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureDirectory(PathBuf);

impl CaptureDirectory {
    /// Prepares a host-internal programmatic capture directory for stream writers.
    ///
    /// External inputs must use [`Self::from_raw`] so non-Unicode values are rejected
    /// without exposing their bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureDirectoryError::Empty`] when `directory` names no location, or
    /// [`CaptureDirectoryError::CreateDirectory`] when it cannot be created.
    pub(crate) fn new(directory: PathBuf) -> Result<Self, CaptureDirectoryError> {
        if directory.as_os_str().is_empty() {
            return Err(CaptureDirectoryError::Empty);
        }
        std::fs::create_dir_all(&directory)
            .map_err(|source| CaptureDirectoryError::CreateDirectory { source })?;
        Ok(Self(directory))
    }

    /// Resolves an inherited capture directory. This is the sole public constructor:
    /// missing or whitespace-only inputs disable capture; every explicit value is prepared
    /// before it reaches a stream writer.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureDirectoryError::NonUnicode`] without exposing invalid input, or
    /// [`CaptureDirectoryError::CreateDirectory`] when a configured directory cannot be
    /// created.
    pub fn from_raw(raw: Option<OsString>) -> Result<Option<Self>, CaptureDirectoryError> {
        let Some(raw) = raw else {
            return Ok(None);
        };
        let raw = raw
            .into_string()
            .map_err(|_| CaptureDirectoryError::NonUnicode)?;
        let directory = PathBuf::from(raw.trim());
        if directory.as_os_str().is_empty() {
            return Ok(None);
        }
        Self::new(directory).map(Some)
    }

    /// Returns the conventional capture file for a stream without touching the filesystem.
    #[must_use]
    pub fn path(&self, stream: Stream) -> PathBuf {
        self.0.join(stream.filename())
    }
}

impl Stream {
    /// The conventional filename this stream writes within the capture dir.
    #[must_use]
    pub fn filename(self) -> &'static str {
        match self {
            Stream::Mail => "mail.jsonl",
            Stream::WebSub => "websub.jsonl",
            Stream::Diag => "diag.log",
        }
    }

    /// Parse a CLI/logical stream key (e.g. `mail`) into a `Stream`. The key, not
    /// the filename, is the stable token shared across the language boundary.
    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        match key {
            "mail" => Some(Stream::Mail),
            "websub" => Some(Stream::WebSub),
            "diag" => Some(Stream::Diag),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_or_blank_raw_directory_disables_capture() {
        assert_eq!(CaptureDirectory::from_raw(None).expect("absent"), None);
        assert_eq!(
            CaptureDirectory::from_raw(Some("  \t ".into())).expect("blank"),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_raw_directory_is_rejected_without_exposing_its_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let secret = "capture-secret";
        let mut raw = secret.as_bytes().to_vec();
        raw.extend_from_slice(b"-\xff");
        let error = CaptureDirectory::from_raw(Some(std::ffi::OsString::from_vec(raw)))
            .expect_err("non-Unicode directory");
        assert!(matches!(error, CaptureDirectoryError::NonUnicode));
        assert!(!format!("{error:?}").contains(secret));
        assert!(!error.to_string().contains(secret));
    }

    #[test]
    fn empty_directory_is_rejected() {
        let error = CaptureDirectory::new(PathBuf::new()).expect_err("empty directory");
        assert!(matches!(error, CaptureDirectoryError::Empty));
        assert_eq!(error.to_string(), "capture directory is empty");
    }

    #[test]
    fn construction_creates_a_missing_directory() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let directory = temporary.path().join("capture");

        let capture = CaptureDirectory::new(directory.clone()).expect("create capture directory");

        assert!(directory.is_dir());
        assert_eq!(capture.path(Stream::Mail), directory.join("mail.jsonl"));
    }

    #[test]
    fn existing_file_is_a_create_error_with_its_io_source() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let file = temporary.path().join("not-a-directory");
        std::fs::write(&file, "not a directory").expect("write file");

        let error = CaptureDirectory::new(file).expect_err("file cannot be a directory");

        let CaptureDirectoryError::CreateDirectory { source } = error else {
            unreachable!("existing file must fail directory creation");
        };
        assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn path_maps_each_stream_without_filesystem_io() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let directory = temporary.path().join("capture");
        let capture = CaptureDirectory::new(directory.clone()).expect("create capture directory");
        std::fs::remove_dir(&directory).expect("remove prepared directory");

        assert_eq!(capture.path(Stream::Mail), directory.join("mail.jsonl"));
        assert_eq!(capture.path(Stream::WebSub), directory.join("websub.jsonl"));
        assert_eq!(capture.path(Stream::Diag), directory.join("diag.log"));
        assert!(
            !directory.exists(),
            "path() must not recreate the directory"
        );
    }

    #[test]
    fn stream_parse_accepts_keys_and_rejects_unknown() {
        assert_eq!(Stream::parse("mail"), Some(Stream::Mail));
        assert_eq!(Stream::parse("websub"), Some(Stream::WebSub));
        assert_eq!(Stream::parse("diag"), Some(Stream::Diag));
        assert_eq!(Stream::parse("bogus"), None);
        assert_eq!(Stream::parse(""), None);
    }
}
