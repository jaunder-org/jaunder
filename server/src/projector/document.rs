use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use common::etag::ETag;
use common::seed::{PageSeed, TimelinePage};
use web::app::{PREPAINT_SCRIPT, render_head, render_shell};
use web::error::{SwallowedSource, report_swallowed};
use web::posts::authored_post;

use super::Shell;

/// Assemble the full cacheable HTML document: per-page `<head>` SEO, the
/// `<div id="app">` full anonymous shell (so the CSR mount causes no reflow), the
/// JSON data blob, and the CSR boot.
#[must_use]
pub fn document(seed: &PageSeed) -> String {
    // Both arrive as `Markup` (trust is type-carried across the crate boundary);
    // this is where they exit to the untyped response body.
    let head = render_head(seed).into_string();
    let body = render_shell(seed).into_string();
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
pub(super) fn cacheable(headers: &HeaderMap, seed: &PageSeed) -> Response {
    let body = document(seed);
    let etag = ETag::sha256_of(body.as_bytes());

    if let Some(inm) = headers.get(header::IF_NONE_MATCH)
        && inm.to_str().ok() == Some(etag.as_ref())
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(etag.as_ref()) {
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
pub(super) fn shell_response(shell: &Shell) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Html(shell.0.to_string()),
    )
        .into_response()
}

/// Map a permalink lookup result to a response. Split from the handler so the
/// storage-error arm — otherwise reachable only under a live DB failure — stays
/// unit-testable.
pub(super) fn permalink_response(
    result: web::error::InternalResult<Option<storage::PostRecord>>,
    headers: &HeaderMap,
    shell: &Shell,
) -> Response {
    match result {
        // Anonymous viewer ⇒ never the author, so `is_author = false`.
        Ok(Some(record)) => cacheable(headers, &PageSeed::Permalink(authored_post(record, false))),
        // No *public* post here: a draft its author must see, or nothing at all.
        // Serve the shell so the CSR client resolves it with the session.
        Ok(None) => shell_response(shell),
        Err(error) => {
            error
                .with_context("boundary", "server.projector.permalink")
                .emit_boundary_failure();
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Map a timeline query result to a projected response, or a 500 on storage
/// error. Split from the handler so the error arm — otherwise reachable only
/// under a live DB failure — stays unit-testable; `into_seed` wraps the page in
/// its route's [`PageSeed`] variant.
pub(super) fn timeline_response(
    result: web::error::InternalResult<TimelinePage>,
    headers: &HeaderMap,
    into_seed: impl FnOnce(TimelinePage) -> PageSeed,
) -> Response {
    match result {
        Ok(page) => cacheable(headers, &into_seed(page)),
        Err(error) => {
            error
                .with_context("boundary", "server.projector.timeline")
                .emit_boundary_failure();
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Map a by-tag listing result to a projected response: the seed on success, or
/// the SPA shell on any error. Unlike [`timeline_response`], an error here serves
/// the shell (not a 500) — an unknown user or a live storage failure is never
/// public content, so the client resolves the URL per session. Split from the
/// handlers so the error arm — otherwise reachable only under a live DB failure —
/// stays unit-testable; `into_seed` wraps the page in its route's [`PageSeed`].
pub(super) fn tag_response(
    result: web::error::InternalResult<TimelinePage>,
    headers: &HeaderMap,
    shell: &Shell,
    context: &'static str,
    into_seed: impl FnOnce(TimelinePage) -> PageSeed,
) -> Response {
    match result {
        Ok(page) => cacheable(headers, &into_seed(page)),
        Err(error) => {
            report_swallowed(
                error.kind(),
                error.class(),
                context,
                SwallowedSource::Error(&error),
            );
            shell_response(shell)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Shell, permalink_response};
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
    fn tag_storage_error_serves_shell() {
        use super::{PageSeed, tag_response};
        let shell = Shell("shell".into());
        let resp = tag_response(
            Err(web::error::InternalError::validation("boom")),
            &HeaderMap::new(),
            &shell,
            "server.projector.test_tag",
            // `into_seed` is never called on the error path; any constructor works.
            PageSeed::SiteTimeline,
        );
        // The shell fallback (an unknown user / live storage failure) is a 200,
        // not a 500 — the client resolves the URL per session.
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn timeline_storage_error_maps_to_500() {
        use super::{PageSeed, timeline_response};
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
        use common::seed::{PageSeed, TimelinePage};
        let doc = document(&PageSeed::SiteTimeline(TimelinePage {
            posts: vec![],
            next_cursor: None,
            has_more: false,
        }));
        assert!(doc.contains(web::app::PREPAINT_SCRIPT), "{doc}");
        assert!(
            doc.contains("<head><script>(function()"),
            "prepaint is first in head: {doc}"
        );
    }

    #[test]
    fn document_boots_the_same_wasm_url_as_the_spa_shell() {
        use super::document;
        use common::seed::{PageSeed, TimelinePage};
        // Drift guard (#234): the projector's server-rendered boot and the SPA
        // shell (`csr/index.html`) are two hand-written copies — they must load the
        // SAME wasm URL, or the CSR boot 404s on projector routes. Cross-checking the
        // two (rather than asserting a literal against itself) means neither can
        // silently drift; `cargo xtask audit-wasm` ties that shared URL to the file
        // the build actually emits.
        fn boot_wasm_url(html: &str) -> &str {
            let marker = "init(\"";
            let start = html.find(marker).expect("boot script calls init(\"…\")") + marker.len();
            let rest = &html[start..];
            &rest[..rest.find('"').expect("init(\"…\") closing quote")]
        }
        let doc = document(&PageSeed::SiteTimeline(TimelinePage {
            posts: vec![],
            next_cursor: None,
            has_more: false,
        }));
        let spa_shell = include_str!("../../../csr/index.html");
        assert_eq!(
            boot_wasm_url(&doc),
            boot_wasm_url(spa_shell),
            "projector and csr/index.html must boot the same wasm URL (drift guard #234)"
        );
        assert_eq!(
            boot_wasm_url(&doc),
            web::app::WASM_URL,
            "the boot URL must be web::app::WASM_URL, the single definition every \
             copy of this path is checked against (#866)"
        );
    }
}
