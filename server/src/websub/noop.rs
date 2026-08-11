use async_trait::async_trait;

use super::{WebSubClient, WebSubError};
use common::tagged_url::{FeedUrl, HubUrl};

pub struct NoopWebSubClient;

#[async_trait]
impl WebSubClient for NoopWebSubClient {
    async fn send_publish(
        &self,
        _hub_url: &HubUrl,
        _feed_url: &FeedUrl,
    ) -> Result<(), WebSubError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_url;

    #[tokio::test]
    async fn send_publish_returns_ok() {
        let c = NoopWebSubClient;
        c.send_publish(&parse_url("https://hub"), &parse_url("https://feed"))
            .await
            .expect("noop returns ok");
    }
}
