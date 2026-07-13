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
use common::username::Username;
use common::visibility::ViewerIdentity;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use storage::{fetch_post_record, PostStorage, UserStorage};
use web::posts::{
    fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag,
    post_response,
};
use web::render::{render_head, render_shell, PageSeed, PREPAINT_SCRIPT};

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
        .route("/", get(site_timeline))
        .route("/~{username}", get(profile))
        .route("/~{username}/{year}/{month}/{day}/{slug}", get(permalink))
        .route("/tags/{tag}", get(site_tag))
        .route("/~{username}/tags/{tag}", get(user_tag))
        .layer(Extension(shell))
}

/// Assemble the full cacheable HTML document: per-page `<head>` SEO, the
/// `<div id="app">` full anonymous shell (so the CSR mount causes no reflow), the
/// JSON data blob, and the CSR boot.
#[must_use]
pub fn document(seed: &PageSeed) -> String {
    let head = render_head(seed);
    let body = render_shell(seed);
    let blob = serde_json::to_string(seed).unwrap_or_else(|_| "null".to_string());
    format!(
        concat!(
            // The pre-paint script is FIRST in <head> (#181, ADR-0044) so it runs
            // before any paint and marks html.authed for the owner.
            "<!DOCTYPE html><html lang=\"en\"><head>{prepaint}{head}</head><body>",
            "<div id=\"app\">{body}</div>",
            "<script type=\"application/json\" id=\"jaunder-seed\">{blob}</script>",
            "<script type=\"module\">import init from \"/pkg/jaunder.js\"; init(\"/pkg/jaunder.wasm\");</script>",
            "</body></html>",
        ),
        prepaint = PREPAINT_SCRIPT,
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

async fn site_timeline(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
) -> Response {
    let result = fetch_local_timeline(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        None,
        None,
        Some(50),
    )
    .await;
    timeline_response(result, &headers, PageSeed::SiteTimeline)
}

/// Map a timeline query result to a projected response, or a 500 on storage
/// error. Split from the handler so the error arm — otherwise reachable only
/// under a live DB failure — stays unit-testable; `into_seed` wraps the page in
/// its route's [`PageSeed`] variant.
fn timeline_response(
    result: web::error::InternalResult<web::posts::TimelinePage>,
    headers: &HeaderMap,
    into_seed: impl FnOnce(web::posts::TimelinePage) -> PageSeed,
) -> Response {
    match result {
        Ok(page) => cacheable(headers, &into_seed(page)),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn profile(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Response {
    // An unparseable username or a storage error is never public content — serve the
    // shell and let the client route it. A valid username (even an unknown one, which
    // yields an empty profile) is cached like any other public page.
    let seed = match username.parse::<Username>() {
        Ok(username) => fetch_user_posts(
            posts.as_ref(),
            &ViewerIdentity::Anonymous,
            &username,
            None,
            None,
            Some(50),
        )
        .await
        .ok()
        .map(|page| PageSeed::Profile { username, page }),
        Err(_) => None,
    };
    match seed {
        Some(seed) => cacheable(&headers, &seed),
        None => shell_response(&shell),
    }
}

async fn site_tag(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path(tag): Path<String>,
) -> Response {
    // Match `SiteTagPage`'s lowercasing so the projected heading and the client
    // render coincide.
    let tag = tag.to_lowercase();
    match fetch_posts_by_tag(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &tag,
        None,
        None,
        Some(50),
    )
    .await
    {
        Ok(page) => cacheable(&headers, &PageSeed::SiteTag { tag, page }),
        // An unparseable tag is never public content — let the client route it.
        Err(_) => shell_response(&shell),
    }
}

async fn user_tag(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(users): Extension<Arc<dyn UserStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path((username, tag)): Path<(String, String)>,
) -> Response {
    let tag = tag.to_lowercase();
    // An unparseable username, an unknown user, or a storage error is never public
    // content — serve the shell and let the client route it.
    let seed = match username.parse::<Username>() {
        Ok(username) => fetch_user_posts_by_tag(
            posts.as_ref(),
            users.as_ref(),
            &ViewerIdentity::Anonymous,
            &username,
            &tag,
            None,
            Some(50),
        )
        .await
        .ok()
        .map(|page| PageSeed::UserTag {
            username,
            tag,
            page,
        }),
        Err(_) => None,
    };
    match seed {
        Some(seed) => cacheable(&headers, &seed),
        None => shell_response(&shell),
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

    #[test]
    fn timeline_storage_error_maps_to_500() {
        use super::{timeline_response, PageSeed};
        let resp = timeline_response(
            Err(web::error::InternalError::validation("boom")),
            &HeaderMap::new(),
            PageSeed::SiteTimeline,
        );
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn document_head_starts_with_the_prepaint_script() {
        use super::document;
        use web::render::PageSeed;
        let doc = document(&PageSeed::SiteTimeline(web::posts::TimelinePage {
            posts: vec![],
            next_cursor_created_at: None,
            next_cursor_post_id: None,
            has_more: false,
        }));
        assert!(doc.contains(web::render::PREPAINT_SCRIPT), "{doc}");
        assert!(
            doc.contains("<head><script>(function()"),
            "prepaint is first in head: {doc}"
        );
    }

    #[test]
    fn document_boots_the_same_wasm_url_as_the_spa_shell() {
        use super::document;
        use web::render::PageSeed;
        // Drift guard (#234): the projector's server-rendered boot and the SPA
        // shell (`csr/index.html`) are two hand-written copies — they must load the
        // SAME wasm URL, or hydration 404s on projector routes. Cross-checking the
        // two (rather than asserting a literal against itself) means neither can
        // silently drift; `cargo xtask audit-wasm` ties that shared URL to the file
        // the build actually emits.
        fn boot_wasm_url(html: &str) -> &str {
            let marker = "init(\"";
            let start = html.find(marker).expect("boot script calls init(\"…\")") + marker.len();
            let rest = &html[start..];
            &rest[..rest.find('"').expect("init(\"…\") closing quote")]
        }
        let doc = document(&PageSeed::SiteTimeline(web::posts::TimelinePage {
            posts: vec![],
            next_cursor_created_at: None,
            next_cursor_post_id: None,
            has_more: false,
        }));
        let spa_shell = include_str!("../../../csr/index.html");
        assert_eq!(
            boot_wasm_url(&doc),
            boot_wasm_url(spa_shell),
            "projector and csr/index.html must boot the same wasm URL (drift guard #234)"
        );
    }
}
