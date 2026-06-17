use async_trait::async_trait;

use super::{WebSubClient, WebSubError};

pub struct NoopWebSubClient;

#[async_trait]
impl WebSubClient for NoopWebSubClient {
    async fn send_publish(&self, _hub_url: &str, _feed_url: &str) -> Result<(), WebSubError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn send_publish_returns_ok() {
        let c = NoopWebSubClient;
        c.send_publish("https://hub", "https://feed")
            .await
            .expect("noop returns ok");
    }
}
