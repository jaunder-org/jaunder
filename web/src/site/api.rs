use crate::error::WebResult;
use common::MutationOutcome;
use common::site::{SiteIdentity, SiteTitle};
use common::tagged_url::BaseUrl;

#[cfg(feature = "server")]
use {
    crate::{
        auth,
        error::{ErrorClass, ErrorKind, InternalError, from_write_scope_error},
    },
    leptos::prelude::*,
    std::sync::Arc,
    storage::{
        BaseUrlMutationError, PasskeyStorage, SiteConfigStorage, WriteScope,
        set_base_url_with_passkey_guard,
    },
};

#[cfg(feature = "server")]
fn map_base_url_mutation_error(error: BaseUrlMutationError) -> InternalError {
    match error {
        error @ BaseUrlMutationError::RpHostLocked(_) => InternalError::masked(
            ErrorKind::Conflict,
            ErrorClass::Client,
            "site.base_url hostname is locked while Passkeys exist",
            anyhow::Error::new(error),
        ),
        error @ BaseUrlMutationError::Validation(_) => InternalError::masked(
            ErrorKind::Validation,
            ErrorClass::Client,
            "invalid site base URL",
            anyhow::Error::new(error),
        ),
        BaseUrlMutationError::Database(error) => InternalError::storage(error),
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::map_base_url_mutation_error;
    use crate::error::ErrorKind;
    use std::error::Error;
    use storage::{BaseUrlMutationError, PasskeyRpHostLocked};

    #[test]
    fn guarded_base_url_errors_preserve_their_public_classification_and_sources() {
        let conflict =
            map_base_url_mutation_error(BaseUrlMutationError::RpHostLocked(PasskeyRpHostLocked));
        assert_eq!(conflict.kind(), ErrorKind::Conflict);
        assert!(
            Error::source(&conflict)
                .and_then(|source| source.downcast_ref::<BaseUrlMutationError>())
                .is_some()
        );

        let validation = map_base_url_mutation_error(BaseUrlMutationError::Validation(Box::new(
            common::tagged_url::InvalidUrl,
        )));
        assert_eq!(validation.kind(), ErrorKind::Validation);
        assert!(
            Error::source(&validation)
                .and_then(|source| source.downcast_ref::<BaseUrlMutationError>())
                .is_some()
        );

        let storage =
            map_base_url_mutation_error(BaseUrlMutationError::Database(sqlx::Error::PoolClosed));
        assert_eq!(storage.kind(), ErrorKind::Storage);
        assert!(
            Error::source(&storage)
                .and_then(|source| source.downcast_ref::<sqlx::Error>())
                .is_some()
        );
    }
}

#[macros::server]
pub async fn get_identity() -> WebResult<SiteIdentity> {
    auth::require_operator().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    site_config
        .get_identity()
        .await
        .map_err(InternalError::storage)
}

#[macros::server]
pub async fn get_media_uploads_enabled() -> WebResult<bool> {
    auth::require_operator().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    site_config
        .get_media_uploads_enabled()
        .await
        .map_err(InternalError::storage)
}

#[macros::server]
pub async fn update_media_uploads_enabled(uploads_enabled: bool) -> WebResult<MutationOutcome<()>> {
    auth::require_operator().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                site_config
                    .set_media_uploads_enabled(transaction, uploads_enabled)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

#[macros::server]
pub async fn update_identity(
    title: SiteTitle,
    base_url: Option<BaseUrl>,
) -> WebResult<MutationOutcome<()>> {
    auth::require_operator().await?;

    // `base_url` is a typed `Option<BaseUrl>` wire arg (ADR-0065): the
    // validating serde bridge already rejected a malformed/non-http(s) value at
    // decode time, and an omitted field decodes to `None` (clearing-via-omit) —
    // no server-side parse/`non_empty` bridge is needed.
    let identity = SiteIdentity { title, base_url };
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let passkeys = expect_context::<Arc<dyn PasskeyStorage>>();
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                set_base_url_with_passkey_guard(
                    transaction,
                    site_config.as_ref(),
                    passkeys.as_ref(),
                    identity.base_url,
                )
                .await
                .map_err(map_base_url_mutation_error)?;
                site_config
                    .set(
                        transaction,
                        host::config_key::SiteConfigKey::SiteTitle,
                        &identity.title,
                    )
                    .await
                    .map_err(InternalError::storage)?;
                Ok(())
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Whether to show the "site base URL not configured" warning banner (#575):
/// `true` only for an operator when `SiteIdentity.base_url` is `None`. Like
/// `backup::is_warning_visible`, this is a **soft** check: non-operators and
/// missing/stale cookie-only credentials yield `Ok(false)`, while failures
/// attributable to an explicit `Authorization` credential reject.
#[macros::server]
pub async fn is_base_url_warning_visible() -> WebResult<bool> {
    if !auth::is_operator_soft().await? {
        return Ok(false);
    }
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    Ok(site_config.get_identity().await?.base_url.is_none())
}
