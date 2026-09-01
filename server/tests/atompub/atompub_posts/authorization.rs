use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use common::test_support::parse_root_relative_url;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    SeededSession, atompub_at, create_user_and_session, make_app, setup_with_base_url,
};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

use super::fixtures::entry_xml;

/// Which cross-user request a `*_forbids_other_user` case issues. Each variant
/// builds a request that `alice` (authenticated) directs at `bob`'s resource.
enum ForbiddenRequest {
    /// GET the collection: `/atompub/bob/posts`.
    Collection,
    /// GET a member: `/atompub/bob/posts/1`.
    Member,
    /// POST a new entry to the collection: `/atompub/bob/posts`.
    Create,
    /// PUT an entry: `/atompub/bob/posts/1`.
    Update,
}

impl ForbiddenRequest {
    fn build(&self, session: &SeededSession) -> Request<Body> {
        let (method, path, body) = match self {
            ForbiddenRequest::Collection => (Method::GET, "/atompub/bob/posts", None),
            ForbiddenRequest::Member => (Method::GET, "/atompub/bob/posts/1", None),
            ForbiddenRequest::Create => (
                Method::POST,
                "/atompub/bob/posts",
                Some(entry_xml("Hello", "text", "the body")),
            ),
            ForbiddenRequest::Update => (
                Method::PUT,
                "/atompub/bob/posts/1",
                Some(entry_xml("New", "text", "new body")),
            ),
        };
        let uri = parse_root_relative_url(path);
        let builder = atompub_at(session, method, &uri);
        match body {
            Some(xml) => builder
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .body(Body::from(xml)),
            None => builder.body(Body::empty()),
        }
        .expect("failed to build atompub request")
    }
}

// Shape B — the `*_forbids_other_user` cluster (collection/member/create/update).
// Each seeds `alice`, then `alice` (authenticated) directs the corresponding
// request at `bob`'s resource and must get FORBIDDEN.
#[apply(backends_matrix)]
#[case::collection(ForbiddenRequest::Collection)]
#[case::member(ForbiddenRequest::Member)]
#[case::create(ForbiddenRequest::Create)]
#[case::update(ForbiddenRequest::Update)]
#[tokio::test]
async fn forbids_other_user(backend: Backend, #[case] request: ForbiddenRequest) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let response = app.oneshot(request.build(&session)).await.unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// A malformed username path segment (`a@b` — `@` is outside `[a-z0-9_-]`) fails to
// parse into `Username` at the axum boundary, so the request is rejected with 400
// before any ownership check — contrast `forbids_other_user`, where a well-formed
// but mismatched username reaches `require_user_match` and gets 403.
#[apply(backends)]
#[tokio::test]
async fn malformed_username_path_returns_400(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let uri = parse_root_relative_url("/atompub/a@b/posts");

    let response = app
        .oneshot(
            atompub_at(&session, Method::GET, &uri)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
