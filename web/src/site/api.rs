use crate::error::WebResult;
use common::MutationOutcome;
use common::site::{SiteIdentity, SiteTagline, SiteTitle};
use common::tagged_url::BaseUrl;
use common::visibility::DefaultAudience;
use serde::{Deserialize, Serialize};

/// One cohesive Site Settings identity mutation (ADR-0129).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateIdentityRequest {
    pub title: SiteTitle,
    pub tagline: Option<SiteTagline>,
    pub base_url: Option<BaseUrl>,
}

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
pub async fn get_default_audience() -> WebResult<DefaultAudience> {
    auth::require_operator().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    site_config
        .get_default_audience()
        .await
        .map_err(InternalError::storage)
}

#[macros::server(skip_all)]
pub async fn update_default_audience(audience: DefaultAudience) -> WebResult<MutationOutcome<()>> {
    auth::require_operator().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                site_config
                    .set_default_audience(transaction, &audience)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
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

#[macros::server(skip_all)]
pub async fn update_identity(request: UpdateIdentityRequest) -> WebResult<MutationOutcome<()>> {
    // The aggregate's fields are validated at the typed wire boundary (ADR-0065).
    update_identity_impl(SiteIdentity {
        title: request.title,
        tagline: request.tagline,
        base_url: request.base_url,
    })
    .await
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

#[cfg(test)]
mod tests {
    use super::UpdateDefaultAudience;
    use common::visibility::DefaultAudience;

    #[test]
    fn site_default_audience_wire_accepts_only_closed_values() {
        let request: UpdateDefaultAudience = serde_qs::from_str("audience=subscribers").unwrap();
        assert_eq!(request.audience, DefaultAudience::Subscribers);
        assert!(serde_qs::from_str::<UpdateDefaultAudience>("audience=friends").is_err());
    }
}
