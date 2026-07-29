use crate::error::WebResult;
use common::absolute_url::AbsoluteUrl;
use common::site::{SiteIdentity, SiteTitle};
use leptos::prelude::*;

#[cfg(feature = "server")]
use {
    crate::auth::{is_operator_soft, require_operator},
    crate::error::InternalError,
    std::sync::Arc,
    storage::SiteConfigStorage,
};

#[server(endpoint = "/get_site_identity")]
#[tracing::instrument(name = "web.site.get_site_identity")]
pub async fn get_site_identity() -> WebResult<SiteIdentity> {
    boundary!("get_site_identity", {
        require_operator().await?;
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        site_config
            .get_identity()
            .await
            .map_err(InternalError::storage)
    })
}

#[server(endpoint = "/update_site_identity")]
#[tracing::instrument(name = "web.site.update_site_identity")]
pub async fn update_site_identity(
    title: SiteTitle,
    base_url: Option<AbsoluteUrl>,
) -> WebResult<()> {
    boundary!("update_site_identity", {
        require_operator().await?;

        // `base_url` is a typed `Option<AbsoluteUrl>` wire arg (ADR-0065): the
        // validating serde bridge already rejected a malformed/non-http(s) value at
        // decode time, and an omitted field decodes to `None` (clearing-via-omit) —
        // no server-side parse/`non_empty` bridge is needed.
        let identity = SiteIdentity { title, base_url };
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        site_config
            .set_identity(&identity)
            .await
            .map_err(InternalError::storage)
    })
}

/// Whether to show the "site base URL not configured" warning banner (#575): `true`
/// only for an operator when `SiteIdentity.base_url` is `None`. Like
/// `backup_warning_visible`, this is a **soft** check — a non-operator or
/// unauthenticated caller yields `Ok(false)` (banner hidden), never an error — so the
/// banner degrades to absent rather than surfacing a failure in the chrome.
#[server(endpoint = "/base_url_warning_visible")]
#[tracing::instrument(name = "web.site.base_url_warning_visible")]
pub async fn base_url_warning_visible() -> WebResult<bool> {
    boundary!("base_url_warning_visible", {
        if !is_operator_soft().await? {
            return Ok(false);
        }
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        Ok(site_config.get_identity().await?.base_url.is_none())
    })
}
