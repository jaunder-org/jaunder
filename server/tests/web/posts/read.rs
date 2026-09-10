use axum::http::StatusCode;
use common::render::PostFormat;
use common::seed::PublicPresentation;
use common::tag::TagLabel;
use common::test_support::parse_post_body;
use web::posts::{AuthoredPostSnapshot, PostInputs};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{confirmed_created_post, create_post_json, create_user_and_session, make_app};
use storage::test_support::{Backend, backends};

use super::fixtures::get_post_form;

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_published_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(
                parse_post_body(
                    "# Permalink

**bold**",
                ),
                PostFormat::Markdown,
            )
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created = confirmed_created_post(&body);

    let record = env
        .posts()
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    let published_at = record
        .published_at
        .expect("published post should have published_at");
    let published_date = jiff::tz::Offset::UTC
        .to_datetime(published_at.value())
        .date();
    let (status, body) = get_post_form(
        app.clone(),
        &session.username,
        i32::from(published_date.year()),
        u32::try_from(published_date.month()).expect("Jiff civil month fits u32"),
        u32::try_from(published_date.day()).expect("Jiff civil day fits u32"),
        &created.slug,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Permalink"));
    assert!(body.contains("rendered_html"));
    assert!(body.contains("published_at"));
}

#[apply(backends)]
#[tokio::test]
async fn get_post_rejects_invalid_username(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) =
        get_post_form(app.clone(), "Invalid Name", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_rejects_invalid_slug(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) =
        get_post_form(app.clone(), "author", 2024, 1, 1, "Invalid Slug", None).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body.contains("slug"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_not_found_for_missing_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = get_post_form(app.clone(), "author", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_carries_tags(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(
                parse_post_body("# Tagged Post\n\nbody"),
                PostFormat::Markdown,
            )
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created = confirmed_created_post(&body);

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        created.post_id,
        session.user_id,
        &["Performance".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    let published_at = env
        .posts()
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap()
        .published_at
        .unwrap();

    let published_date = jiff::tz::Offset::UTC
        .to_datetime(published_at.value())
        .date();
    let (status, body) = get_post_form(
        app.clone(),
        &session.username,
        i32::from(published_date.year()),
        u32::try_from(published_date.month()).expect("Jiff civil month fits u32"),
        u32::try_from(published_date.day()).expect("Jiff civil day fits u32"),
        &created.slug,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    let response = serde_json::from_str::<PublicPresentation<AuthoredPostSnapshot>>(&body)
        .unwrap()
        .page
        .post;
    assert_eq!(response.post.tags.len(), 1);
    assert_eq!(response.post.tags[0].slug, "performance");
    assert_eq!(response.post.tags[0].display, "Performance");
}
