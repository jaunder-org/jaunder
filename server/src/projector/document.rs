use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use common::seed::{PageSeed, PublicPresentation};
use host::etag;
use web::app;
use web::posts;

use super::Shell;

/// Assemble a document from a server-resolved public presentation.
#[must_use]
pub fn document_presentation(presentation: &PublicPresentation<PageSeed>) -> String {
    let head = app::render_head(&presentation.page).into_string();
    let body = app::render_shell(presentation).into_string();
    let blob = serde_json::to_string(presentation).unwrap_or_else(|_| "null".to_string());
    format!(
        concat!(
            // The pre-paint script is FIRST in <head> (#181, ADR-0044) so it runs
            // before any paint and marks html.authed for the owner.
            "<!DOCTYPE html><html lang=\"en\"><head>{prepaint}{head}</head><body>",
            "<div id=\"app\">{body}</div>",
            "<script type=\"application/json\" id=\"jaunder-seed\">{blob}</script>",
            "<script type=\"module\">import {{initMeasured}} from \"{glue_url}\"; performance.mark(\"{module_before_init_mark}\"); initMeasured(window.__jaunderWasmFetch ?? \"{wasm_url}\");</script>",
            "</body></html>",
        ),
        prepaint = app::PREPAINT_SCRIPT,
        head = head,
        body = body,
        glue_url = app::GLUE_URL,
        module_before_init_mark = app::MODULE_BEFORE_INIT_MARK,
        wasm_url = app::WASM_URL,
        // A verbatim `</script` inside the JSON would close the blob script
        // early; `<\/` is an equivalent JSON escape the parser reads back as
        // `</`. This is the only HTML-in-JSON breakout to neutralize.
        blob = blob.replace("</", "<\\/"),
    )
}

/// Build a cacheable response from the route's already resolved presentation.
pub(super) fn cacheable_presentation(
    headers: &HeaderMap,
    presentation: &PublicPresentation<PageSeed>,
) -> Response {
    let body = document_presentation(presentation);
    let etag = etag::sha256_of(body.as_bytes());

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
    theme: common::theme::Theme,
) -> Response {
    match result {
        // Anonymous viewer ⇒ never the author, so `is_author = false`.
        Ok(Some(record)) => cacheable_presentation(
            headers,
            &PublicPresentation {
                theme,
                page: PageSeed::Permalink(posts::authored_post(record, false)),
            },
        ),
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

#[cfg(test)]
mod tests {
    use super::{Shell, cacheable_presentation, document_presentation, permalink_response};
    use axum::http::{HeaderMap, StatusCode, header};
    use common::{
        seed::{Page, PageSeed, PublicPresentation},
        theme::Theme,
    };

    fn presentation(theme: Theme) -> PublicPresentation<PageSeed> {
        PublicPresentation {
            theme,
            page: PageSeed::SiteTimeline(Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            }),
        }
    }

    #[test]
    fn permalink_storage_error_maps_to_500() {
        let shell = Shell("shell".into());
        let response = permalink_response(
            Err(web::error::InternalError::validation("boom")),
            &HeaderMap::new(),
            &shell,
            Theme::Studio,
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn absent_public_permalink_serves_shell() {
        let shell = Shell("shell".into());
        let response = permalink_response(Ok(None), &HeaderMap::new(), &shell, Theme::Studio);
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn identical_presentations_have_identical_bytes_and_etags() {
        let presentation = presentation(Theme::Terminal);
        assert_eq!(
            document_presentation(&presentation),
            document_presentation(&presentation)
        );

        let first = cacheable_presentation(&HeaderMap::new(), &presentation);
        let second = cacheable_presentation(&HeaderMap::new(), &presentation);
        assert_eq!(
            first.headers()[header::ETAG],
            second.headers()[header::ETAG]
        );
    }

    #[test]
    fn changing_theme_changes_projector_bytes_and_etag() {
        let terminal = presentation(Theme::Terminal);
        let reader = presentation(Theme::Reader);
        assert_ne!(
            document_presentation(&terminal),
            document_presentation(&reader)
        );

        let terminal_response = cacheable_presentation(&HeaderMap::new(), &terminal);
        let reader_response = cacheable_presentation(&HeaderMap::new(), &reader);
        assert_ne!(
            terminal_response.headers()[header::ETAG],
            reader_response.headers()[header::ETAG]
        );
    }
}
