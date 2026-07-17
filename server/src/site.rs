//! The embedded CSR site tree + a precompression-aware serving handler.
//!
//! `server/build.rs` stages the runtime site (`pkg/jaunder.{js,wasm}` plus the
//! precompressed `.br`/`.gz` siblings and wasm-bindgen `snippets/`, plus the
//! `public/` assets) into `$OUT_DIR/site/`; [`Site`] embeds it (#237,
//! ADR-0003/0008). This replaces the disk `ServeDir::new(&site_root)` fallback,
//! so a released `--release` binary serves its own client with no external
//! files.
//!
//! `axum-embed`'s `ServeEmbed` does no `Accept-Encoding` negotiation, so
//! [`serve_site`] is a small custom handler: it negotiates br/gzip/identity
//! against the embedded precompressed variants, sets `Content-Type` from the
//! *logical* path, emits a per-representation `ETag`, and honors
//! `If-None-Match` (→ `304`). A path with no embedded file falls through to the
//! SPA shell, exactly as `ServeDir(...).fallback(spa_shell)` did.
//!
//! The header/status logic lives in **pure functions** ([`choose_encoding`],
//! [`content_type_for`], [`etag_for`], [`not_modified`], [`build_response`]) that
//! are unit-tested without a live embed. The `Site` lookup itself is exercised
//! end-to-end by [`serve_site`]'s integration tests: the Nix coverage build
//! stages the real bundle (`flake.nix` sets `JAUNDER_CSR_BUNDLE_DIR`), so a
//! populated [`Site`] is measured under instrumentation.

use std::borrow::Cow;
use std::fmt::Write as _;

use axum::body::Bytes;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)] // cov:ignore
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

/// A strong, quoted-hex `ETag` derived from a rust-embed file's SHA-256 hash.
/// Stable for identical bytes and distinct for different bytes, so each
/// representation (identity / `.br` / `.gz`) gets its own tag.
fn etag_for(sha256: &[u8]) -> String {
    let mut etag = String::with_capacity(2 + sha256.len() * 2);
    etag.push('"');
    for byte in sha256 {
        // Writing to a String is infallible.
        let _ = write!(etag, "{byte:02x}");
    }
    etag.push('"');
    etag
}

/// Whether an `If-None-Match` header matches `etag` (a validator in the list is
/// enough → the representation is unchanged, serve `304`).
fn not_modified(if_none_match: Option<&str>, etag: &str) -> bool {
    if_none_match.is_some_and(|inm| inm.split(',').any(|tag| tag.trim() == etag))
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

/// The SPA-shell fallthrough: the same embedded `index.html` boot document the
/// old `ServeDir(...).fallback(spa_shell)` served for unknown paths.
fn spa_shell() -> Response {
    Html(web::render::SPA_SHELL).into_response()
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
    sha256: &[u8],
    encoding: Encoding,
    if_none_match: Option<&str>,
) -> Response {
    let etag = etag_for(sha256);
    let mut headers = HeaderMap::new();
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    insert_etag(&mut headers, &etag);

    if not_modified(if_none_match, &etag) {
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }

    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type_for(logical_path))
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    if let Some(coding) = encoding.content_encoding() {
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(coding));
    }
    (StatusCode::OK, headers, body).into_response()
}

/// Serve an embedded site asset with content negotiation + conditional support,
/// falling through to the SPA shell for any path with no embedded file. The
/// header/status logic lives in the unit-tested pure fns above; the live `Site`
/// lookup is exercised end-to-end by [`serve_site`]'s integration tests (the
/// coverage build stages the real bundle — see the module docs).
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

    match Site::get(&variant_path(&logical, encoding)) {
        Some(file) => {
            let hash = file.metadata.sha256_hash();
            // Zero-copy for the embedded (`'static`-borrowed) case; only a
            // runtime disk-read (debug) yields an owned buffer. The coverage
            // build is debug (disk → `Owned`), so the release-only `Borrowed`
            // arm is unreachable under instrumentation.
            let body = match file.data {
                // cov:ignore-start -- release-embed-only (debug coverage disk-reads → Owned).
                Cow::Borrowed(bytes) => Bytes::from_static(bytes),
                // cov:ignore-stop
                Cow::Owned(bytes) => Bytes::from(bytes),
            };
            build_response(&logical, body, hash.as_ref(), encoding, if_none_match)
        }
        None => spa_shell(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn content_type_maps_wasm_and_js() {
        assert_eq!(content_type_for("pkg/jaunder.wasm"), "application/wasm");
        assert_eq!(content_type_for("pkg/jaunder.js"), "text/javascript");
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
    fn etag_is_quoted_hex_and_stable() {
        let bytes = [0x00u8, 0x0f, 0xa1, 0xff];
        let etag = etag_for(&bytes);
        assert_eq!(etag, "\"000fa1ff\"");
        // Stable for identical input.
        assert_eq!(etag_for(&bytes), etag);
    }

    #[test]
    fn etag_differs_for_different_bytes() {
        assert_ne!(etag_for(&[1, 2, 3]), etag_for(&[3, 2, 1]));
    }

    #[test]
    fn not_modified_true_on_exact_match() {
        assert!(not_modified(Some("\"abc\""), "\"abc\""));
    }

    #[test]
    fn not_modified_true_when_present_in_list() {
        assert!(not_modified(Some("\"other\", \"abc\""), "\"abc\""));
    }

    #[test]
    fn not_modified_false_on_mismatch_or_absent() {
        assert!(!not_modified(Some("\"xyz\""), "\"abc\""));
        assert!(!not_modified(None, "\"abc\""));
    }

    #[test]
    fn variant_path_appends_coding_suffix() {
        assert_eq!(
            variant_path("pkg/jaunder.wasm", Encoding::Br),
            "pkg/jaunder.wasm.br"
        );
        assert_eq!(
            variant_path("pkg/jaunder.wasm", Encoding::Gzip),
            "pkg/jaunder.wasm.gz"
        );
        assert_eq!(
            variant_path("pkg/jaunder.wasm", Encoding::Identity),
            "pkg/jaunder.wasm"
        );
    }

    use axum::body::{to_bytes, Body};

    #[tokio::test]
    async fn build_response_identity_sets_type_body_and_no_encoding() {
        let resp = build_response(
            "pkg/jaunder.wasm",
            Bytes::from_static(b"WASM"),
            &[1, 2, 3],
            Encoding::Identity,
            None,
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/wasm"
        );
        assert_eq!(resp.headers().get(header::VARY).unwrap(), "Accept-Encoding");
        assert!(resp.headers().get(header::CONTENT_ENCODING).is_none());
        assert!(resp.headers().get(header::ETAG).is_some());
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), b"WASM");
    }

    #[tokio::test]
    async fn build_response_brotli_sets_content_encoding_and_logical_type() {
        // Content-Type is from the LOGICAL path (`.js`), not the `.br` variant.
        let resp = build_response(
            "pkg/jaunder.js",
            Bytes::from_static(b"code"),
            &[9],
            Encoding::Br,
            None,
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
        let sha = [0xabu8, 0xcd];
        let etag = etag_for(&sha);
        let resp = build_response(
            "pkg/jaunder.wasm",
            Bytes::from_static(b"ignored"),
            &sha,
            Encoding::Br,
            Some(&etag),
        );
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(resp.headers().get(header::ETAG).unwrap(), etag.as_str());
        assert_eq!(resp.headers().get(header::VARY).unwrap(), "Accept-Encoding");
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

    #[tokio::test]
    async fn serve_site_serves_embedded_wasm_negotiated_brotli_and_conditional() {
        // Exercises the live-embed found branch end-to-end. The Nix coverage
        // build stages the real bundle (JAUNDER_CSR_BUNDLE_DIR, flake.nix), so
        // `Site` is populated and this measures the branch. A bare local
        // `cargo test` without `cargo xtask build-csr` has an empty `Site`: the
        // request falls through to the SPA shell, so the asset-specific
        // assertions are guarded rather than false-failing. `serve_site` still
        // runs unconditionally, so the found branch is covered where it can be.
        let bundle_present = Site::get("pkg/jaunder.wasm").is_some();

        let req = Request::builder()
            .uri("/pkg/jaunder.wasm")
            .header(header::ACCEPT_ENCODING, "br")
            .body(Body::empty())
            .unwrap();
        let resp = serve_site(req).await;

        if bundle_present {
            assert_eq!(resp.status(), StatusCode::OK);
            assert_eq!(
                resp.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/wasm"
            );
            assert_eq!(resp.headers().get(header::CONTENT_ENCODING).unwrap(), "br");
            assert_eq!(resp.headers().get(header::VARY).unwrap(), "Accept-Encoding");
            let etag = resp
                .headers()
                .get(header::ETAG)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();

            // Conditional re-request with the returned ETag → 304.
            let req2 = Request::builder()
                .uri("/pkg/jaunder.wasm")
                .header(header::ACCEPT_ENCODING, "br")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap();
            assert_eq!(serve_site(req2).await.status(), StatusCode::NOT_MODIFIED);
        }
    }
}
