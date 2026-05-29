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

#[cfg(not(target_arch = "wasm32"))]
pub mod http;
pub mod noop;

#[cfg(any(test, feature = "test-utils"))]
pub mod capturing;

#[cfg(not(target_arch = "wasm32"))]
pub use http::HttpWebSubClient;
pub use noop::NoopWebSubClient;

#[cfg(any(test, feature = "test-utils"))]
pub use capturing::CapturingWebSubClient;
