use std::sync::Arc;

use chrono::Utc;
use common::ids::{AudienceId, PostId, UserId};
use common::post_title::PostTitle;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::test_support::{
    parse_audience_name, parse_page_size, parse_post_body, parse_post_summary, parse_post_title,
    parse_row_limit, parse_slug,
};
use common::time::UtcInstant;
use common::visibility::{AudienceTarget, ViewerIdentity};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{
    Backend, SeedRawPost, SeedUser, TestEnv, UpdateRawPost, backends, confirmed, confirmed_for,
    media_url_for,
};
use storage::{
    CreatePostError, PostBookkeepingExpectation, PostFormat, PostLifecycle, PostUpdate,
    PublishUpdate, RenderedPostContent, UpdatePostError, create_rendered_post, perform_post_update,
};

use super::fixtures::{anon_by_tag, open_pool};

async fn create_audience_confirmed(
    state: &storage::AppState,
    author: UserId,
    name: common::audience::AudienceName,
) -> AudienceId {
    let audiences = Arc::clone(&state.audiences);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { audiences.create_audience(transaction, author, &name).await })
        })
        .await
        .expect("audience fixture setup should succeed");
    confirmed_for(outcome, "audience fixture setup")
}

macro_rules! update_post {
    ($state:expr, $post_id:expr, $user_id:expr, $update:expr) => {{
        let posts = Arc::clone(&$state.posts);
        let update = $update;
        $state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    posts
                        .update_post(transaction, $post_id, $user_id, &update)
                        .await
                })
            })
            .await
    }};
}

macro_rules! soft_delete_post {
    ($state:expr, $post_id:expr, $user_id:expr) => {{
        let posts = Arc::clone(&$state.posts);
        $state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    posts
                        .soft_delete_post(transaction, $post_id, $user_id)
                        .await
                })
            })
            .await
    }};
}

macro_rules! create_posts {
    ($state:expr, $inputs:expr) => {{
        let posts = Arc::clone(&$state.posts);
        let inputs = ($inputs).clone();
        $state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { posts.create_posts(transaction, &inputs).await })
            })
            .await
    }};
}

// Post tests (backend-parametrized)

#[apply(backends)]
#[tokio::test]
async fn post_create_and_get_by_id_works(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let post = SeedRawPost::new(user_id).draft().seed(state).await;

    let record = state
        .posts
        .get_post_by_id(post.post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.post_id, post.post_id);
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.title, Some(post.title));
    assert_eq!(record.slug, post.slug);
    assert_eq!(record.format, PostFormat::Markdown);
    assert!(record.published_at.is_none());
    assert!(record.deleted_at.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn post_slug_conflict_returns_slug_conflict(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    // Two published posts with the same slug on the same date conflict on the
    // unique index (user_id, date(COALESCE(published_at, created_at)), slug).
    let first = SeedRawPost::new(user_id).seed(state).await;

    let err = SeedRawPost::new(user_id)
        .slug(first.slug.as_ref())
        .published_at(first.published_at.unwrap())
        .create(state)
        .await
        .unwrap_err();
    assert!(
        matches!(err, CreatePostError::SlugConflict),
        "expected SlugConflict, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn post_update_writes_revision_and_updates_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user_id).draft().seed(state).await.post_id;

    let update_input = UpdateRawPost::new("update-test")
        .format(PostFormat::Org)
        .unpublish()
        .build();
    let record = confirmed(update_post!(state, post_id, user_id, update_input).unwrap());

    assert_eq!(record.title.as_deref(), Some("Updated Title"));
    assert_eq!(record.format, PostFormat::Org);
    assert_eq!(record.body, "updated body");
}

#[apply(backends)]
#[tokio::test]
async fn post_update_not_found_returns_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let update_input = UpdateRawPost::new("nope").unpublish().build();
    let err = update_post!(state, PostId::from(9999), UserId::from(1), update_input).unwrap_err();
    assert!(
        matches!(
            err,
            storage::WriteScopeError::Operation(UpdatePostError::NotFound)
        ),
        "expected NotFound, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn post_update_by_non_owner_returns_unauthorized(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = SeedUser::new().seed(state).await.user_id;
    let other = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(owner).draft().seed(state).await.post_id;

    let err = update_post!(
        state,
        post_id,
        other,
        UpdateRawPost::new("hijacked")
            .title("Hijacked")
            .body(parse_post_body("Nope"))
            .unpublish()
            .build()
    )
    .expect_err("non-owner update must fail");

    assert!(matches!(
        err,
        storage::WriteScopeError::Operation(UpdatePostError::Unauthorized)
    ));
}

/// Builds a `PostUpdate` with the given publish verb and otherwise-valid,
/// stable fields. `slug` is pinned via `slug_override` so repeated updates on
/// different posts never collide on a derived slug.
fn update_input<'a>(
    post_id: PostId,
    editor_user_id: UserId,
    title: &'a PostTitle,
    slug: &'a Slug,
    publish: PublishUpdate,
) -> PostUpdate<'a> {
    PostUpdate {
        post_id,
        editor_user_id,
        body: parse_post_body("updated body"),
        title: Some(title),
        format: PostFormat::Markdown,
        slug_override: Some(slug),
        publish,
        summary: None,
        audiences: vec![AudienceTarget::Public],
        tags: vec![],
        previous_tag_slugs: vec![],
        request_clock: common::time::UtcInstant::now(),
        expectations: PostBookkeepingExpectation::default(),
    }
}

// Issue #70/#747: the storage update's publication verb is an explicit
// `PublishUpdate`, not a bool. One common test across both backends locks the
// five publish-timestamp cases.
#[apply(backends)]
#[tokio::test]
async fn update_publish_timestamp_semantics(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let alice = SeedUser::new().seed(state).await.user_id;

    // A fresh draft (published_at NULL).
    let draft = SeedRawPost::new(alice).draft().seed(state).await.post_id;

    // Pinned override slugs (already valid, as they arrive at the storage layer).
    let p: Slug = "p".parse().unwrap();
    let q: Slug = "q".parse().unwrap();
    let title = parse_post_title("Updated Title");

    // Publish { at: Some(future) } on a draft => scheduled at that instant.
    let future = UtcInstant::from(now + Duration::days(1));
    let rec = confirmed(
        perform_post_update(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            update_input(
                draft,
                alice,
                &title,
                &p,
                PublishUpdate::Publish { at: Some(future) },
            ),
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        rec.published_at,
        Some(future),
        "explicit future timestamp is stored"
    );

    // Publish { at: Some(past) } stores the exact backdated instant.
    let past = UtcInstant::from(now - Duration::days(1));
    let backdated = confirmed(
        perform_post_update(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            update_input(
                draft,
                alice,
                &title,
                &p,
                PublishUpdate::Publish { at: Some(past) },
            ),
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        backdated.published_at,
        Some(past),
        "explicit past timestamp is stored"
    );

    // Publish { at: None } on an already-published post keeps the existing timestamp.
    let rec2 = confirmed(
        perform_post_update(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            update_input(
                draft,
                alice,
                &title,
                &p,
                PublishUpdate::Publish { at: None },
            ),
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        rec2.published_at,
        Some(past),
        "publish-without-timestamp keeps existing"
    );

    // Unpublish clears it.
    let rec3 = confirmed(
        perform_post_update(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            update_input(draft, alice, &title, &p, PublishUpdate::Unpublish),
        )
        .await
        .unwrap(),
    );
    assert_eq!(rec3.published_at, None, "unpublish clears published_at");

    // Publish { at: None } on a never-published draft stamps ~now.
    let draft2 = SeedRawPost::new(alice).draft().seed(state).await.post_id;
    let rec4 = confirmed(
        perform_post_update(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            update_input(
                draft2,
                alice,
                &title,
                &q,
                PublishUpdate::Publish { at: None },
            ),
        )
        .await
        .unwrap(),
    );
    assert!(
        rec4.published_at.is_some(),
        "publish-now stamps a timestamp"
    );
}

// Raw read of a post's `post_audiences` rows as `(target_kind name, audience_id)`,
// ordered by kind name. Used by the audience-targeting persistence test.
async fn post_audience_rows(
    backend: Backend,
    env: &TestEnv,
    post_id: PostId,
) -> Vec<(String, Option<AudienceId>)> {
    let sql = "SELECT tk.name, pa.audience_id \
               FROM post_audiences pa \
               JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id \
               WHERE pa.post_id = $1 \
               ORDER BY tk.name, pa.audience_id";
    match backend {
        Backend::Sqlite => sqlx::query_as(&sql.replace("$1", "?"))
            .bind(post_id)
            .fetch_all(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_as(sql)
                .bind(post_id)
                .fetch_all(pool)
                .await
                .unwrap()
        }
    }
}

// Create persists `post_audiences` rows matching the input vec; update replaces
// them (delete-all-then-insert). `Private`/empty → no rows. See ADR-0020.
#[apply(backends)]
#[tokio::test]
async fn post_audiences_are_persisted_and_replaced(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = SeedUser::new().seed(state).await.user_id;
    let aud = create_audience_confirmed(state, author, parse_audience_name("Friends")).await;

    // Create targeting [Public, Named(aud)] → two rows.
    let post_id = SeedRawPost::new(author)
        .draft()
        .audiences(vec![AudienceTarget::Public, AudienceTarget::Named(aud)])
        .seed(state)
        .await
        .post_id;
    let rows = post_audience_rows(backend, &env, post_id).await;
    assert_eq!(
        rows,
        vec![
            ("named".to_string(), Some(aud)),
            ("public".to_string(), None),
        ],
        "create should persist one public and one named row"
    );

    // Every update below is the same edit with different targeting, so they share a base
    // and vary only `audiences`.
    let edit = UpdateRawPost::new("audience-post")
        .title("Post audience-post")
        .body(parse_post_body("body text"))
        .unpublish();

    // Update to [Private] → zero rows.
    confirmed(
        update_post!(
            state,
            post_id,
            author,
            edit.clone()
                .audiences(vec![AudienceTarget::Private])
                .build()
        )
        .unwrap(),
    );
    assert!(
        post_audience_rows(backend, &env, post_id).await.is_empty(),
        "[Private] should leave no rows"
    );

    // Update to [] (empty) → also zero rows (equivalent to private).
    confirmed(
        update_post!(
            state,
            post_id,
            author,
            edit.clone().audiences(vec![]).build()
        )
        .unwrap(),
    );
    assert!(
        post_audience_rows(backend, &env, post_id).await.is_empty(),
        "an empty audience vec should leave no rows"
    );

    // Update to [Subscribers] → one subscribers row.
    confirmed(
        update_post!(
            state,
            post_id,
            author,
            edit.audiences(vec![AudienceTarget::Subscribers]).build()
        )
        .unwrap(),
    );
    assert_eq!(
        post_audience_rows(backend, &env, post_id).await,
        vec![("subscribers".to_string(), None)],
        "update to [Subscribers] should leave exactly one subscribers row"
    );
}

// `get_post_audiences` reads a post's targeting back as a `Vec<AudienceTarget>`
// (owner-only, no viewer). Round-trips create → read for each shape.
#[apply(backends)]
#[tokio::test]
async fn get_post_audiences_round_trips(#[case] backend: Backend) {
    use std::collections::HashSet;

    let env = backend.setup().await;
    let state = &env.state;
    let author = SeedUser::new().seed(state).await.user_id;
    let aud = create_audience_confirmed(state, author, parse_audience_name("Friends")).await;

    // Public + Named(aud) → union read back (order-independent compare).
    let post_id = SeedRawPost::new(author)
        .draft()
        .audiences(vec![AudienceTarget::Public, AudienceTarget::Named(aud)])
        .seed(state)
        .await
        .post_id;
    let read: HashSet<_> = state
        .posts
        .get_post_audiences(post_id)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(
        read,
        HashSet::from([AudienceTarget::Public, AudienceTarget::Named(aud)]),
        "should read back the Public + Named union"
    );

    // One edit, two targetings.
    let edit = UpdateRawPost::new("round-trip")
        .title("Post round-trip")
        .body(parse_post_body("body text"))
        .unpublish();

    // Subscribers-only.
    confirmed(
        update_post!(
            state,
            post_id,
            author,
            edit.clone()
                .audiences(vec![AudienceTarget::Subscribers])
                .build()
        )
        .unwrap(),
    );
    assert_eq!(
        state.posts.get_post_audiences(post_id).await.unwrap(),
        vec![AudienceTarget::Subscribers],
        "should read back Subscribers"
    );

    // Private / empty → no rows → empty vec.
    confirmed(
        update_post!(
            state,
            post_id,
            author,
            edit.audiences(vec![AudienceTarget::Private]).build()
        )
        .unwrap(),
    );
    assert!(
        state
            .posts
            .get_post_audiences(post_id)
            .await
            .unwrap()
            .is_empty(),
        "Private should read back as an empty vec"
    );
}

#[apply(backends)]
#[tokio::test]
async fn post_update_invalid_slug(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user).draft().seed(state).await.post_id;

    let _post_id2 = SeedRawPost::new(user)
        .draft()
        .slug("second-slug")
        .seed(state)
        .await
        .post_id;

    let update_result = update_post!(
        state,
        post_id,
        user,
        UpdateRawPost::new("second-slug")
            .title("Updated")
            .body(parse_post_body("Updated content"))
            .unpublish()
            .build()
    );

    match update_result {
        Err(storage::WriteScopeError::Operation(UpdatePostError::Internal(_))) => {
            // Expected: unique constraint violation on slug
        }
        other => panic!("Expected Internal error, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn soft_delete_then_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user).seed(state).await.post_id;

    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post_id,
        user,
        &["delete-tag".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    confirmed(soft_delete_post!(state, post_id, user).expect("soft_delete_post failed"));

    // Try to get by ID (should still exist internally)
    let post = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id failed");
    assert!(post.is_none() || post.unwrap().deleted_at.is_some());

    let tag: Tag = "delete-tag".parse().unwrap();
    let posts = anon_by_tag(state, &tag, "10").await;
    assert!(posts.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn update_soft_deleted_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user).draft().seed(state).await.post_id;

    confirmed(soft_delete_post!(state, post_id, user).expect("soft_delete_post failed"));

    // The update's outcome on a soft-deleted post is not part of this contract,
    // so its result is deliberately unasserted.
    let _result = update_post!(
        state,
        post_id,
        user,
        UpdateRawPost::new("updated-slug")
            .title("Updated")
            .body(parse_post_body("New content"))
            .build()
    );

    // What is pinned: no update path resurrects a soft-deleted post.
    let post = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id failed");
    assert!(post.is_none() || post.unwrap().deleted_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn get_post_by_id_nonexistent(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let result = state
        .posts
        .get_post_by_id(PostId::from(999_999), &ViewerIdentity::Anonymous)
        .await;
    match result {
        Ok(None) => {}
        other => panic!("Expected Ok(None), got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn post_revisions_created(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user).draft().seed(state).await.post_id;

    let result = confirmed(
        update_post!(
            state,
            post_id,
            user,
            UpdateRawPost::new("revision-test")
                .title("Updated")
                .body(parse_post_body("Updated content"))
                .build()
        )
        .expect("update_post failed"),
    );

    assert_eq!(result.title.as_deref(), Some("Updated"));
    assert_eq!(result.body, "Updated content");
    assert!(result.published_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn owner_revision_history_is_keyset_ordered_and_scoped(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = SeedUser::new().seed(state).await.user_id;
    let stranger = SeedUser::new().seed(state).await.user_id;
    let first = SeedRawPost::new(owner).draft().seed(state).await.post_id;
    let second = SeedRawPost::new(owner).draft().seed(state).await.post_id;
    let foreign = SeedRawPost::new(stranger).draft().seed(state).await.post_id;

    for (post_id, user_id, slug) in [
        (first, owner, "history-first"),
        (second, owner, "history-second"),
        (foreign, stranger, "history-foreign"),
    ] {
        confirmed(
            update_post!(
                state,
                post_id,
                user_id,
                UpdateRawPost::new(slug).unpublish().build()
            )
            .unwrap(),
        );
    }
    confirmed(
        update_post!(
            state,
            first,
            owner,
            UpdateRawPost::new("history-first-again")
                .unpublish()
                .build()
        )
        .unwrap(),
    );

    let first_page = state
        .posts
        .list_owned_revision_history(owner, None, parse_page_size("2"))
        .await
        .unwrap();
    assert_eq!(first_page.revisions.len(), 2);
    assert!(
        first_page
            .revisions
            .iter()
            .all(|revision| revision.post_id != foreign)
    );
    assert!(first_page.revisions[0].revision_id > first_page.revisions[1].revision_id);

    let second_page = state
        .posts
        .list_owned_revision_history(owner, first_page.next_cursor, parse_page_size("2"))
        .await
        .unwrap();
    let ids = first_page
        .revisions
        .iter()
        .chain(&second_page.revisions)
        .map(|revision| revision.revision_id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 3);
    assert!(ids.windows(2).all(|ids| ids[0] > ids[1]));
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        ids.len()
    );

    let post_page = state
        .posts
        .list_post_revision_history(owner, first, None, parse_page_size("10"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(post_page.revisions.len(), 2);
    assert!(
        post_page
            .revisions
            .iter()
            .all(|revision| revision.post_id == first)
    );
}

#[apply(backends)]
#[tokio::test]
async fn revision_history_keeps_deleted_owner_post_and_hides_foreign_details(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = SeedUser::new().seed(state).await.user_id;
    let stranger = SeedUser::new().seed(state).await.user_id;
    let post_id = SeedRawPost::new(owner).draft().seed(state).await.post_id;

    confirmed(
        update_post!(
            state,
            post_id,
            owner,
            UpdateRawPost::new("captured-state").unpublish().build()
        )
        .unwrap(),
    );
    confirmed(soft_delete_post!(state, post_id, owner).unwrap());

    let page = state
        .posts
        .list_post_revision_history(owner, post_id, None, parse_page_size("10"))
        .await
        .unwrap()
        .unwrap();
    let revision = page.revisions.first().unwrap();
    assert!(revision.current_deleted);
    assert_eq!(revision.snapshot_lifecycle, PostLifecycle::Draft);

    let detail = state
        .posts
        .get_post_revision_detail(owner, post_id, revision.revision_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.revision.post_id, post_id);
    assert_eq!(detail.revision.user_id, owner);
    assert!(detail.revision.tags.is_empty());
    assert_eq!(detail.revision.audiences, vec![AudienceTarget::Public]);
    assert!(detail.revision.media.is_empty());

    assert!(
        state
            .posts
            .list_post_revision_history(stranger, post_id, None, parse_page_size("10"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .posts
            .get_post_revision_detail(stranger, post_id, revision.revision_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .posts
            .get_post_revision_detail(owner, PostId::from(999_999), revision.revision_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn revision_detail_round_trips_complete_snapshot_and_rejects_invalid_media_form(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = SeedUser::new().seed(state).await.user_id;
    let named =
        create_audience_confirmed(state, owner, parse_audience_name("Revision readers")).await;
    let media_url = media_url_for("revision-detail.png");
    let post_id = SeedRawPost::new(owner)
        .slug("complete-revision")
        .body(parse_post_body(&format!(
            "before <img src=\"{media_url}\">"
        )))
        .summary(parse_post_summary("complete snapshot summary"))
        .audiences(vec![AudienceTarget::Public, AudienceTarget::Named(named)])
        .tags(["Rust", "Storage"])
        .seed(state)
        .await
        .post_id;
    let prior = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Local { user_id: owner })
        .await
        .unwrap()
        .unwrap();

    confirmed(
        update_post!(
            state,
            post_id,
            owner,
            UpdateRawPost::new("complete-revision-updated")
                .body(parse_post_body("after"))
                .unpublish()
                .build()
        )
        .unwrap(),
    );
    let revision = state
        .posts
        .list_post_revision_history(owner, post_id, None, parse_page_size("10"))
        .await
        .unwrap()
        .unwrap()
        .revisions
        .pop()
        .unwrap();
    let detail = state
        .posts
        .get_post_revision_detail(owner, post_id, revision.revision_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.revision.post_id, prior.post_id);
    assert_eq!(detail.revision.user_id, prior.user_id);
    assert_eq!(detail.revision.title, prior.title);
    assert_eq!(detail.revision.slug, prior.slug);
    assert_eq!(detail.revision.body, prior.body);
    assert_eq!(detail.revision.format, prior.format);
    assert_eq!(detail.revision.rendered_html, prior.rendered_html);
    assert_eq!(detail.revision.summary, prior.summary);
    assert_eq!(detail.revision.created_at, prior.created_at);
    assert_eq!(detail.revision.updated_at, prior.updated_at);
    assert_eq!(detail.revision.published_at, prior.published_at);
    assert_eq!(detail.revision.deleted_at, prior.deleted_at);
    assert_eq!(
        detail
            .revision
            .tags
            .iter()
            .map(|tag| (tag.tag.as_ref(), tag.display.as_ref()))
            .collect::<Vec<_>>(),
        vec![("rust", "Rust"), ("storage", "Storage")]
    );
    assert_eq!(
        detail.revision.audiences,
        vec![AudienceTarget::Named(named), AudienceTarget::Public]
    );
    assert_eq!(
        detail.revision.media,
        vec![common::media::parse_media_url(&media_url).unwrap()]
    );

    env.base
        .pool()
        .execute(&format!(
            "UPDATE post_media SET reference_form = 'invalid media form'
             WHERE post_id = {post_id} AND subject_kind = 'revision'
               AND revision_id = {}",
            revision.revision_id
        ))
        .await
        .unwrap();
    let error = state
        .posts
        .get_post_revision_detail(owner, post_id, revision.revision_id)
        .await
        .expect_err("invalid persisted revision media form must fail decoding");
    assert!(
        matches!(
            &error,
            sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_)
        ),
        "expected typed persisted-media decode failure, got {error:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn current_revision_summary_derives_lifecycle_from_request_clock(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = SeedUser::new().seed(state).await.user_id;
    let now = UtcInstant::now();
    let scheduled_at = UtcInstant::from(now.value() + chrono::Duration::hours(1));
    let post_id = SeedRawPost::new(owner)
        .published_at(scheduled_at)
        .seed(state)
        .await
        .post_id;

    let before = state
        .posts
        .get_current_revision_summary(owner, post_id, now)
        .await
        .unwrap()
        .unwrap();
    let after = state
        .posts
        .get_current_revision_summary(
            owner,
            post_id,
            UtcInstant::from(scheduled_at.value() + chrono::Duration::seconds(1)),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.lifecycle, PostLifecycle::Scheduled);
    assert_eq!(after.lifecycle, PostLifecycle::Published);
}
#[apply(backends)]
#[tokio::test]
async fn current_revision_summary_reports_draft_and_deleted_states(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = SeedUser::new().seed(state).await.user_id;
    let draft_id = SeedRawPost::new(owner).draft().seed(state).await.post_id;
    let deleted_id = SeedRawPost::new(owner).draft().seed(state).await.post_id;
    confirmed(soft_delete_post!(state, deleted_id, owner).unwrap());

    let draft = state
        .posts
        .get_current_revision_summary(owner, draft_id, UtcInstant::now())
        .await
        .unwrap()
        .unwrap();
    let deleted = state
        .posts
        .get_current_revision_summary(owner, deleted_id, UtcInstant::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.lifecycle, PostLifecycle::Draft);
    assert_eq!(deleted.lifecycle, PostLifecycle::Deleted);
    assert_eq!(deleted.post_id, deleted_id);
    assert!(deleted.deleted_at.is_some());
}

// =============================================================================
// create_rendered_post / perform_post_update integration tests
// =============================================================================

#[apply(backends)]
#[tokio::test]
async fn create_rendered_post_markdown_renders_and_stores(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let post_id = confirmed(
        create_rendered_post(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            RenderedPostContent {
                user_id,
                title: Some(parse_post_title("Rendered Markdown")),
                slug: "rendered-markdown".parse().unwrap(),
                body: parse_post_body("**bold**"),
                format: PostFormat::Markdown,
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: vec![],
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap(),
    )
    .post_id;

    let record = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.title.as_deref(), Some("Rendered Markdown"));
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<strong>bold</strong>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_rendered_post_org_renders_and_stores(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let post_id = confirmed(
        create_rendered_post(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            RenderedPostContent {
                user_id,
                title: Some(parse_post_title("Rendered Org")),
                slug: "rendered-org".parse().unwrap(),
                body: parse_post_body("*bold*"),
                format: PostFormat::Org,
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: vec![],
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap(),
    )
    .post_id;

    let record = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.title.as_deref(), Some("Rendered Org"));
    assert!(
        record.rendered_html.as_ref().contains("<b>bold</b>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_rendered_post_slug_conflict_returns_storage_error(#[case] backend: Backend) {
    use storage::CreatePostError;

    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let now = UtcInstant::now();

    let occ = SeedRawPost::new(user_id)
        .published_at(now)
        .seed(state)
        .await;

    // Second create with same slug+date conflicts
    let err = create_rendered_post(
        &state.write_scope,
        Arc::clone(&state.posts),
        Arc::clone(&state.feed_events),
        RenderedPostContent {
            user_id,
            title: Some(parse_post_title("Second Post")),
            slug: occ.slug.clone(),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: vec![],
            idempotency_key: None,
            expectations: PostBookkeepingExpectation::default(),
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, CreatePostError::SlugConflict),
        "expected Storage error, got {err:?}"
    );
    assert!(
        err.to_string().contains("slug"),
        "expected slug conflict message, got: {err}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_post_foreign_key_violation_maps_to_internal(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    // A post referencing a non-existent user violates the `posts.user_id` foreign
    // key on both backends (SQLite enforces FKs here — sqlx's SqliteConnectOptions
    // defaults `foreign_keys` to ON), a *non-unique* DB error. So `create_post`
    // (via the shared `write_post_in_tx`) maps it to `Internal`, not `SlugConflict`
    // — exercising the generic-error arm.
    let err = SeedRawPost::new(UserId::from(999_999))
        .create(state)
        .await
        .unwrap_err();
    assert!(
        matches!(err, CreatePostError::Internal(_)),
        "expected Internal for FK violation, got {err:?}"
    );
}

// =============================================================================
// create_posts (single-transaction batch insert) — issue #9
// =============================================================================

#[apply(backends)]
#[tokio::test]
async fn create_posts_empty_slice_is_noop(#[case] backend: Backend) {
    let env = backend.setup().await;
    let ids = confirmed(create_posts!(&env.state, Vec::new()).unwrap());
    assert!(ids.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn create_posts_batches_all_rows_in_order(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let inputs: Vec<_> = (0..3).map(|_| SeedRawPost::new(user_id).build()).collect();

    let ids = confirmed(create_posts!(state, inputs).unwrap());
    assert_eq!(ids.len(), 3);

    // Each id resolves to the matching row, and its Public audience is honored
    // (visible to Anonymous — get_post_by_id filters on audience).
    for (i, id) in ids.iter().enumerate() {
        let rec = state
            .posts
            .get_post_by_id(*id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rec.title, inputs[i].title);
    }
}

#[apply(backends)]
#[tokio::test]
async fn create_posts_conflict_rolls_back_whole_batch(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let dup = parse_slug("dup");
    // Rows 0 and 2 collide on slug — the batch must fail on row 2 and undo 0/1.
    let inputs = vec![
        SeedRawPost::new(user_id).slug(dup.as_ref()).build(),
        SeedRawPost::new(user_id).build(),
        SeedRawPost::new(user_id).slug(dup.as_ref()).build(),
    ];

    let err = create_posts!(state, inputs).unwrap_err();
    assert!(
        matches!(
            err,
            storage::WriteScopeError::Operation(CreatePostError::SlugConflict)
        ),
        "expected SlugConflict, got {err:?}"
    );

    // Nothing persisted: the author's collection (drafts + published) is empty.
    let collection = state
        .posts
        .list_collection_by_user(user_id, None, parse_row_limit("50"))
        .await
        .unwrap();
    assert!(
        collection.is_empty(),
        "expected full rollback, found {} rows",
        collection.len()
    );
}

#[apply(backends)]
#[tokio::test]
async fn perform_post_update_markdown_renders_and_updates(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let post = SeedRawPost::new(user_id).draft().seed(state).await;
    let title = parse_post_title("Updated Title");

    let record = confirmed(
        perform_post_update(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            PostUpdate {
                post_id: post.post_id,
                editor_user_id: user_id,
                title: Some(&title),
                slug_override: Some(&post.slug),
                body: parse_post_body("**updated**"),
                format: PostFormat::Markdown,
                publish: PublishUpdate::Unpublish,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: vec![],
                previous_tag_slugs: vec![],
                request_clock: common::time::UtcInstant::now(),
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap(),
    );

    assert_eq!(record.title.as_deref(), Some("Updated Title"));
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<strong>updated</strong>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

#[apply(backends)]
#[tokio::test]
async fn perform_post_update_org_renders_and_updates(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let post = SeedRawPost::new(user_id).draft().seed(state).await;
    let title = parse_post_title("Updated Org Title");

    // `*bold org*` is emphasis, not a heading — `* ` (with the space) is what marks a
    // title source — so canonicalization leaves it alone and it must still render.
    let record = confirmed(
        perform_post_update(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
            PostUpdate {
                post_id: post.post_id,
                editor_user_id: user_id,
                title: Some(&title),
                slug_override: Some(&post.slug),
                body: parse_post_body("*bold org*"),
                format: PostFormat::Org,
                publish: PublishUpdate::Unpublish,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: vec![],
                previous_tag_slugs: vec![],
                request_clock: common::time::UtcInstant::now(),
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap(),
    );

    assert_eq!(record.title.as_deref(), Some("Updated Org Title"));
    assert!(
        record.rendered_html.as_ref().contains("<b>bold org</b>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}
