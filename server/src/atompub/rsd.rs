//! `RSD` (Really Simple Discovery) endpoint for `AtomPub` autodiscovery.
//!
//! Editors like `MarsEdit` fetch `/~{username}/rsd.xml` (linked from the user
//! page via `<link rel="EditURI">`) to learn the `AtomPub` service URL.

use std::sync::Arc;

use axum::extract::Path;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Extension;

use common::absolute_url::compose;
use common::atompub::render_rsd_document;
use common::username::Username;
use storage::SiteConfigStorage;

use super::{required_base_url, HandlerError};

/// `GET /~{username}/rsd.xml` — the public `RSD` discovery document.
///
/// This is intentionally unauthenticated: it only advertises the `AtomPub`
/// service endpoint, which is itself protected.
///
/// # Errors
///
/// Returns `500` ([`HandlerError::BaseUrlRequired`], logged) when `site.base_url` is
/// unset — the discovery URLs cannot be composed absolute without it (#560).
#[tracing::instrument(name = "atompub.rsd_document", skip_all)]
pub async fn rsd_document(
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Path(username): Path<Username>,
) -> Result<Response, HandlerError> {
    let base = required_base_url(site_config.as_ref()).await?;
    let service_path = "/atompub/service".to_owned();
    let service_url = compose(&base, &service_path);
    let homepage_path = format!("/~{username}");
    let homepage_url = compose(&base, &homepage_path);
    let xml = render_rsd_document(&service_url, &homepage_url);

    Ok((
        [(header::CONTENT_TYPE, "application/rsd+xml;charset=utf-8")],
        xml,
    )
        .into_response())
}
