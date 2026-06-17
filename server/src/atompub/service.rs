//! `AtomPub` Service Document endpoint (`GET /atompub/service`).

use std::sync::Arc;

use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Extension;

use common::atompub::{render_service_document, CollectionDecl, ServiceDocument};
use storage::AppState;
use web::auth::AuthUser;

use super::{base_url, HandlerError};

/// Media types the media collection accepts.
const MEDIA_ACCEPT: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// `GET /atompub/service` — the authenticated user's `AtomPub` service document.
///
/// # Errors
///
/// Returns `500` if storage or serialization fails.
pub async fn service_document(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Response, HandlerError> {
    let base = base_url(&state).await;
    let username = auth_user.username.as_str();

    let categories = state
        .posts
        .list_tags(None, 100)
        .await?
        .into_iter()
        .map(|t| t.tag_slug.to_string())
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
