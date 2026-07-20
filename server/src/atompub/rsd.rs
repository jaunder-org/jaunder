//! `RSD` (Really Simple Discovery) endpoint for `AtomPub` autodiscovery.
//!
//! Editors like `MarsEdit` fetch `/~{username}/rsd.xml` (linked from the user
//! page via `<link rel="EditURI">`) to learn the `AtomPub` service URL.

use std::sync::Arc;

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;

use common::absolute_url::compose;
use common::atompub::render_rsd_document;
use common::username::Username;
use storage::SiteConfigStorage;

use super::base_url;

/// `GET /~{username}/rsd.xml` — the public `RSD` discovery document.
///
/// This is intentionally unauthenticated: it only advertises the `AtomPub`
/// service endpoint, which is itself protected.
///
/// # Errors
///
/// Infallible in practice; returns `Result` for handler-signature uniformity.
#[tracing::instrument(name = "atompub.rsd_document", skip_all)]
pub async fn rsd_document(
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Path(username): Path<Username>,
) -> Result<Response, StatusCode> {
    let base = base_url(site_config.as_ref()).await;
    let service_path = "/atompub/service".to_owned();
    let service_url = compose(base.as_ref(), &service_path).unwrap_or(service_path);
    let homepage_path = format!("/~{username}");
    let homepage_url = compose(base.as_ref(), &homepage_path).unwrap_or(homepage_path);
    let xml = render_rsd_document(&service_url, &homepage_url);

    Ok((
        [(header::CONTENT_TYPE, "application/rsd+xml;charset=utf-8")],
        xml,
    )
        .into_response())
}
