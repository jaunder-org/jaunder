use async_trait::async_trait;
use common::absolute_url::AbsoluteUrl;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebSubError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("hub returned non-2xx: {status}")]
    HubRefused { status: u16 },
    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),
}

#[async_trait]
pub trait WebSubClient: Send + Sync {
    async fn send_publish(
        &self,
        hub_url: &AbsoluteUrl,
        feed_url: &AbsoluteUrl,
    ) -> Result<(), WebSubError>;
}

pub mod file_capture;
pub mod http;
pub mod noop;

pub use file_capture::FileCapturingWebSubClient;
pub use http::HttpWebSubClient;
pub use noop::NoopWebSubClient;

/// Build the `WebSubClient` for the given capture configuration.
///
/// A `Some` capture path (resolved from `JAUNDER_CAPTURE_DIR` at the composition
/// root — see the `host` crate) returns a [`FileCapturingWebSubClient`] recording
/// pings to `<dir>/websub.jsonl` (end-to-end tests). `None` (the production default)
/// returns the live [`HttpWebSubClient`].
#[must_use]
pub fn default_client(
    websub_capture: Option<std::path::PathBuf>,
) -> std::sync::Arc<dyn WebSubClient> {
    if let Some(path) = websub_capture {
        std::sync::Arc::new(FileCapturingWebSubClient::new(path))
    } else {
        std::sync::Arc::new(HttpWebSubClient::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_absolute_url;
    use rstest::*;

    #[fixture]
    fn capture_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // The injected path selects the transport — no env, no lock (spec Decision 5).
    #[rstest]
    #[tokio::test]
    async fn default_client_selects_file_capture_when_path_given(capture_dir: tempfile::TempDir) {
        // None ⇒ the live HTTP client fails on an unreachable hub. Port 1 on
        // loopback has no listener, so the connect is refused immediately — no
        // DNS lookup, no network egress, deterministic offline and in CI.
        let http = default_client(None);
        assert!(
            http.send_publish(
                &parse_absolute_url("http://127.0.0.1:1/"),
                &parse_absolute_url("https://example.com/feed.rss"),
            )
            .await
            .is_err()
        );

        // Some ⇒ the file-capture client records the ping to <dir>/websub.jsonl.
        let path = capture_dir.path().join("websub.jsonl");
        let captured = default_client(Some(path.clone()));
        captured
            .send_publish(
                &parse_absolute_url("https://hub.example.com/"),
                &parse_absolute_url("https://example.com/feed.rss"),
            )
            .await
            .expect("file capture write");

        let contents = std::fs::read_to_string(&path).expect("read capture file");
        assert!(contents.contains("https://example.com/feed.rss"));
    }
}
