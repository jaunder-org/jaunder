//! Non-reactive server-side HTML for the public discoverability routes (#178).
//!
//! When a public URL resolves to **public** content, the projector emits one
//! cacheable document (semantic content, an embedded `#jaunder-seed` data blob,
//! and the CSR boot script) with **no `reactive_graph` on the request path**,
//! the #173 escape and the same posture as the feed handlers. Every page renders
//! the **anonymous** view (`ViewerIdentity::Anonymous`), so the bytes are
//! identical per URL for every visitor — CDN-cacheable.
//!
//! When the URL has no anonymous-public content (a draft the author must see,
//! a not-yet-existing post, an unparseable segment), the projector serves the
//! **SPA shell** instead — identical to the pre-projector fallback — so the CSR
//! client boots and resolves it with the viewer's session (drafts, client-side
//! 404s, and the authed owner's affordances all keep working). The projector
//! only ever *adds* server rendering for content that is already public.
//!
//! `register`/`document`/the handlers are always compiled (so they stay
//! unit-testable and covered under default features); they are wired into the
//! axum router only under `--features csr` (see `create_router`), ahead of the
//! static-SPA fallback.

use axum::{
    extract::{Extension, Path},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use common::visibility::ViewerIdentity;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use storage::PostStorage;
use web::posts::{fetch_post_record, post_response};
use web::render::{render_body, render_head, PageSeed};

/// The static SPA shell (`index.html`) the projector falls back to when a public
/// URL has no anonymous-public content. Cheap to clone (shared `Arc`).
#[derive(Clone)]
pub struct Shell(pub Arc<str>);

/// Register the public projector routes. Generic over the router state because
/// the handlers extract only request `Extension`s (the storage traits + the
/// shell), never `State`, so they compose onto the live `Router<LeptosOptions>`
/// in `create_router` and a bare `Router<()>` in tests alike.
///
/// Only the permalink route lands here for now; the profile / timeline / tag
/// routes arrive with their verticals. Until then those URLs keep hitting the
/// SPA fallback unchanged.
pub fn register<S>(router: Router<S>, shell: Shell) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .route("/~{username}/{year}/{month}/{day}/{slug}", get(permalink))
        .layer(Extension(shell))
}

/// Assemble the full cacheable HTML document: per-page `<head>` SEO, the
/// `<div id="app">` semantic content, the JSON data blob, and the CSR boot.
#[must_use]
pub fn document(seed: &PageSeed) -> String {
    let head = render_head(seed);
    let body = render_body(seed);
    let blob = serde_json::to_string(seed).unwrap_or_else(|_| "null".to_string());
    format!(
        concat!(
            "<!DOCTYPE html><html lang=\"en\"><head>{head}</head><body>",
            "<div id=\"app\">{body}</div>",
            "<script type=\"application/json\" id=\"jaunder-seed\">{blob}</script>",
            "<script type=\"module\">import init from \"/pkg/jaunder.js\"; init();</script>",
            "</body></html>",
        ),
        head = head,
        body = body,
        // A verbatim `</script` inside the JSON would close the blob script
        // early; `<\/` is an equivalent JSON escape the parser reads back as
        // `</`. This is the only HTML-in-JSON breakout to neutralize.
        blob = blob.replace("</", "<\\/"),
    )
}

/// Build a 200 response for `seed` — with a strong `ETag` (content hash, feed
/// convention) and cache headers — or a 304 when the client's `If-None-Match`
/// already matches. Identical `seed` ⇒ identical bytes ⇒ identical `ETag`.
fn cacheable(headers: &HeaderMap, seed: &PageSeed) -> Response {
    let body = document(seed);
    let etag = format!("\"sha256-{:x}\"", Sha256::digest(body.as_bytes()));

    if let Some(inm) = headers.get(header::IF_NONE_MATCH) {
        if inm.to_str().ok() == Some(etag.as_str()) {
            return StatusCode::NOT_MODIFIED.into_response();
        }
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(&etag) {
        resp_headers.insert(header::ETAG, value);
    }
    resp_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    (StatusCode::OK, resp_headers, body).into_response()
}

/// Serve the SPA shell for a URL with no anonymous-public content. Not cached as
/// the URL's content — the client resolves it per session (auth/draft/404).
fn shell_response(shell: &Shell) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Html(shell.0.to_string()),
    )
        .into_response()
}

async fn permalink(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path((username, year, month, day, slug)): Path<(String, i32, u32, u32, String)>,
) -> Response {
    let (Ok(username), Ok(slug)) = (username.parse(), slug.parse()) else {
        // An unparseable segment is never public content — let the client route
        // it (it may be a server URL the SPA reloads for).
        return shell_response(&shell);
    };
    let result = fetch_post_record(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &username,
        year,
        month,
        day,
        &slug,
    )
    .await;
    permalink_response(result, &headers, &shell)
}

/// Map a permalink lookup result to a response. Split from the handler so the
/// storage-error arm — otherwise reachable only under a live DB failure — stays
/// unit-testable.
fn permalink_response(
    result: web::error::InternalResult<Option<storage::PostRecord>>,
    headers: &HeaderMap,
    shell: &Shell,
) -> Response {
    match result {
        // Anonymous viewer ⇒ never the author, so `is_author = false`.
        Ok(Some(record)) => cacheable(headers, &PageSeed::Permalink(post_response(record, false))),
        // No *public* post here: a draft its author must see, or nothing at all.
        // Serve the shell so the CSR client resolves it with the session.
        Ok(None) => shell_response(shell),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{permalink_response, Shell};
    use axum::http::{HeaderMap, StatusCode};

    #[test]
    fn storage_error_maps_to_500() {
        let shell = Shell("shell".into());
        let resp = permalink_response(
            Err(web::error::InternalError::validation("boom")),
            &HeaderMap::new(),
            &shell,
        );
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn no_public_post_serves_shell() {
        let shell = Shell("shell".into());
        let resp = permalink_response(Ok(None), &HeaderMap::new(), &shell);
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
