//! Server-side `AtomPub` surface: the boundary mapping Jaunder posts/media to
//! `AtomPub` wire types, plus the HTTP handlers.

use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use storage::AppState;
use web::auth::AuthUser;

pub mod mapping;
pub mod media;
pub mod posts;
pub mod rsd;
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
        .route("/~{username}/rsd.xml", get(rsd::rsd_document))
}

/// Authorizes that `auth_user` may act on resources scoped to `username`.
///
/// `AtomPub` collection handlers are addressed by `{username}`; a user may only
/// act on their own resources, so a mismatch yields `403 Forbidden`. The member
/// handlers fold the same check into `owned_post`.
pub(crate) fn require_user_match(auth_user: &AuthUser, username: &str) -> Result<(), StatusCode> {
    if auth_user.username.as_str() == username {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
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

/// Logs a genuine internal failure (typically a storage or I/O error) and maps
/// it to `500 Internal Server Error`.
///
/// The raw `AtomPub` handlers previously discarded these with
/// `.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)`, producing blank 500s with
/// nothing in the logs and no way to diagnose the failure. Routing them through
/// this helper records the underlying error at `error` level first. The error is
/// a storage/IO failure, not user content, so it carries no PII.
pub(crate) fn internal_error<E: std::error::Error>(err: E) -> StatusCode {
    tracing::error!(error = %err, "AtomPub handler internal error");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::internal_error;
    use axum::http::StatusCode;

    #[test]
    fn internal_error_maps_to_500() {
        assert_eq!(
            internal_error(sqlx::Error::PoolClosed),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
