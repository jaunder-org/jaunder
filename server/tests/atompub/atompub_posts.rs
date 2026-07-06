use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use base64::Engine as _;
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
#[expect(
    clippy::single_component_path_imports,
    reason = "rstest_reuse needs the bare `use rstest_reuse;` import in scope for its #[template]/#[apply] macros; a glob import would trip wildcard_imports instead"
)]
use rstest_reuse;
use rstest_reuse::*;

use crate::helpers::{
    backends, backends_matrix, ensure_server_fns_registered, noop_mailer, test_options, Backend,
    TestEnv,
};

fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    jaunder::create_router(test_options(), state, noop_mailer(), false, storage_path)
}

fn basic_header(username: &str, password: &str) -> String {
    let raw = format!("{username}:{password}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
    format!("Basic {encoded}")
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[apply(backends)]
#[tokio::test]
async fn collection_lists_user_posts(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    let _post1 = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Hello body one".to_string(),
            title: Some("Hello Title One"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let _post2 = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Hello body two".to_string(),
            title: Some("Hello Title Two"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let ctype = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        ctype.contains("type=feed"),
        "content-type was {ctype}, should contain type=feed"
    );
    let body = body_string(response).await;
    assert!(body.contains("<feed"), "body should contain <feed");
    assert!(
        body.contains("Hello Title One"),
        "body should contain first post title"
    );
    assert!(
        body.contains("Hello Title Two"),
        "body should contain second post title"
    );
    assert!(
        body.contains("rel=\"edit\""),
        "body should contain rel=edit link"
    );
}

#[apply(backends)]
#[tokio::test]
async fn member_returns_native_source_with_etag(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "# Markdown body".to_string(),
            title: Some("My Post"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok());
    assert!(etag.is_some(), "response should have ETag header");
    let body = body_string(response).await;
    assert!(
        body.contains("type=\"text/markdown\""),
        "body should carry the text/markdown media type (native source, ADR-0023)"
    );
    assert!(
        body.contains("# Markdown body"),
        "body should contain markdown"
    );
}

#[apply(backends)]
#[tokio::test]
async fn member_get_unknown_returns_404(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts/999999")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn delete_then_get_is_404(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Delete me".to_string(),
            title: Some("Temporary Post"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    // First, delete the post
    let delete_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    // Then, try to get it
    let get_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn collection_paging_emits_next_link(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    for i in 0..2 {
        storage::perform_post_creation(
            state.posts.as_ref(),
            storage::PostCreation {
                user_id,
                body: format!("Body {i}"),
                title: Some(&format!("Title {i}")),
                format: storage::PostFormat::Markdown,
                slug_override: None,
                published_at: Some(chrono::Utc::now()),
                max_attempts: 100,
                summary: None,
                audiences: vec![common::visibility::AudienceTarget::Public],
            },
        )
        .await
        .unwrap();
    }

    let app = make_app(state, &base);

    // Page size 1 with 2 posts -> a next link must be present.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts?limit=1")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("rel=\"next\""), "missing next link: {body}");
    assert!(
        body.contains("updated_before="),
        "next link lacks cursor: {body}"
    );
    assert_eq!(
        body.matches("<entry").count(),
        1,
        "expected exactly one entry"
    );
}

/// Seeds a user named `alice` and returns `(user_id, session_token)`.
async fn seed_alice(state: &Arc<storage::AppState>) -> (i64, String) {
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();
    (user_id, token)
}

// Shape B — the cursor accept/reject pair. Both seed `alice`, issue a GET to the
// collection with a cursor query string, and assert the resulting status. They
// differ only in whether a post is seeded, the cursor query, and the expected
// status.
#[apply(backends_matrix)]
#[case::valid_cursor(
    true,
    "updated_before=2099-01-01T00:00:00Z&id_before=999999",
    StatusCode::OK
)]
#[case::invalid_cursor(
    false,
    "updated_before=not-a-date&id_before=1",
    StatusCode::BAD_REQUEST
)]
#[tokio::test]
async fn collection_cursor_validation(
    backend: Backend,
    #[case] seed_post: bool,
    #[case] query: &str,
    #[case] expected: StatusCode,
) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;
    if seed_post {
        storage::perform_post_creation(
            state.posts.as_ref(),
            storage::PostCreation {
                user_id,
                body: "Body".to_string(),
                title: Some("Title"),
                format: storage::PostFormat::Markdown,
                slug_override: None,
                published_at: Some(chrono::Utc::now()),
                max_attempts: 100,
                summary: None,
                audiences: vec![common::visibility::AudienceTarget::Public],
            },
        )
        .await
        .unwrap();
    }
    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/atompub/alice/posts?{query}"))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), expected);
}

#[apply(backends)]
#[tokio::test]
async fn collection_empty_returns_feed_without_entries(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("<feed"));
    assert_eq!(body.matches("<entry").count(), 0);
}

fn entry_xml(title: &str, content_type: &str, content: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>{title}</title>
  <content type="{content_type}">{content}</content>
  <category term="rust"/>
</entry>"#
    )
}

/// A non-draft text entry carrying an optional `<published>` element (RFC 3339).
/// `published == None` omits the element entirely (publish-now semantics).
fn entry_xml_with_published(title: &str, content: &str, published: Option<&str>) -> String {
    let published_elem =
        published.map_or_else(String::new, |ts| format!("\n  <published>{ts}</published>"));
    format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>{title}</title>
  <content type="text">{content}</content>{published_elem}
</entry>"#
    )
}

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
    fn build(&self, token: &str) -> Request<Body> {
        let auth = basic_header("alice", token);
        match self {
            ForbiddenRequest::Collection => Request::builder()
                .uri("/atompub/bob/posts")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .unwrap(),
            ForbiddenRequest::Member => Request::builder()
                .uri("/atompub/bob/posts/1")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .unwrap(),
            ForbiddenRequest::Create => Request::builder()
                .method("POST")
                .uri("/atompub/bob/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, auth)
                .body(Body::from(entry_xml("Hello", "text", "the body")))
                .unwrap(),
            ForbiddenRequest::Update => Request::builder()
                .method("PUT")
                .uri("/atompub/bob/posts/1")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, auth)
                .body(Body::from(entry_xml("New", "text", "new body")))
                .unwrap(),
        }
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
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let response = app.oneshot(request.build(&token)).await.unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[apply(backends)]
#[tokio::test]
async fn create_post_returns_201_and_is_retrievable(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;
    // Set default format to Markdown so text entries round-trip properly
    storage::set_default_post_format(
        state.user_config.as_ref(),
        user_id,
        storage::PostFormat::Markdown,
    )
    .await
    .unwrap();
    let app = make_app(state.clone(), &base);

    let xml = entry_xml("Hello", "text", "the body");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let loc = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string);
    assert!(
        loc.is_some(),
        "response should have Location header: {loc:?}"
    );

    let app2 = make_app(state, &base);
    let loc_path = loc.unwrap();
    let get_response = app2
        .oneshot(
            Request::builder()
                .uri(&loc_path)
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let body = body_string(get_response).await;
    assert!(
        body.contains("the body"),
        "retrieved entry should contain body"
    );
    assert!(
        body.contains("type=\"text/markdown\""),
        "a Markdown post round-trips as the text/markdown media type (ADR-0023)"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_post_applies_categories(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let xml = entry_xml("Hello", "text", "the body");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_string(response).await;
    assert!(
        body.contains("term=\"rust\""),
        "returned entry should contain category term=rust"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_html_entry_is_stored_as_html(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let xml = entry_xml("H", "html", "&lt;p&gt;hi&lt;/p&gt;");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_string(response).await;
    assert!(
        body.contains("type=\"html\""),
        "entry should be stored with type=html"
    );
}

// Shape B — per-entry format media type (ADR-0023, Task 1). POSTing a content
// `type` media type stores the matching format, and the round-tripped member
// echoes the same media type. `text/org`→Org, `text/markdown`→Markdown. The
// account default format is irrelevant here: the explicit media type wins.
#[apply(backends_matrix)]
#[case::org("text/org", "* Org heading\nbody")]
#[case::markdown("text/markdown", "# Markdown heading\nbody")]
#[tokio::test]
async fn create_format_media_type_round_trips(
    backend: Backend,
    #[case] content_type: &str,
    #[case] content: &str,
) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state.clone(), &base);

    let xml = entry_xml("Formatted", content_type, content);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // GET the member back: it must echo the same content media type.
    let get = make_app(state, &base)
        .oneshot(
            Request::builder()
                .uri(&location)
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get.status(), StatusCode::OK);
    let body = body_string(get).await;
    assert!(
        body.contains(&format!("type=\"{content_type}\"")),
        "member should round-trip type={content_type}: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_replaces_post_body(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Old body".to_string(),
            title: Some("Old"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    let xml = entry_xml("New", "text", "new body");
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(
        body.contains("new body"),
        "response entry should contain new body"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_with_stale_if_match_returns_412(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Old body".to_string(),
            title: Some("Old"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    let xml = entry_xml("New", "text", "new body");
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, "\"0\"") // Wrong ETag
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
}

#[apply(backends)]
#[tokio::test]
async fn create_rejects_malformed_entry(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from("not xml"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn update_removes_categories_not_in_new_entry(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Body".to_string(),
            title: Some("Title"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    state
        .posts
        .tag_post(post.post_id, "original-tag")
        .await
        .unwrap();

    let app = make_app(state, &base);

    // Update without the tag
    let xml = entry_xml("Title", "text", "new body");
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    // The original tag should not be in the response since we didn't include it
    assert!(!body.contains("original-tag"));
}

#[apply(backends)]
#[tokio::test]
async fn update_with_put_returns_200_and_etag(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Original".to_string(),
            title: Some("Title"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    let xml = entry_xml("Updated", "text", "updated body");
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok());
    assert!(etag.is_some(), "PUT response should include ETag header");
}

/// An empty Atom entry (neither title nor content), shared by the
/// `*_with_no_title_or_content_returns_400` cases.
const EMPTY_ENTRY_XML: &str = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
</entry>"#;

/// Whether the empty-entry submission is a create (POST to the collection) or an
/// update (PUT to a pre-existing post).
enum EmptyEntryOp {
    Create,
    Update,
}

// Shape B — the `*_with_no_title_or_content_returns_400` pair. Both submit an
// entry with neither title nor content and must fail with BAD_REQUEST
// (EmptyPost); create POSTs to the collection, update PUTs to a pre-existing
// post.
#[apply(backends_matrix)]
#[case::create(EmptyEntryOp::Create)]
#[case::update(EmptyEntryOp::Update)]
#[tokio::test]
async fn empty_entry_returns_400(backend: Backend, #[case] op: EmptyEntryOp) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    let request = match op {
        EmptyEntryOp::Create => Request::builder()
            .method("POST")
            .uri("/atompub/alice/posts")
            .header(header::CONTENT_TYPE, "application/atom+xml")
            .header(header::AUTHORIZATION, basic_header("alice", &token))
            .body(Body::from(EMPTY_ENTRY_XML))
            .unwrap(),
        EmptyEntryOp::Update => {
            // Create an initial post to update.
            let post = storage::perform_post_creation(
                state.posts.as_ref(),
                storage::PostCreation {
                    user_id,
                    body: "Original body".to_string(),
                    title: Some("Original"),
                    format: storage::PostFormat::Markdown,
                    slug_override: None,
                    published_at: Some(chrono::Utc::now()),
                    max_attempts: 100,
                    summary: None,
                    audiences: vec![common::visibility::AudienceTarget::Public],
                },
            )
            .await
            .unwrap();
            Request::builder()
                .method("PUT")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(EMPTY_ENTRY_XML))
                .unwrap()
        }
    };

    let app = make_app(state, &base);

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn create_draft_entry_is_unpublished(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Draft</title>
  <content type="text">draft body</content>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let get = app
        .oneshot(
            Request::builder()
                .uri(&location)
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let body = body_string(get).await;
    // A draft post round-trips the app:draft marker.
    assert!(body.contains("app:draft"), "draft marker missing: {body}");
    // The read-only j:slug is emitted on every entry, drafts included (ADR-0023).
    assert!(
        body.contains("xmlns:j=\"https://jaunder.org/ns/atompub\""),
        "draft entry should declare xmlns:j: {body}"
    );
    assert!(
        body.contains("<j:slug>"),
        "draft entry should carry j:slug: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn member_carries_read_only_j_slug(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Body".to_string(),
            title: Some("My Post"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(
        body.contains("xmlns:j=\"https://jaunder.org/ns/atompub\""),
        "member should declare xmlns:j: {body}"
    );
    assert!(
        body.contains(&format!("<j:slug>{}</j:slug>", post.slug.as_str())),
        "member should carry the post's slug as j:slug: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn incoming_j_slug_is_ignored(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state.clone(), &base);

    // A client-supplied <j:slug> must NOT determine the stored slug — the server
    // derives its own from the title (ADR-0023: j:slug is read-only).
    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:j="https://jaunder.org/ns/atompub">
  <title>Server Derives This</title>
  <content type="text">body</content>
  <j:slug>client-supplied</j:slug>
</entry>"#;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    let viewer = common::visibility::ViewerIdentity::Anonymous;
    let rec = state
        .posts
        .get_post_by_id(post_id, &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        rec.slug.as_str(),
        "client-supplied",
        "incoming j:slug must not become the stored slug"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_skips_invalid_category(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Cat</title>
  <content type="text">body</content>
  <category term="has spaces"/>
</entry>"#;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    // The invalid term was skipped, not stored.
    let body = body_string(response).await;
    assert!(
        !body.contains("has spaces"),
        "invalid category leaked: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_keeps_unchanged_category(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let with_rust = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <content type="text">body</content>
  <category term="rust"/>
</entry>"#;

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(with_rust))
                .unwrap(),
        )
        .await
        .unwrap();
    let location = created
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // PUT the same category back -> add-loop and remove-loop both take their
    // "already in sync" branches.
    let updated = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(with_rust))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let body = body_string(updated).await;
    assert!(body.contains("term=\"rust\""), "category dropped: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_with_matching_if_match_succeeds(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <content type="text">body</content>
</entry>"#;

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();
    let location = created
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let etag = created
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // A matching If-Match passes the precondition and the update proceeds.
    let updated = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, etag)
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn update_preserves_non_public_targeting(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    // A Subscribers-targeted post is hidden from an anonymous viewer. Editing it
    // via AtomPub must still succeed (the handler loads it as the authenticated
    // owner) AND must preserve the targeting across the edit (AtomPub has no
    // audience picker). Before owner-viewer threading, owned_post loaded the
    // post as Anonymous and the PUT 404'd before reaching this preservation.
    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Old body".to_string(),
            title: Some("Old"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Subscribers],
        },
    )
    .await
    .unwrap();

    let app = make_app(state.clone(), &base);

    let xml = entry_xml("New", "text", "new body");
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
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
    let (user_id, token) = seed_alice(&state).await;

    // A Subscribers-targeted post is hidden from Anonymous; the owner must still
    // be able to GET it via AtomPub (handler loads as the authenticated owner).
    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Secret body".to_string(),
            title: Some("Secret"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Subscribers],
        },
    )
    .await
    .unwrap();

    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
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

#[apply(backends)]
#[tokio::test]
async fn create_adopts_default_audience(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;

    // The instance default audience is Subscribers; an AtomPub POST (which has no
    // audience field) must adopt it.
    state
        .site_config
        .set_default_audience(&common::visibility::AudienceTarget::Subscribers)
        .await
        .unwrap();

    let app = make_app(state.clone(), &base);

    let xml = entry_xml("Hello", "text", "the body");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let loc = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|p| p.rsplit('/').next())
        .and_then(|id| id.parse::<i64>().ok())
        .unwrap();

    let audiences = state.posts.get_post_audiences(loc).await.unwrap();
    assert_eq!(
        audiences,
        vec![common::visibility::AudienceTarget::Subscribers],
        "AtomPub create must adopt the configured default audience"
    );
}

/// Extracts the created post's id from a `POST` response's `Location` header.
fn location_post_id(response: &axum::response::Response) -> i64 {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|p| p.rsplit('/').next())
        .and_then(|id| id.parse::<i64>().ok())
        .expect("Location header should carry the new post id")
}

#[apply(backends)]
#[tokio::test]
async fn create_with_future_published_is_scheduled(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state.clone(), &base);

    // A non-draft entry whose <published> is in the far future schedules the post.
    let xml = entry_xml_with_published("Future post", "body", Some("2099-01-01T00:00:00Z"));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    // The stored post carries the explicit future timestamp.
    let viewer = common::visibility::ViewerIdentity::Anonymous;
    let rec = state
        .posts
        .get_post_by_id(post_id, &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().to_rfc3339(),
        "2099-01-01T00:00:00+00:00"
    );

    // ...and it is invisible on the public permalink at "now".
    let username = "alice".parse().unwrap();
    let public = state
        .posts
        .get_post_by_permalink(
            &username,
            storage::PermalinkDate {
                year: 2099,
                month: 1,
                day: 1,
            },
            &rec.slug,
            &viewer,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert!(
        public.is_none(),
        "future-published AtomPub post must be hidden until due"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_with_past_published_is_live_backdated(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state.clone(), &base);

    // A non-draft entry whose <published> is in the past is live, backdated.
    let xml = entry_xml_with_published("Old post", "body", Some("2000-01-01T00:00:00Z"));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    let viewer = common::visibility::ViewerIdentity::Anonymous;
    let rec = state
        .posts
        .get_post_by_id(post_id, &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().to_rfc3339(),
        "2000-01-01T00:00:00+00:00"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_with_future_published_schedules_post(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (user_id, token) = seed_alice(&state).await;

    // Start from a live post, then PUT a non-draft entry with a future
    // <published>: it must become scheduled (future published_at, hidden).
    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "Old body".to_string(),
            title: Some("Old"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    let app = make_app(state.clone(), &base);

    let xml = entry_xml_with_published("Rescheduled", "new body", Some("2099-06-01T00:00:00Z"));
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let viewer = common::visibility::ViewerIdentity::Anonymous;
    let rec = state
        .posts
        .get_post_by_id(post.post_id, &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().to_rfc3339(),
        "2099-06-01T00:00:00+00:00",
        "update must honor the wire <published> timestamp"
    );
}
