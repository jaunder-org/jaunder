use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use common::ids::PostId;
use common::root_relative_url::RootRelativeUrl;
use common::tag::{MAX_TAGS_PER_POST, TagLabel};
use common::test_support::{
    parse_post_body, parse_post_title, parse_root_relative_url, permalink_date,
};
use common::time::UtcInstant;
use common::visibility::{AudienceTarget, DefaultAudience};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    SeededSession, atompub, atompub_at, atompub_get, atompub_location, atompub_post_xml,
    atompub_put_xml, body_string, create_user_and_session, make_app, setup_with_base_url,
};
use storage::test_support::{
    Backend, TestEnv, backends, backends_matrix, fetch_post_media, media_ref_for, media_url_for,
};

// #560: the AtomPub surface composes absolute URLs, so it *requires* `site.base_url`.
// With base UNSET the handler returns `500` (`HandlerError::BaseUrlRequired`) rather than
// emitting a relative `atom:id` — the negative case for the require-base guard.
#[apply(backends)]
#[tokio::test]
async fn collection_get_without_base_url_returns_500(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    // Deliberately do NOT seed_base_url.
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let response = app.oneshot(atompub_get(&session, "posts")).await.unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn collection_lists_user_posts(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let _post1 = session
        .seed_post()
        .title(parse_post_title("Hello Title One"))
        .seed(&state)
        .await;

    let _post2 = session
        .seed_post()
        .title(parse_post_title("Hello Title Two"))
        .seed(&state)
        .await;

    let app = make_app(&state, &base);

    let response = app.oneshot(atompub_get(&session, "posts")).await.unwrap();

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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session
        .seed_post()
        .body(parse_post_body("# Markdown body"))
        .seed(&state)
        .await;

    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, "posts/999999"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn delete_then_get_is_404(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session.seed_post().seed(&state).await;

    let app = make_app(&state, &base);

    // First, delete the post
    let delete_response = app
        .clone()
        .oneshot(
            atompub(&session, Method::DELETE, &format!("posts/{}", post.post_id))
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    // Then, try to get it
    let get_response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn collection_paging_emits_next_link(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    for _ in 0..2 {
        session.seed_post().seed(&state).await;
    }

    let app = make_app(&state, &base);

    // Page size 1 with 2 posts -> a next link must be present.
    let response = app
        .oneshot(atompub_get(&session, "posts?limit=1"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("rel=\"next\""), "missing next link: {body}");
    assert!(
        body.contains("updated_before="),
        "next link lacks cursor: {body}"
    );
    let href = body
        .split("<link ")
        .find(|link| link.contains("rel=\"next\""))
        .and_then(|link| link.split("href=\"").nth(1))
        .and_then(|tail| tail.split('"').next())
        .expect("next link exposes href")
        .replace("&amp;", "&");
    let updated_before = url::Url::parse(&href)
        .expect("next link href is an absolute URL")
        .query_pairs()
        .find_map(|(key, value)| (key == "updated_before").then(|| value.into_owned()))
        .expect("next link exposes updated_before");
    let parsed_cursor: UtcInstant = updated_before
        .parse()
        .expect("next link cursor is an RFC3339 instant");
    assert_eq!(
        updated_before,
        parsed_cursor.to_string(),
        "next link cursor should use canonical UTC spelling"
    );
    assert_eq!(
        body.matches("<entry").count(),
        1,
        "expected exactly one entry"
    );
}

#[apply(backends)]
#[tokio::test]
async fn collection_clamps_out_of_range_limit(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    // Seed 51 posts so the `1..=50` page-size cap is observable (50 < 51).
    for _ in 0..51 {
        session.seed_post().seed(&state).await;
    }

    let app = make_app(&state, &base);

    // `?limit=999` clamps to PageSize::MAX (50), not 51.
    let over = app
        .clone()
        .oneshot(atompub_get(&session, "posts?limit=999"))
        .await
        .unwrap();
    assert_eq!(over.status(), StatusCode::OK);
    let over_body = body_string(over).await;
    assert_eq!(
        over_body.matches("<entry").count(),
        50,
        "?limit=999 should clamp to the 50-item max"
    );

    // `?limit=0` clamps to PageSize::MIN (1).
    let under = app
        .oneshot(atompub_get(&session, "posts?limit=0"))
        .await
        .unwrap();
    assert_eq!(under.status(), StatusCode::OK);
    let under_body = body_string(under).await;
    assert_eq!(
        under_body.matches("<entry").count(),
        1,
        "?limit=0 should clamp to the 1-item min"
    );
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    if seed_post {
        session.seed_post().seed(&state).await;
    }
    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, &format!("posts?{query}")))
        .await
        .unwrap();

    assert_eq!(response.status(), expected);
}

#[apply(backends)]
#[tokio::test]
async fn collection_empty_returns_feed_without_entries(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let response = app.oneshot(atompub_get(&session, "posts")).await.unwrap();

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

#[apply(backends)]
#[tokio::test]
async fn create_post_returns_201_and_is_retrievable(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    // Set default format to Markdown so text entries round-trip properly
    storage::set_default_post_format(
        state.user_config.as_ref(),
        session.user_id,
        storage::PostFormat::Markdown,
    )
    .await
    .unwrap();
    let app = make_app(&state, &base);

    let xml = entry_xml("Hello", "text", "the body");
    let response = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", &xml))
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

    let app2 = make_app(&state, &base);
    let loc_path = atompub_location(&loc.unwrap());
    let get_response = app2
        .oneshot(
            atompub_at(&session, Method::GET, &loc_path)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = entry_xml("Hello", "text", "the body");
    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = entry_xml("H", "html", "&lt;p&gt;hi&lt;/p&gt;");
    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = entry_xml("Formatted", content_type, content);
    let response = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let location = atompub_location(
        response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    // GET the member back: it must echo the same content media type.
    let get = make_app(&state, &base)
        .oneshot(
            atompub_at(&session, Method::GET, &location)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session.seed_post().seed(&state).await;

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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session.seed_post().seed(&state).await;

    let app = make_app(&state, &base);

    let xml = entry_xml("New", "text", "new body");
    let response = app
        .oneshot(
            atompub(&session, Method::PUT, &format!("posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, "\"0\"") // Wrong ETag
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_post_xml(&session, "posts", "not xml"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn update_removes_categories_not_in_new_entry(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session.seed_post().seed(&state).await;

    state
        .posts
        .set_post_tags(post.post_id, &["original-tag".parse::<TagLabel>().unwrap()])
        .await
        .unwrap();

    let app = make_app(&state, &base);

    // Update without the tag
    let xml = entry_xml("Title", "text", "new body");
    let response = app
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session.seed_post().seed(&state).await;

    let app = make_app(&state, &base);

    let xml = entry_xml("Updated", "text", "updated body");
    let response = app
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let request = match op {
        EmptyEntryOp::Create => atompub_post_xml(&session, "posts", EMPTY_ENTRY_XML),
        EmptyEntryOp::Update => {
            // Create an initial post to update.
            let post = session.seed_post().seed(&state).await;
            atompub_put_xml(
                &session,
                &format!("posts/{}", post.post_id),
                EMPTY_ENTRY_XML,
            )
        }
    };

    let app = make_app(&state, &base);

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// The boundary twin of `perform_post_creation_rejects_title_only_org_body`.
//
// An Org entry whose whole content is a title source canonicalizes to nothing
// (ADR-0024) and is rejected (#811 decision 2). Asserting 400 and not 500 is the
// point — this is the client's input being wrong, not the server falling over.
#[apply(backends)]
#[tokio::test]
async fn create_title_only_org_entry_returns_400(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let response = make_app(&state, &base)
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &entry_xml("Some Title", "text/org", "* My Title"),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // The discriminator: identical bytes as Markdown are ordinary content, so the
    // rejection is Org's title-stripping rather than anything about the request.
    let ok = make_app(&state, &base)
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &entry_xml("Some Title", "text/markdown", "* My Title"),
        ))
        .await
        .unwrap();

    assert_eq!(ok.status(), StatusCode::CREATED);
}

#[apply(backends)]
#[tokio::test]
async fn create_draft_entry_is_unpublished(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Draft</title>
  <content type="text">draft body</content>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;

    let response = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let location = atompub_location(
        response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let get = app
        .oneshot(
            atompub_at(&session, Method::GET, &location)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session
        .seed_post()
        .title(parse_post_title("My Post"))
        .seed(&state)
        .await;

    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(
        body.contains("xmlns:j=\"https://jaunder.org/ns/atompub\""),
        "member should declare xmlns:j: {body}"
    );
    assert!(
        body.contains(&format!("<j:slug>{}</j:slug>", post.slug.as_ref())),
        "member should carry the post's slug as j:slug: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn incoming_j_slug_is_ignored(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

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
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    let viewer = common::visibility::ViewerIdentity::local(session.user_id);
    let rec = state
        .posts
        .get_post_by_id(PostId::from(post_id), &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        rec.slug, "client-supplied",
        "incoming j:slug must not become the stored slug"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_with_blank_title_stores_an_untitled_post(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // A whitespace-only <title> means the client supplied no title — it is absence,
    // not a client error (#830). `PostTitle`'s FromStr rejects it and the mapping
    // turns that into `None`, so the post is stored untitled with a body-derived
    // slug rather than carrying a blank title or failing with a 400.
    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>   </title>
  <content type="text">first body line</content>
</entry>"#;

    let response = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    let viewer = common::visibility::ViewerIdentity::local(session.user_id);
    let rec = state
        .posts
        .get_post_by_id(PostId::from(post_id), &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rec.title, None, "a blank <title> must store as untitled");
    assert_eq!(rec.slug, "first-body-line");
}

#[apply(backends)]
#[tokio::test]
async fn create_skips_invalid_category(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Cat</title>
  <content type="text">body</content>
  <category term="has spaces"/>
</entry>"#;

    let response = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", xml))
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

/// #771 (D9/AC8, ADR-0092): an entry carrying more than `MAX_TAGS_PER_POST`
/// distinct categories is *rejected* rather than written — the batched tag write
/// stays capped by construction. Validation runs before any storage mutation, so
/// the post is not created either.
#[apply(backends)]
#[tokio::test]
async fn create_with_over_cap_categories_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let categories = (0..=MAX_TAGS_PER_POST)
        .map(|n| format!("  <category term=\"tag{n}\"/>"))
        .collect::<Vec<_>>()
        .join("\n");
    let xml = format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Too many</title>
  <content type="text">body</content>
{categories}
</entry>"#
    );

    let response = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // "Rejected rather than written": nothing was created on the way to the 400.
    let listed = app.oneshot(atompub_get(&session, "posts")).await.unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body = body_string(listed).await;
    assert_eq!(
        body.matches("<entry").count(),
        0,
        "over-cap entry must not create a post: {body}"
    );
}

/// #771 (D9/D12): the update door carries the same cap guard as the create door.
/// Without it `set_post_tags` would receive an unbounded `desired`, which is the
/// precondition ADR-0092 relies on being enforced at the callers — so the guard
/// needs its own test rather than riding on the create test's coverage.
#[apply(backends)]
#[tokio::test]
async fn update_with_over_cap_categories_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let post = session.seed_post().seed(&state).await;

    state
        .posts
        .set_post_tags(post.post_id, &["original-tag".parse::<TagLabel>().unwrap()])
        .await
        .unwrap();

    let app = make_app(&state, &base);

    let categories = (0..=MAX_TAGS_PER_POST)
        .map(|n| format!("  <category term=\"tag{n}\"/>"))
        .collect::<Vec<_>>()
        .join("\n");
    let xml = format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Too many</title>
  <content type="text">body</content>
{categories}
</entry>"#
    );

    let response = app
        .clone()
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // "Rejected rather than written": the post's tags are exactly what they were.
    let fetched = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    let body = body_string(fetched).await;
    assert!(
        body.contains("term=\"original-tag\""),
        "over-cap update must leave the existing tag in place: {body}"
    );
    assert_eq!(
        body.matches("<category").count(),
        1,
        "over-cap update must not add tags: {body}"
    );
}

/// #771 (D9): categories that collapse to the same canonical slug become one tag,
/// and the first occurrence's casing is the one that survives — the same dedupe
/// the web door applies, now on the `AtomPub` door too.
#[apply(backends)]
#[tokio::test]
async fn create_dedupes_categories_keeping_first_casing(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Dupes</title>
  <content type="text">body</content>
  <category term="Rust"/>
  <category term="rust"/>
</entry>"#;

    let response = app
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_string(response).await;
    assert_eq!(
        body.matches("<category").count(),
        1,
        "duplicate categories should collapse to one tag: {body}"
    );
    assert!(
        body.contains("term=\"Rust\""),
        "the first occurrence's casing should win: {body}"
    );
}

/// #771 (D9) narrows `AtomPub` category handling for *over-cap* entries only — a
/// single *malformed* term is still skipped leniently rather than failing the
/// whole entry (R5, `docs/atompub-marsedit-acceptance.md`).
#[apply(backends)]
#[tokio::test]
async fn create_skips_malformed_category_beside_a_valid_one(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Lenient</title>
  <content type="text">body</content>
  <category term="rust"/>
  <category term="has spaces"/>
</entry>"#;

    let response = app
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_string(response).await;
    assert_eq!(
        body.matches("<category").count(),
        1,
        "the valid term should survive alone: {body}"
    );
    assert!(
        body.contains("term=\"rust\""),
        "the valid term should be kept: {body}"
    );
    assert!(
        !body.contains("has spaces"),
        "malformed category leaked: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_keeps_unchanged_category(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let with_rust = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <content type="text">body</content>
  <category term="rust"/>
</entry>"#;

    let created = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", with_rust))
        .await
        .unwrap();
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    // PUT the same category back -> add-loop and remove-loop both take their
    // "already in sync" branches.
    let updated = app
        .oneshot(
            atompub_at(&session, Method::PUT, &location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .body(Body::from(with_rust.to_owned()))
                .expect("failed to build atompub request"),
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <content type="text">body</content>
</entry>"#;

    let created = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
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
            atompub_at(&session, Method::PUT, &location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, etag)
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
}

const ETAG_POST_XML: &str = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <category term="rust"/>
  <content type="text">body</content>
</entry>"#;

/// POST `ETAG_POST_XML` as alice; return the create response's (`Location`, `ETag`).
async fn create_location_etag(
    app: axum::Router,
    session: &SeededSession,
) -> (RootRelativeUrl, String) {
    let created = app
        .oneshot(atompub_post_xml(session, "posts", ETAG_POST_XML))
        .await
        .unwrap();
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
    let etag = created
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    (location, etag)
}

/// POST `ETAG_POST_XML` as alice and return the create response's `ETag`.
async fn create_etag(app: axum::Router, session: &SeededSession) -> String {
    create_location_etag(app, session).await.1
}

/// GET `location` as alice, returning the response status.
async fn get_status(
    app: axum::Router,
    session: &SeededSession,
    location: &RootRelativeUrl,
) -> StatusCode {
    app.oneshot(
        atompub_at(session, Method::GET, location)
            .body(Body::empty())
            .expect("failed to build atompub GET request"),
    )
    .await
    .unwrap()
    .status()
}

/// How a DELETE request carries (or omits) `If-Match`. `MatchingEtag` uses the
/// post's current `ETag` (captured at creation); the literals model a stale
/// precondition (`"0"`) and the `*` wildcard.
enum DeleteIfMatch {
    Absent,
    Literal(&'static str),
    MatchingEtag,
}

// AC7: `If-Match` on DELETE. A stale precondition is refused (412) and the post
// survives; a matching ETag, an absent header, and the `*` wildcard each delete
// unconditionally (204) and the post is then gone. Uses `backends_matrix` so each
// precondition case runs on both backends.
#[apply(backends_matrix)]
#[case::stale(DeleteIfMatch::Literal("\"0\""), StatusCode::PRECONDITION_FAILED, true)]
#[case::matching(DeleteIfMatch::MatchingEtag, StatusCode::NO_CONTENT, false)]
#[case::absent(DeleteIfMatch::Absent, StatusCode::NO_CONTENT, false)]
#[case::wildcard(DeleteIfMatch::Literal("*"), StatusCode::NO_CONTENT, false)]
#[tokio::test]
async fn delete_if_match_precondition(
    backend: Backend,
    #[case] if_match: DeleteIfMatch,
    #[case] expected_status: StatusCode,
    #[case] post_survives: bool,
) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let (location, etag) = create_location_etag(app.clone(), &session).await;

    let builder = atompub_at(&session, Method::DELETE, &location);
    let builder = match if_match {
        DeleteIfMatch::Absent => builder,
        DeleteIfMatch::Literal(value) => builder.header(header::IF_MATCH, value),
        DeleteIfMatch::MatchingEtag => builder.header(header::IF_MATCH, etag),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), expected_status);

    let expected_after = if post_survives {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    assert_eq!(get_status(app, &session, &location).await, expected_after);
}

#[apply(backends)]
#[tokio::test]
async fn editing_content_via_put_changes_etag(#[case] backend: Backend) {
    // AC4 (HTTP): a PUT that changes the body changes the ETag end-to-end.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let (location, e1) = create_location_etag(app.clone(), &session).await;

    let edited = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <category term="rust"/>
  <content type="text">a different body</content>
</entry>"#;
    let updated = app
        .oneshot(
            atompub_at(&session, Method::PUT, &location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, &e1)
                .body(Body::from(edited))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let e2 = updated
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(e1, e2);
}

#[apply(backends)]
#[tokio::test]
async fn etag_is_content_hash_format(#[case] backend: Backend) {
    // AC1: the emitted ETag is a strong, quoted "sha256-<64 lowercase hex>" token.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let etag = create_etag(app, &session).await;
    let hex = etag
        .strip_prefix("\"sha256-")
        .and_then(|s| s.strip_suffix('"'))
        .expect("ETag is a quoted sha256- token");
    assert_eq!(hex.len(), 64);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    );
}

#[apply(backends)]
#[tokio::test]
async fn identical_posts_share_etag(#[case] backend: Backend) {
    // AC2: two distinct posts with identical content get the same ETag — the
    // per-post id / tag ids / slug are excluded from the hash.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let e1 = create_etag(app.clone(), &session).await;
    let e2 = create_etag(app, &session).await;
    assert_eq!(e1, e2);
}

#[apply(backends)]
#[tokio::test]
async fn idempotent_reput_keeps_etag(#[case] backend: Backend) {
    // AC3 + AC5: re-PUT byte-identical content → the ETag is unchanged (a
    // timestamp ETag would have bumped on the write).
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let created = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", ETAG_POST_XML))
        .await
        .unwrap();
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
    let e1 = created
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let updated = app
        .oneshot(
            atompub_at(&session, Method::PUT, &location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, &e1)
                .body(Body::from(ETAG_POST_XML))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let e2 = updated
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(e1, e2);
}

#[apply(backends)]
#[tokio::test]
async fn update_preserves_non_public_targeting(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    // AtomPub has no audience field, so post creation is the per-Post boundary
    // that widens this instance-wide default.
    state
        .site_config
        .set_default_audience(&default_audience)
        .await
        .unwrap();

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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // A non-draft entry whose <published> is in the far future schedules the post.
    let xml = entry_xml_with_published("Future post", "body", Some("2099-01-01T00:00:00Z"));
    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    // The owner may inspect the scheduled private post's persisted timestamp.
    let owner = common::visibility::ViewerIdentity::local(session.user_id);
    let rec = state
        .posts
        .get_post_by_id(PostId::from(post_id), &owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().to_rfc3339(),
        "2099-01-01T00:00:00+00:00"
    );

    let viewer = common::visibility::ViewerIdentity::Anonymous;

    // ...and it is invisible on the public permalink at "now".
    let public = state
        .posts
        .get_post_by_permalink(
            &session.username,
            permalink_date(2099, 1, 1),
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // A non-draft entry whose <published> is in the past is live, backdated.
    let xml = entry_xml_with_published("Old post", "body", Some("2000-01-01T00:00:00Z"));
    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    let viewer = common::visibility::ViewerIdentity::local(session.user_id);
    let rec = state
        .posts
        .get_post_by_id(PostId::from(post_id), &viewer)
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
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    // Start from a live post, then PUT a non-draft entry with a future
    // <published>: it must become scheduled (future published_at, hidden).
    let post = session.seed_post().seed(&state).await;

    let app = make_app(&state, &base);

    let xml = entry_xml_with_published("Rescheduled", "new body", Some("2099-06-01T00:00:00Z"));
    let response = app
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
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

/// POST a create as alice, optionally with an `Idempotency-Key`.
async fn create_post_keyed(
    app: axum::Router,
    session: &SeededSession,
    xml: &str,
    idempotency_key: Option<&str>,
) -> axum::response::Response {
    let mut builder = atompub(session, Method::POST, "posts")
        .header(header::CONTENT_TYPE, "application/atom+xml");
    if let Some(key) = idempotency_key {
        builder = builder.header("Idempotency-Key", key);
    }
    app.oneshot(builder.body(Body::from(xml.to_string())).unwrap())
        .await
        .unwrap()
}

fn location_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("response has a Location header")
        .to_string()
}

#[apply(backends)]
#[tokio::test]
async fn create_with_same_idempotency_key_dedups(#[case] backend: Backend) {
    // AC-S1: the same key creates one post; the retry returns it as 200.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    let first = create_post_keyed(app.clone(), &session, &xml, Some("idem-1")).await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let loc1 = location_of(&first);
    let etag1 = etag_of(&first);
    let body1 = body_string(first).await;

    let second = create_post_keyed(app, &session, &xml, Some("idem-1")).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        location_of(&second),
        loc1,
        "retry returns the original post"
    );
    assert_eq!(etag_of(&second), etag1, "retry returns the same ETag");
    assert_eq!(
        body_string(second).await,
        body1,
        "retry returns the same body"
    );
}

fn etag_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("response has an ETag header")
        .to_string()
}

#[apply(backends)]
#[tokio::test]
async fn create_with_fresh_idempotency_key_is_201(#[case] backend: Backend) {
    // AC-S2: distinct keys create distinct posts.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    let first = create_post_keyed(app.clone(), &session, &xml, Some("k-a")).await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let second = create_post_keyed(app, &session, &xml, Some("k-b")).await;
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_ne!(location_of(&first), location_of(&second));
}

#[apply(backends)]
#[tokio::test]
async fn create_without_idempotency_key_is_201(#[case] backend: Backend) {
    // AC-S3: no header → create as today.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    let response = create_post_keyed(app, &session, &xml, None).await;
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[apply(backends)]
#[tokio::test]
async fn create_writes_the_entrys_media_rows(#[case] backend: Backend) {
    // The AtomPub half of A14 (#711). The storage tests cover the web path; this one
    // drives the router, because the AtomPub handler reaches storage by its own route
    // and nothing in `storage` would notice if that path stopped recording references.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // An `html` entry, so the escaped `<img>` is unescaped into the stored body and the
    // renderer sees a real element rather than literal text.
    let content = format!("&lt;img src=\"{}\"&gt;", media_url_for("photo.jpg"));
    let xml = entry_xml("Photo entry", "html", &content);

    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = PostId::from(location_post_id(&response));

    assert_eq!(
        fetch_post_media(&base, post_id).await,
        vec![media_ref_for("photo.jpg")]
    );
}
