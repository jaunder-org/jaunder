use std::error::Error;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use common::{MutationOutcome, site::SiteIdentity};
use leptos::prelude::use_context;
use storage::{BaseUrlMutationError, SiteIdentityMutationError};

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

fn find_error<'a, T: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a T> {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(found) = error.downcast_ref::<T>() {
            return Some(found);
        }
        current = error.source();
    }
    None
}

fn map_identity_publisher_error(error: SiteIdentityPublisherError) -> InternalError {
    let source = error.source;
    if let Some(error) = find_error::<SiteIdentityMutationError>(source.as_ref()) {
        return match error {
            SiteIdentityMutationError::BaseUrl(BaseUrlMutationError::RpHostLocked(_)) => {
                InternalError::masked(
                    crate::error::ErrorKind::Conflict,
                    crate::error::ErrorClass::Client,
                    "site.base_url hostname is locked while Passkeys exist",
                    anyhow::Error::from_boxed(source),
                )
            }
            SiteIdentityMutationError::BaseUrl(BaseUrlMutationError::Validation(_)) => {
                InternalError::masked(
                    crate::error::ErrorKind::Validation,
                    crate::error::ErrorClass::Client,
                    "invalid site base URL",
                    anyhow::Error::from_boxed(source),
                )
            }
            SiteIdentityMutationError::BaseUrl(BaseUrlMutationError::Database(_))
            | SiteIdentityMutationError::Config(_)
            | SiteIdentityMutationError::Publisher(_) => InternalError::masked(
                crate::error::ErrorKind::Storage,
                crate::error::ErrorClass::Bug,
                "storage operation failed",
                anyhow::Error::from_boxed(source),
            ),
        };
    }
    if find_error::<sqlx::Error>(source.as_ref()).is_some() {
        return InternalError::masked(
            crate::error::ErrorKind::Storage,
            crate::error::ErrorClass::Bug,
            "storage operation failed",
            anyhow::Error::from_boxed(source),
        );
    }
    InternalError::server_boxed(source)
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
        .map_err(map_identity_publisher_error)
}

#[cfg(test)]
mod tests {
    use super::{
        SiteIdentityMutationError, SiteIdentityPublisherError, map_identity_publisher_error,
    };
    use crate::error::ErrorKind;
    use storage::{BaseUrlMutationError, PasskeyRpHostLocked};

    fn map(error: SiteIdentityMutationError) -> crate::error::InternalError {
        map_identity_publisher_error(SiteIdentityPublisherError::new(Box::new(error)))
    }

    #[test]
    fn aggregate_identity_errors_keep_their_public_classification() {
        let conflict = map(SiteIdentityMutationError::BaseUrl(
            BaseUrlMutationError::RpHostLocked(PasskeyRpHostLocked),
        ));
        assert_eq!(conflict.kind(), ErrorKind::Conflict);

        let validation = map(SiteIdentityMutationError::BaseUrl(
            BaseUrlMutationError::Validation(Box::new(common::tagged_url::InvalidUrl)),
        ));
        assert_eq!(validation.kind(), ErrorKind::Validation);

        let storage = map(SiteIdentityMutationError::Config(sqlx::Error::PoolClosed));
        assert_eq!(storage.kind(), ErrorKind::Storage);
    }
}
