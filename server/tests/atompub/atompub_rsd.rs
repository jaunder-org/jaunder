use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{body_string, create_user_and_session, make_app};
use storage::test_support::{Backend, TestEnv, backends_matrix};

// The plain `use` suffices: `#[apply]` resolves a cross-module `#[template]` by
// bare name (docs/adr/0124-rstest-reuse-cross-module-templates.md).
use storage::test_support::backends;

// Shape A — non-clustered behavior, backend-parametrized via cross-module apply.
#[apply(backends)]
#[tokio::test]
async fn rsd_document_advertises_service_url(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let identity = common::site::SiteIdentity {
        title: common::test_support::parse_site_title("Test"),
        base_url: Some(common::test_support::parse_url("https://example.test/")),
    };
    let site_config = std::sync::Arc::clone(&state.site_config);
    storage::test_support::confirmed(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { site_config.set_identity(transaction, &identity).await })
            })
            .await
            .unwrap(),
    );
    let app = make_app(&state, &base);

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

// Shape B — backend×value matrix: `#[apply(backends_matrix)]` supplies the
// backend axis, the named `#[case]`s the value axis; 2 rows × 2 backends = 4
// cases (docs/adr/0124-rstest-reuse-cross-module-templates.md).
#[apply(backends_matrix)]
#[case::edituri_rel("rel=\"EditURI\"")]
#[case::rsd_href("/~alice/rsd.xml")]
#[tokio::test]
async fn user_page_includes_rsd_autodiscovery_link(
    backend: Backend,
    #[case] expected_fragment: &str,
) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // The projector's render of the user page hoists the EditURI autodiscovery
    // link into the document head.
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/~{}", session.username))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let expected_fragment = expected_fragment.replace("alice", &session.username);
    assert!(body.contains(&expected_fragment), "{body}");
}
