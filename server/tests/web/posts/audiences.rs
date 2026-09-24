use axum::http::StatusCode;
use common::render::PostFormat;
use common::test_support::{parse_audience_name, parse_post_body};
use common::visibility::{AudienceSelection, AudienceTarget};
use server_fn::ServerFn;
use web::posts::PostInputs;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    confirmed_created_post, confirmed_mutation, create_post_json, create_user_and_session,
    make_app, post_form, post_json, update_post_json,
};
use storage::test_support::{Backend, backends, confirmed_for, fp};

use super::fixtures::get_post_form;
use storage::{SessionStorage, UserStorage, WriteScope};

// ── Audience-picker server fns ────────────────────────────────

/// Creates a user and returns a session cookie for the audience-picker tests.
async fn author_with_cookie(
    users: std::sync::Arc<dyn UserStorage>,
    sessions: std::sync::Arc<dyn SessionStorage>,
    write_scope: WriteScope,
) -> String {
    user_with_cookie(users, sessions, write_scope).await
}

/// Creates a user and returns a session cookie.
async fn user_with_cookie(
    users: std::sync::Arc<dyn UserStorage>,
    sessions: std::sync::Arc<dyn SessionStorage>,
    write_scope: WriteScope,
) -> String {
    create_user_and_session(users, sessions, write_scope)
        .await
        .cookie()
}

#[apply(backends)]
#[tokio::test]
async fn default_audience_selection_returns_private_by_default(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = author_with_cookie(env.users(), env.sessions(), env.write_scope()).await;

    let (status, body) = post_json(
        app.clone(),
        <web::posts::GetDefaultAudienceSelection as ServerFn>::PATH,
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let selection: AudienceSelection = serde_json::from_str(&body).unwrap();
    assert!(!selection.public);
    assert!(!selection.subscribers);
    assert!(selection.named.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn default_audience_selection_rejects_unauthenticated(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = post_json(
        app.clone(),
        <web::posts::GetDefaultAudienceSelection as ServerFn>::PATH,
        serde_json::json!({}),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_returns_private_for_new_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = author_with_cookie(env.users(), env.sessions(), env.write_scope()).await;

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("Hello"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created = confirmed_created_post(&body);

    let (status, body) = post_form(
        app.clone(),
        <web::posts::GetAudienceSelection as ServerFn>::PATH,
        format!("post_id={}", created.post_id),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let selection: AudienceSelection = serde_json::from_str(&body).unwrap();
    // With no configured defaults, an omitted audience resolves to Private.
    assert!(!selection.public);
    assert!(!selection.subscribers);
    assert!(selection.named.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn web_audience_set_survives_create_update_and_owner_read(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = author.cookie();
    let author_id = author.user_id;
    let audiences = env.audiences();
    let new_audience = |name: &'static str| {
        let audiences = std::sync::Arc::clone(&audiences);
        let scope = env.write_scope();
        async move {
            confirmed_for(
                scope
                    .run(move |tx| {
                        Box::pin(async move {
                            audiences
                                .create_audience(tx, author_id, &parse_audience_name(name))
                                .await
                        })
                    })
                    .await
                    .unwrap(),
                "named audience",
            )
        }
    };
    let friends = new_audience("Friends").await;
    let family = new_audience("Family").await;
    let selected = AudienceSelection {
        public: true,
        subscribers: true,
        named: vec![friends, family],
    };
    let read_selection = |post_id| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            let (status, body) = post_form(
                app,
                <web::posts::GetAudienceSelection as ServerFn>::PATH,
                format!("post_id={post_id}"),
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "read body: {body}");
            serde_json::from_str::<AudienceSelection>(&body).unwrap()
        }
    };

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            audience: Some(selected.clone()),
            ..PostInputs::new(parse_post_body("# Audience set"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created = confirmed_created_post(&body);
    assert_eq!(read_selection(created.post_id).await, selected);
    let stored = env
        .posts()
        .get_post_audiences(created.post_id)
        .await
        .unwrap();
    assert_eq!(stored.len(), 4);
    for target in [
        AudienceTarget::Public,
        AudienceTarget::Subscribers,
        AudienceTarget::Named(friends),
        AudienceTarget::Named(family),
    ] {
        assert!(stored.contains(&target), "missing target {target:?}");
    }

    // A Public target still admits anonymous readers and syndication even
    // alongside Subscribers and Named targets; removing it must remove both.
    let record = env
        .posts()
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::local(author_id),
        )
        .await
        .unwrap()
        .unwrap();
    let date = jiff::tz::Offset::UTC
        .to_datetime(record.created_at.value())
        .date();
    let (year, month, day) = (
        i32::from(date.year()),
        u32::try_from(date.month()).unwrap(),
        u32::try_from(date.day()).unwrap(),
    );
    let (status, body) = get_post_form(
        app.clone(),
        &author.username,
        year,
        month,
        day,
        &created.slug,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "mixed-target anonymous read: {body}"
    );
    let feed_path = fp(&format!("/~{}/feed.rss", author.username));
    let snapshot = env.publisher().snapshot().await.unwrap();
    let mixed_feed =
        jaunder::feed::regenerate::render(&snapshot, env.posts().as_ref(), feed_path.clone())
            .await
            .unwrap();
    assert!(mixed_feed.representation().body().contains("Audience set"));

    let narrower = AudienceSelection {
        public: false,
        ..selected.clone()
    };
    let (status, body) = update_post_json(
        app.clone(),
        created.post_id,
        PostInputs {
            publish: Some(true),
            audience: Some(narrower.clone()),
            ..PostInputs::new(parse_post_body("# Audience set"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let _ = confirmed_mutation::<web::posts::SavedPost>(&body);
    assert_eq!(read_selection(created.post_id).await, narrower);
    let (status, _) = get_post_form(
        app.clone(),
        &author.username,
        year,
        month,
        day,
        &created.slug,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let narrower_feed =
        jaunder::feed::regenerate::render(&snapshot, env.posts().as_ref(), feed_path)
            .await
            .unwrap();
    assert!(
        !narrower_feed
            .representation()
            .body()
            .contains("Audience set")
    );

    let named_only = AudienceSelection {
        subscribers: false,
        ..narrower
    };
    let (status, body) = update_post_json(
        app.clone(),
        created.post_id,
        PostInputs {
            publish: Some(true),
            audience: Some(named_only.clone()),
            ..PostInputs::new(parse_post_body("# Audience set"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "named-only body: {body}");
    assert_eq!(read_selection(created.post_id).await, named_only);

    let (status, body) = update_post_json(
        app.clone(),
        created.post_id,
        PostInputs {
            publish: Some(true),
            audience: Some(AudienceSelection::default()),
            ..PostInputs::new(parse_post_body("# Audience set"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "private body: {body}");
    assert_eq!(
        read_selection(created.post_id).await,
        AudienceSelection::default()
    );
    assert!(
        env.posts()
            .get_post_audiences(created.post_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_rejects_missing_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = author_with_cookie(env.users(), env.sessions(), env.write_scope()).await;

    let (status, body) = post_form(
        app.clone(),
        <web::posts::GetAudienceSelection as ServerFn>::PATH,
        "post_id=99999".to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_rejects_non_owner(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author_cookie = user_with_cookie(env.users(), env.sessions(), env.write_scope()).await;
    let other_cookie = user_with_cookie(env.users(), env.sessions(), env.write_scope()).await;

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("Hello"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created = confirmed_created_post(&body);

    // A different user must not learn another author's targeting.
    let (status, body) = post_form(
        app.clone(),
        <web::posts::GetAudienceSelection as ServerFn>::PATH,
        format!("post_id={}", created.post_id),
        Some(&other_cookie),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}
