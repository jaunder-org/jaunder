use chrono::Utc;
use common::ids::{AudienceId, PostId, UserId};
use common::post_title::PostTitle;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::test_support::{
    parse_audience_name, parse_post_body, parse_post_title, parse_row_limit, parse_slug,
};
use common::visibility::{AudienceTarget, ViewerIdentity};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedRawPost, SeedUser, TestEnv, UpdateRawPost, backends};
use storage::{
    CreatePostError, PostFormat, PostUpdate, PublishUpdate, RenderedPostContent, UpdatePostError,
    create_rendered_post, perform_post_update,
};

use super::fixtures::{anon_by_tag, open_pool};

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
    let record = state
        .posts
        .update_post(post_id, user_id, &update_input)
        .await
        .unwrap();

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
    let err = state
        .posts
        .update_post(PostId::from(9999), UserId::from(1), &update_input)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdatePostError::NotFound),
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

    let err = state
        .posts
        .update_post(
            post_id,
            other,
            &UpdateRawPost::new("hijacked")
                .title("Hijacked")
                .body(parse_post_body("Nope"))
                .unpublish()
                .build(),
        )
        .await
        .expect_err("non-owner update must fail");

    assert!(matches!(err, UpdatePostError::Unauthorized));
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
    }
}

// Issue #70: the storage update's publication verb is an explicit
// `PublishUpdate`, not a bool. One common test, both backends, with an injected
// `now` pinning the boundary; locks the four publish-timestamp cases.
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
    let future = now + Duration::days(1);
    let rec = perform_post_update(
        &*state.posts,
        update_input(
            draft,
            alice,
            &title,
            &p,
            PublishUpdate::Publish { at: Some(future) },
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        rec.published_at,
        Some(future),
        "explicit future timestamp is stored"
    );

    // Publish { at: None } on an already-published post keeps the existing timestamp.
    let rec2 = perform_post_update(
        &*state.posts,
        update_input(
            draft,
            alice,
            &title,
            &p,
            PublishUpdate::Publish { at: None },
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        rec2.published_at,
        Some(future),
        "publish-without-timestamp keeps existing"
    );

    // Unpublish clears it.
    let rec3 = perform_post_update(
        &*state.posts,
        update_input(draft, alice, &title, &p, PublishUpdate::Unpublish),
    )
    .await
    .unwrap();
    assert_eq!(rec3.published_at, None, "unpublish clears published_at");

    // Publish { at: None } on a never-published draft stamps ~now.
    let draft2 = SeedRawPost::new(alice).draft().seed(state).await.post_id;
    let rec4 = perform_post_update(
        &*state.posts,
        update_input(
            draft2,
            alice,
            &title,
            &q,
            PublishUpdate::Publish { at: None },
        ),
    )
    .await
    .unwrap();
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
    let aud = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();

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
    state
        .posts
        .update_post(
            post_id,
            author,
            &edit
                .clone()
                .audiences(vec![AudienceTarget::Private])
                .build(),
        )
        .await
        .unwrap();
    assert!(
        post_audience_rows(backend, &env, post_id).await.is_empty(),
        "[Private] should leave no rows"
    );

    // Update to [] (empty) → also zero rows (equivalent to private).
    state
        .posts
        .update_post(post_id, author, &edit.clone().audiences(vec![]).build())
        .await
        .unwrap();
    assert!(
        post_audience_rows(backend, &env, post_id).await.is_empty(),
        "an empty audience vec should leave no rows"
    );

    // Update to [Subscribers] → one subscribers row.
    state
        .posts
        .update_post(
            post_id,
            author,
            &edit.audiences(vec![AudienceTarget::Subscribers]).build(),
        )
        .await
        .unwrap();
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
    let aud = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();

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
    state
        .posts
        .update_post(
            post_id,
            author,
            &edit
                .clone()
                .audiences(vec![AudienceTarget::Subscribers])
                .build(),
        )
        .await
        .unwrap();
    assert_eq!(
        state.posts.get_post_audiences(post_id).await.unwrap(),
        vec![AudienceTarget::Subscribers],
        "should read back Subscribers"
    );

    // Private / empty → no rows → empty vec.
    state
        .posts
        .update_post(
            post_id,
            author,
            &edit.audiences(vec![AudienceTarget::Private]).build(),
        )
        .await
        .unwrap();
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

    let update_result = state
        .posts
        .update_post(
            post_id,
            user,
            &UpdateRawPost::new("second-slug")
                .title("Updated")
                .body(parse_post_body("Updated content"))
                .unpublish()
                .build(),
        )
        .await;

    match update_result {
        Err(UpdatePostError::Internal(_)) => {
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

    state
        .posts
        .set_post_tags(post_id, &["delete-tag".parse::<TagLabel>().unwrap()])
        .await
        .expect("set_post_tags failed");

    state
        .posts
        .soft_delete_post(post_id)
        .await
        .expect("soft_delete_post failed");

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

    state
        .posts
        .soft_delete_post(post_id)
        .await
        .expect("soft_delete_post failed");

    // The update's outcome on a soft-deleted post is not part of this contract,
    // so its result is deliberately unasserted.
    let _result = state
        .posts
        .update_post(
            post_id,
            user,
            &UpdateRawPost::new("updated-slug")
                .title("Updated")
                .body(parse_post_body("New content"))
                .build(),
        )
        .await;

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

    let result = state
        .posts
        .update_post(
            post_id,
            user,
            &UpdateRawPost::new("revision-test")
                .title("Updated")
                .body(parse_post_body("Updated content"))
                .build(),
        )
        .await
        .expect("update_post failed");

    assert_eq!(result.title.as_deref(), Some("Updated"));
    assert_eq!(result.body, "Updated content");
    assert!(result.published_at.is_some());
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

    let post_id = create_rendered_post(
        state.posts.as_ref(),
        RenderedPostContent {
            user_id,
            title: Some(parse_post_title("Rendered Markdown")),
            slug: "rendered-markdown".parse().unwrap(),
            body: parse_post_body("**bold**"),
            format: PostFormat::Markdown,
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

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

    let post_id = create_rendered_post(
        state.posts.as_ref(),
        RenderedPostContent {
            user_id,
            title: Some(parse_post_title("Rendered Org")),
            slug: "rendered-org".parse().unwrap(),
            body: parse_post_body("*bold*"),
            format: PostFormat::Org,
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

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

    let now = Utc::now();

    let occ = SeedRawPost::new(user_id)
        .published_at(now)
        .seed(state)
        .await;

    // Second create with same slug+date conflicts
    let err = create_rendered_post(
        state.posts.as_ref(),
        RenderedPostContent {
            user_id,
            title: Some(parse_post_title("Second Post")),
            slug: occ.slug.clone(),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
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
    let ids = env.state.posts.create_posts(&[]).await.unwrap();
    assert!(ids.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn create_posts_batches_all_rows_in_order(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let inputs: Vec<_> = (0..3).map(|_| SeedRawPost::new(user_id).build()).collect();

    let ids = state.posts.create_posts(&inputs).await.unwrap();
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

    let err = state.posts.create_posts(&inputs).await.unwrap_err();
    assert!(
        matches!(err, CreatePostError::SlugConflict),
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

    let record = perform_post_update(
        state.posts.as_ref(),
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
        },
    )
    .await
    .unwrap();

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
    let record = perform_post_update(
        state.posts.as_ref(),
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
        },
    )
    .await
    .unwrap();

    assert_eq!(record.title.as_deref(), Some("Updated Org Title"));
    assert!(
        record.rendered_html.as_ref().contains("<b>bold org</b>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}
