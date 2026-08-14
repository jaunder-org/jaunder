//! The **`JAUNDER_CAPTURE_DIR` contract** (issue #227, ADR-0057). The e2e harness sets one
//! directory; each capture stream writes a well-known filename within it. This module is
//! the single source of the dir-var name and the per-stream filenames, so `server` (which
//! writes the streams) and `test-support` (which resets/queries them) agree without
//! restating any path.

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

type CreateDirOperation = fn(&std::path::Path) -> std::io::Result<()>;

fn create_capture_dir(path: &std::path::Path) -> std::io::Result<()> {
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

/// Returns the conventional capture path, creating the configured directory.
///
/// A missing or blank `JAUNDER_CAPTURE_DIR` disables capture. Directory
/// creation failure is intentionally non-fatal and reported once; the
/// conventional path is still returned so the stream writer surfaces its error.
#[must_use]
pub fn file(stream: Stream) -> Option<PathBuf> {
    file_with(stream, create_capture_dir)
}

fn file_with(stream: Stream, create_dir: CreateDirOperation) -> Option<PathBuf> {
    let raw = match std::env::var(DIR_ENV) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return None,
        Err(std::env::VarError::NotUnicode(_)) => {
            crate::error::report_swallowed(
                crate::error::ErrorKind::Internal,
                crate::error::ErrorClass::Bug,
                "host.capture.directory_config",
                crate::error::SwallowedSource::Redacted,
            );
            return None;
        }
    };
    let configured = raw.trim();
    if configured.is_empty() {
        return None;
    }
    let dir = PathBuf::from(configured);
    if let Err(error) = create_dir(&dir) {
        crate::error::report_swallowed(
            crate::error::ErrorKind::Internal,
            crate::error::ErrorClass::Transient,
            "host.capture.create_directory",
            crate::error::SwallowedSource::Error(&error),
        );
    }
    Some(dir.join(stream.filename()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::with_env;

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
        with_env(|env| {
            env.set(DIR_ENV, "/private/capture");
            let (result, trace) = capture(|| file_with(Stream::WebSub, denied_create));
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
        });
    }

    #[cfg(unix)]
    #[test]
    fn invalid_unicode_capture_directory_disables_capture_and_reports_redacted_once() {
        use std::os::unix::ffi::OsStringExt;

        with_env(|env| {
            env.set(
                DIR_ENV,
                std::ffi::OsString::from_vec(b"capture-secret-\xff".to_vec()),
            );
            let (result, trace) = capture(|| file(Stream::WebSub));
            assert_eq!(result, None);
            assert_eq!(
                trace
                    .matches(r#""error.context":"host.capture.directory_config""#)
                    .count(),
                1,
                "trace: {trace}"
            );
            assert!(trace.contains(r#""error.source":"redacted""#));
            assert!(!trace.contains("capture-secret"));
        });
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
    fn file_joins_and_creates_dir_when_set() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("capture"); // does not exist yet
        with_env(|env| {
            env.set(DIR_ENV, &d);
            let p = file(Stream::Mail).expect("Some when set");
            assert_eq!(p, d.join("mail.jsonl"));
            assert!(d.is_dir(), "file() must create the capture dir");
        });
    }

    #[test]
    fn file_is_none_when_unset_or_blank() {
        // Two env states with an assertion between them, in one critical section:
        // splitting this into two `with_env` calls would reopen the window the single
        // lock closes.
        with_env(|env| {
            env.remove(DIR_ENV);
            assert_eq!(file(Stream::Diag), None);
            env.set(DIR_ENV, "   ");
            assert_eq!(file(Stream::Diag), None, "blank ⇒ None");
        });
    }
}
