use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use common::test_support::parse_root_relative_url;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{atompub_at, atompub_get, create_user_and_session, make_app};
use storage::test_support::{Backend, backends};

/// Every `AtomPub` route, including error and bodyless responses, must tell
/// intermediaries not to turn Jaunder's strong Post `ETags` into coded variants.
#[apply(backends)]
#[tokio::test]
async fn atompub_routes_forbid_intermediary_transformation(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let routes = [
        (Method::GET, "/atompub/service"),
        (Method::GET, "/atompub/alice/posts"),
        (Method::POST, "/atompub/alice/posts"),
        (Method::GET, "/atompub/alice/posts/1"),
        (Method::PUT, "/atompub/alice/posts/1"),
        (Method::DELETE, "/atompub/alice/posts/1"),
        (Method::POST, "/atompub/alice/media"),
        (Method::GET, "/atompub/alice/media/sha/file.jpg"),
        (Method::DELETE, "/atompub/alice/media/sha/file.jpg"),
    ];
    for (method, path) in routes {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-transform",
            "{method} {path}"
        );
    }

    // A path with no AtomPub handler still belongs to the /atompub/*
    // response contract; other application routes do not.
    let unknown = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/atompub/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.headers()[header::CACHE_CONTROL], "no-transform");
    let outside = app
        .oneshot(
            Request::builder()
                .uri("/~alice/rsd.xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        !outside
            .headers()
            .get_all(header::CACHE_CONTROL)
            .iter()
            .any(|value| value
                .as_bytes()
                .windows(b"no-transform".len())
                .any(|part| part == b"no-transform"))
    );
}

#[apply(backends)]
#[tokio::test]
async fn successful_service_and_post_collection_keep_atompub_encoding_contract(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, &env.base);
    let service = atompub_at(
        &session,
        Method::GET,
        &parse_root_relative_url("/atompub/service"),
    )
    .body(Body::empty())
    .unwrap();
    for (path, request) in [
        ("service", service),
        ("posts", atompub_get(&session, "posts")),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-transform");
    }
}
