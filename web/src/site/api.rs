use crate::error::WebResult;
use common::MutationOutcome;
use common::site::{SiteIdentity, SiteTagline, SiteTitle};
use common::tagged_url::BaseUrl;

#[cfg(feature = "server")]
use {
    crate::{
        auth,
        error::{InternalError, from_write_scope_error},
        site::server::update_identity_impl,
    },
    leptos::prelude::*,
    std::sync::Arc,
    storage::{SiteConfigStorage, WriteScope},
};

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
    tagline: Option<SiteTagline>,
    base_url: Option<BaseUrl>,
) -> WebResult<MutationOutcome<()>> {
    // `base_url` is a typed `Option<BaseUrl>` wire arg (ADR-0065): the
    // validating serde bridge already rejected a malformed/non-http(s) value at
    // decode time, and an omitted field decodes to `None` (clearing-via-omit) —
    // no server-side parse/`non_empty` bridge is needed.
    let identity = SiteIdentity {
        title,
        tagline,
        base_url,
    };
    update_identity_impl(identity).await
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
