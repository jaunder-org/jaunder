use axum::http::StatusCode;
use chrono::Datelike;
use common::test_support::parse_row_limit;
use server_fn::ServerFn;
use storage::PostFormat;
use web::posts::SavedPost;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, post_form, post_json};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

use super::fixtures::{create_post_json, login_and_state};

#[apply(backends)]
#[tokio::test]
async fn create_post_persists_rendered_published_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    // Title embedded as # heading in the body (verbatim storage)
    let (status, body) = create_post_json(
        &state,
        "# Hello World

**bold**",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "hello-world");
    assert!(created.published_at.is_some());

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Hello World"));
    assert_eq!(record.slug.to_string(), "hello-world");
    assert_eq!(record.format, PostFormat::Markdown);
    assert!(record.published_at.is_some());
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<strong>bold</strong>"),
        "rendered_html: {}",
        record.rendered_html
    );
    let published_at = record.published_at.expect("published post");
    let expected_permalink = format!(
        "/~{}/{:04}/{:02}/{:02}/{}",
        session.username,
        published_at.value().year(),
        published_at.value().month(),
        published_at.value().day(),
        record.slug.as_ref()
    );
    assert_eq!(created.permalink, *expected_permalink);
}

#[apply(backends)]
#[tokio::test]
async fn create_post_retries_slug_conflicts_for_same_user_and_date(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    // Title embedded as # heading; two posts with same heading produce conflicting slugs
    let (first_status, first_body) = create_post_json(
        &state,
        "# Repeated Title

first",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK, "body: {first_body}");

    let (second_status, second_body) = create_post_json(
        &state,
        "# Repeated Title

second",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(second_status, StatusCode::OK, "body: {second_body}");
    let created: SavedPost = serde_json::from_str(&second_body).unwrap();
    assert_eq!(created.slug, "repeated-title-2");
}

#[apply(backends)]
#[tokio::test]
async fn create_post_accepts_slug_override_and_saves_draft(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        &state,
        "*bold*",
        "org",
        Some("Custom-Slug"),
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "custom-slug");
    assert!(created.published_at.is_none());
    // A draft carries its canonical (created_at-based) permalink; the permalink
    // view renders the draft for the author (#24).
    assert!(
        created
            .permalink
            .as_ref()
            .starts_with(&format!("/~{}/", session.username)),
        "draft should carry a canonical permalink: {}",
        created.permalink
    );

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.slug.to_string(), "custom-slug");
    assert_eq!(record.format, PostFormat::Org);
    assert!(record.published_at.is_none());
    assert!(record.rendered_html.as_ref().contains("<b>bold</b>"));
}

#[apply(backends)]
#[tokio::test]
async fn create_post_accepts_titleless_body(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "Titleless note",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "titleless-note");
    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title, None);
    // A stored Markdown body ends with exactly one newline: canonicalization applies
    // to every format except HTML (#811). Rendering is unaffected — CommonMark treats a
    // trailing newline as insignificant — but the stored bytes are canonical, not raw.
    assert_eq!(record.body, "Titleless note\n");
}

#[apply(backends)]
#[tokio::test]
async fn create_post_extracts_markdown_heading_title(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Extracted Title

Body text",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "extracted-title");
    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Extracted Title"));
    // The heading survives in the body — unlike Org, Markdown's title source is content,
    // not a header line, so extracting the title does not consume it. Only whitespace is
    // canonicalized (#811), hence the terminating newline.
    assert_eq!(record.body, "# Extracted Title\n\nBody text\n");
    // Rendered HTML contains the heading because body is rendered verbatim
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<h1>Extracted Title</h1>")
    );
}

// Shape B — create_post rejection cluster. Identical setup (author + session)
// and assertion structure (INTERNAL_SERVER_ERROR + body substring); only the
// request body/format and the expected error message vary. An invalid
// `slug_override` and a blank body are both typed-wire-arg decode rejections
// (`Option<Slug>`, `PostBody` — ADR-0065, #811), so the expected messages are
// the types', not the handler's; client pre-validation is the user-facing path,
// and the serde-bridge rejection is unit-tested in `common::slug`.
#[apply(backends_matrix)]
#[case::empty_post(
    "",
    "markdown",
    None,
    "post body must contain at least one non-blank line"
)]
#[case::invalid_format("body", "invalid_format", None, "post format must be")]
#[tokio::test]
async fn create_post_rejects(
    backend: Backend,
    #[case] request_body: &str,
    #[case] format: &str,
    #[case] slug_override: Option<&str>,
    #[case] expected: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        request_body,
        format,
        slug_override,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains(expected), "body: {body}");
}

// A future `publish_at` on create schedules the post: storage records the exact
// future instant and the post stays off the public timeline until then (#70).
#[apply(backends)]
#[tokio::test]
async fn create_post_with_future_publish_at_is_scheduled(#[case] backend: Backend) {
    use chrono::TimeZone;
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let future = chrono::Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap();
    let payload = serde_json::json!({
        "post": {
            "body": "scheduled body",
            "format": "markdown",
            "publish": true,
            "publish_at": future.to_rfc3339(),
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(
        record.published_at,
        Some(common::time::UtcInstant::from(future))
    );

    // The scheduled post is invisible on the public timeline at "now".
    let published = state
        .posts
        .list_published(
            None,
            parse_row_limit("50"),
            &common::visibility::ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .unwrap();
    assert!(
        !published.iter().any(|p| p.post_id == created.post_id),
        "scheduled post must not appear in the public timeline"
    );
}

// Publishing without a `publish_at` goes live immediately: the post is stamped
// ~now and appears on the public timeline (#70).
#[apply(backends)]
#[tokio::test]
async fn create_post_publish_without_publish_at_is_live_now(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "live now body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    let published_at = record
        .published_at
        .expect("published post has published_at");
    let now = chrono::Utc::now();
    assert!(
        (now - published_at.value()).num_seconds().abs() < 60,
        "publish-now should stamp ~now, got {published_at}"
    );

    let published = state
        .posts
        .list_published(
            None,
            parse_row_limit("50"),
            &common::visibility::ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(now),
        )
        .await
        .unwrap();
    assert!(
        published.iter().any(|p| p.post_id == created.post_id),
        "publish-now post must appear in the public timeline"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_post_applies_tags_from_param(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let payload = serde_json::json!({
        "post": {
            "body": "# Tagged via API\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": true,
            "tags": ["Rust", "web-dev"],
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let stored_tags = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post exists")
        .tags;
    let slugs: Vec<&str> = stored_tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs, vec!["rust", "web-dev"]);
    assert!(stored_tags.iter().any(|t| t.tag_display == "Rust"));
}

#[apply(backends)]
#[tokio::test]
async fn create_post_rejects_invalid_tag_token(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let payload = serde_json::json!({
        "post": {
            "body": "# Bad Tag\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": true,
            "tags": ["rust", "not a valid tag!"],
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    // The invalid token is rejected at the wire→TagLabel parse, surfacing
    // InvalidTagLabel's own message — the single validation source.
    assert!(body.contains("tag must be non-empty"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn create_post_rejects_more_than_25_tags(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;
    let many: Vec<String> = (0..26).map(|n| format!("tag{n}")).collect();

    let payload = serde_json::json!({
        "post": {
            "body": "# Too Many\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": true,
            "tags": many,
        }
    });
    let (status, body) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("too many tags"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_default_post_format_returns_markdown_by_default(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let (status, body) = post_form(
        &state,
        <web::profile::GetDefaultPostFormat as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    assert_eq!(
        body, "\"markdown\"",
        "expected default format to be markdown"
    );
}

#[apply(backends)]
#[tokio::test]
async fn set_default_post_format_persists_and_retrieves_markdown(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let (status, body) = post_form(
        &state,
        <web::profile::SetDefaultPostFormat as ServerFn>::PATH,
        "format=markdown",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "set body: {body}");

    let (status, body) = post_form(
        &state,
        <web::profile::GetDefaultPostFormat as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    assert_eq!(
        body, "\"markdown\"",
        "expected format to be markdown after setting"
    );
}
