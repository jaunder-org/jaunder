//! Server-side `AtomPub` surface: the boundary mapping Jaunder posts/media to
//! `AtomPub` wire types, plus the HTTP handlers.

use axum::routing::{get, post};
use axum::Router;
use storage::AppState;

pub mod mapping;
pub mod media;
pub mod posts;
pub mod service;

/// Builds the `AtomPub` routes (mergeable into the main application router).
///
/// The handlers read shared state via `Extension`, so the routes are generic
/// over the application's router state type.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/atompub/service", get(service::service_document))
        .route(
            "/atompub/{username}/posts",
            get(posts::collection_get).post(posts::collection_post),
        )
        .route(
            "/atompub/{username}/posts/{post_id}",
            get(posts::member_get)
                .put(posts::member_put)
                .delete(posts::member_delete),
        )
        .route("/atompub/{username}/media", post(media::collection_post))
        .route(
            "/atompub/{username}/media/{sha}/{filename}",
            get(media::member_get).delete(media::member_delete),
        )
}

/// Returns the site's public base URL (scheme + host, no trailing slash), or an
/// empty string when unconfigured (callers then emit root-relative URLs).
pub(crate) async fn base_url(state: &AppState) -> String {
    state
        .site_config
        .get_identity()
        .await
        .ok()
        .and_then(|identity| identity.base_url)
        .unwrap_or_default()
}
