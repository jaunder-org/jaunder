use axum::{Router, http::StatusCode};
use common::ids::PostId;
use common::render::PostFormat;
use common::test_support::parse_post_body;
use common::visibility::{AudienceBase, AudienceSelection};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::test_support::{Backend, backends};
use web::posts::{
    BulkManageOperation, BulkManageResult, BulkSelectionSnapshot, ManageAudienceFilter,
    ManagePostsPage, ManagePublicationState, ManageSelectionIntent, PostInputs,
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
                base: AudienceBase::Public,
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
    post_json(
        app,
        <web::posts::ListManagedPosts as ServerFn>::PATH,
        serde_json::to_value(web::posts::ListManagedPosts {
            state: ManagePublicationState::All,
            audience: ManageAudienceFilter::All,
            search: String::new(),
            cursor: None,
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
    snapshot: BulkSelectionSnapshot,
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

    let (status, body) = list_managed(app.clone(), Some(&owner_cookie), 1).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: ManagePostsPage = serde_json::from_str(&body).unwrap();
    assert_eq!(page.posts.len(), 1);
    assert!(page.has_more);
    assert!(page.posts[0].post_id == first || page.posts[0].post_id == second);

    let (status, body) = resolve(
        app.clone(),
        &owner_cookie,
        ManageSelectionIntent::Explicit {
            post_ids: vec![first, second],
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let explicit: BulkSelectionSnapshot = serde_json::from_str(&body).unwrap();
    assert_eq!(explicit.selected_count, 2, "explicit IDs span list pages");

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
    let snapshot: BulkSelectionSnapshot = serde_json::from_str(&body).unwrap();
    assert_eq!(snapshot.selected_count, 2);
    assert_eq!(
        snapshot
            .targets
            .iter()
            .map(|target| target.post_id)
            .collect::<Vec<_>>(),
        vec![first, second]
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
    let initial: BulkSelectionSnapshot = serde_json::from_str(&body).unwrap();

    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        initial.clone(),
        BulkManageOperation::ChangeAudience {
            audience: AudienceSelection {
                base: AudienceBase::Subscribers,
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
    let current: BulkSelectionSnapshot = serde_json::from_str(&body).unwrap();
    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        current.clone(),
        BulkManageOperation::ChangeAudience {
            audience: AudienceSelection {
                base: AudienceBase::Subscribers,
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
        BulkManageOperation::Delete,
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
    let still_active: BulkSelectionSnapshot = serde_json::from_str(&body).unwrap();
    assert_eq!(still_active.selected_count, 2, "conflict deletes nothing");

    let (status, body) = execute(
        app.clone(),
        Some(&cookie),
        current,
        BulkManageOperation::Delete,
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
async fn manage_posts_http_requires_authentication(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = list_managed(app.clone(), None, 10).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");

    let (status, body) = execute(
        app,
        None,
        BulkSelectionSnapshot {
            targets: Vec::new(),
            selected_count: 0,
        },
        BulkManageOperation::Delete,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}
