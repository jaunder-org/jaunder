use async_trait::async_trait;
use chrono::Utc;
use std::path::PathBuf;

use super::{WebSubClient, WebSubError};

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
}

#[async_trait]
impl WebSubClient for FileCapturingWebSubClient {
    async fn send_publish(&self, hub_url: &str, feed_url: &str) -> Result<(), WebSubError> {
        use std::io::Write;

        // Value::to_string is infallible for this all-string structure.
        let mut line = serde_json::json!({
            "hub_url": hub_url,
            "feed_url": feed_url,
            "sent_at": Utc::now().to_rfc3339(),
        })
        .to_string();
        line.push('\n');

        // A direct blocking append is acceptable here: this client only runs in
        // the e2e capture path, writing one small line per worker tick.
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| WebSubError::Http(e.to_string()))?;
        file.write_all(line.as_bytes())
            .map_err(|e| WebSubError::Http(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn appends_one_json_line_per_ping() {
        let dir = std::env::temp_dir().join(format!("websub-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("websub.jsonl");
        let _ = std::fs::remove_file(&path);

        let client = FileCapturingWebSubClient::new(&path);
        client
            .send_publish("https://hub.example.com/", "https://site/~alice/feed.rss")
            .await
            .expect("first ping");
        client
            .send_publish("https://hub.example.com/", "https://site/~bob/feed.rss")
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
            .send_publish("https://hub.example.com/", "https://site/~alice/feed.rss")
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
            .send_publish("https://hub.example.com/", "https://site/~alice/feed.rss")
            .await
            .expect_err("write should fail");
        assert!(matches!(err, WebSubError::Http(_)));
    }
}
