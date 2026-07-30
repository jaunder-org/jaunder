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

#[server(endpoint = "/site/get_identity")]
#[tracing::instrument(name = "web.site.get_identity")]
pub async fn get_identity() -> WebResult<SiteIdentity> {
    boundary!("get_identity", {
        require_operator().await?;
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        site_config
            .get_identity()
            .await
            .map_err(InternalError::storage)
    })
}

#[server(endpoint = "/site/update_identity")]
#[tracing::instrument(name = "web.site.update_identity")]
pub async fn update_identity(title: SiteTitle, base_url: Option<AbsoluteUrl>) -> WebResult<()> {
    boundary!("update_identity", {
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
/// `backup::is_warning_visible`, this is a **soft** check — a non-operator or
/// unauthenticated caller yields `Ok(false)` (banner hidden), never an error — so the
/// banner degrades to absent rather than surfacing a failure in the chrome.
#[server(endpoint = "/site/is_base_url_warning_visible")]
#[tracing::instrument(name = "web.site.is_base_url_warning_visible")]
pub async fn is_base_url_warning_visible() -> WebResult<bool> {
    boundary!("is_base_url_warning_visible", {
        if !is_operator_soft().await? {
            return Ok(false);
        }
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        Ok(site_config.get_identity().await?.base_url.is_none())
    })
}
