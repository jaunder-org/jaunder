use async_trait::async_trait;
use common::tagged_url::{FeedUrl, HubUrl};
use thiserror::Error;

/// The publication disposition supplied by a [`WebSubClient`].
///
/// The client classifies the HTTP exchange so callers never need to inspect
/// response statuses or headers. A retryable failure can carry a hub-selected
/// delay; scheduling policy remains with the caller.
#[derive(Debug, Error)]
pub enum WebSubError {
    #[error("WebSub publish is retryable: {reason}")]
    Retryable {
        #[source]
        reason: RetryableWebSubError,
        retry_after: Option<std::time::Duration>,
    },
    #[error("WebSub publish is terminal: {reason}")]
    Terminal { reason: TerminalWebSubError },
}

#[derive(Debug, Error)]
pub enum RetryableWebSubError {
    #[error("WebSub transport failed")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("hub returned retryable HTTP {status}")]
    Http { status: u16 },
}

#[derive(Debug, Error)]
pub enum TerminalWebSubError {
    #[error("hub returned terminal HTTP {status}")]
    Http { status: u16 },
    #[error("hub returned redirect HTTP {status} without Location")]
    MissingLocation { status: u16 },
    #[error("hub returned redirect HTTP {status} with invalid Location")]
    InvalidLocation { status: u16 },
    #[error("hub returned redirect HTTP {status} with non-HTTP(S) Location")]
    UnsupportedLocationScheme { status: u16 },
    #[error("hub redirect loop detected after HTTP {status}")]
    RedirectLoop { status: u16 },
    #[error("hub redirect limit exceeded after HTTP {status}")]
    TooManyRedirects { status: u16 },
}

#[async_trait]
pub trait WebSubClient: Send + Sync {
    /// Ping a `WebSub` hub, announcing that `feed_url` has new content.
    ///
    /// The two parameters carry distinct roles, so transposing them is a compile
    /// error rather than a ping sent to the feed about the hub (#875):
    ///
    /// ```compile_fail
    /// # use jaunder::websub::{NoopWebSubClient, WebSubClient};
    /// # use common::tagged_url::{FeedUrl, HubUrl};
    /// # async fn f(client: &NoopWebSubClient, hub: &HubUrl, feed: &FeedUrl) {
    /// let _ = client.send_publish(feed, hub).await;
    /// # }
    /// ```
    ///
    /// The correct order compiles — same fixture, so the negative above can only be
    /// failing for the transposition:
    ///
    /// ```
    /// # use jaunder::websub::{NoopWebSubClient, WebSubClient};
    /// # use common::tagged_url::{FeedUrl, HubUrl};
    /// # async fn f(client: &NoopWebSubClient, hub: &HubUrl, feed: &FeedUrl) {
    /// let _ = client.send_publish(hub, feed).await;
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`WebSubError::Retryable`] for transport failures, timeouts, and
    /// retryable hub responses; its optional delay is the already-validated
    /// `Retry-After` value. Returns [`WebSubError::Terminal`] for a final hub
    /// response the caller must not retry.
    async fn send_publish(&self, hub_url: &HubUrl, feed_url: &FeedUrl) -> Result<(), WebSubError>;
}
