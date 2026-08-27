//! The **`JAUNDER_CAPTURE_DIR` contract** (issue #227, ADR-0057). The e2e harness sets one
//! directory; each capture stream writes a well-known filename within it. This module is
//! the single source of the dir-var name and the per-stream filenames, so `server` (which
//! writes the streams) and `test-support` (which resets/queries them) agree without
//! restating any path.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

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

/// Immutable capture configuration resolved by an executable composition root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureConfig {
    directory: Option<PathBuf>,
}

impl CaptureConfig {
    /// Resolves a raw inherited capture directory. A missing, blank, or non-Unicode
    /// value disables capture; non-Unicode input is reported without exposing it.
    #[must_use]
    pub fn from_raw(raw: Option<OsString>) -> Self {
        let Some(raw) = raw else {
            return Self::default();
        };
        let Ok(raw) = raw.into_string() else {
            crate::error::report_swallowed(
                crate::error::ErrorKind::Internal,
                crate::error::ErrorClass::Bug,
                "host.capture.directory_config",
                crate::error::SwallowedSource::Redacted,
            );
            return Self::default();
        };
        let directory = PathBuf::from(raw.trim());
        if directory.as_os_str().is_empty() {
            Self::default()
        } else {
            Self {
                directory: Some(directory),
            }
        }
    }

    /// Returns the conventional capture path, creating the configured directory.
    ///
    /// Directory creation failure is intentionally non-fatal and reported once; the
    /// conventional path is still returned so the stream writer surfaces its error.
    #[must_use]
    pub fn file(&self, stream: Stream) -> Option<PathBuf> {
        self.file_with(stream, create_capture_dir)
    }

    fn file_with(&self, stream: Stream, create_dir: CreateDirOperation) -> Option<PathBuf> {
        let dir = self.directory.as_ref()?;
        if let Err(error) = create_dir(dir) {
            crate::error::report_swallowed(
                crate::error::ErrorKind::Internal,
                crate::error::ErrorClass::Transient,
                "host.capture.create_directory",
                crate::error::SwallowedSource::Error(&error),
            );
        }
        Some(dir.join(stream.filename()))
    }
}

type CreateDirOperation = fn(&Path) -> std::io::Result<()>;

fn create_capture_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
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

    #[derive(Clone)]
    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture<T>(operation: impl FnOnce() -> T) -> (T, String) {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        let value = tracing::subscriber::with_default(subscriber, operation);
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        let text =
            String::from_utf8(output.lock().expect("capture lock").clone()).expect("utf8 trace");
        (value, text)
    }

    fn denied_create(_: &std::path::Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "capture directory denied",
        ))
    }

    #[test]
    fn directory_creation_failure_preserves_path_and_reports_once() {
        let config = CaptureConfig::from_raw(Some("/private/capture".into()));
        let (result, trace) = capture(|| config.file_with(Stream::WebSub, denied_create));
        assert_eq!(result, Some(PathBuf::from("/private/capture/websub.jsonl")));
        assert_eq!(
            trace.matches(r#""error.disposition":"swallowed""#).count(),
            1,
            "trace: {trace}"
        );
        assert!(
            trace.contains(r#""error.context":"host.capture.create_directory""#),
            "trace: {trace}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn invalid_unicode_capture_directory_disables_capture_and_reports_redacted_once() {
        use std::os::unix::ffi::OsStringExt;

        let raw = std::ffi::OsString::from_vec(b"capture-secret-\xff".to_vec());
        let (config, trace) = capture(|| CaptureConfig::from_raw(Some(raw)));
        assert_eq!(config.file(Stream::WebSub), None);
        assert_eq!(
            trace
                .matches(r#""error.context":"host.capture.directory_config""#)
                .count(),
            1,
            "trace: {trace}"
        );
        assert!(trace.contains(r#""error.source":"redacted""#));
        assert!(!trace.contains("capture-secret"));
    }

    #[test]
    fn stream_filenames_are_the_convention() {
        assert_eq!(Stream::Mail.filename(), "mail.jsonl");
        assert_eq!(Stream::WebSub.filename(), "websub.jsonl");
        assert_eq!(Stream::Diag.filename(), "diag.log");
    }

    #[test]
    fn stream_parse_accepts_keys_and_rejects_unknown() {
        assert_eq!(Stream::parse("mail"), Some(Stream::Mail));
        assert_eq!(Stream::parse("websub"), Some(Stream::WebSub));
        assert_eq!(Stream::parse("diag"), Some(Stream::Diag));
        assert_eq!(Stream::parse("bogus"), None);
        assert_eq!(Stream::parse(""), None);
    }

    #[test]
    fn file_joins_and_creates_dir_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let directory = tmp.path().join("capture"); // does not exist yet
        let config = CaptureConfig::from_raw(Some(directory.clone().into_os_string()));
        let path = config.file(Stream::Mail).expect("configured capture");
        assert_eq!(path, directory.join("mail.jsonl"));
        assert!(directory.is_dir(), "file() must create the capture dir");
    }

    #[test]
    fn file_is_none_when_missing_or_blank() {
        assert_eq!(CaptureConfig::default().file(Stream::Diag), None);
        assert_eq!(
            CaptureConfig::from_raw(Some("   ".into())).file(Stream::Diag),
            None,
            "blank ⇒ None"
        );
    }
}
