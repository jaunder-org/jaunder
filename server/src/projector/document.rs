use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use common::seed::{PageSeed, PublicPresentation};
use host::etag;
use web::app;
use web::posts;

use crate::bundle;

use super::Shell;

/// Assemble a document from a server-resolved public presentation.
#[must_use]
pub fn document_presentation(presentation: &PublicPresentation<PageSeed>) -> String {
    document_with_urls(presentation, bundle::boot_urls())
}

fn document_with_urls(
    presentation: &PublicPresentation<PageSeed>,
    urls: Option<bundle::BootUrls>,
) -> String {
    // Both arrive as `Markup` (trust is type-carried across the crate boundary);
    // this is where they exit to the untyped response body.
    let seed = &presentation.page;
    let early_fetch = urls.map(bundle::early_wasm_fetch_script);
    let mut head = app::render_head(seed, early_fetch.as_deref()).into_string();
    head.push_str(&app::render_theme_stylesheet(&presentation.theme).into_string());
    let body = app::render_shell(presentation).into_string();
    let blob = serde_json::to_string(presentation).unwrap_or_else(|_| "null".to_string());
    let boot = urls.map_or_else(String::new, |urls| {
        bundle::module_init_script(urls, app::MODULE_BEFORE_INIT_MARK)
    });
    format!(
        concat!(
            "<!DOCTYPE html><html lang=\"en\"><head>{prepaint}{head}</head><body>",
            "<div id=\"app\">{body}</div>",
            "<script type=\"application/json\" id=\"jaunder-seed\">{blob}</script>{boot}",
            "</body></html>",
        ),
        prepaint = app::PREPAINT_SCRIPT,
        head = head,
        body = body,
        boot = boot,
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
    theme: common::theme::PublishedThemePresentation,
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
            theme: common::theme::PublishedThemePresentation::built_in(theme),
            page: PageSeed::SiteTimeline(Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            }),
        }
    }

    fn custom_presentation(
        revision: char,
        stylesheet: char,
        logo: char,
        header: char,
    ) -> PublicPresentation<PageSeed> {
        PublicPresentation {
            theme: common::theme::PublishedThemePresentation {
                identity: common::theme::PublishedThemeIdentity::Custom(
                    common::ids::ThemeId::from(42),
                ),
                revision: Some(revision.to_string().repeat(64).parse().unwrap()),
                stylesheet_url: format!("/theme/{}", stylesheet.to_string().repeat(64))
                    .parse()
                    .unwrap(),
                logo_url: Some(
                    format!("/theme/{}", logo.to_string().repeat(64))
                        .parse()
                        .unwrap(),
                ),
                header_url: Some(
                    format!("/theme/{}", header.to_string().repeat(64))
                        .parse()
                        .unwrap(),
                ),
            },
            page: presentation(Theme::Studio).page,
        }
    }

    #[test]
    fn permalink_storage_error_maps_to_500() {
        let shell = Shell("shell".into());
        let response = permalink_response(
            Err(web::error::InternalError::validation("boom")),
            &HeaderMap::new(),
            &shell,
            common::theme::PublishedThemePresentation::built_in(Theme::Studio),
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn custom_revision_and_role_bindings_change_document_bytes_and_etag() {
        let baseline = custom_presentation('a', 'b', 'c', 'd');
        let revision_changed = custom_presentation('e', 'b', 'c', 'd');
        let binding_changed = custom_presentation('a', 'b', 'f', 'd');
        let pool_changed = custom_presentation('a', 'b', 'c', 'f');

        for changed in [&revision_changed, &binding_changed, &pool_changed] {
            assert_ne!(
                document_presentation(&baseline),
                document_presentation(changed)
            );
            let baseline_etag = cacheable_presentation(&HeaderMap::new(), &baseline);
            let changed_etag = cacheable_presentation(&HeaderMap::new(), changed);
            assert_ne!(
                baseline_etag.headers()[header::ETAG],
                changed_etag.headers()[header::ETAG]
            );
        }
    }

    #[test]
    fn custom_presentation_puts_marked_css_in_head_and_decorations_in_shell() {
        let document = document_presentation(&custom_presentation('a', 'b', 'c', 'd'));
        let stylesheet = document
            .find("data-jaunder-theme-stylesheet")
            .expect("custom stylesheet");
        assert!(
            stylesheet < document.find("<body>").expect("body"),
            "{document}"
        );
        assert_eq!(document.matches("data-jaunder-part=\"logo\"").count(), 1);
        assert_eq!(
            document
                .matches("data-jaunder-part=\"header-image\"")
                .count(),
            1
        );
    }

    #[test]
    fn absent_public_permalink_serves_shell() {
        let shell = Shell("shell".into());
        let response = permalink_response(
            Ok(None),
            &HeaderMap::new(),
            &shell,
            common::theme::PublishedThemePresentation::built_in(Theme::Studio),
        );
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

    #[test]
    fn projector_shell_uses_each_host_url_once_in_boot_order() {
        use super::document_with_urls;

        let urls = crate::bundle::BootUrls {
            glue: "/pkg/glue-content-hash.js",
            wasm: "/pkg/wasm-content-hash.wasm",
        };
        let doc = document_with_urls(&presentation(Theme::Studio), Some(urls));

        for url in [urls.glue, urls.wasm] {
            assert_eq!(doc.matches(url).count(), 1, "{doc}");
        }
        let fetch = doc
            .find("window.__jaunderWasmFetch = fetch")
            .expect("early fetch");
        let stylesheet = doc
            .find(r#"<link rel="stylesheet" href="/style/jaunder.css">"#)
            .expect("stylesheet");
        let import = doc.find("import {initMeasured}").expect("glue import");
        let mark = doc.find("performance.mark").expect("init mark");
        let init = doc
            .find("initMeasured(window.__jaunderWasmFetch")
            .expect("initializer");
        assert!(
            fetch < stylesheet && stylesheet < import && import < mark && mark < init,
            "{doc}"
        );
        assert!(
            !doc.contains("modulepreload") && !doc.contains(r#"rel="preload""#),
            "{doc}"
        );
    }

    #[test]
    fn local_no_bundle_projector_omits_boot_scripts() {
        use super::document_with_urls;

        let doc = document_with_urls(&presentation(Theme::Studio), None);
        assert!(
            !doc.contains("initMeasured") && !doc.contains("__jaunderWasmFetch"),
            "{doc}"
        );
    }
}
