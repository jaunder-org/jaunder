use crate::error::WebResult;
use common::absolute_url::AbsoluteUrl;
use common::site::SiteIdentity;
use leptos::prelude::*;

#[cfg(feature = "server")]
use crate::backup::server::require_operator;

#[cfg(feature = "server")]
use {crate::error::InternalError, std::sync::Arc, storage::SiteConfigStorage};

#[server(endpoint = "/get_site_identity")]
#[tracing::instrument(name = "web.site.get_identity")]
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
#[tracing::instrument(name = "web.site.update_identity", skip(title, base_url))]
pub async fn update_site_identity(title: String, base_url: Option<AbsoluteUrl>) -> WebResult<()> {
    boundary!("update_site_identity", {
        require_operator().await?;

        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(InternalError::validation("site title cannot be empty"));
        }

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
