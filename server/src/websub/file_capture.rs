use async_trait::async_trait;
use chrono::Utc;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use super::{WebSubClient, WebSubError};
use common::tagged_url::{FeedUrl, HubUrl};

type OpenOperation<W> = fn(&Path) -> io::Result<W>;

fn open_capture(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// A [`WebSubClient`] that appends each ping as a JSON line to a file on disk
/// instead of contacting a hub.  Used for the `websub.jsonl` stream of the
/// `JAUNDER_CAPTURE_DIR` contract (end-to-end tests only); see the `host` crate.
pub struct FileCapturingWebSubClient {
    path: PathBuf,
}

impl FileCapturingWebSubClient {
    /// Create a new client that appends pings to `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn send_publish_with<W>(
        &self,
        hub_url: &HubUrl,
        feed_url: &FeedUrl,
        open: OpenOperation<W>,
    ) -> Result<(), WebSubError>
    where
        W: Write,
    {
        let mut line = serde_json::json!({
            "hub_url": hub_url,
            "feed_url": feed_url,
            "sent_at": Utc::now().to_rfc3339(),
        })
        .to_string();
        line.push('\n');

        let mut file = open(&self.path).map_err(|source| WebSubError::Http(Box::new(source)))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.flush())
            .map_err(|source| WebSubError::Http(Box::new(source)))
    }
}

#[async_trait]
impl WebSubClient for FileCapturingWebSubClient {
    async fn send_publish(&self, hub_url: &HubUrl, feed_url: &FeedUrl) -> Result<(), WebSubError> {
        self.send_publish_with(hub_url, feed_url, open_capture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_url;

    struct FailingWriter {
        write_succeeds: bool,
    }

    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.write_succeeds {
                Ok(bytes.len())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "capture write denied",
                ))
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "capture flush denied",
            ))
        }
    }

    #[test]
    fn file_capture_write_failure_preserves_typed_io_source() {
        let client = FileCapturingWebSubClient::new("/private/websub.jsonl");
        let open: OpenOperation<FailingWriter> = |_| {
            Ok(FailingWriter {
                write_succeeds: false,
            })
        };
        let error = client
            .send_publish_with(&hub_url(), &feed_url("alice"), open)
            .expect_err("injected write must propagate");
        let source = std::error::Error::source(&error)
            .and_then(|error| error.downcast_ref::<io::Error>())
            .expect("typed I/O source");
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn file_capture_flush_failure_preserves_typed_io_source() {
        let client = FileCapturingWebSubClient::new("/private/websub.jsonl");
        let open: OpenOperation<FailingWriter> = |_| {
            Ok(FailingWriter {
                write_succeeds: true,
            })
        };
        let error = client
            .send_publish_with(&hub_url(), &feed_url("alice"), open)
            .expect_err("injected flush must propagate");
        let source = std::error::Error::source(&error)
            .and_then(|error| error.downcast_ref::<io::Error>())
            .expect("typed I/O source");
        assert_eq!(source.kind(), io::ErrorKind::WriteZero);
    }

    /// The hub every test in this module pings; its value is incidental.
    fn hub_url() -> HubUrl {
        parse_url("https://hub.example.com/")
    }

    /// `user`'s RSS feed — the pings differ only by whose feed regenerated.
    fn feed_url(user: &str) -> FeedUrl {
        parse_url(&format!("https://site/~{user}/feed.rss"))
    }

    #[tokio::test]
    async fn appends_one_json_line_per_ping() {
        let dir = std::env::temp_dir().join(format!("websub-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("websub.jsonl");
        let _ = std::fs::remove_file(&path);

        let client = FileCapturingWebSubClient::new(&path);
        client
            .send_publish(&hub_url(), &feed_url("alice"))
            .await
            .expect("first ping");
        client
            .send_publish(&hub_url(), &feed_url("bob"))
            .await
            .expect("second ping");

        let contents = std::fs::read_to_string(&path).expect("read capture file");
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("valid json");
        assert_eq!(first["hub_url"], "https://hub.example.com/");
        assert_eq!(first["feed_url"], "https://site/~alice/feed.rss");
        assert!(first["sent_at"].is_string());

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn returns_error_when_file_cannot_be_opened() {
        // A path whose parent directory does not exist cannot be opened for
        // append, so the open fails and the error is surfaced.
        let client = FileCapturingWebSubClient::new("/nonexistent-dir-xyz/websub.jsonl");
        let err = client
            .send_publish(&hub_url(), &feed_url("alice"))
            .await
            .expect_err("open should fail");
        assert!(matches!(err, WebSubError::Http(_)));
    }

    // /dev/full opens successfully but every write fails with ENOSPC, which
    // exercises the write-failure path distinct from the open-failure path.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn returns_error_when_write_fails() {
        let client = FileCapturingWebSubClient::new("/dev/full");
        let err = client
            .send_publish(&hub_url(), &feed_url("alice"))
            .await
            .expect_err("write should fail");
        assert!(matches!(err, WebSubError::Http(_)));
    }
}
