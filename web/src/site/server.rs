use std::error::Error;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use common::{MutationOutcome, site::SiteIdentity};
use leptos::prelude::use_context;

use crate::{
    auth,
    error::{InternalError, InternalResult},
};

/// Erased site-identity publisher failure at the web/server composition seam.
#[derive(Debug)]
pub struct SiteIdentityPublisherError {
    source: Box<dyn Error + Send + Sync>,
}

impl SiteIdentityPublisherError {
    #[must_use]
    pub fn new(source: Box<dyn Error + Send + Sync>) -> Self {
        Self { source }
    }
}

impl fmt::Display for SiteIdentityPublisherError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("site identity publisher operation failed")
    }
}

impl Error for SiteIdentityPublisherError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Aggregate identity mutation capability supplied by the composition root.
#[async_trait]
pub trait SiteIdentityPublisher: Send + Sync {
    async fn mutate_identity(
        &self,
        identity: SiteIdentity,
    ) -> Result<MutationOutcome<()>, SiteIdentityPublisherError>;
}

pub(super) async fn update_identity_impl(
    identity: SiteIdentity,
) -> InternalResult<MutationOutcome<()>> {
    auth::require_operator().await?;
    let publisher = use_context::<Arc<dyn SiteIdentityPublisher>>()
        .ok_or_else(|| InternalError::server_message("site identity publisher is unavailable"))?;
    publisher
        .mutate_identity(identity)
        .await
        .map_err(InternalError::server)
}
