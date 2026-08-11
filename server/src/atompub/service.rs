//! `AtomPub` Service Document endpoint (`GET /atompub/service`).

use std::sync::Arc;

use axum::Extension;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use common::atompub::{CollectionDecl, ServiceDocument, render_service_document};
use common::pagination::RowLimit;
use common::tagged_url::{CollectionHref, compose};
use storage::{PostStorage, SiteConfigStorage};
use web::auth::AuthUser;

use super::{HandlerError, required_base_url};

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
    let base = required_base_url(site_config.as_ref()).await?;
    let username = &*auth_user.username;

    // A flat cap on the service-document category list, not a page — there is no
    // pagination behind it, and 100 exceeds `PageSize`'s range by design (#696).
    let categories = posts
        .list_tags(None, RowLimit::at_most(100))
        .await?
        .into_iter()
        .map(|t| t.tag_slug)
        .collect();

    let posts_path = format!("/atompub/{username}/posts");
    let media_path = format!("/atompub/{username}/media");
    let doc = ServiceDocument {
        workspace_title: username.to_string(),
        posts_collection: CollectionDecl {
            // A struct-literal field cannot be ascribed, so the role is spelled as a
            // turbofish on the tag — the alias rule's stated exception.
            href: compose::<CollectionHref>(&base, &posts_path),
            title: "Posts".to_string(),
            accept: vec!["application/atom+xml;type=entry".to_string()],
            categories,
        },
        media_collection: CollectionDecl {
            href: compose::<CollectionHref>(&base, &media_path),
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
