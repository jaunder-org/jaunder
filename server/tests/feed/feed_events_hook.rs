use axum::{
    body::Body,
    http::{Method, StatusCode},
};
use common::{
    test_support::{parse_post_body, parse_tag_label},
    visibility::DefaultAudience,
};
use server_fn::ServerFn;
use storage::PostFormat;

use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    atompub, atompub_post_xml, atompub_put_xml, confirmed_mutation, create_post_json,
    create_user_and_session, make_app, post_form, update_post_json,
};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};
use web::posts::{PostInputs, SavedPost};

async fn claim_pending(state: &std::sync::Arc<storage::AppState>) -> Vec<storage::FeedEventRecord> {
    let feed_events = state.feed_events.clone();
    storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    feed_events
                        .claim_pending_batch(transaction, 100, chrono::Duration::seconds(86400))
                        .await
                })
            })
            .await
            .expect("claim batch"),
        "claim batch acknowledgement",
    )
}

fn confirmed_post_id(response: &str) -> i64 {
    i64::from(confirmed_mutation::<SavedPost>(response).post_id)
}

async fn use_public_default(state: &std::sync::Arc<storage::AppState>) {
    let site_config = std::sync::Arc::clone(&state.site_config);
    storage::test_support::confirmed(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    site_config
                        .set_default_audience(transaction, &DefaultAudience::Public)
                        .await
                })
            })
            .await
            .expect("set Public default audience"),
    );
}

fn assert_public_atom_paths(
    events: Vec<storage::FeedEventRecord>,
    username: &common::username::Username,
) {
    let tag = "rust".parse().expect("valid Tag");
    let mut expected = host::feed::affected_feed_urls(username, std::iter::once(&tag))
        .into_iter()
        .map(|path| path.to_string())
        .collect::<Vec<_>>();
    let mut actual = events
        .into_iter()
        .map(|event| event.feed_path.to_string())
        .collect::<Vec<_>>();
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual, expected);
}

fn atom_entry(title: &str, draft: bool) -> String {
    let draft = if draft { "yes" } else { "no" };
    format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom"
       xmlns:app="http://www.w3.org/2007/app">
  <title>{title}</title>
  <content type="text">body</content>
  <category term="rust"/>
  <app:control><app:draft>{draft}</app:draft></app:control>
</entry>"#
    )
}

// Creating a published post enqueues the Site and User feeds (3 formats each =
// 6 rows), plus 2 rows per tag (SiteTag + UserTag) × 3 formats. With no tags
// that's 6 rows; with two tags it's 6 + 2×2×3 = 18 rows.
#[apply(backends_matrix)]
#[case::no_tags(None::<Vec<String>>, 6)]
#[case::two_tags(Some(vec!["rust".to_string(), "web".to_string()]), 18)]
#[tokio::test]
async fn create_published_post_enqueues_expected_feeds(
    backend: Backend,
    #[case] tags: Option<Vec<String>>,
    #[case] expected_rows: usize,
) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;

    let (status, _response) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            tags: tags.map(|tags| tags.into_iter().map(|tag| parse_tag_label(&tag)).collect()),
            ..PostInputs::new(parse_post_body("Test post"), PostFormat::Markdown)
        },
        Some(&session.cookie()),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let batch = claim_pending(&state).await;

    assert_eq!(
        batch.len(),
        expected_rows,
        "Expected {expected_rows} feed events for published post"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_with_tag_change_enqueues_old_and_new_tags(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, create_response) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            tags: Some(vec![parse_tag_label("rust"), parse_tag_label("web")]),
            ..PostInputs::new(parse_post_body("Test post"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain initial create events
    let _initial_batch = claim_pending(&state).await;

    // Union should be {leptos, rust, web} = 3 tags
    let (status, _) = update_post_json(
        &state,
        common::ids::PostId::from(post_id),
        PostInputs {
            publish: Some(false),
            tags: Some(vec![parse_tag_label("rust"), parse_tag_label("leptos")]),
            ..PostInputs::new(parse_post_body("Updated post"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let update_batch = claim_pending(&state).await;

    // Expected: Site (3) + User (3) + 3 tags × (SiteTag + UserTag) × 3 formats = 6 + 18 = 24 rows
    assert_eq!(
        update_batch.len(),
        24,
        "Expected 24 feed events from update with tag change: {update_batch:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn unpublish_enqueues_site_and_user_and_tag_feeds(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, create_response) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            tags: Some(vec![parse_tag_label("rust")]),
            ..PostInputs::new(parse_post_body("Test post"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain initial create events
    let _initial_batch = claim_pending(&state).await;

    let unpublish_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        &state,
        <web::posts::Unpublish as ServerFn>::PATH,
        unpublish_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let unpublish_batch = claim_pending(&state).await;

    // Expected: Site (3) + User (3) + 1 tag × (SiteTag + UserTag) × 3 formats = 6 + 6 = 12 rows
    assert_eq!(
        unpublish_batch.len(),
        12,
        "Expected 12 feed events from unpublish with 1 tag"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_published_post_enqueues_feeds(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, create_response) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            tags: Some(vec![parse_tag_label("rust")]),
            ..PostInputs::new(parse_post_body("Test post"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain initial create events
    let _initial_batch = claim_pending(&state).await;

    let delete_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        &state,
        <web::posts::Delete as ServerFn>::PATH,
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let delete_batch = claim_pending(&state).await;

    // Expected: Site (3) + User (3) + 1 tag × (SiteTag + UserTag) × 3 formats = 6 + 6 = 12 rows
    assert_eq!(
        delete_batch.len(),
        12,
        "Expected 12 feed events from deleting published post with 1 tag"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_draft_post_enqueues_nothing(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, create_response) = create_post_json(
        &state,
        PostInputs {
            publish: Some(false),
            tags: Some(vec![parse_tag_label("rust")]),
            ..PostInputs::new(parse_post_body("Test draft"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain any events from create (drafts still enqueue as per spec)
    let _initial_batch = claim_pending(&state).await;

    let delete_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        &state,
        <web::posts::Delete as ServerFn>::PATH,
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let delete_batch = claim_pending(&state).await;

    // Expected: 0 rows (draft posts don't affect feeds)
    assert_eq!(
        delete_batch.len(),
        0,
        "Expected 0 feed events from deleting draft post"
    );
}

#[apply(backends)]
#[tokio::test]
async fn atompub_publication_transitions_enqueue_expected_feeds(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    use_public_default(&state).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let response = app
        .clone()
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &atom_entry("Published", false),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = response
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .and_then(|id| id.parse::<i64>().ok())
        .expect("Location should end in the Post id");
    assert_public_atom_paths(claim_pending(&state).await, &session.username);

    let suffix = format!("posts/{post_id}");
    let response = app
        .clone()
        .oneshot(atompub_put_xml(
            &session,
            &suffix,
            &atom_entry("Draft", true),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_public_atom_paths(claim_pending(&state).await, &session.username);

    let response = app
        .oneshot(atompub_put_xml(
            &session,
            &suffix,
            &atom_entry("Republished", false),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_public_atom_paths(claim_pending(&state).await, &session.username);
}

#[apply(backends_matrix)]
#[case::published(false, 12)]
#[case::draft(true, 0)]
#[tokio::test]
async fn atompub_delete_enqueues_only_for_public_posts(
    backend: Backend,
    #[case] draft: bool,
    #[case] expected_rows: usize,
) {
    let TestEnv { state, base } = backend.setup().await;
    use_public_default(&state).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let response = app
        .clone()
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &atom_entry("Delete", draft),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = response
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .and_then(|id| id.parse::<i64>().ok())
        .expect("Location should end in the Post id");
    let _creation_events = claim_pending(&state).await;

    let request = atompub(&session, Method::DELETE, &format!("posts/{post_id}"))
        .body(Body::empty())
        .expect("DELETE request");
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let events = claim_pending(&state).await;
    if expected_rows == 0 {
        assert!(events.is_empty());
    } else {
        assert_public_atom_paths(events, &session.username);
    }
}
