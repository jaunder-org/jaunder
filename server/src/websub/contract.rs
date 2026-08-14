use async_trait::async_trait;
use common::tagged_url::{FeedUrl, HubUrl};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebSubError {
    #[error("WebSub transport failed")]
    Http(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("hub returned non-2xx: {status}")]
    HubRefused { status: u16 },
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
    /// Returns [`WebSubError`] if the hub is unreachable, times out, or answers
    /// non-2xx.
    async fn send_publish(&self, hub_url: &HubUrl, feed_url: &FeedUrl) -> Result<(), WebSubError>;
}
