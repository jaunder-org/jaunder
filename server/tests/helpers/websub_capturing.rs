use async_trait::async_trait;
use std::sync::Mutex;

use common::absolute_url::AbsoluteUrl;
use jaunder::websub::{WebSubClient, WebSubError};

#[derive(Debug, Clone)]
pub struct CapturedPing {
    pub hub_url: AbsoluteUrl,
    pub feed_url: AbsoluteUrl,
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
    async fn send_publish(
        &self,
        hub_url: &AbsoluteUrl,
        feed_url: &AbsoluteUrl,
    ) -> Result<(), WebSubError> {
        self.pings
            .lock()
            .expect("mutex not poisoned")
            .push(CapturedPing {
                hub_url: hub_url.clone(),
                feed_url: feed_url.clone(),
            });
        Ok(())
    }
}
