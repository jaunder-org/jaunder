use axum::{Router, http::StatusCode};
use common::ids::PostId;
use common::render::PostFormat;
use common::test_support::parse_post_body;
use common::visibility::AudienceSelection;
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::test_support::{Backend, backends};
use web::posts::{
    BulkManageOperation, BulkManageResult, ManageAudienceFilter, ManagePostsPage,
    ManagePublicationState, ManageSelectionIntent, ManagementSelectionSnapshot, PostInputs,
};

use crate::helpers::{
    confirmed_created_post, create_post_json, create_user_and_session, make_app, post_form,
    post_json,
};

async fn create_draft(app: Router, cookie: &str, title: &str) -> PostId {
    let (status, body) = create_post_json(
        app,
        PostInputs {
            publish: Some(false),
            audience: Some(AudienceSelection {
                public: true,
                subscribers: false,
                named: Vec::new(),
            }),
            ..PostInputs::new(
                parse_post_body(&format!("# {title}\n\nbody")),
                PostFormat::Markdown,
            )
        },
        Some(cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    confirmed_created_post(&body).post_id
}

async fn list_managed(app: Router, cookie: Option<&str>, limit: u32) -> (StatusCode, String) {
    list_managed_after(app, cookie, limit, None).await
}

async fn list_managed_after(
    app: Router,
    cookie: Option<&str>,
    limit: u32,
    cursor: Option<web::posts::ManagePostsCursor>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::ListManagedPosts as ServerFn>::PATH,
        serde_json::to_value(web::posts::ListManagedPosts {
            state: ManagePublicationState::All,
            audience: ManageAudienceFilter::All,
            search: String::new(),
            cursor,
            limit: Some(limit.to_string().parse().expect("valid page size")),
        })
        .unwrap(),
        cookie,
    )
    .await
}

async fn resolve(app: Router, cookie: &str, intent: ManageSelectionIntent) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::ResolveManagementSelection as ServerFn>::PATH,
        serde_json::to_value(web::posts::ResolveManagementSelection { intent }).unwrap(),
        Some(cookie),
    )
    .await
}

async fn execute(
    app: Router,
    cookie: Option<&str>,
    snapshot: ManagementSelectionSnapshot,
    operation: BulkManageOperation,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::ExecuteManagementOperation as ServerFn>::PATH,
        serde_json::to_value(web::posts::ExecuteManagementOperation {
            snapshot,
            operation,
        })
        .unwrap(),
        cookie,
    )
    .await
}

#[apply(backends)]
#[tokio::test]
async fn manage_posts_http_is_owner_scoped_bounded_and_snapshots_all_matches(
    #[case] backend: Backend,
) {
    // Listing and confirmation both enforce ownership in storage; all-matching is
    // independent of the one-row page requested by the browser.
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let foreign = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let owner_cookie = owner.cookie();
    let foreign_cookie = foreign.cookie();
    let first = create_draft(app.clone(), &owner_cookie, "First").await;
    let second = create_draft(app.clone(), &owner_cookie, "Second").await;
    let third = create_draft(app.clone(), &owner_cookie, "Third").await;
    // Three equal update keys force the returned HTTP cursor to carry the ID
    // tiebreaker. This does not rely on the creation clock being coarse.
    let tied: common::time::UtcInstant = "2020-01-01T00:00:00.000000000Z".parse().unwrap();
    storage::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query("UPDATE posts SET updated_at = $1 WHERE post_id IN ($2, $3, $4)")
            .bind(tied)
            .bind(first)
            .bind(second)
            .bind(third)
            .execute(pool)
            .await
            .unwrap();
    });
    let deleted = create_draft(app.clone(), &owner_cookie, "Deleted").await;
    let foreign_post = create_draft(app.clone(), &foreign_cookie, "Foreign").await;
    let (status, body) = post_form(
        app.clone(),
        <web::posts::Delete as ServerFn>::PATH,
        format!("post_id={deleted}"),
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");

    let mut cursor = None;
    let mut actual = Vec::new();
    let mut terminal = None;
    loop {
        let (status, body) = list_managed_after(app.clone(), Some(&owner_cookie), 2, cursor).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let page: ManagePostsPage = serde_json::from_str(&body).unwrap();
        assert_eq!(page.has_more, page.next_cursor.is_some());
        assert!(page.posts.len() <= 2);
        actual.extend(page.posts.iter().map(|post| post.post_id));
        assert!(actual.len() <= 3, "returned cursor must not repeat IDs");
        if let Some(last) = page.posts.last() {
            terminal = Some(web::posts::ManagePostsCursor {
                updated_at: last.updated_at,
                post_id: last.post_id,
            });
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        cursor = Some(next);
    }
    assert_eq!(actual, [third, second, first]);
    let (status, body) = list_managed_after(app.clone(), Some(&owner_cookie), 2, terminal).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let empty: ManagePostsPage = serde_json::from_str(&body).unwrap();
    assert!(empty.posts.is_empty());
    assert!(!empty.has_more);
    assert!(empty.next_cursor.is_none());

    let (status, body) = resolve(
        app.clone(),
        &owner_cookie,
        ManageSelectionIntent::Explicit {
            post_ids: vec![first, second, third],
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let explicit: ManagementSelectionSnapshot = serde_json::from_str(&body).unwrap();
    assert_eq!(explicit.selected_count, 3, "explicit IDs span list pages");

    let (status, body) = resolve(
        app.clone(),
        &owner_cookie,
        ManageSelectionIntent::AllMatching {
            state: ManagePublicationState::All,
            audience: ManageAudienceFilter::All,
            search: String::new(),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let snapshot: ManagementSelectionSnapshot = serde_json::from_str(&body).unwrap();
    assert_eq!(snapshot.selected_count, 3);
    assert_eq!(
        snapshot
            .targets
            .iter()
            .map(|target| target.post_id)
            .collect::<Vec<_>>(),
        vec![first, second, third]
    );
    assert!(
        !snapshot
            .targets
            .iter()
            .any(|target| { target.post_id == deleted || target.post_id == foreign_post })
    );

    let (status, body) = resolve(
        app,
        &owner_cookie,
        ManageSelectionIntent::Explicit {
            post_ids: vec![first, foreign_post],
        },
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Selection changed"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn bulk_management_http_preserves_noops_conflicts_and_atomic_delete(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = owner.cookie();
    let first = create_draft(app.clone(), &cookie, "First").await;
    let second = create_draft(app.clone(), &cookie, "Second").await;
    let (_, body) = resolve(
        app.clone(),
        &cookie,
        ManageSelectionIntent::Explicit {
            post_ids: vec![first, second],
        },
    )
    .await;
    let initial: ManagementSelectionSnapshot = serde_json::from_str(&body).unwrap();

    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        initial.clone(),
        BulkManageOperation::ChangeAudience {
            audience: AudienceSelection {
                public: false,
                subscribers: true,
                named: Vec::new(),
            },
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let outcome: common::MutationOutcome<BulkManageResult> = serde_json::from_str(&body).unwrap();
    assert!(matches!(outcome, common::MutationOutcome::Confirmed(_)));
    let result = outcome.value();
    assert_eq!(result.selected_count, 2);
    assert_eq!(result.changed_count, 2);

    let (_, body) = resolve(
        app.clone(),
        &cookie,
        ManageSelectionIntent::Explicit {
            post_ids: vec![first, second],
        },
    )
    .await;
    let current: ManagementSelectionSnapshot = serde_json::from_str(&body).unwrap();
    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        current.clone(),
        BulkManageOperation::ChangeAudience {
            audience: AudienceSelection {
                public: false,
                subscribers: true,
                named: Vec::new(),
            },
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let outcome: common::MutationOutcome<BulkManageResult> = serde_json::from_str(&body).unwrap();
    assert!(matches!(outcome, common::MutationOutcome::Confirmed(_)));
    assert_eq!(outcome.value().changed_count, 0);

    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        initial,
        BulkManageOperation::Delete {
            confirmed_count: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Selection changed"), "body: {body}");
    let (_, body) = resolve(
        app.clone(),
        &cookie,
        ManageSelectionIntent::Explicit {
            post_ids: vec![first, second],
        },
    )
    .await;
    let still_active: ManagementSelectionSnapshot = serde_json::from_str(&body).unwrap();
    assert_eq!(still_active.selected_count, 2, "conflict deletes nothing");

    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        current,
        BulkManageOperation::Delete {
            confirmed_count: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let outcome: common::MutationOutcome<BulkManageResult> = serde_json::from_str(&body).unwrap();
    assert!(matches!(outcome, common::MutationOutcome::Confirmed(_)));
    assert_eq!(outcome.value().changed_count, 2);
    let (_, body) = list_managed(app, Some(&cookie), 10).await;
    let page: ManagePostsPage = serde_json::from_str(&body).unwrap();
    assert!(page.posts.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn bulk_delete_requires_exact_server_side_count_for_ten_or_more(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = owner.cookie();
    let mut post_ids = Vec::new();
    for index in 0..10 {
        post_ids.push(create_draft(app.clone(), &cookie, &format!("Count {index}")).await);
    }
    let (_, body) = resolve(
        app.clone(),
        &cookie,
        ManageSelectionIntent::Explicit { post_ids },
    )
    .await;
    let snapshot: ManagementSelectionSnapshot = serde_json::from_str(&body).unwrap();

    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        snapshot.clone(),
        BulkManageOperation::Delete {
            confirmed_count: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("exact selected Post count"), "body: {body}");
    let (_, body) = list_managed(app.clone(), Some(&cookie), 20).await;
    let page: ManagePostsPage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page.posts.len(),
        10,
        "rejected confirmation deletes nothing"
    );

    let (status, body) = execute(
        app,
        Some(&cookie),
        snapshot,
        BulkManageOperation::Delete {
            confirmed_count: Some(10),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn manage_posts_http_requires_authentication(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = list_managed(app.clone(), None, 10).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");

    let (status, body) = execute(
        app,
        None,
        ManagementSelectionSnapshot {
            targets: Vec::new(),
            selected_count: 0,
        },
        BulkManageOperation::Delete {
            confirmed_count: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}
