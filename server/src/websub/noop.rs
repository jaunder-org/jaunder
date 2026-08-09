use async_trait::async_trait;

use super::{WebSubClient, WebSubError};
use common::absolute_url::AbsoluteUrl;

pub struct NoopWebSubClient;

#[async_trait]
impl WebSubClient for NoopWebSubClient {
    async fn send_publish(
        &self,
        _hub_url: &AbsoluteUrl,
        _feed_url: &AbsoluteUrl,
    ) -> Result<(), WebSubError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_absolute_url;

    #[tokio::test]
    async fn send_publish_returns_ok() {
        let c = NoopWebSubClient;
        c.send_publish(
            &parse_absolute_url("https://hub"),
            &parse_absolute_url("https://feed"),
        )
        .await
        .expect("noop returns ok");
    }
}
