//! `AtomPub` Service Document endpoint (`GET /atompub/service`).

use std::sync::Arc;

use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Extension;

use common::atompub::{render_service_document, CollectionDecl, ServiceDocument};
use storage::{PostStorage, SiteConfigStorage};
use web::auth::AuthUser;

use super::{base_url, HandlerError};

/// Media types the media collection accepts.
const MEDIA_ACCEPT: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// `GET /atompub/service` — the authenticated user's `AtomPub` service document.
///
/// # Errors
///
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.service_document", skip_all)]
pub async fn service_document(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: AuthUser,
) -> Result<Response, HandlerError> {
    let base = base_url(site_config.as_ref()).await;
    let username = &*auth_user.username;

    let categories = posts
        .list_tags(None, 100)
        .await?
        .into_iter()
        .map(|t| t.tag_slug)
        .collect();

    let doc = ServiceDocument {
        workspace_title: username.to_string(),
        posts_collection: CollectionDecl {
            href: format!("{base}/atompub/{username}/posts"),
            title: "Posts".to_string(),
            accept: vec!["application/atom+xml;type=entry".to_string()],
            categories,
        },
        media_collection: CollectionDecl {
            href: format!("{base}/atompub/{username}/media"),
            title: "Media".to_string(),
            accept: MEDIA_ACCEPT.iter().map(|s| (*s).to_string()).collect(),
            categories: Vec::new(),
        },
    };

    let xml = render_service_document(&doc);
    Ok((
        [(
            header::CONTENT_TYPE,
            "application/atomsvc+xml;charset=utf-8",
        )],
        xml,
    )
        .into_response())
}
