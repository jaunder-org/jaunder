use std::sync::Arc;

use axum::http::StatusCode;
use common::ids::PostId;
use common::test_support::parse_post_body;
use common::visibility::{AudienceTarget, DefaultAudience};
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    atompub_get, atompub_post_xml, atompub_put_xml, body_string, create_user_and_session, make_app,
};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

use super::fixtures::{entry_xml, location_post_id};

/// Named audience metadata is resolved in the authenticated author's namespace;
/// foreign and nonexistent IDs deliberately share `AtomPub`'s opaque 400 outcome.
#[apply(backends)]
#[tokio::test]
async fn org_named_audiences_are_author_scoped(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let foreign = create_user_and_session(&state).await;
    let owned_name = common::test_support::parse_audience_name("Owned");
    let audiences = Arc::clone(&state.audiences);
    let owned = storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    audiences
                        .create_audience(transaction, author.user_id, &owned_name)
                        .await
                })
            })
            .await
            .expect("create author's audience"),
        "author's audience fixture",
    );
    let foreign_name = common::test_support::parse_audience_name("Foreign");
    let audiences = Arc::clone(&state.audiences);
    let foreign_audience = storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    audiences
                        .create_audience(transaction, foreign.user_id, &foreign_name)
                        .await
                })
            })
            .await
            .expect("create foreign audience"),
        "foreign audience fixture",
    );

    let owned_xml = entry_xml(
        "Audience",
        "text/org",
        &format!(
            "#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_AUDIENCE named:{owned}\n\nBody"
        ),
    );
    let response = make_app(&state, &base)
        .oneshot(atompub_post_xml(&author, "posts", &owned_xml))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    for audience_id in [foreign_audience.to_string(), "999999999".to_string()] {
        let xml = entry_xml(
            "Rejected",
            "text/org",
            &format!(
                "#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_AUDIENCE named:{audience_id}\n\nBody"
            ),
        );
        let response = make_app(&state, &base)
            .oneshot(atompub_post_xml(&author, "posts", &xml))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "audience {audience_id} must remain opaque"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn update_preserves_non_public_targeting(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    // A Subscribers-targeted post is hidden from an anonymous viewer. Editing it
    // via AtomPub must still succeed (the handler loads it as the authenticated
    // owner) AND must preserve the targeting across the edit (AtomPub has no
    // audience picker).
    let post = session
        .seed_post()
        .audiences(vec![common::visibility::AudienceTarget::Subscribers])
        .seed(&state)
        .await;

    let app = make_app(&state, &base);

    let xml = entry_xml("New", "text", "new body");
    let response = app
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "owner must be able to edit a non-Public post via AtomPub"
    );

    let audiences = state.posts.get_post_audiences(post.post_id).await.unwrap();
    assert_eq!(
        audiences,
        vec![common::visibility::AudienceTarget::Subscribers],
        "the edit must preserve the post's Subscribers targeting"
    );
}

#[apply(backends)]
#[tokio::test]
async fn member_get_serves_owner_non_public_post(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    // A Subscribers-targeted post is hidden from Anonymous; the owner must still
    // be able to GET it via AtomPub (handler loads as the authenticated owner).
    let post = session
        .seed_post()
        .body(parse_post_body("Secret body"))
        .audiences(vec![common::visibility::AudienceTarget::Subscribers])
        .seed(&state)
        .await;

    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "owner must be able to read their own non-Public post via AtomPub"
    );
    let body = body_string(response).await;
    assert!(body.contains("Secret body"), "body should contain content");
}

#[apply(backends_matrix)]
#[case(DefaultAudience::Public, vec![AudienceTarget::Public])]
#[case(
    DefaultAudience::Subscribers,
    vec![AudienceTarget::Subscribers]
)]
// Private is the empty per-Post audience, so it persists no audience rows.
#[case(DefaultAudience::Private, vec![])]
#[tokio::test]
async fn create_widens_each_default_audience(
    backend: Backend,
    #[case] default_audience: DefaultAudience,
    #[case] expected_audiences: Vec<AudienceTarget>,
) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    // AtomPub has no audience field, so post creation is the per-Post boundary
    // that widens this instance-wide default.
    let site_config = std::sync::Arc::clone(&state.site_config);
    storage::test_support::confirmed(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    site_config
                        .set_default_audience(transaction, &default_audience)
                        .await
                })
            })
            .await
            .unwrap(),
    );

    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");
    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let audiences = state
        .posts
        .get_post_audiences(PostId::from(location_post_id(&response)))
        .await
        .unwrap();
    assert_eq!(
        audiences, expected_audiences,
        "AtomPub create must widen the configured DefaultAudience"
    );
}
