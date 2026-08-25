use axum::{
    body::Body,
    http::{Method, Request, Uri, header},
};
use common::root_relative_url::RootRelativeUrl;
use common::test_support::parse_root_relative_url;
use common::token::RawToken;
use common::username::Username;

use super::session::{SeededSession, basic_header};

/// A `Request::builder()` preloaded with a typed `method`, root-relative `uri`,
/// and an `Authorization: Basic <username:token>` header — the base every
/// authenticated `AtomPub` request shares. Callers add any extra headers
/// (`If-Match`, `slug`, `Idempotency-Key`, a content type) and finish with
/// `.body(...)`.
pub fn atompub_authed(
    method: Method,
    uri: &RootRelativeUrl,
    username: &Username,
    token: &RawToken,
) -> axum::http::request::Builder {
    Request::builder().method(method).uri(uri.as_ref()).header(
        header::AUTHORIZATION,
        basic_header(username.as_ref(), token),
    )
}

/// The dominant `AtomPub` request: Basic auth plus an optional
/// `application/atom+xml` body. `Some(xml)` sends the entry body (POST/PUT);
/// `None` sends an empty body (GET/DELETE).
pub fn atompub_xml(
    method: Method,
    uri: &RootRelativeUrl,
    username: &Username,
    token: &RawToken,
    body: Option<&str>,
) -> Request<Body> {
    let builder = atompub_authed(method, uri, username, token);
    match body {
        Some(xml) => builder
            .header(header::CONTENT_TYPE, "application/atom+xml")
            .body(Body::from(xml.to_owned())),
        None => builder.body(Body::empty()),
    }
    .expect("failed to build atompub request")
}

/// The full `AtomPub` URI `/atompub/{username}/{suffix}` for `session`'s user — the
/// one place the per-user prefix is written and parsed, so a call site passes only
/// the suffix after it (e.g. `"posts"`, `"posts/{id}"`, `"media"`).
fn atompub_uri(session: &SeededSession, suffix: &str) -> RootRelativeUrl {
    parse_root_relative_url(&format!("/atompub/{}/{suffix}", session.username))
}

/// A chainable Basic-authed `Request::builder()` against `session`'s own
/// `/atompub/{username}/{suffix}` resource — the base every session-keyed `AtomPub`
/// request shares, and the composition point for the extra-header cases (`If-Match`,
/// `Idempotency-Key`, a media `slug`/content type). Username + token come from the
/// `SeededSession`, so neither is ever re-typed. Callers add headers and finish with
/// `.body(...)`.
pub fn atompub(
    session: &SeededSession,
    method: Method,
    suffix: &str,
) -> axum::http::request::Builder {
    atompub_authed(
        method,
        &atompub_uri(session, suffix),
        &session.username,
        &session.token,
    )
}

/// Like [`atompub`] but against a caller-held root-relative `uri` rather than a
/// per-user suffix. Auth still comes from the `session`, so the username is not
/// doubled and the request target is not parsed again.
pub fn atompub_at(
    session: &SeededSession,
    method: Method,
    uri: &RootRelativeUrl,
) -> axum::http::request::Builder {
    atompub_authed(method, uri, &session.username, &session.token)
}

/// Convert an absolute `AtomPub` response `Location` into its root-relative
/// path-and-query for a follow-up request.
///
/// # Panics
///
/// Panics if `location` is not an HTTP URI or has no path-and-query.
#[must_use]
pub fn atompub_location(location: &str) -> RootRelativeUrl {
    let uri = location.parse::<Uri>().expect("valid AtomPub Location URI");
    let path_and_query = uri
        .path_and_query()
        .expect("AtomPub Location has a path and query");
    parse_root_relative_url(path_and_query.as_str())
}

/// `GET session`'s `suffix` resource with an empty body — the dominant read request.
pub fn atompub_get(session: &SeededSession, suffix: &str) -> Request<Body> {
    atompub(session, Method::GET, suffix)
        .body(Body::empty())
        .expect("failed to build atompub GET request")
}

/// Send an `application/atom+xml` `xml` body with `method` to `session`'s `suffix`
/// resource — the shared body behind [`atompub_post_xml`] / [`atompub_put_xml`], the
/// two verbs call sites actually use. Private: no caller needs an arbitrary method.
fn atompub_send_xml(
    session: &SeededSession,
    method: Method,
    suffix: &str,
    xml: &str,
) -> Request<Body> {
    atompub(session, method, suffix)
        .header(header::CONTENT_TYPE, "application/atom+xml")
        .body(Body::from(xml.to_owned()))
        .expect("failed to build atompub xml request")
}

/// `POST` an `application/atom+xml` entry to `session`'s `suffix` (create).
pub fn atompub_post_xml(session: &SeededSession, suffix: &str, xml: &str) -> Request<Body> {
    atompub_send_xml(session, Method::POST, suffix, xml)
}

/// `PUT` an `application/atom+xml` entry to `session`'s `suffix` (replace).
pub fn atompub_put_xml(session: &SeededSession, suffix: &str, xml: &str) -> Request<Body> {
    atompub_send_xml(session, Method::PUT, suffix, xml)
}

/// A media upload for `session`'s user: `POST /atompub/{username}/media` with an
/// `image/png` body, the `slug` header, and `bytes` — the dominant media request.
/// The odd cases (a non-`image/png` content type, a slug like `".."`, a foreign
/// username) compose the chainable [`atompub`] / [`atompub_at`] builders directly.
pub fn atompub_upload(session: &SeededSession, slug: &str, bytes: &'static [u8]) -> Request<Body> {
    atompub(session, Method::POST, "media")
        .header(header::CONTENT_TYPE, "image/png")
        .header("slug", slug)
        .body(Body::from(bytes))
        .expect("failed to build atompub media upload request")
}
