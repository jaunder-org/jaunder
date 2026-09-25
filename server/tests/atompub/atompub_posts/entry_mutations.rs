use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, StatusCode, header},
};
use common::MutationOutcome;
use common::ids::PostId;
use common::tag::{MAX_TAGS_PER_POST, TagLabel};
use common::test_support::parse_post_body;
use common::visibility::ViewerIdentity;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    atompub_at, atompub_get, atompub_location, atompub_post_xml, atompub_put_xml, body_string,
    create_user_and_session, make_app,
};
use storage::{
    PostFormat,
    test_support::{Backend, backends, backends_matrix},
};

use super::fixtures::{entry_xml, etag_of, location_post_id};

#[apply(backends)]
#[tokio::test]
async fn create_post_returns_201_and_is_retrievable(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    // Set default format to Markdown so text entries round-trip properly.
    let user_config = Arc::clone(&env.user_config());
    let user_id = session.user_id;
    let outcome = env
        .write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::set_default_post_format(
                    user_config.as_ref(),
                    transaction,
                    user_id,
                    storage::PostFormat::Markdown,
                )
                .await
            })
        })
        .await
        .unwrap();
    assert!(matches!(outcome, MutationOutcome::Confirmed(())));
    let app = make_app!(&env, base);

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

    let app2 = make_app!(&env, base);
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
async fn invalid_authored_titles_reject_atompub_create_and_update_without_mutation(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, &env.base);
    let created = app
        .clone()
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &entry_xml("Original", "text", "Original body"),
        ))
        .await
        .expect("create a valid Post");
    assert_eq!(created.status(), StatusCode::CREATED);
    let post_id = PostId::from(location_post_id(&created));
    let viewer = ViewerIdentity::local(session.user_id);
    let before = env
        .posts()
        .get_post_by_id(post_id, &viewer)
        .await
        .expect("read original Post")
        .expect("original Post exists");

    for invalid in [
        entry_xml("First\nSecond", "text", "Changed body"),
        entry_xml("\u{2028}", "text", "Changed body"),
        entry_xml(
            "Structured title",
            "text/org",
            "#+TITLE: First\n#+TITLE: Second\nChanged body",
        ),
    ] {
        let rejected_create = app
            .clone()
            .oneshot(atompub_post_xml(&session, "posts", &invalid))
            .await
            .expect("reject invalid title on create");
        assert_eq!(rejected_create.status(), StatusCode::BAD_REQUEST);

        let rejected_update = app
            .clone()
            .oneshot(atompub_put_xml(
                &session,
                &format!("posts/{post_id}"),
                &invalid,
            ))
            .await
            .expect("reject invalid title on update");
        assert_eq!(rejected_update.status(), StatusCode::BAD_REQUEST);
        let after = env
            .posts()
            .get_post_by_id(post_id, &viewer)
            .await
            .expect("read retained Post")
            .expect("original Post remains");
        assert_eq!(after.title, before.title);
        assert_eq!(after.body, before.body);
        assert_eq!(after.slug, before.slug);
        assert_eq!(after.updated_at, before.updated_at);
    }
}

#[apply(backends_matrix)]
#[case::markdown_to_org(
    "{{< youtube dQw4w9WgXcQ >}}",
    "text/markdown",
    PostFormat::Markdown,
    "{{< vimeo 123456789 >}}",
    "text/org",
    PostFormat::Org
)]
#[case::org_to_markdown(
    "{{< vimeo 123456789 >}}",
    "text/org",
    PostFormat::Org,
    "{{< youtube dQw4w9WgXcQ >}}",
    "text/markdown",
    PostFormat::Markdown
)]
#[tokio::test]
async fn shortcode_create_and_update_preserve_member_source(
    backend: Backend,
    #[case] create_source: &str,
    #[case] create_content_type: &str,
    #[case] create_format: PostFormat,
    #[case] update_source: &str,
    #[case] update_content_type: &str,
    #[case] update_format: PostFormat,
) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let canonical_create_source = format!("{create_source}\n");
    let canonical_update_source = format!("{update_source}\n");

    let created = make_app!(&env, base)
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &format!(
                r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Shortcode</title>
  <content type="{create_content_type}"><![CDATA[{create_source}]]></content>
  <app:control><app:draft>no</app:draft></app:control>
</entry>"#
            ),
        ))
        .await
        .expect("create shortcode member");
    assert_eq!(created.status(), StatusCode::CREATED);
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .expect("created member has location")
            .to_str()
            .expect("location is text"),
    );
    let post_id = location_post_id(&created);
    let viewer = common::visibility::ViewerIdentity::local(session.user_id);
    let stored = env
        .posts()
        .get_post_by_id(PostId::from(post_id), &viewer)
        .await
        .expect("read created post")
        .expect("created post exists");
    assert_eq!(stored.body.as_ref(), canonical_create_source);
    assert_eq!(stored.format, create_format);

    let member = make_app!(&env, base)
        .oneshot(
            atompub_at(&session, Method::GET, &location)
                .body(Body::empty())
                .expect("build Member GET"),
        )
        .await
        .expect("read created member");
    assert_eq!(member.status(), StatusCode::OK);
    let member = body_string(member).await;
    assert!(
        member.contains(&format!("type=\"{create_content_type}\"")),
        "AtomPub Member has create content type {create_content_type}: {member}"
    );
    assert!(
        !member.contains("<iframe"),
        "Member must not contain rendered markup: {member}"
    );
    let entry = member
        .parse::<host::atompub::Entry>()
        .expect("Member is an Atom entry");
    assert_eq!(
        entry.content().and_then(|content| content.value()),
        Some(canonical_create_source.as_str()),
        "AtomPub Member preserves canonical create source rather than generated markup"
    );

    let updated = make_app!(&env, base)
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{post_id}"),
            &format!(
                r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Shortcode</title>
  <content type="{update_content_type}"><![CDATA[{update_source}]]></content>
  <app:control><app:draft>no</app:draft></app:control>
</entry>"#
            ),
        ))
        .await
        .expect("update shortcode member");
    assert_eq!(updated.status(), StatusCode::OK);
    let stored = env
        .posts()
        .get_post_by_id(PostId::from(post_id), &viewer)
        .await
        .expect("read updated post")
        .expect("updated post exists");
    assert_eq!(stored.body.as_ref(), canonical_update_source);
    assert_eq!(stored.format, update_format);

    let member = make_app!(&env, base)
        .oneshot(
            atompub_at(&session, Method::GET, &location)
                .body(Body::empty())
                .expect("build Member GET"),
        )
        .await
        .expect("read updated member");
    assert_eq!(member.status(), StatusCode::OK);
    let member = body_string(member).await;
    assert!(
        member.contains(&format!("type=\"{update_content_type}\"")),
        "AtomPub Member has update content type {update_content_type}: {member}"
    );
    assert!(
        !member.contains("<iframe"),
        "Member must not contain rendered markup: {member}"
    );
    let entry = member
        .parse::<host::atompub::Entry>()
        .expect("Member is an Atom entry");
    assert_eq!(
        entry.content().and_then(|content| content.value()),
        Some(canonical_update_source.as_str()),
        "AtomPub Member preserves canonical update source rather than generated markup"
    );
}

#[apply(backends_matrix)]
#[case::org_elisp(
    "text/org",
    PostFormat::Org,
    "#+begin_src elisp\n(message \"created\")\n#+end_src",
    "#+begin_src elisp\n(message \"updated\")\n#+end_src"
)]
#[case::org_haskell(
    "text/org",
    PostFormat::Org,
    "#+begin_src haskell\ngreeting = \"created\"\n#+end_src",
    "#+begin_src haskell\ngreeting = \"updated\"\n#+end_src"
)]
#[case::markdown_elisp(
    "text/markdown",
    PostFormat::Markdown,
    "```elisp\n(message \"created\")\n```",
    "```elisp\n(message \"updated\")\n```"
)]
#[case::markdown_haskell(
    "text/markdown",
    PostFormat::Markdown,
    "```haskell\ngreeting = \"created\"\n```",
    "```haskell\ngreeting = \"updated\"\n```"
)]
#[tokio::test]
async fn highlighted_code_atompub_create_and_update_preserve_member_source(
    backend: Backend,
    #[case] content_type: &str,
    #[case] format: PostFormat,
    #[case] create_source: &str,
    #[case] update_source: &str,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        Arc::clone(&env.users()),
        Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, &env.base);
    let xml = |source: &str| {
        format!(
            "<entry xmlns=\"http://www.w3.org/2005/Atom\"><title>Highlighted</title><content type=\"{content_type}\"><![CDATA[{source}]]></content></entry>"
        )
    };
    let created = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", &xml(create_source)))
        .await
        .expect("create highlighted Post");
    assert_eq!(created.status(), StatusCode::CREATED);
    let post_id = PostId::from(location_post_id(&created));
    let create_etag = etag_of(&created);
    let post = env
        .posts()
        .get_post_by_id(post_id, &ViewerIdentity::local(session.user_id))
        .await
        .unwrap()
        .expect("created Post");
    assert_eq!(post.format, format);
    assert_eq!(post.body.as_ref(), format!("{create_source}\n"));
    assert!(post.rendered_html.as_ref().contains("class=\"j-syn-"));
    let member = app
        .clone()
        .oneshot(atompub_get(&session, &format!("posts/{post_id}")))
        .await
        .expect("Member GET");
    assert_eq!(etag_of(&member), create_etag);
    let entry = body_string(member)
        .await
        .parse::<host::atompub::Entry>()
        .expect("native Member source");
    assert_eq!(
        entry.content().and_then(|content| content.value()),
        Some(format!("{create_source}\n").as_str())
    );

    let updated = app
        .clone()
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{post_id}"),
            &xml(update_source),
        ))
        .await
        .expect("update highlighted Post");
    assert_eq!(updated.status(), StatusCode::OK);
    let update_etag = etag_of(&updated);
    assert_ne!(update_etag, create_etag);
    let post = env
        .posts()
        .get_post_by_id(post_id, &ViewerIdentity::local(session.user_id))
        .await
        .unwrap()
        .expect("updated Post");
    assert_eq!(post.format, format);
    assert_eq!(post.body.as_ref(), format!("{update_source}\n"));
    assert!(post.rendered_html.as_ref().contains("class=\"j-syn-"));
    let member = app
        .oneshot(atompub_get(&session, &format!("posts/{post_id}")))
        .await
        .expect("updated Member GET");
    assert_eq!(etag_of(&member), update_etag);
    let entry = body_string(member)
        .await
        .parse::<host::atompub::Entry>()
        .expect("updated native Member source");
    assert_eq!(
        entry.content().and_then(|content| content.value()),
        Some(format!("{update_source}\n").as_str())
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_post_applies_categories(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let get = make_app!(&env, base)
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

#[apply(backends_matrix)]
#[case::markdown("text/markdown", "Markdown *em* &amp; _under_", "body")]
#[case::org("text/org", "Org /em/ &amp; _under_", "* heading\nbody")]
#[case::html(
    "html",
    "&lt;em&gt;HTML&lt;/em&gt; &amp; source",
    "&lt;p&gt;body&lt;/p&gt;"
)]
#[tokio::test]
async fn atompub_create_get_update_get_preserves_authored_title_syntax(
    backend: Backend,
    #[case] content_type: &str,
    #[case] title_xml: &str,
    #[case] content: &str,
) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);
    let create = app
        .clone()
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &entry_xml(title_xml, content_type, content),
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let post_id = PostId::from(location_post_id(&create));
    let location = atompub_location(
        create
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
    let first_read = body_string(
        make_app!(&env, base)
            .oneshot(
                atompub_at(&session, Method::GET, &location)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(
        first_read.contains(&format!("<title>{title_xml}</title>")),
        "AtomPub GET must preserve authored title source syntax: {first_read}"
    );
    let expected = title_xml
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    assert_eq!(
        env.posts()
            .get_post_by_id(post_id, &ViewerIdentity::local(session.user_id))
            .await
            .unwrap()
            .unwrap()
            .title
            .as_ref()
            .map(ToString::to_string),
        Some(expected.clone())
    );

    let updated_xml = format!("{title_xml} updated");
    let update = make_app!(&env, base)
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{post_id}"),
            &entry_xml(&updated_xml, content_type, content),
        ))
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);
    let expected_updated = updated_xml
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    let reread = body_string(
        make_app!(&env, base)
            .oneshot(
                atompub_at(&session, Method::GET, &location)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    assert!(
        reread.contains(&format!("<title>{updated_xml}</title>")),
        "AtomPub GET after update must preserve authored title source syntax: {reread}"
    );
    assert_eq!(
        env.posts()
            .get_post_by_id(post_id, &ViewerIdentity::local(session.user_id))
            .await
            .unwrap()
            .unwrap()
            .title
            .as_ref()
            .map(ToString::to_string),
        Some(expected_updated)
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_replaces_post_body(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let post = session
        .seed_post()
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;

    let app = make_app!(&env, base);

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
async fn create_rejects_malformed_entry(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

    let response = app
        .oneshot(atompub_post_xml(&session, "posts", "not xml"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn update_removes_categories_not_in_new_entry(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let post = session
        .seed_post()
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post.post_id,
        session.user_id,
        &["original-tag".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let post = session
        .seed_post()
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;

    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let request = match op {
        EmptyEntryOp::Create => atompub_post_xml(&session, "posts", EMPTY_ENTRY_XML),
        EmptyEntryOp::Update => {
            // Create an initial post to update.
            let post = session
                .seed_post()
                .seed(
                    std::sync::Arc::clone(&env.posts()),
                    std::sync::Arc::clone(&env.feed_events()),
                    env.write_scope(),
                )
                .await;
            atompub_put_xml(
                &session,
                &format!("posts/{}", post.post_id),
                EMPTY_ENTRY_XML,
            )
        }
    };

    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let response = make_app!(&env, base)
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
    let ok = make_app!(&env, base)
        .oneshot(atompub_post_xml(
            &session,
            "posts",
            &entry_xml("Some Title", "text/markdown", "* My Title"),
        ))
        .await
        .unwrap();

    assert_eq!(ok.status(), StatusCode::CREATED);
}

/// Org metadata errors are `AtomPub` client errors and must not partially replace
/// the existing member before the full header has been accepted.
#[apply(backends)]
#[tokio::test]
async fn malformed_org_header_update_returns_400_without_mutation(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let post = session
        .seed_post()
        .body(parse_post_body("Original body"))
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;
    let invalid = entry_xml(
        "Replacement",
        "text/org",
        "#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_STATUS published\n\nReplacement body",
    );

    let response = make_app!(&env, base)
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &invalid,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = make_app!(&env, base)
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("Original body"), "body: {body}");
    assert!(!body.contains("Replacement body"), "body: {body}");
}

/// Metadata without remaining Org content names no post and cannot replace an
/// existing member.
#[apply(backends)]
#[tokio::test]
async fn metadata_only_org_update_returns_400_without_mutation(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let post = session
        .seed_post()
        .body(parse_post_body("Original body"))
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;
    let metadata_only = entry_xml(
        "Replacement",
        "text/org",
        "#+TITLE: Header title\n#+PROPERTY: JAUNDER_STATUS draft",
    );

    let response = make_app!(&env, base)
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &metadata_only,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = make_app!(&env, base)
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("Original body"), "body: {body}");
}

/// Update bookkeeping identifies the addressed member; a mismatched
/// `JAUNDER_ID` is rejected before any replacement is persisted.
#[apply(backends)]
#[tokio::test]
async fn mismatched_org_id_returns_400_without_mutation(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let post = session
        .seed_post()
        .body(parse_post_body("Original body"))
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;
    let xml = entry_xml(
        "Replacement",
        "text/org",
        "#+PROPERTY: JAUNDER_ID 999999999\n#+PROPERTY: JAUNDER_STATUS draft\n\nReplacement body",
    );

    let response = make_app!(&env, base)
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = make_app!(&env, base)
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        body_string(response).await.contains("Original body"),
        "mismatched ID must leave the member unchanged"
    );
}

#[apply(backends)]
#[tokio::test]
async fn incoming_j_slug_is_ignored(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let rec = env
        .posts()
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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let rec = env
        .posts()
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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let post = session
        .seed_post()
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post.post_id,
        session.user_id,
        &["original-tag".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

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
