use std::sync::Arc;

use axum::http::StatusCode;
use common::ids::PostId;
use common::tag::MAX_TAGS_PER_POST;
use server_fn::ServerFn;
use web::posts::SavedPost;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, post_form, post_json};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

use super::fixtures::{
    create_post_json, get_post_form, list_drafts, list_local_timeline, list_user_posts,
    login_and_state, publish_post_form, update_post_json,
};

async fn unpublish_post_form(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_form(
        state,
        <web::posts::Unpublish as ServerFn>::PATH,
        format!("post_id={post_id}"),
        cookie,
    )
    .await
}

#[apply(backends)]
#[tokio::test]
async fn update_post_updates_draft_content_and_slug(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "original", "markdown", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    let post_id = created.post_id;

    // Title embedded as # heading; slug_override takes precedence over the derived slug
    let (status, body) = update_post_json(
        &state,
        post_id,
        "# Updated Title

**new body**",
        "markdown",
        Some("updated-slug"),
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(updated.slug, "updated-slug");
    assert!(updated.published_at.is_none());

    let record = state
        .posts
        .get_post_by_id(post_id, &common::visibility::ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Updated Title"));
    assert_eq!(record.slug.to_string(), "updated-slug");
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<strong>new body</strong>")
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_post_freezes_slug_when_published(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "body", "markdown", None, true, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    let post_id = created.post_id;
    let original_slug = created.slug.clone();

    let (status, body) = update_post_json(
        &state,
        post_id,
        "new body",
        "markdown",
        Some("new-slug"),
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(
        updated.slug, original_slug,
        "slug must not change after publication"
    );
    assert!(updated.published_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn update_post_publishes_draft(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "draft body", "markdown", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_none());
    let post_id = created.post_id;

    let (status, body) = update_post_json(
        &state,
        post_id,
        "draft body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(updated.published_at.is_some());
    assert!(!updated.permalink.as_ref().is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn update_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = create_user_and_session(&state).await.cookie();
    let stranger_cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "body",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = update_post_json(
        &state,
        created.post_id,
        "hacked",
        "markdown",
        None,
        false,
        Some(&stranger_cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

// Shape B — update_post rejection cluster. Identical setup (author + session +
// a freshly created draft) and assertion structure (INTERNAL_SERVER_ERROR +
// body substring); only the update body/format and expected message vary. The
// initial draft body is immaterial to the assertion, so it is fixed. As on the
// create side, a blank body is an arg-decode rejection carrying `PostBody`'s
// message rather than the handler's (#811).
#[apply(backends_matrix)]
#[case::empty_post("", "markdown", "post body must contain at least one non-blank line")]
#[case::invalid_format("body", "invalid_format", "post format must be")]
#[tokio::test]
async fn update_post_rejects(
    backend: Backend,
    #[case] update_body: &str,
    #[case] update_format: &str,
    #[case] expected: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "original", "markdown", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = update_post_json(
        &state,
        created.post_id,
        update_body,
        update_format,
        None,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains(expected), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_post_returns_not_found_for_missing_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = update_post_json(
        &state,
        PostId::from(99999),
        "body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_post_returns_not_found_for_deleted_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "body", "markdown", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    state.posts.soft_delete_post(created.post_id).await.unwrap();

    let (status, body) = update_post_json(
        &state,
        created.post_id,
        "body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_publishes_draft_and_returns_permalink(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) =
        create_post_json(&state, "draft body", "markdown", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_none());

    let (status, body) = publish_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "publish body: {body}");
    let published: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(published.post_id, created.post_id);
    assert!(
        published
            .permalink
            .contains(&format!("/~{}/", session.username))
    );

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(record.published_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = create_user_and_session(&state).await.cookie();
    let stranger_cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "secret",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = publish_post_form(&state, created.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_returns_not_found_for_missing_or_deleted_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = publish_post_form(&state, PostId::from(999_999), Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");

    let (status, body) =
        create_post_json(&state, "body", "markdown", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    state.posts.soft_delete_post(created.post_id).await.unwrap();

    let (status, body) = publish_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

async fn delete_post_form(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_form(
        state,
        <web::posts::Delete as ServerFn>::PATH,
        format!("post_id={post_id}"),
        cookie,
    )
    .await
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_soft_deletes_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "gone", "markdown", None, true, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let post = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(post.deleted_at.is_some(), "expected deleted_at to be set");
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = create_user_and_session(&state).await.cookie();
    let stranger_cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "mine", "markdown", None, true, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(&state, created.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_rejects_unauthenticated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "body", "markdown", None, true, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(&state, created.post_id, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_returns_not_found_for_already_deleted_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) =
        create_post_json(&state, "body", "markdown", None, true, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "first delete body: {body}");

    let (status, body) = delete_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn deleted_post_excluded_from_timelines_and_returns_404_at_permalink(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Deletable Post

body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    let permalink = String::from(created.permalink);

    // Presence before deletion proves the exclusions below are the delete's doing.
    let (status, body) = list_user_posts(&state, &session.username, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Deletable Post"), "expected post in timeline");

    let (status, body) = delete_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");

    let (status, body) = list_user_posts(&state, &session.username, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains("Deletable Post"),
        "expected post excluded from timeline: {body}"
    );

    let (status, body) = list_local_timeline(&state, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains("Deletable Post"),
        "expected post excluded from local timeline: {body}"
    );

    // permalink format: /~username/year/month/day/slug
    let parts: Vec<&str> = permalink.trim_start_matches('/').split('/').collect();
    let year: i32 = parts[1].parse().unwrap();
    let month: u32 = parts[2].parse().unwrap();
    let day: u32 = parts[3].parse().unwrap();
    let slug = parts[4];

    let (status, body) =
        get_post_form(&state, &session.username, year, month, day, slug, None).await;
    assert_eq!(StatusCode::NOT_FOUND, status, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn unpublish_post_reverts_published_post_to_draft(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Unpublish Me

body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_some(), "should be published");

    let (status, body) = unpublish_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "unpublish body: {body}");

    // Should no longer appear in the user timeline
    let (status, body) = list_user_posts(&state, &session.username, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains("Unpublish Me"),
        "expected post removed from timeline: {body}"
    );

    // Should appear in drafts
    let (status, body) = list_drafts(&state, None, 50, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("unpublish-me"),
        "expected post in drafts: {body}"
    );
}

// Unpublish reports where the post lives *after* reverting to draft. A permalink is
// `published_at.unwrap_or(created_at)`-based, so reverting moves it back to the
// created_at-based URL — and an implementation that reads the permalink off the
// pre-unpublish record would hand back the published one it just left.
//
// The fixture deliberately forces the two dates apart: the post is created as a draft
// (created_at = today) and then published with a backdated `publish_at` in another
// year, so the published and draft permalinks cannot coincide. Publishing at "now"
// would make them byte-identical and the test would pass either way.
#[apply(backends)]
#[tokio::test]
async fn unpublish_post_returns_the_draft_permalink(#[case] backend: Backend) {
    use chrono::TimeZone;
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let body_text = "# Moved Permalink\n\nbody";
    let (status, body) =
        create_post_json(&state, body_text, "markdown", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let draft: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(draft.published_at.is_none(), "should start as a draft");

    // `publish` stamps `now`, so the backdate has to come through `update`'s
    // explicit `publish_at`.
    let backdated = chrono::Utc.with_ymd_and_hms(2020, 3, 5, 12, 0, 0).unwrap();
    let payload = serde_json::json!({
        "post_id": draft.post_id,
        "post": {
            "body": body_text,
            "format": "markdown",
            "publish": true,
            "publish_at": backdated.to_rfc3339(),
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let published: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(
        published.permalink.contains("/2020/03/05/"),
        "published permalink should carry the backdated date: {}",
        published.permalink
    );

    let (status, body) = unpublish_post_form(&state, draft.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "unpublish body: {body}");
    let unpublished: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(unpublished.published_at.is_none(), "reverted to draft");
    assert_eq!(
        unpublished.permalink, draft.permalink,
        "unpublish must report the created_at-based draft permalink"
    );
    // Fails loudly if the fixture ever stops making the two dates differ, which would
    // make the assertion above vacuous.
    assert_ne!(
        unpublished.permalink, published.permalink,
        "fixture must keep the draft and published permalinks distinct"
    );
}

#[apply(backends)]
#[tokio::test]
async fn unpublish_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = create_user_and_session(&state).await.cookie();
    let other_cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Others Post

body",
        "markdown",
        None,
        true,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let (status, body) = unpublish_post_form(&state, created.post_id, Some(&other_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_post_applies_tag_set_diff(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    // Create with two tags.
    let create_payload = serde_json::json!({
        "post": {
            "body": "# Diff Me\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": ["rust", "old-tag"],
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        create_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    // Update: replace old-tag with new-tag, keep rust.
    let update_payload = serde_json::json!({
        "post_id": created.post_id,
        "post": {
            "body": "# Diff Me\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": ["rust", "new-tag"],
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        update_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");

    let stored = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post exists")
        .tags;
    let slugs: Vec<&str> = stored.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs, vec!["new-tag", "rust"]);
}

#[apply(backends)]
#[tokio::test]
async fn update_post_rejects_over_limit_tags_without_mutating_post_or_tags(
    #[case] backend: Backend,
) {
    let (_base, state, cookie) = login_and_state(backend).await;
    let original_body = "# Original Title\n\noriginal body";
    let create_payload = serde_json::json!({
        "post": {
            "body": original_body,
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": ["original-tag"],
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        create_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let original = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("created post exists");
    let original_title = original.title;
    let original_body = original.body;
    let original_tags: Vec<String> = original
        .tags
        .iter()
        .map(|tag| tag.tag_slug.to_string())
        .collect();

    let replacement_tags: Vec<String> =
        (0..=MAX_TAGS_PER_POST).map(|n| format!("tag{n}")).collect();
    let update_payload = serde_json::json!({
        "post_id": created.post_id,
        "post": {
            "body": "# Replacement Title\n\nreplacement body",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": replacement_tags,
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        update_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("too many tags"), "body: {body}");

    let stored = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post exists");
    assert_eq!(stored.title, original_title);
    assert_eq!(stored.body, original_body);
    let tags: Vec<String> = stored
        .tags
        .iter()
        .map(|tag| tag.tag_slug.to_string())
        .collect();
    assert_eq!(tags, original_tags);
}

#[apply(backends)]
#[tokio::test]
async fn update_post_with_tags_unset_leaves_existing_tags_alone(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    // Create with one tag.
    let create_payload = serde_json::json!({
        "post": {
            "body": "# Untouched\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": ["keep"],
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        create_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    // Update without including the tags key (None on the server side).
    let update_payload = serde_json::json!({
        "post_id": created.post_id,
        "post": {
            "body": "# Untouched edited\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        update_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");

    let stored = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post exists")
        .tags;
    let slugs: Vec<&str> = stored.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs, vec!["keep"]);
}

#[apply(backends)]
#[tokio::test]
async fn update_org_header_applies_tags_and_rejects_mismatched_bookkeeping(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();
    let (status, body) =
        create_post_json(&state, "original", "org", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let org_body = format!(
        "#+TITLE: Canonical title\n#+KEYWORDS: org-tag, other\n#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_ID {}\n\nUpdated body",
        created.post_id
    );
    let update_payload = serde_json::json!({
        "post_id": created.post_id,
        "post": {
            "body": org_body,
            "format": "org",
            "publish": false,
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        update_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: SavedPost = serde_json::from_str(&body).unwrap();
    let record = state
        .posts
        .get_post_by_id(
            updated.post_id,
            &common::visibility::ViewerIdentity::Local {
                user_id: session.user_id,
            },
        )
        .await
        .unwrap()
        .expect("updated post exists");
    assert_eq!(record.title.as_deref(), Some("Canonical title"));
    assert_eq!(record.body, "Updated body\n");
    assert_eq!(
        record
            .tags
            .iter()
            .map(|tag| tag.tag_slug.as_ref())
            .collect::<Vec<_>>(),
        vec!["org-tag", "other"]
    );

    let rejection_payload = serde_json::json!({
        "post_id": updated.post_id,
        "post": {
            "body": "#+TITLE: rejected\n#+PROPERTY: JAUNDER_ID 999\n\nRejected body",
            "format": "org",
            "publish": false,
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        rejection_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("JAUNDER_ID does not match update target"),
        "body: {body}"
    );
    let unchanged = state
        .posts
        .get_post_by_id(
            updated.post_id,
            &common::visibility::ViewerIdentity::Local {
                user_id: session.user_id,
            },
        )
        .await
        .unwrap()
        .expect("post survives rejected write");
    assert_eq!(unchanged.body, record.body);
    assert_eq!(
        unchanged
            .tags
            .iter()
            .map(|tag| (&tag.tag_slug, &tag.tag_display))
            .collect::<Vec<_>>(),
        record
            .tags
            .iter()
            .map(|tag| (&tag.tag_slug, &tag.tag_display))
            .collect::<Vec<_>>()
    );
    assert_eq!(unchanged.title, record.title);
}

#[apply(backends)]
#[tokio::test]
async fn update_org_current_sync_succeeds_and_stale_sync_preserves_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();
    let (status, body) =
        create_post_json(&state, "original", "org", None, false, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    let before = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Local {
                user_id: session.user_id,
            },
        )
        .await
        .unwrap()
        .expect("created post exists");
    let current_etag = common::etag::post_content_etag(
        before.title.as_ref(),
        &before.body,
        &before.format,
        before.summary.as_ref(),
        before.tags.iter().map(|tag| &tag.tag_display),
        before.published_at.is_none(),
    );
    let matching_payload = serde_json::json!({
        "post_id": created.post_id,
        "post": {
            "body": format!("#+TITLE: Changed\n#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_ID {}\n#+PROPERTY: JAUNDER_SYNCED {current_etag}\n\nChanged body", created.post_id),
            "format": "org",
            "publish": false,
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        matching_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "matching sync update: {body}");
    let changed: SavedPost = serde_json::from_str(&body).unwrap();
    let before_stale = state
        .posts
        .get_post_by_id(
            changed.post_id,
            &common::visibility::ViewerIdentity::Local {
                user_id: session.user_id,
            },
        )
        .await
        .unwrap()
        .expect("changed post exists");

    let stale_payload = serde_json::json!({
        "post_id": changed.post_id,
        "post": {
            "body": format!("#+TITLE: Stale\n#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_ID {}\n#+PROPERTY: JAUNDER_SYNCED {current_etag}\n\nStale body", changed.post_id),
            "format": "org",
            "publish": false,
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        stale_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "stale sync body: {body}"
    );
    assert!(body.contains("\"conflict\""), "stale sync body: {body}");
    let unchanged = state
        .posts
        .get_post_by_id(
            changed.post_id,
            &common::visibility::ViewerIdentity::Local {
                user_id: session.user_id,
            },
        )
        .await
        .unwrap()
        .expect("stale update must not remove post");
    assert_eq!(unchanged.body, before_stale.body);
    assert_eq!(unchanged.title, before_stale.title);
    assert_eq!(
        unchanged
            .tags
            .iter()
            .map(|tag| (&tag.tag_slug, &tag.tag_display))
            .collect::<Vec<_>>(),
        before_stale
            .tags
            .iter()
            .map(|tag| (&tag.tag_slug, &tag.tag_display))
            .collect::<Vec<_>>()
    );
}
