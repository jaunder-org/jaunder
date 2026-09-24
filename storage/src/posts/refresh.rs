//! Bounded, resumable presentation-only refresh of current Post projections.
//!
//! A migration seeds the one versioned checkpoint; this host-side operation
//! renders only after migrations. One writer transaction locks progress, reads
//! at most 100 ascending current Org/Markdown Posts, and commits the changed
//! projection, its current Media references, feed events, and cursor together.

use std::collections::HashSet;
use std::sync::Arc;

use common::ids::PostId;
use common::mutation::MutationOutcome;
use common::render::PostFormat;
use common::time::UtcInstant;
use sqlx::{Encode, Executor, Type};

use super::media;
use super::models::PostRecord;
use super::store::PostDialect;
use crate::sql::{Exists, QueryStorageExt};
use crate::{FeedEventError, FeedEventStorage, WriteScope, WriteScopeError, WriteTransaction};

// Every persisted role has its own SQLx bridge, including the zero cursor and
// completion flag. Raw scalars cannot enter SQL or emerge from decoded rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::SqlxBridge)]
pub(crate) struct RefreshVersion(i32);
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::SqlxBridge)]
pub(crate) struct RefreshCursor(i64);
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::SqlxBridge)]
pub(crate) struct RefreshCompleted(bool);
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::SqlxBridge)]
pub(crate) struct RefreshBatchLimit(i64);

const VERSION: RefreshVersion = RefreshVersion(1);
const BATCH_SIZE: u8 = 100;

/// A host-side presentation refresh failure. Nothing in the current batch was
/// checkpointed if this came from a write callback.
#[derive(Debug, thiserror::Error)]
pub enum PostProjectionRefreshError {
    /// The database could not complete the read or mutation.
    #[error("Post projection refresh storage failure: {0}")]
    Storage(#[from] sqlx::Error),
    /// Rendering unexpectedly failed; a partial refresh is never accepted.
    #[error("Post projection refresh rendering failure: {0}")]
    Render(#[from] host::render::HighlightError),
    /// Feed regeneration could not be scheduled within the same transaction.
    #[error("Post projection refresh feed-event failure: {0}")]
    Feed(#[from] FeedEventError),
    /// The migration's single progress row is missing or not this version.
    #[error("Post projection refresh checkpoint is missing or has an unsupported version")]
    Checkpoint,
    /// A current Post changed despite its writer lock and CAS guard.
    #[error("Post {0} changed during projection refresh")]
    Stale(PostId),
    /// A commit without acknowledgement could not be confirmed on retry.
    #[error("Post projection refresh could not confirm three consecutive commits")]
    IndeterminateCommits,
}

#[derive(Debug)]
struct BatchProgress {
    cursor: i64,
    completed: bool,
    changed: usize,
}

/// Runs until the durable checkpoint reports completion; when commit
/// acknowledgement is lost the next transaction re-reads progress before
/// deciding whether another batch is necessary.
///
/// # Errors
/// Returns a typed failure on an invalid checkpoint, render failure, CAS
/// mismatch, or storage/feed-event failure; no batch advances on callback error.
pub(crate) async fn run<DB>(
    scope: &WriteScope,
    feed_events: Arc<dyn FeedEventStorage>,
) -> Result<(), PostProjectionRefreshError>
where
    DB: PostDialect,
    for<'r> (RefreshVersion, RefreshCursor, RefreshCompleted): sqlx::FromRow<'r, DB::Row>,
    for<'r> (PostId,): sqlx::FromRow<'r, DB::Row>,
    for<'r> (RefreshCursor,): sqlx::FromRow<'r, DB::Row>,
    for<'r> (Exists,): sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> i32: Encode<'q, DB> + Type<DB>,
    for<'q> bool: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let mut unconfirmed = 0;
    loop {
        let events = Arc::clone(&feed_events);
        let outcome = scope
            .run(|transaction| Box::pin(batch::<DB>(transaction, events)))
            .await
            .map_err(|error| match error {
                WriteScopeError::Begin(error) => PostProjectionRefreshError::Storage(error),
                WriteScopeError::Operation(error) => error,
            })?;
        match outcome {
            MutationOutcome::Confirmed(progress) => {
                tracing::info!(
                    cursor = progress.cursor,
                    changed = progress.changed,
                    completed = progress.completed,
                    "current Post projection refresh batch committed"
                );
                unconfirmed = 0;
                if progress.completed {
                    return Ok(());
                }
            }
            MutationOutcome::CommitIndeterminate(_) => {
                unconfirmed += 1;
                if unconfirmed >= 3 {
                    return Err(PostProjectionRefreshError::IndeterminateCommits);
                }
            }
        }
    }
}

async fn batch<DB>(
    transaction: &mut WriteTransaction,
    feed_events: Arc<dyn FeedEventStorage>,
) -> Result<BatchProgress, PostProjectionRefreshError>
where
    DB: PostDialect,
    for<'r> (RefreshVersion, RefreshCursor, RefreshCompleted): sqlx::FromRow<'r, DB::Row>,
    for<'r> (PostId,): sqlx::FromRow<'r, DB::Row>,
    for<'r> (RefreshCursor,): sqlx::FromRow<'r, DB::Row>,
    for<'r> (Exists,): sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> i32: Encode<'q, DB> + Type<DB>,
    for<'q> bool: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let conn = DB::write_connection(transaction)?;
    let (version, mut cursor, completed) = sqlx::query_as::<
        DB,
        (RefreshVersion, RefreshCursor, RefreshCompleted),
    >(DB::PROJECTION_REFRESH_PROGRESS_SQL)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(PostProjectionRefreshError::Checkpoint)?;
    if version != VERSION || cursor.0 < 0 {
        return Err(PostProjectionRefreshError::Checkpoint);
    }
    if completed.0 {
        return Ok(BatchProgress {
            cursor: cursor.0,
            completed: true,
            changed: 0,
        });
    }

    let candidates = sqlx::query_scalar::<DB, PostId>(
        "SELECT post_id FROM posts
         WHERE post_id > $1 AND deleted_at IS NULL AND format IN ('org', 'markdown')
         ORDER BY post_id LIMIT $2",
    )
    .bind_storage(cursor)
    .bind_storage(RefreshBatchLimit(i64::from(BATCH_SIZE)))
    .fetch_all(&mut *conn)
    .await?;
    let done = candidates.len() < usize::from(BATCH_SIZE);
    let mut changed = 0;
    let mut affected = HashSet::new();

    for post_id in candidates {
        cursor = RefreshCursor(i64::from(post_id));
        if let Some((record, public)) = refresh_candidate::<DB>(transaction, post_id).await? {
            changed += 1;
            affected.extend(crate::post_service::affected_post_feed_paths(
                None,
                (&record, public),
                UtcInstant::now(),
            ));
        }
    }

    // Enqueue once per affected URL, regardless of how many changed Posts
    // share a Site/User/Tag feed. All queue rows join the cursor transaction.
    let mut paths = affected.into_iter().collect::<Vec<_>>();
    paths.sort_by(|a, b| a.as_ref().cmp(b.as_ref()));
    if !paths.is_empty() {
        feed_events.enqueue_many(transaction, &paths).await?;
    }
    let conn = DB::write_connection(transaction)?;
    let progress = sqlx::query_scalar::<DB, RefreshCursor>(
        "UPDATE post_projection_refresh_progress
         SET cursor_post_id = $1, completed = $2
         WHERE id = 1 AND version = $3 AND cursor_post_id <= $1 AND completed = FALSE
         RETURNING cursor_post_id",
    )
    .bind_storage(cursor)
    .bind_storage(RefreshCompleted(done))
    .bind_storage(VERSION)
    .fetch_optional(&mut *conn)
    .await?;
    if progress != Some(cursor) {
        return Err(PostProjectionRefreshError::Checkpoint);
    }
    Ok(BatchProgress {
        cursor: cursor.0,
        completed: done,
        changed,
    })
}

/// Read after acquiring the Post lock, not from the earlier ID-only candidate
/// scan: a competing author edit or deletion may have committed meanwhile.
async fn refresh_candidate<DB>(
    transaction: &mut WriteTransaction,
    post_id: PostId,
) -> Result<Option<(PostRecord, bool)>, PostProjectionRefreshError>
where
    DB: PostDialect,
    for<'r> (PostId,): sqlx::FromRow<'r, DB::Row>,
    for<'r> (Exists,): sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let conn = DB::write_connection(transaction)?;
    let locked = sqlx::query(DB::LIFECYCLE_STATE_SQL)
        .bind_storage(post_id)
        .fetch_optional(&mut *conn)
        .await?
        .is_some();
    if !locked {
        return Ok(None);
    }
    let record = DB::fetch_lifecycle_post(conn, post_id).await?;
    if record.deleted_at.is_some()
        || !matches!(record.format, PostFormat::Org | PostFormat::Markdown)
    {
        return Ok(None);
    }
    let rendered = host::render::with_media(&record.body, &record.format)?;
    if record.rendered_html.as_ref() == rendered.html().as_ref() {
        return Ok(None);
    }
    DB::lock_lifecycle_media_references(conn, post_id).await?;
    let persisted = sqlx::query_scalar::<DB, PostId>(
        "UPDATE posts SET rendered_html = $1
         WHERE post_id = $2 AND body = $3 AND format = $4 AND rendered_html = $5
           AND deleted_at IS NULL RETURNING post_id",
    )
    .bind_storage(rendered.html())
    .bind_storage(post_id)
    .bind_storage(&record.body)
    .bind_storage(record.format)
    .bind_storage(&record.rendered_html)
    .fetch_optional(&mut *conn)
    .await?;
    if persisted != Some(post_id) {
        return Err(PostProjectionRefreshError::Stale(post_id));
    }
    media::replace_post_media::<DB>(conn, post_id, rendered.media()).await?;
    let public = sqlx::query_scalar::<DB, Exists>(
        "SELECT EXISTS(
            SELECT 1 FROM post_audiences pa JOIN target_kinds tk
                ON tk.kind_id = pa.target_kind_id
            WHERE pa.post_id = $1 AND tk.name = 'public'
         )",
    )
    .bind_storage(post_id)
    .fetch_one(&mut *conn)
    .await?
    .into_bool();
    Ok(Some((record, public)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorageFactory;
    use crate::test_support::{Backend, CloseablePool, SeedRawPost, SeedUser, TestEnv, backends};
    use common::test_support::parse_post_body;
    use common::visibility::ViewerIdentity;
    use rstest::*;
    use rstest_reuse::*;

    fn factory(env: &TestEnv) -> StorageFactory {
        match env.base.pool() {
            CloseablePool::Sqlite(pool) => StorageFactory::sqlite(pool.clone()),
            CloseablePool::Postgres(pool) => StorageFactory::postgres(pool.clone()),
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn public_tagged_posts_enqueue_exact_affected_feed_paths_once_per_batch(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let mut ids = Vec::new();
        for _ in 0..2 {
            ids.push(
                SeedRawPost::new(owner)
                    .body(parse_post_body("```rust\nfn main() {}\n```"))
                    .tags(["Rust"])
                    .seed(env.posts(), env.write_scope().clone())
                    .await
                    .post_id,
            );
        }
        env.base
            .pool()
            .execute("UPDATE posts SET rendered_html = '<p>old presentation</p>'")
            .await
            .expect("stale fixtures");
        let record = env
            .posts()
            .get_post_by_id(ids[0], &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("lookup")
            .expect("first Post");
        let mut expected = host::feed::affected_feed_urls(
            &record.author_username,
            record.tags.iter().map(|tag| &tag.tag_slug),
        )
        .into_iter()
        .map(|path| path.as_ref().to_owned())
        .collect::<Vec<String>>();
        expected.sort();
        factory(&env)
            .refresh_current_post_projections()
            .await
            .expect("refresh");
        let actual: Vec<String> = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar("SELECT feed_url FROM feed_events ORDER BY feed_url")
                .fetch_all(pool)
                .await
                .expect("feed paths")
        });
        assert_eq!(actual, expected);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn existing_post_revisions_remain_immutable_during_projection_refresh(
        #[case] backend: Backend,
    ) {
        use crate::test_support::UpdateRawPost;

        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let post = SeedRawPost::new(owner)
            .format(PostFormat::Org)
            .body(parse_post_body(
                "#+begin_src elisp\n(message \"prior\")\n#+end_src",
            ))
            .seed(env.posts(), env.write_scope().clone())
            .await;
        let input = UpdateRawPost::new(post.slug.as_ref())
            .body(parse_post_body("```python\nprint(\"current\")\n```"))
            .format(PostFormat::Markdown)
            .build();
        let posts = env.posts();
        let result = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    posts
                        .update_post(transaction, post.post_id, owner, &input)
                        .await
                })
            })
            .await
            .expect("update Post with revision");
        assert!(matches!(result, MutationOutcome::Confirmed(_)));
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE post_revisions SET rendered_html = '<p>historical projection</p>' WHERE post_id = $1")
                .bind_storage(post.post_id)
                .execute(pool)
                .await
                .expect("old revision fixture");
            sqlx::query("UPDATE posts SET rendered_html = '<p>old current projection</p>' WHERE post_id = $1")
                .bind_storage(post.post_id)
                .execute(pool)
                .await
                .expect("old current fixture");
        });
        factory(&env)
            .refresh_current_post_projections()
            .await
            .expect("refresh");
        let (current, history): (String, Vec<String>) = crate::with_closeable_pool!(
            env.base.pool(),
            pool,
            {
                let current =
                    sqlx::query_scalar("SELECT rendered_html FROM posts WHERE post_id = $1")
                        .bind_storage(post.post_id)
                        .fetch_one(pool)
                        .await
                        .expect("current projection");
                let history = sqlx::query_scalar(
                    "SELECT rendered_html FROM post_revisions WHERE post_id = $1 ORDER BY revision_id",
                )
                .bind_storage(post.post_id)
                .fetch_all(pool)
                .await
                .expect("historical projections");
                (current, history)
            }
        );
        assert!(current.contains("j-syn-"));
        assert_eq!(history, vec!["<p>historical projection</p>"]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn changed_html_replaces_only_current_media_references(#[case] backend: Backend) {
        use crate::test_support::media_url_for;

        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let image = media_url_for("code.png");
        let source = parse_post_body(&format!(
            "![diagram]({image})\n\n```rust\nfn main() {{}}\n```"
        ));
        let post = SeedRawPost::new(owner)
            .body(source)
            .seed(env.posts(), env.write_scope().clone())
            .await;
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("DELETE FROM post_media WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0")
                .bind_storage(post.post_id)
                .execute(pool)
                .await
                .expect("simulate stale media projection");
            sqlx::query(
                "UPDATE posts SET rendered_html = '<p>old presentation</p>' WHERE post_id = $1",
            )
            .bind_storage(post.post_id)
            .execute(pool)
            .await
            .expect("simulate stale HTML projection");
        });
        factory(&env)
            .refresh_current_post_projections()
            .await
            .expect("refresh");
        let (rendered, references): (String, i64) = crate::with_closeable_pool!(
            env.base.pool(),
            pool,
            {
                let rendered =
                    sqlx::query_scalar("SELECT rendered_html FROM posts WHERE post_id = $1")
                        .bind_storage(post.post_id)
                        .fetch_one(pool)
                        .await
                        .expect("current HTML");
                let references = sqlx::query_scalar(
                    "SELECT count(*) FROM post_media WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0",
                )
                .bind_storage(post.post_id)
                .fetch_one(pool)
                .await
                .expect("current media count");
                (rendered, references)
            }
        );
        assert!(rendered.contains("j-syn-"));
        assert!(rendered.contains(&image));
        assert_eq!(references, 1);
        assert_eq!(
            env.count_post_revisions(post.post_id)
                .await
                .expect("revisions"),
            0
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn failed_feed_enqueue_rolls_back_projection_and_cursor_then_resumes(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let post = SeedRawPost::new(owner)
            .body(parse_post_body("```rust\nfn main() {}\n```"))
            .seed(env.posts(), env.write_scope().clone())
            .await;
        env.base
            .pool()
            .execute("UPDATE posts SET rendered_html = '<p>old presentation</p>'")
            .await
            .expect("stale fixture");
        match env.base.pool() {
            CloseablePool::Sqlite(_) => env
                .base
                .pool()
                .execute("CREATE TRIGGER reject_refresh_event BEFORE INSERT ON feed_events BEGIN SELECT RAISE(ABORT, 'blocked by test'); END")
                .await
                .expect("SQLite failure injection"),
            CloseablePool::Postgres(_) => {
                env.base
                    .pool()
                    .execute("CREATE FUNCTION reject_refresh_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'blocked by test'; END; $$")
                    .await
                    .expect("Postgres trigger function");
                env.base
                    .pool()
                    .execute("CREATE TRIGGER reject_refresh_event BEFORE INSERT ON feed_events FOR EACH ROW EXECUTE FUNCTION reject_refresh_event()")
                    .await
                    .expect("Postgres failure injection");
            }
        }
        assert!(matches!(
            factory(&env).refresh_current_post_projections().await,
            Err(PostProjectionRefreshError::Feed(_))
        ));
        let (html, cursor, complete, events): (String, i64, bool, i64) = crate::with_closeable_pool!(
            env.base.pool(),
            pool,
            {
                let html = sqlx::query_scalar("SELECT rendered_html FROM posts WHERE post_id = $1")
                    .bind_storage(post.post_id)
                    .fetch_one(pool)
                    .await
                    .expect("rolled-back Post");
                let (cursor, complete) = sqlx::query_as(
                    "SELECT cursor_post_id, completed FROM post_projection_refresh_progress WHERE id = 1",
                )
                .fetch_one(pool)
                .await
                .expect("progress");
                let events = sqlx::query_scalar("SELECT count(*) FROM feed_events")
                    .fetch_one(pool)
                    .await
                    .expect("feed count");
                (html, cursor, complete, events)
            }
        );
        assert_eq!(html, "<p>old presentation</p>");
        assert_eq!(cursor, 0);
        assert!(!complete);
        assert_eq!(events, 0);
        match env.base.pool() {
            CloseablePool::Sqlite(_) => env
                .base
                .pool()
                .execute("DROP TRIGGER reject_refresh_event")
                .await
                .expect("remove SQLite fault"),
            CloseablePool::Postgres(_) => env
                .base
                .pool()
                .execute("DROP TRIGGER reject_refresh_event ON feed_events")
                .await
                .expect("remove Postgres fault"),
        }
        factory(&env)
            .refresh_current_post_projections()
            .await
            .expect("resume");
        let updated = env
            .posts()
            .get_post_by_id(post.post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("lookup")
            .expect("Post exists");
        assert!(updated.rendered_html.as_ref().contains("j-syn-"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn concurrent_startups_share_one_checkpoint_and_one_feed_fanout(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let post = SeedRawPost::new(owner)
            .body(parse_post_body("```rust\nfn main() {}\n```"))
            .seed(env.posts(), env.write_scope().clone())
            .await;
        env.base
            .pool()
            .execute("UPDATE posts SET rendered_html = '<p>old presentation</p>'")
            .await
            .expect("stale fixture");
        let first = factory(&env);
        let second = factory(&env);
        let (a, b) = tokio::join!(
            first.refresh_current_post_projections(),
            second.refresh_current_post_projections()
        );
        a.expect("first startup");
        b.expect("second startup");
        let record = env
            .posts()
            .get_post_by_id(post.post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("lookup")
            .expect("Post exists");
        assert!(record.rendered_html.as_ref().contains("j-syn-"));
        let (events, complete): (i64, bool) = crate::with_closeable_pool!(env.base.pool(), pool, {
            let events = sqlx::query_scalar("SELECT count(*) FROM feed_events")
                .fetch_one(pool)
                .await
                .expect("feed count");
            let complete = sqlx::query_scalar(
                "SELECT completed FROM post_projection_refresh_progress WHERE id = 1",
            )
            .fetch_one(pool)
            .await
            .expect("progress");
            (events, complete)
        });
        assert!(complete);
        assert_eq!(events, 6);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn committed_cursor_resumes_after_one_bounded_batch_without_skipping_posts(
        #[case] backend: Backend,
    ) {
        use crate::test_support::create_posts_confirmed;

        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let inputs = (0..205)
            .map(|_| {
                SeedRawPost::new(owner)
                    .body(parse_post_body("```rust\nfn main() {}\n```"))
                    .build()
            })
            .collect();
        let ids = create_posts_confirmed(env.posts(), env.write_scope().clone(), inputs).await;
        env.base
            .pool()
            .execute("UPDATE posts SET rendered_html = '<p>old presentation</p>'")
            .await
            .expect("stale fixtures");

        let scope = env.write_scope();
        let events = env.feed_events();
        let first = match env.base.pool() {
            CloseablePool::Sqlite(_) => {
                scope
                    .run(|transaction| Box::pin(batch::<sqlx::Sqlite>(transaction, events)))
                    .await
            }
            CloseablePool::Postgres(_) => {
                scope
                    .run(|transaction| Box::pin(batch::<sqlx::Postgres>(transaction, events)))
                    .await
            }
        }
        .expect("first batch");
        let MutationOutcome::Confirmed(first) = first else {
            panic!("fixture batch commit must be confirmed");
        };
        assert_eq!(first.cursor, i64::from(ids[99]));
        assert_eq!(first.changed, 100);
        assert!(!first.completed);

        // An author edit and a deletion after the committed cursor must be
        // observed at the next batch's locked read, not from a stale snapshot.
        crate::with_closeable_pool!(env.base.pool(), pool, {
            let new_source = parse_post_body("#+begin_src elisp\n(message \"new\")\n#+end_src");
            sqlx::query("UPDATE posts SET body = $1, format = 'org' WHERE post_id = $2")
                .bind_storage(&new_source)
                .bind_storage(ids[100])
                .execute(pool)
                .await
                .expect("concurrent author edit");
            sqlx::query("UPDATE posts SET deleted_at = created_at WHERE post_id = $1")
                .bind_storage(ids[101])
                .execute(pool)
                .await
                .expect("concurrent deletion");
        });
        factory(&env)
            .refresh_current_post_projections()
            .await
            .expect("resume");
        let edited = env
            .posts()
            .get_post_by_id(ids[100], &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("edited Post lookup")
            .expect("edited Post exists");
        assert_eq!(edited.format, PostFormat::Org);
        assert!(edited.rendered_html.as_ref().contains("j-syn-"));
        let (cursor, completed, remaining, events): (i64, bool, i64, i64) = crate::with_closeable_pool!(
            env.base.pool(),
            pool,
            {
                let (cursor, completed) = sqlx::query_as(
                    "SELECT cursor_post_id, completed FROM post_projection_refresh_progress WHERE id = 1",
                )
                .fetch_one(pool)
                .await
                .expect("progress");
                let remaining = sqlx::query_scalar(
                    "SELECT count(*) FROM posts WHERE deleted_at IS NULL AND rendered_html = '<p>old presentation</p>'",
                )
                .fetch_one(pool)
                .await
                .expect("stale Post count");
                let events = sqlx::query_scalar("SELECT count(*) FROM feed_events")
                    .fetch_one(pool)
                    .await
                    .expect("feed count");
                (cursor, completed, remaining, events)
            }
        );
        assert_eq!(cursor, i64::from(ids[204]));
        assert!(completed);
        assert_eq!(remaining, 0);
        assert_eq!(events, 18, "six feed paths per changed batch");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn refreshes_private_draft_and_scheduled_but_skips_html_and_deleted(
        #[case] backend: Backend,
    ) {
        use common::test_support::parse_utc_instant;
        use common::visibility::AudienceTarget;

        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let source = parse_post_body("```python\nprint(\"hi\")\n```");
        let private = SeedRawPost::new(owner)
            .body(source.clone())
            .audiences(vec![AudienceTarget::Private])
            .seed(env.posts(), env.write_scope().clone())
            .await;
        let draft = SeedRawPost::new(owner)
            .body(source.clone())
            .draft()
            .seed(env.posts(), env.write_scope().clone())
            .await;
        let scheduled = SeedRawPost::new(owner)
            .body(source.clone())
            .published_at(parse_utc_instant("2999-01-01T00:00:00Z"))
            .seed(env.posts(), env.write_scope().clone())
            .await;
        let deleted = SeedRawPost::new(owner)
            .body(source)
            .seed(env.posts(), env.write_scope().clone())
            .await;
        let html = SeedRawPost::new(owner)
            .format(PostFormat::Html)
            .body(parse_post_body("<pre><code>plain HTML</code></pre>"))
            .seed(env.posts(), env.write_scope().clone())
            .await;
        env.base
            .pool()
            .execute("UPDATE posts SET rendered_html = '<p>old presentation</p>'")
            .await
            .expect("seed stale projections");
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE posts SET deleted_at = created_at WHERE post_id = $1")
                .bind_storage(deleted.post_id)
                .execute(pool)
                .await
                .expect("mark deleted");
        });

        factory(&env)
            .refresh_current_post_projections()
            .await
            .expect("refresh");
        for post in [private.post_id, draft.post_id, scheduled.post_id] {
            let record = env
                .posts()
                .get_post_by_id(post, &ViewerIdentity::Local { user_id: owner })
                .await
                .expect("lookup")
                .expect("active Post");
            assert!(record.rendered_html.as_ref().contains("j-syn-"), "{post}");
            assert_eq!(env.count_post_revisions(post).await.expect("revisions"), 0);
        }
        crate::with_closeable_pool!(env.base.pool(), pool, {
            for post in [deleted.post_id, html.post_id] {
                let stored: String =
                    sqlx::query_scalar("SELECT rendered_html FROM posts WHERE post_id = $1")
                        .bind_storage(post)
                        .fetch_one(pool)
                        .await
                        .expect("stored projection");
                assert_eq!(stored, "<p>old presentation</p>");
            }
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM feed_events")
                .fetch_one(pool)
                .await
                .expect("feed count");
            assert_eq!(events, 0, "nonpublic and skipped rows do not enqueue feeds");
        });
    }

    #[apply(backends)]
    #[tokio::test]
    async fn refreshes_existing_org_and_markdown_once_without_authored_changes(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let org = SeedRawPost::new(owner)
            .format(PostFormat::Org)
            .body(parse_post_body(
                "#+begin_src elisp\n(message \"hi\")\n#+end_src",
            ))
            .seed(env.posts(), env.write_scope().clone())
            .await;
        let markdown = SeedRawPost::new(owner)
            .body(parse_post_body("```rust\nfn main() {}\n```"))
            .seed(env.posts(), env.write_scope().clone())
            .await;
        env.base
            .pool()
            .execute("UPDATE posts SET rendered_html = '<p>old presentation</p>'")
            .await
            .expect("simulate pre-upgrade rendered HTML");
        let original = env
            .posts()
            .get_post_by_id(org.post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("post lookup")
            .expect("org Post exists");
        let factory = factory(&env);
        factory
            .refresh_current_post_projections()
            .await
            .expect("refresh");
        let updated_org = env
            .posts()
            .get_post_by_id(org.post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("post lookup")
            .expect("org Post exists");
        let updated_md = env
            .posts()
            .get_post_by_id(markdown.post_id, &ViewerIdentity::Local { user_id: owner })
            .await
            .expect("post lookup")
            .expect("Markdown Post exists");
        assert!(updated_org.rendered_html.as_ref().contains("j-syn-"));
        assert!(updated_md.rendered_html.as_ref().contains("j-syn-"));
        assert_eq!(updated_org.body, original.body);
        assert_eq!(updated_org.created_at, original.created_at);
        assert_eq!(updated_org.updated_at, original.updated_at);
        assert_eq!(updated_org.title, original.title);
        assert_eq!(updated_org.rendered_title, original.rendered_title);
        let member_etag = |record: &PostRecord| {
            host::etag::PostContentEtag {
                title: record.title.as_ref(),
                slug: &record.slug,
                body: &record.body,
                format: record.format,
                summary: record.summary.as_ref(),
                tags: record.tags.iter().map(|tag| &tag.tag_display).collect(),
                audiences: vec![common::visibility::AudienceTarget::Public],
                draft: false,
            }
            .etag()
        };
        assert_eq!(member_etag(&original), member_etag(&updated_org));
        assert_eq!(
            env.count_post_revisions(org.post_id)
                .await
                .expect("revisions"),
            0
        );
        let (version, cursor, completed, events): (i32, i64, bool, i64) = crate::with_closeable_pool!(
            env.base.pool(),
            pool,
            {
                let (version, cursor, completed) = sqlx::query_as(
                    "SELECT version, cursor_post_id, completed FROM post_projection_refresh_progress WHERE id = 1",
                )
                .fetch_one(pool)
                .await
                .expect("progress");
                let events = sqlx::query_scalar("SELECT count(*) FROM feed_events")
                    .fetch_one(pool)
                    .await
                    .expect("feed count");
                (version, cursor, completed, events)
            }
        );
        assert_eq!(version, VERSION.0);
        assert_eq!(cursor, i64::from(markdown.post_id));
        assert!(completed);
        assert_eq!(events, 6, "one Site and User event per feed format");
        factory
            .refresh_current_post_projections()
            .await
            .expect("idempotent resume");
        let event_count: i64 = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar("SELECT count(*) FROM feed_events")
                .fetch_one(pool)
                .await
                .expect("feed count after resume")
        });
        assert_eq!(event_count, events);
    }
}
