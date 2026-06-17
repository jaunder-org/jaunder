use async_trait::async_trait;
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
    async fn send_publish(&self, hub_url: &str, feed_url: &str) -> Result<(), WebSubError>;
}

pub mod file_capture;
pub mod http;
pub mod noop;

pub use file_capture::FileCapturingWebSubClient;
pub use http::HttpWebSubClient;
pub use noop::NoopWebSubClient;

/// Build the `WebSubClient` selected by the environment.
///
/// When `JAUNDER_WEBSUB_CAPTURE_FILE` is set, returns a
/// [`FileCapturingWebSubClient`] that records pings to that file (used by
/// end-to-end tests, mirroring the `JAUNDER_MAIL_CAPTURE_FILE` mail-capture
/// path).  Otherwise returns the live [`HttpWebSubClient`].
#[must_use]
pub fn default_client_from_env() -> std::sync::Arc<dyn WebSubClient> {
    if let Ok(path) = std::env::var("JAUNDER_WEBSUB_CAPTURE_FILE") {
        std::sync::Arc::new(FileCapturingWebSubClient::new(path))
    } else {
        std::sync::Arc::new(HttpWebSubClient::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV_KEY: &str = "JAUNDER_WEBSUB_CAPTURE_FILE";

    // Exercises both branches of default_client_from_env in one test so the
    // process-global env var is set and cleared without racing other tests.
    #[tokio::test]
    async fn selects_file_capture_when_env_set_else_http() {
        // Unset branch: the live HTTP client rejects an unroutable host.
        std::env::remove_var(ENV_KEY);
        let http = default_client_from_env();
        assert!(http
            .send_publish("not-a-valid-url", "https://example.com/feed.rss")
            .await
            .is_err());

        // Set branch: the file-capture client records the ping to disk.
        let path =
            std::env::temp_dir().join(format!("websub-env-select-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::env::set_var(ENV_KEY, &path);
        let captured = default_client_from_env();
        captured
            .send_publish("https://hub.example.com/", "https://example.com/feed.rss")
            .await
            .expect("file capture write");
        std::env::remove_var(ENV_KEY);

        let contents = std::fs::read_to_string(&path).expect("read capture file");
        assert!(contents.contains("https://example.com/feed.rss"));
        let _ = std::fs::remove_file(&path);
    }
}
