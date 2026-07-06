use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

use jaunder::websub::{WebSubClient, WebSubError};

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
