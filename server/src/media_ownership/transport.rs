use async_trait::async_trait;
use reqwest::Client;
use thiserror::Error;
use url::Url;

/// Typed failures at the live HEAD boundary.
#[derive(Debug, Error)]
pub enum HeadTransportError {
    #[error("ownership HEAD request failed")]
    Request(#[source] reqwest::Error),
}

/// The response fact needed for ownership classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadResponse {
    instance_headers: Vec<Vec<u8>>,
}

impl HeadResponse {
    #[must_use]
    pub fn new(instance_headers: Vec<Vec<u8>>) -> Self {
        Self { instance_headers }
    }

    #[must_use]
    pub fn instance_headers(&self) -> &[Vec<u8>] {
        &self.instance_headers
    }
}

/// The narrow, fakeable network boundary below ownership policy.
#[async_trait]
pub trait HeadTransport: Send + Sync {
    /// Sends one ordinary HEAD request. Reqwest owns connection and redirect policy.
    async fn head(&self, target: &Url) -> Result<HeadResponse, HeadTransportError>;
}

/// Shared reqwest client for live ownership probes.
#[derive(Default)]
pub struct ReqwestHeadTransport {
    client: Client,
}

#[async_trait]
impl HeadTransport for ReqwestHeadTransport {
    async fn head(&self, target: &Url) -> Result<HeadResponse, HeadTransportError> {
        let response = self
            .client
            .head(target.clone())
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map_err(HeadTransportError::Request)?;
        Ok(HeadResponse::new(
            response
                .headers()
                .get_all("x-jaunder-instance")
                .iter()
                .map(|value| value.as_bytes().to_vec())
                .collect(),
        ))
    }
}
