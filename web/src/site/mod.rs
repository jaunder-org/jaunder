use crate::error::WebResult;
use common::site::SiteIdentity;
use leptos::prelude::*;

#[cfg(feature = "server")]
use crate::backup::server::require_operator;

#[cfg(feature = "server")]
use {crate::error::InternalError, std::sync::Arc, storage::SiteConfigStorage};

#[server(endpoint = "/get_site_identity")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.site.get_identity")
)]
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
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.site.update_identity", skip(title, base_url))
)]
pub async fn update_site_identity(title: String, base_url: String) -> WebResult<()> {
    boundary!("update_site_identity", {
        require_operator().await?;

        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(InternalError::validation("site title cannot be empty"));
        }

        let base_url = match common::text::non_empty(&base_url) {
            None => None,
            Some(trimmed) => {
                let trimmed = trimmed.trim_end_matches('/');
                if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                    return Err(InternalError::validation(
                        "base URL must be an absolute http or https URL",
                    ));
                }
                Some(trimmed.to_string())
            }
        };

        let identity = SiteIdentity { title, base_url };
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        site_config
            .set_identity(&identity)
            .await
            .map_err(InternalError::storage)
    })
}
