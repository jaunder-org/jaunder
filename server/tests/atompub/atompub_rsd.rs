use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{body_string, make_app};
use storage::test_support::{backends_matrix, Backend, TestEnv};

// SPIKE (jaunder Task 1):
// - Shape A below (`rsd_document_advertises_service_url`) confirms cross-module
//   `#[apply]` resolves a `#[template]` defined in the `helpers` module simply by
//   importing it into scope (`use storage::test_support::backends;`) and then `#[apply(backends)]`.
//   No `#[apply(storage::test_support::backends)]` path and no `pub use` re-export are needed:
//   a `#[template]` expands to a name-mangled `macro_rules!` brought into scope by
//   the plain `use`, and `#[apply]` resolves it by bare name.
// - Shape B below (`user_page_includes_rsd_autodiscovery_link`) confirms the
//   backend×value matrix: the backend axis is supplied by the
//   `#[apply(backends_matrix)]` template (a `#[values]`-based dual-backend
//   template, issue #127) and composes with the test's own named `#[case]`
//   rows. Attribute ordering: `#[apply(backends_matrix)]` first, then the
//   `#[case::name(..)]` rows, then `#[tokio::test]`.
//   It generates rows × 2 cases (2 rows × 2 backends = 4).
use storage::test_support::backends;

// Shape A — non-clustered behavior, backend-parametrized via cross-module apply.
#[apply(backends)]
#[tokio::test]
async fn rsd_document_advertises_service_url(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    state
        .site_config
        .set_identity(&common::site::SiteIdentity {
            title: common::test_support::parse_site_title("Test"),
            base_url: Some(common::test_support::parse_absolute_url(
                "https://example.test/",
            )),
        })
        .await
        .unwrap();
    let app = make_app(state, &base);

    // RSD is public — no authentication required.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/~alice/rsd.xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.contains("application/rsd+xml"),
        "content-type was {content_type}"
    );

    let body = body_string(response).await;
    assert!(body.contains("<engineName>Jaunder</engineName>"), "{body}");
    assert!(
        body.contains("apiLink=\"https://example.test/atompub/service\""),
        "{body}"
    );
    assert!(body.contains("https://example.test/~alice"), "{body}");
}

// Shape B — backend×value matrix. The backend axis comes from the
// `#[apply(backends_matrix)]` template (a `#[values]`-based axis, because a
// `#[case]`-based axis can't coexist with the value `#[case]` rows); the value
// axis is the named `#[case]`s. 2 rows × 2 backends = 4 cases.
#[apply(backends_matrix)]
#[case::edituri_rel("rel=\"EditURI\"")]
#[case::rsd_href("/~alice/rsd.xml")]
#[tokio::test]
async fn user_page_includes_rsd_autodiscovery_link(
    backend: Backend,
    #[case] expected_fragment: &str,
) {
    let TestEnv { state, base } = backend.setup().await;
    state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let app = make_app(state, &base);

    // Rendering the user page (server-side) hoists the EditURI autodiscovery
    // link into the document head.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/~alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains(expected_fragment), "{body}");
}
