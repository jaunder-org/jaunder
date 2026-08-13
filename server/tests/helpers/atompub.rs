use axum::{
    body::Body,
    http::{Request, header},
};
use common::token::RawToken;

use super::session::{SeededSession, basic_header};

/// A `Request::builder()` preloaded with `method`, `uri`, and an
/// `Authorization: Basic <username:token>` header — the base every authenticated
/// `AtomPub` request shares. Callers add any extra headers (`If-Match`, `slug`,
/// `Idempotency-Key`, a content type) and finish with `.body(...)`.
pub fn atompub_authed(
    method: &str,
    uri: &str,
    username: &str,
    token: &RawToken,
) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, basic_header(username, token))
}

/// The dominant `AtomPub` request: Basic auth plus an optional
/// `application/atom+xml` body. `Some(xml)` sends the entry body (POST/PUT);
/// `None` sends an empty body (GET/DELETE).
pub fn atompub_xml(
    method: &str,
    uri: &str,
    username: &str,
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
/// one place the per-user prefix is written, so a call site passes only the suffix
/// after it (e.g. `"posts"`, `"posts/{id}"`, `"media"`).
fn atompub_uri(session: &SeededSession, suffix: &str) -> String {
    format!("/atompub/{}/{suffix}", session.username)
}

/// A chainable Basic-authed `Request::builder()` against `session`'s own
/// `/atompub/{username}/{suffix}` resource — the base every session-keyed `AtomPub`
/// request shares, and the composition point for the extra-header cases (`If-Match`,
/// `Idempotency-Key`, a media `slug`/content type). Username + token come from the
/// `SeededSession`, so neither is ever re-typed. Callers add headers and finish with
/// `.body(...)`.
pub fn atompub(
    session: &SeededSession,
    method: &str,
    suffix: &str,
) -> axum::http::request::Builder {
    atompub_authed(
        method,
        &atompub_uri(session, suffix),
        &session.username,
        &session.token,
    )
}

/// Like [`atompub`] but against a **verbatim** `uri` rather than a per-user suffix —
/// for a follow-up request to a URI captured from a prior response's `Location`
/// header (an absolute path). Auth still comes from the `session`, so the username is
/// not doubled; only the URI is passed through unchanged.
pub fn atompub_at(
    session: &SeededSession,
    method: &str,
    uri: &str,
) -> axum::http::request::Builder {
    atompub_authed(method, uri, &session.username, &session.token)
}

/// `GET session`'s `suffix` resource with an empty body — the dominant read request.
pub fn atompub_get(session: &SeededSession, suffix: &str) -> Request<Body> {
    atompub(session, "GET", suffix)
        .body(Body::empty())
        .expect("failed to build atompub GET request")
}

/// Send an `application/atom+xml` `xml` body with `method` to `session`'s `suffix`
/// resource — the shared body behind [`atompub_post_xml`] / [`atompub_put_xml`], the
/// two verbs call sites actually use. Private: no caller needs an arbitrary method.
fn atompub_send_xml(
    session: &SeededSession,
    method: &str,
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
    atompub_send_xml(session, "POST", suffix, xml)
}

/// `PUT` an `application/atom+xml` entry to `session`'s `suffix` (replace).
pub fn atompub_put_xml(session: &SeededSession, suffix: &str, xml: &str) -> Request<Body> {
    atompub_send_xml(session, "PUT", suffix, xml)
}

/// A media upload for `session`'s user: `POST /atompub/{username}/media` with an
/// `image/png` body, the `slug` header, and `bytes` — the dominant media request.
/// The odd cases (a non-`image/png` content type, a slug like `".."`, a foreign
/// username) compose the chainable [`atompub`] / [`atompub_at`] builders directly.
pub fn atompub_upload(session: &SeededSession, slug: &str, bytes: &'static [u8]) -> Request<Body> {
    atompub(session, "POST", "media")
        .header(header::CONTENT_TYPE, "image/png")
        .header("slug", slug)
        .body(Body::from(bytes))
        .expect("failed to build atompub media upload request")
}
