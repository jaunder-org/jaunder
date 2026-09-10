//! The embedded CSR site tree + a precompression-aware serving handler.
//!
//! `server/build.rs` verifies a bundle manifest, stages its rendered `index.html`
//! and content-addressed `pkg/**` representations (plus `public/` assets) into
//! `$OUT_DIR/site/`; [`Site`] embeds the result (#237, #869, ADR-0003).
//! This replaces the disk `ServeDir::new(&site_root)` fallback, so a released
//! binary serves its own client with no external files.
//!
//! `axum-embed`'s `ServeEmbed` does no `Accept-Encoding` negotiation, so
//! [`serve_site`] is a small custom handler: it negotiates br/gzip/identity
//! against the embedded precompressed variants, sets `Content-Type` from the
//! *logical* path, emits a per-representation `ETag`, and honors
//! `If-None-Match` (→ `304`). A path with no embedded file falls through to the
//! SPA shell, exactly as `ServeDir(...).fallback(spa_shell)` did.
//!
//! The header/status logic lives in **pure functions** ([`choose_encoding`],
//! [`content_type_for`], [`not_modified`], [`build_response`]) that
//! are unit-tested without a live embed. The `Site` lookup itself is exercised
//! end-to-end by [`serve_site`]'s integration tests: the Nix coverage build
//! stages the real bundle (`nix/checks.nix` sets `JAUNDER_CSR_BUNDLE_DIR`), so
//! a populated [`Site`] is measured under instrumentation.

use std::{borrow::Cow, sync::Arc};

use axum::body::Bytes;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use common::etag::ETag;
use host::etag;
use rust_embed::RustEmbed;

// cov:ignore-start: RustEmbed derive expansion is compiler-generated rather than handwritten runtime behavior.
#[derive(RustEmbed)]
// cov:ignore-stop
#[folder = "$OUT_DIR/site"]
pub struct Site;

/// The content coding chosen for a response representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Br,
    Gzip,
    Identity,
}

impl Encoding {
    /// The `Content-Encoding` header value, or `None` for the identity coding
    /// (which carries no `Content-Encoding`).
    fn content_encoding(self) -> Option<&'static str> {
        match self {
            Encoding::Br => Some("br"),
            Encoding::Gzip => Some("gzip"),
            Encoding::Identity => None,
        }
    }
}

/// Pick the best available content coding for a request.
///
/// Prefer `br` when the client accepts it and a `.br` variant exists; else
/// `gzip` under the same rule; else identity. A simple comma/`;`-token match on
/// `Accept-Encoding` (q-values are not weighed — good enough for a static
/// bundle).
fn choose_encoding(accept_encoding: Option<&str>, has_br: bool, has_gz: bool) -> Encoding {
    let accept = accept_encoding.unwrap_or_default();
    if has_br && accepts(accept, "br") {
        Encoding::Br
    } else if has_gz && accepts(accept, "gzip") {
        Encoding::Gzip
    } else {
        Encoding::Identity
    }
}

/// Whether an `Accept-Encoding` header accepts `coding`: the token appears
/// (case-insensitively, whitespace-trimmed) and is not explicitly rejected with
/// `q=0` (RFC 9110 — a zero qvalue means "not acceptable"). Other qvalues aren't
/// ranked (fine for a two-choice static bundle).
fn accepts(accept_encoding: &str, coding: &str) -> bool {
    accept_encoding.split(',').any(|part| {
        let mut segments = part.split(';');
        let token = segments.next().unwrap_or("").trim();
        if !token.eq_ignore_ascii_case(coding) {
            return false;
        }
        // A `q=0` weight explicitly rejects the coding.
        !segments.any(|param| {
            param
                .trim()
                .to_ascii_lowercase()
                .strip_prefix("q=")
                .and_then(|q| q.trim().parse::<f32>().ok())
                .is_some_and(|q| q == 0.0)
        })
    })
}

/// The `Content-Type` for a *logical* path (no `.br`/`.gz` suffix), via
/// `mime_guess`. `.wasm` → `application/wasm`, `.js` → `text/javascript`
/// (`mime_guess` 2.x); unknown extensions fall back to
/// `application/octet-stream`.
fn content_type_for(logical_path: &str) -> String {
    mime_guess::from_path(logical_path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}

/// Whether an `If-None-Match` header matches `etag` (a validator in the list is
/// enough → the representation is unchanged, serve `304`).
fn not_modified(if_none_match: Option<&str>, etag: &ETag) -> bool {
    if_none_match.is_some_and(|inm| inm.split(',').any(|tag| *etag == tag.trim()))
}

/// The embedded key for a logical path under a chosen coding: `<path>.br`,
/// `<path>.gz`, or the bare path for identity. Pure — unit-tested for all three.
fn variant_path(logical: &str, encoding: Encoding) -> String {
    match encoding {
        Encoding::Br => format!("{logical}.br"),
        Encoding::Gzip => format!("{logical}.gz"),
        Encoding::Identity => logical.to_owned(),
    }
}

/// Return the embedded generated shell, or the explicit local no-bundle fallback.
#[must_use]
fn shell_html_from(bytes: Option<&[u8]>) -> Arc<str> {
    bytes.map_or_else(
        || Arc::from("<!doctype html><title>CSR bundle unavailable</title>"),
        |bytes| Arc::from(String::from_utf8_lossy(bytes).into_owned()),
    )
}

/// Return the embedded generated shell, or the explicit local no-bundle fallback.
#[must_use]
pub fn shell_html() -> Arc<str> {
    shell_html_from(
        Site::get("index.html")
            .as_ref()
            .map(|file| file.data.as_ref()),
    )
}

/// The embedded static shell is the bundle producer's rendered `index.html`.
fn spa_shell() -> Response {
    Html(shell_html().to_string()).into_response()
}

/// Insert a validated `ETag` header, skipping it if the value can't be a header
/// (our hex tags always can — this is a defensive no-panic fallback).
fn insert_etag(headers: &mut HeaderMap, etag: &str) {
    if let Ok(value) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, value);
    }
}

/// Build the `200`/`304` response for one embedded representation. Pure over an
/// injected `body` + `sha256` (no live `Site`), so the full header/status logic
/// — `Content-Type`/`Content-Encoding`/`Vary`/`ETag` and the conditional `304` —
/// is unit-tested directly by constructing inputs and inspecting the `Response`.
/// `body` is a [`Bytes`] so an embedded (`'static`-borrowed) asset serves
/// zero-copy — no per-request heap copy of the multi-MB wasm.
fn build_response(
    logical_path: &str,
    body: Bytes,
    sha256: [u8; 32],
    encoding: Encoding,
    if_none_match: Option<&str>,
    immutable: bool,
) -> Response {
    let etag = etag::from_sha256(sha256);
    let mut headers = HeaderMap::new();
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    insert_etag(&mut headers, etag.as_ref());
    if immutable {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type_for(logical_path))
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    if let Some(coding) = encoding.content_encoding() {
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(coding));
    }
    if not_modified(if_none_match, &etag) {
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }
    (StatusCode::OK, headers, body).into_response()
}

fn embedded_body(data: Cow<'static, [u8]>) -> Bytes {
    match data {
        Cow::Borrowed(bytes) => Bytes::from_static(bytes),
        Cow::Owned(bytes) => Bytes::from(bytes),
    }
}

/// Serve an embedded site asset with content negotiation + conditional support,
/// falling through to the SPA shell for any path with no embedded file. The
/// header/status logic lives in the unit-tested pure fns above; the live `Site`
/// lookup is exercised end-to-end by [`serve_site`]'s integration tests (the
/// coverage build stages the real bundle — see the module docs).
///
/// **The negotiated encoding is recorded, not inferrable (#818).** The `request`
/// span already carries the client's `accept-encoding`, but what the server
/// actually *served* was only derivable by re-deriving [`choose_encoding`]'s logic
/// and checking which `.br`/`.gz` variants the bundle happens to embed. #818 had to
/// do exactly that to rule out "the two browsers were fed different bytes" as the
/// cause of a fetch-duration asymmetry — a question the traces should have answered
/// directly. `site.encoding` and `site.bytes` are the served representation, so a
/// future comparison can check that premise instead of reconstructing it.
///
/// **`site.bytes` is the size of the selected representation, not bytes put on the
/// wire.** It is recorded before the conditional check, so a request answered with
/// `304 Not Modified` still reports the full representation size while sending no
/// body at all — content-addressed manifest-role asset requests can be correlated
/// with `site.status`, the authoritative body-or-no-body signal.

#[tracing::instrument(
    name = "site.serve",
    skip_all,
    fields(
        site.path = tracing::field::Empty,
        site.encoding = tracing::field::Empty,
        site.bytes = tracing::field::Empty,
        site.embedded = tracing::field::Empty,
        site.status = tracing::field::Empty,
    )
)]
pub async fn serve_site(req: Request) -> Response {
    // An empty logical path (`/`, `//`) has no embedded key, so it falls through
    // to the SPA shell via the `None` arm below — no separate guard needed.
    let logical = req.uri().path().trim_start_matches('/').to_owned();

    let headers = req.headers();
    let accept_encoding = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok());
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());

    let has_br = Site::get(&variant_path(&logical, Encoding::Br)).is_some();
    let has_gz = Site::get(&variant_path(&logical, Encoding::Gzip)).is_some();
    let encoding = choose_encoding(accept_encoding, has_br, has_gz);

    let span = tracing::Span::current();
    span.record("site.path", logical.as_str());
    // `identity` rather than absent: a missing field and a deliberately
    // uncompressed response must not read the same way.
    span.record(
        "site.encoding",
        encoding.content_encoding().unwrap_or("identity"),
    );

    if let Some(file) = Site::get(&variant_path(&logical, encoding)) {
        let hash = file.metadata.sha256_hash();
        // Zero-copy for the embedded (`'static`-borrowed) case; only a
        // runtime disk-read (debug) yields an owned buffer.
        let body = embedded_body(file.data);
        span.record("site.bytes", body.len());
        span.record("site.embedded", true);
        let response = build_response(
            &logical,
            body,
            hash,
            encoding,
            if_none_match,
            crate::bundle::is_manifest_asset(&logical),
        );
        // The authoritative "did a body go over the wire" signal: `304` means none
        // did, whatever the client later reports. The browsers disagree about
        // `PerformanceResourceTiming.transferSize` on a revalidated response —
        // firefox reports the full body size where chromium reports ~300 B, while
        // both send `if-none-match` at the same rate — so a cache check written
        // against `transferSize` would read as a browser difference that isn't
        // there. This field is engine-independent because the server sets it (#818).
        span.record("site.status", response.status().as_u16());
        response
    } else {
        // The SPA-shell fall-through. Recorded so an asset that silently stops
        // being embedded is visible as `embedded=false`, rather than as an
        // unexplained shell response.
        span.record("site.embedded", false);
        let response = spa_shell();
        span.record("site.status", response.status().as_u16());
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_html_from_preserves_embedded_bytes_or_uses_the_no_bundle_fallback() {
        assert_eq!(
            shell_html_from(None).as_ref(),
            "<!doctype html><title>CSR bundle unavailable</title>"
        );
        assert_eq!(
            shell_html_from(Some(b"<main>Jaunder</main>")).as_ref(),
            "<main>Jaunder</main>"
        );
    }

    #[test]
    fn prefers_brotli_when_accepted_and_available() {
        assert_eq!(choose_encoding(Some("gzip, br"), true, true), Encoding::Br);
    }

    #[test]
    fn falls_back_to_gzip_when_brotli_absent() {
        assert_eq!(
            choose_encoding(Some("gzip, br"), false, true),
            Encoding::Gzip
        );
    }

    #[test]
    fn falls_back_to_gzip_when_client_rejects_brotli() {
        assert_eq!(choose_encoding(Some("gzip"), true, true), Encoding::Gzip);
    }

    #[test]
    fn identity_when_client_accepts_nothing_compressible() {
        assert_eq!(
            choose_encoding(Some("identity"), true, true),
            Encoding::Identity
        );
    }

    #[test]
    fn identity_when_no_accept_encoding_header() {
        assert_eq!(choose_encoding(None, true, true), Encoding::Identity);
    }

    #[test]
    fn identity_when_variants_absent_even_if_accepted() {
        assert_eq!(
            choose_encoding(Some("br, gzip"), false, false),
            Encoding::Identity
        );
    }

    #[test]
    fn accepts_handles_q_values_whitespace_and_case() {
        assert!(accepts("br;q=1.0, gzip;q=0.5", "gzip"));
        assert!(accepts("  BR ", "br"));
        assert!(!accepts("deflate", "br"));
        // An explicit q=0 rejects the coding (RFC 9110).
        assert!(!accepts("gzip;q=0", "gzip"));
        assert!(!accepts("br; q=0.0, gzip", "br"));
        assert!(accepts("br; q=0.0, gzip", "gzip"));
    }

    #[test]
    fn content_encoding_header_value_per_coding() {
        assert_eq!(Encoding::Br.content_encoding(), Some("br"));
        assert_eq!(Encoding::Gzip.content_encoding(), Some("gzip"));
        assert_eq!(Encoding::Identity.content_encoding(), None);
    }

    #[test]
    fn embedded_body_keeps_borrowed_embed_bytes_zero_copy() {
        let bytes: &'static [u8] = b"embedded bytes";
        let body = embedded_body(Cow::Borrowed(bytes));

        assert_eq!(body.as_ref(), bytes);
        assert_eq!(body.as_ptr(), bytes.as_ptr());
    }

    #[test]
    fn content_type_maps_wasm_and_js() {
        assert_eq!(
            content_type_for("pkg/content-addressed.wasm"),
            "application/wasm"
        );
        assert_eq!(
            content_type_for("pkg/content-addressed.js"),
            "text/javascript"
        );
    }

    #[test]
    fn content_type_resolves_favicon() {
        // mime_guess returns an image type for .ico; assert it is non-empty and
        // an image (either x-icon or vnd.microsoft.icon per the mime_guess db).
        let ct = content_type_for("favicon.ico");
        assert!(ct.starts_with("image/"), "unexpected content-type: {ct}");
    }

    #[test]
    fn content_type_falls_back_for_unknown_extension() {
        assert_eq!(
            content_type_for("pkg/mystery.unknownext"),
            "application/octet-stream"
        );
    }

    #[test]
    fn not_modified_true_on_exact_match() {
        assert!(not_modified(Some("\"abc\""), &"\"abc\"".parse().unwrap()));
    }

    #[test]
    fn not_modified_true_when_present_in_list() {
        assert!(not_modified(
            Some("\"other\", \"abc\""),
            &"\"abc\"".parse().unwrap()
        ));
    }

    #[test]
    fn not_modified_false_on_mismatch_or_absent() {
        let etag: ETag = "\"abc\"".parse().unwrap();
        assert!(!not_modified(Some("\"xyz\""), &etag));
        assert!(!not_modified(None, &etag));
    }

    #[test]
    fn variant_path_appends_coding_suffix() {
        assert_eq!(
            variant_path("pkg/content-addressed.wasm", Encoding::Br),
            "pkg/content-addressed.wasm.br"
        );
        assert_eq!(
            variant_path("pkg/content-addressed.wasm", Encoding::Gzip),
            "pkg/content-addressed.wasm.gz"
        );
        assert_eq!(
            variant_path("pkg/content-addressed.wasm", Encoding::Identity),
            "pkg/content-addressed.wasm"
        );
    }

    use axum::body::{Body, to_bytes};

    #[tokio::test]
    async fn build_response_identity_sets_type_body_and_no_encoding() {
        let resp = build_response(
            "pkg/content-addressed.wasm",
            Bytes::from_static(b"WASM"),
            [1u8; 32],
            Encoding::Identity,
            None,
            true,
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/wasm"
        );
        assert_eq!(resp.headers().get(header::VARY).unwrap(), "Accept-Encoding");
        assert!(resp.headers().get(header::CONTENT_ENCODING).is_none());
        assert!(resp.headers().get(header::ETAG).is_some());
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), b"WASM");
    }

    #[tokio::test]
    async fn build_response_brotli_sets_content_encoding_and_logical_type() {
        // Content-Type is from the LOGICAL path (`.js`), not the `.br` variant.
        let resp = build_response(
            "pkg/content-addressed.js",
            Bytes::from_static(b"code"),
            [9u8; 32],
            Encoding::Br,
            None,
            true,
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_ENCODING).unwrap(), "br");
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/javascript"
        );
    }

    #[tokio::test]
    async fn build_response_304_empty_body_when_if_none_match_matches() {
        let sha = [0xabu8; 32];
        let etag = etag::from_sha256(sha);
        let resp = build_response(
            "pkg/content-addressed.wasm",
            Bytes::from_static(b"ignored"),
            sha,
            Encoding::Br,
            Some(etag.as_ref()),
            true,
        );
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/wasm"
        );
        assert_eq!(resp.headers().get(header::CONTENT_ENCODING).unwrap(), "br");
        assert_eq!(resp.headers().get(header::ETAG).unwrap(), etag.as_ref());
        assert_eq!(resp.headers().get(header::VARY).unwrap(), "Accept-Encoding");
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn serve_site_falls_through_to_spa_shell_for_unknown_path() {
        let req = Request::builder()
            .uri("/definitely-not-an-embedded-asset-xyz")
            .body(Body::empty())
            .unwrap();
        let resp = serve_site(req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        assert!(
            ct.starts_with("text/html"),
            "expected SPA-shell html, got {ct}"
        );
    }

    /// `public/` assets reach the served site — the regression guard #291 asked
    /// for after `GET /favicon.ico` 404'd on the host loop.
    ///
    /// The #237 embed is the mechanism: `server/build.rs` stages `public/` into
    /// `$OUT_DIR/site/` from `JAUNDER_PUBLIC_DIR`, or from `<workspace>/public`
    /// when that is unset (the host path). Without this test, a staging break
    /// surfaces as a 404 no spec asserts, not a build failure.
    ///
    /// **Deliberately unguarded**, unlike the wasm test below. `pkg/` needs
    /// `cargo xtask build-csr` to exist, so that test guards its assertions;
    /// `public/` has no build prerequisite and is always in the tree, so a guard
    /// here would only let the regression pass silently — the one outcome this
    /// test exists to prevent.
    #[tokio::test]
    async fn serve_site_serves_embedded_public_favicon() {
        let req = Request::builder()
            .uri("/favicon.ico")
            .body(Body::empty())
            .unwrap();
        let resp = serve_site(req).await;

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "GET /favicon.ico must serve the embedded `public/` asset (#291)"
        );
        // An image type, not the exact string: `mime_guess` may answer
        // `image/x-icon` or `image/vnd.microsoft.icon` for `.ico`, and pinning
        // one would break on a dependency bump without the served behaviour
        // having changed (same reasoning as `content_type_resolves_favicon`).
        // What matters is that it is not the SPA shell's `text/html`, which is
        // exactly what a fall-through would return.
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("a served favicon has a content type")
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            ct.starts_with("image/"),
            "the favicon must serve as an image, not fall through to the SPA shell; got {ct} (#291)"
        );

        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(
            !body.is_empty(),
            "the embedded favicon must have bytes (#291)"
        );
    }

    #[tokio::test]
    async fn manifest_wasm_variants_keep_logical_headers_and_immutable_304s() {
        let Some(urls) = crate::bundle::boot_urls() else {
            return; // cov:ignore: Host test builds omit the generated CSR bundle required to exercise manifest variants.
        };
        let logical = urls.wasm.trim_start_matches('/');
        for (accept_encoding, expected_encoding) in [
            ("identity", None),
            ("gzip", Some("gzip")),
            ("br", Some("br")),
        ] {
            let req = Request::builder()
                .uri(urls.wasm)
                .header(header::ACCEPT_ENCODING, accept_encoding)
                .body(Body::empty())
                .unwrap();
            let response = serve_site(req).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/wasm"
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_ENCODING)
                    .and_then(|value| value.to_str().ok()),
                expected_encoding
            );
            assert_eq!(
                response.headers().get(header::VARY).unwrap(),
                "Accept-Encoding"
            );
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "public, max-age=31536000, immutable"
            );
            let etag = response
                .headers()
                .get(header::ETAG)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let expected = Site::get(&variant_path(
                logical,
                match expected_encoding {
                    Some("br") => Encoding::Br,
                    Some("gzip") => Encoding::Gzip,
                    _ => Encoding::Identity,
                },
            ))
            .expect("manifest representation is embedded");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(body.as_ref(), expected.data.as_ref());

            let conditional = Request::builder()
                .uri(urls.wasm)
                .header(header::ACCEPT_ENCODING, accept_encoding)
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap();
            let response = serve_site(conditional).await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert_eq!(response.headers().get(header::ETAG).unwrap(), etag.as_str());
            assert_eq!(
                response.headers().get(header::VARY).unwrap(),
                "Accept-Encoding"
            );
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "public, max-age=31536000, immutable"
            );
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/wasm"
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_ENCODING)
                    .and_then(|value| value.to_str().ok()),
                expected_encoding
            );
        }
    }
    #[test]
    fn rendered_static_shell_uses_each_manifest_role_url_once_in_boot_order() {
        let Some(urls) = crate::bundle::boot_urls() else {
            return; // cov:ignore: Host test builds omit the generated CSR bundle required to render this manifest-backed shell.
        };
        let shell = shell_html();
        for url in [urls.glue, urls.wasm] {
            assert_eq!(shell.matches(url).count(), 1, "{shell}");
        }
        let fetch = shell
            .find("window.__jaunderWasmFetch = fetch")
            .expect("early fetch");
        let stylesheet = shell
            .find(r#"<link rel="stylesheet" href="/style/jaunder.css" />"#)
            .expect("stylesheet");
        let import = shell.find("import {initMeasured}").expect("module import");
        let mark = shell.find("performance.mark").expect("init mark");
        let init = shell
            .find("initMeasured(window.__jaunderWasmFetch")
            .expect("initializer");
        assert!(
            fetch < stylesheet && stylesheet < import && import < mark && mark < init,
            "{shell}"
        );
    }

    #[test]
    fn non_manifest_assets_remain_without_immutable_cache_control() {
        let response = build_response(
            "favicon.ico",
            Bytes::from_static(b"icon"),
            [2u8; 32],
            Encoding::Identity,
            None,
            false,
        );
        assert!(response.headers().get(header::CACHE_CONTROL).is_none());
    }
}
