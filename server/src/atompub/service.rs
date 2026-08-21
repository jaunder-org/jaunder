//! `AtomPub` Service Document endpoint (`GET /atompub/service`).

use std::sync::Arc;

use axum::Extension;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use common::atompub::{
    CollectionDecl, CollectionTitle, ServiceDocument, WorkspaceTitle, render_service_document,
};
use common::media::ContentType;
use common::pagination::RowLimit;
use common::tagged_url::{CollectionHref, compose};
use storage::{PostStorage, SiteConfigStorage};
use web::auth::AuthUser;

use super::{HandlerError, required_base_url};

fn media_accept() -> Vec<ContentType> {
    vec![
        ContentType::image_png(),
        ContentType::image_jpeg(),
        ContentType::image_gif(),
        ContentType::image_webp(),
    ]
}

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
    let username = &auth_user.username;

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
        workspace_title: WorkspaceTitle::for_user(username),
        posts_collection: CollectionDecl {
            // A struct-literal field cannot be ascribed, so the role is spelled as a
            // turbofish on the tag — the alias rule's stated exception.
            href: compose::<CollectionHref>(&base, &posts_path),
            title: CollectionTitle::posts(),
            accept: vec![ContentType::atom_entry()],
            categories,
        },
        media_collection: CollectionDecl {
            href: compose::<CollectionHref>(&base, &media_path),
            title: CollectionTitle::media(),
            accept: media_accept(),
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
