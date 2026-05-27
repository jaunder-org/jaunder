use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

use super::{WebSubClient, WebSubError};

#[derive(Debug, Clone)]
pub struct CapturedPing {
    pub hub_url: String,
    pub feed_url: String,
    pub sent_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct CapturingWebSubClient {
    pings: Mutex<Vec<CapturedPing>>,
}

impl CapturingWebSubClient {
    /// Returns a clone of all captured pings.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned, which should never happen in normal operation.
    pub fn pings(&self) -> Vec<CapturedPing> {
        self.pings.lock().expect("mutex not poisoned").clone()
    }
}

#[async_trait]
impl WebSubClient for CapturingWebSubClient {
    async fn send_publish(&self, hub_url: &str, feed_url: &str) -> Result<(), WebSubError> {
        self.pings
            .lock()
            .expect("mutex not poisoned")
            .push(CapturedPing {
                hub_url: hub_url.to_string(),
                feed_url: feed_url.to_string(),
                sent_at: Utc::now(),
            });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn capturing_records_each_ping() {
        let c = CapturingWebSubClient::default();
        c.send_publish("https://hub", "https://feed")
            .await
            .expect("ok");
        c.send_publish("https://hub", "https://feed")
            .await
            .expect("ok");
        let pings = c.pings();
        assert_eq!(pings.len(), 2);
        assert_eq!(pings[0].hub_url, "https://hub");
        assert_eq!(pings[0].feed_url, "https://feed");
        let cloned = pings[0].clone();
        assert_eq!(cloned.hub_url, pings[0].hub_url);
        assert_eq!(cloned.feed_url, pings[0].feed_url);
    }
}
