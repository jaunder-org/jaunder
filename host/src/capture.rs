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

/// The capture directory named by `JAUNDER_CAPTURE_DIR`, trimmed. `None` when the var is
/// unset or blank — i.e. capture is off (the production default). Internal helper for
/// [`file`]; not part of the public surface until a caller needs the bare directory.
fn dir() -> Option<PathBuf> {
    let raw = std::env::var(DIR_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Resolve a stream's capture file within the capture dir, creating the directory so a
/// writer can open the file. `None` when capture is off (`JAUNDER_CAPTURE_DIR` unset or
/// blank).
///
/// The `create_dir_all` error is intentionally discarded: a writer opening the returned
/// path surfaces any real failure itself, matching the pre-contract behavior where each
/// stream simply tried to open its `_FILE` path.
#[must_use]
pub fn file(stream: Stream) -> Option<PathBuf> {
    let dir = dir()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(stream.filename()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `dir`/`file` read a process-global env var. Under `cargo nextest` each test is its
    // own process, but a plain threaded `cargo test` shares one — so the two env-mutating
    // tests below serialize on this lock. This is the ONLY place in the codebase that
    // mutates `JAUNDER_CAPTURE_DIR` in-process (server seams inject the path instead), so
    // the lock is local to this crate's test binary.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("capture"); // does not exist yet
        std::env::set_var(DIR_ENV, &d);
        let p = file(Stream::Mail).expect("Some when set");
        assert_eq!(p, d.join("mail.jsonl"));
        assert!(d.is_dir(), "file() must create the capture dir");
        std::env::remove_var(DIR_ENV);
    }

    #[test]
    fn file_is_none_when_unset_or_blank() {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::remove_var(DIR_ENV);
        assert_eq!(file(Stream::Diag), None);
        std::env::set_var(DIR_ENV, "   ");
        assert_eq!(file(Stream::Diag), None, "blank ⇒ None");
        std::env::remove_var(DIR_ENV);
    }
}
