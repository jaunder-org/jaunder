use axum::http::{StatusCode, header};
use common::test_support::parse_post_title;
use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{atompub_get, body_string, create_user_and_session, make_app};
use storage::test_support::{Backend, backends, backends_matrix};

// #560: the AtomPub surface composes absolute URLs, so it *requires* `site.base_url`.
// With base UNSET the handler returns `500` (`HandlerError::BaseUrlRequired`) rather than
// emitting a relative `atom:id` — the negative case for the require-base guard.
#[apply(backends)]
#[tokio::test]
async fn collection_get_without_base_url_returns_500(#[case] backend: Backend) {
    let env = backend.setup().base_url(None).await;
    let base = &env.base;
    // Deliberately omit the shared fixture's base URL.
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

    let response = app.oneshot(atompub_get(&session, "posts")).await.unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn collection_lists_user_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let _post1 = session
        .seed_post()
        .title(parse_post_title("Hello Title One"))
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;

    let _post2 = session
        .seed_post()
        .title(parse_post_title("Hello Title Two"))
        .seed(
            std::sync::Arc::clone(&env.posts()),
            std::sync::Arc::clone(&env.feed_events()),
            env.write_scope(),
        )
        .await;

    let app = make_app!(&env, base);

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
    assert!(
        !body.contains("<rights") && !body.contains("rel=\"license\""),
        "AtomPub Collection must not gain Syndication Feed rights metadata: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn collection_paging_emits_next_link(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    for _ in 0..2 {
        session
            .seed_post()
            .seed(
                std::sync::Arc::clone(&env.posts()),
                std::sync::Arc::clone(&env.feed_events()),
                env.write_scope(),
            )
            .await;
    }

    let app = make_app!(&env, base);

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

// Each Collection Entry advertises the same validator as its own Member response,
// even when the user retrieves the Collection a page at a time.
#[apply(backends)]
#[tokio::test]
async fn collection_entries_advertise_member_etags_across_pages(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    for _ in 0..2 {
        session
            .seed_post()
            .seed(env.posts(), env.feed_events(), env.write_scope())
            .await;
    }
    let app = make_app!(&env, base);
    let first = app
        .clone()
        .oneshot(atompub_get(&session, "posts?limit=1"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = body_string(first).await;
    let first_feed: host::atompub::Feed = first_body.parse().expect("valid Atom feed");
    let next = first_feed
        .links()
        .iter()
        .find(|link| link.rel() == "next")
        .expect("second page")
        .href()
        .to_string();
    let prefix = format!("https://example.com/atompub/{}/", session.username);
    let second = app
        .clone()
        .oneshot(atompub_get(
            &session,
            next.strip_prefix(&prefix)
                .expect("same-origin collection cursor"),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = body_string(second).await;
    let second_feed: host::atompub::Feed = second_body.parse().expect("valid Atom feed");
    for (xml, feed) in [(&first_body, &first_feed), (&second_body, &second_feed)] {
        assert_eq!(feed.entries().len(), 1);
        let entry = &feed.entries()[0];
        let edit = entry
            .links()
            .iter()
            .find(|link| link.rel() == "edit")
            .expect("edit link");
        let member = app
            .clone()
            .oneshot(atompub_get(
                &session,
                edit.href()
                    .strip_prefix(&prefix)
                    .expect("same-origin member"),
            ))
            .await
            .unwrap();
        assert_eq!(member.status(), StatusCode::OK);
        let etag = member.headers()[header::ETAG].to_str().unwrap();
        let marker = format!(
            "<j:etag xmlns:j=\"https://jaunder.org/ns/atompub\">&quot;{}&quot;</j:etag>",
            etag.trim_matches('"')
        );
        assert_eq!(xml.matches("<j:etag ").count(), 1, "one validator: {xml}");
        assert!(
            xml.contains(&marker),
            "validator must match Member header: {xml}"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn collection_clamps_out_of_range_limit(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    // Seed 51 posts so the `1..=50` page-size cap is observable (50 < 51).
    for _ in 0..51 {
        session
            .seed_post()
            .seed(
                std::sync::Arc::clone(&env.posts()),
                std::sync::Arc::clone(&env.feed_events()),
                env.write_scope(),
            )
            .await;
    }

    let app = make_app!(&env, base);

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
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    if seed_post {
        session
            .seed_post()
            .seed(
                std::sync::Arc::clone(&env.posts()),
                std::sync::Arc::clone(&env.feed_events()),
                env.write_scope(),
            )
            .await;
    }
    let app = make_app!(&env, base);

    let response = app
        .oneshot(atompub_get(&session, &format!("posts?{query}")))
        .await
        .unwrap();

    assert_eq!(response.status(), expected);
}

#[apply(backends)]
#[tokio::test]
async fn collection_empty_returns_feed_without_entries(#[case] backend: Backend) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, base);

    let response = app.oneshot(atompub_get(&session, "posts")).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("<feed"));
    assert_eq!(body.matches("<entry").count(), 0);
}
