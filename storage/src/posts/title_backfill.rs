//! Startup repair of persisted Rendered Title derivatives.
//!
//! Legacy rows are rendered before each bounded write transaction. Conditional
//! installs make a restart safe when another opener completes a chunk first.

use sqlx::{Database, Encode, Executor, Pool, Type};

use common::ids::{PostId, RevisionId};
use common::mutation::MutationOutcome;
use common::post_title::PostTitle;
use common::render::{PostFormat, RenderedPostTitle};
use host::render::render_title;

use crate::{
    WriteScopeError,
    backend::{Backend, WriteScopeFactoryBackend},
    site_config::StoredSiteConfigKey,
    sql::{Exists, QueryStorageExt, RowCount},
};

/// Raw internal `site_config` key recording an incomplete migration backfill.
pub(crate) const RENDERED_POST_TITLE_BACKFILL_PENDING_KEY: &str =
    "migration.0038.rendered_title_backfill_pending";

pub(crate) async fn backfill_rendered_post_titles<DB>(pool: &Pool<DB>) -> sqlx::Result<()>
where
    DB: Backend + WriteScopeFactoryBackend,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedPostTitle: Encode<'q, DB> + Type<DB>,
    for<'r> PostId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> RevisionId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostTitle: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostFormat: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> i64: sqlx::Decode<'r, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    backfill_rendered_post_titles_with_control(pool, &mut BackfillControl::default()).await
}

/// Test-only deterministic control at the rendered-to-install boundary.
///
/// The production entry point always supplies the inert default control. Keeping
/// the control adjacent to the loop lets migration-contract tests prove restart
/// behavior at the actual committed-chunk boundary without timing a lock.
#[cfg(test)]
pub(crate) struct BackfillTestControl {
    control: BackfillControl,
}

#[cfg(test)]
impl BackfillTestControl {
    pub(crate) fn complete() -> Self {
        Self {
            control: BackfillControl::default(),
        }
    }

    pub(crate) fn interrupt_after(target: BackfillTarget) -> Self {
        Self {
            control: BackfillControl {
                interrupt_after: Some(target),
                ..BackfillControl::default()
            },
        }
    }

    pub(crate) fn stale_source(target: BackfillTarget) -> Self {
        Self {
            control: BackfillControl {
                stale_source: Some(target),
                ..BackfillControl::default()
            },
        }
    }

    pub(crate) fn rendered_chunks(&self) -> usize {
        self.control.rendered_chunks
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackfillTarget {
    Posts,
    Revisions,
}

#[derive(Default)]
struct BackfillControl {
    #[cfg(test)]
    interrupt_after: Option<BackfillTarget>,
    #[cfg(test)]
    stale_source: Option<BackfillTarget>,
    #[cfg(test)]
    rendered_chunks: usize,
}

#[cfg(test)]
pub(crate) async fn backfill_rendered_post_titles_with_test_control<DB>(
    pool: &Pool<DB>,
    test_control: &mut BackfillTestControl,
) -> sqlx::Result<()>
where
    DB: Backend + WriteScopeFactoryBackend,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedPostTitle: Encode<'q, DB> + Type<DB>,
    for<'r> PostId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> RevisionId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostTitle: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostFormat: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> i64: sqlx::Decode<'r, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    backfill_rendered_post_titles_with_control(pool, &mut test_control.control).await
}

async fn backfill_rendered_post_titles_with_control<DB>(
    pool: &Pool<DB>,
    control: &mut BackfillControl,
) -> sqlx::Result<()>
where
    DB: Backend + WriteScopeFactoryBackend,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedPostTitle: Encode<'q, DB> + Type<DB>,
    for<'r> PostId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> RevisionId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostTitle: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostFormat: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> i64: sqlx::Decode<'r, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    backfill_posts(pool, control).await?;
    backfill_revisions(pool, control).await?;
    validate_rendered_post_title_presence(pool).await
}

/// Returns whether startup must resume the bounded migration backfill.
pub(crate) async fn rendered_post_title_backfill_is_pending<DB>(
    pool: &Pool<DB>,
) -> sqlx::Result<bool>
where
    DB: Database,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'r> Exists: sqlx::Decode<'r, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    sqlx::query_scalar::<DB, Exists>("SELECT EXISTS(SELECT 1 FROM site_config WHERE key = $1)")
        .bind_storage(StoredSiteConfigKey::raw(
            RENDERED_POST_TITLE_BACKFILL_PENDING_KEY,
        ))
        .fetch_one(pool)
        .await
        .map(Exists::into_bool)
}

/// Clears the marker only after the complete bounded backfill and final validation succeed.
pub(crate) async fn clear_rendered_post_title_backfill_pending<DB>(
    pool: &Pool<DB>,
) -> sqlx::Result<()>
where
    DB: Database,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    sqlx::query("DELETE FROM site_config WHERE key = $1")
        .bind_storage(StoredSiteConfigKey::raw(
            RENDERED_POST_TITLE_BACKFILL_PENDING_KEY,
        ))
        .execute(pool)
        .await?;
    Ok(())
}

/// Rejects any persisted title/derivative presence mismatch without repairing it.
pub(crate) async fn validate_rendered_post_title_presence<DB>(pool: &Pool<DB>) -> sqlx::Result<()>
where
    DB: Database,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'r> RowCount: sqlx::Decode<'r, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let missing_posts: RowCount = sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts WHERE (title IS NULL) <> (rendered_title IS NULL)",
    )
    .fetch_one(pool)
    .await?;
    let missing_revisions: RowCount = sqlx::query_scalar(
        "SELECT COUNT(*) FROM post_revisions WHERE (title IS NULL) <> (rendered_title IS NULL)",
    )
    .fetch_one(pool)
    .await?;
    if missing_posts.into_u64() != 0 || missing_revisions.into_u64() != 0 {
        return Err(sqlx::Error::Protocol(format!(
            "rendered title backfill incomplete: {} posts, {} revisions",
            missing_posts.into_u64(),
            missing_revisions.into_u64(),
        )));
    }
    Ok(())
}

async fn backfill_posts<DB>(pool: &Pool<DB>, control: &mut BackfillControl) -> sqlx::Result<()>
where
    DB: Backend + WriteScopeFactoryBackend,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> PostTitle: Encode<'q, DB> + Type<DB>,
    for<'q> PostFormat: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedPostTitle: Encode<'q, DB> + Type<DB>,
    for<'r> PostId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostTitle: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostFormat: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> i64: sqlx::Decode<'r, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    #[cfg(not(test))]
    let _ = control;
    let mut after = PostId::from(0);
    loop {
        let rows: Vec<(PostId, PostTitle, PostFormat)> = sqlx::query_as(
            "SELECT post_id, title, format FROM posts
             WHERE post_id > $1 AND title IS NOT NULL AND rendered_title IS NULL
             ORDER BY post_id LIMIT 100",
        )
        .bind_storage(after)
        .fetch_all(pool)
        .await?;
        let Some(last) = rows.last().map(|(id, _, _)| *id) else {
            return Ok(());
        };
        // Rendering is intentionally outside the write transaction (ADR-0092).
        let rendered: Vec<_> = rows
            .into_iter()
            .map(|(id, title, format)| {
                let rendered = render_title(&title, &format);
                (id, title, format, rendered)
            })
            .collect();
        #[cfg(test)]
        {
            control.rendered_chunks += 1;
            if control.stale_source == Some(BackfillTarget::Posts) {
                sqlx::query(
                    "UPDATE posts SET title = 'changed while rendering' WHERE post_id = $1",
                )
                .bind_storage(rendered[0].0)
                .execute(pool)
                .await?;
                control.stale_source = None;
            }
        }
        install_posts_chunk::<DB>(pool, rendered).await?;
        #[cfg(test)]
        if control.interrupt_after == Some(BackfillTarget::Posts) {
            control.interrupt_after = None;
            return Err(sqlx::Error::Protocol(
                "interrupted after posts chunk commit".into(),
            ));
        }
        after = last;
    }
}

async fn backfill_revisions<DB>(pool: &Pool<DB>, control: &mut BackfillControl) -> sqlx::Result<()>
where
    DB: Backend + WriteScopeFactoryBackend,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'q> PostTitle: Encode<'q, DB> + Type<DB>,
    for<'q> PostFormat: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedPostTitle: Encode<'q, DB> + Type<DB>,
    for<'r> RevisionId: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostTitle: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> PostFormat: sqlx::Decode<'r, DB> + Type<DB>,
    for<'r> i64: sqlx::Decode<'r, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    #[cfg(not(test))]
    let _ = control;
    let mut after = RevisionId::from(0);
    loop {
        let rows: Vec<(RevisionId, PostTitle, PostFormat)> = sqlx::query_as(
            "SELECT revision_id, title, format FROM post_revisions
             WHERE revision_id > $1 AND title IS NOT NULL AND rendered_title IS NULL
             ORDER BY revision_id LIMIT 100",
        )
        .bind_storage(after)
        .fetch_all(pool)
        .await?;
        let Some(last) = rows.last().map(|(id, _, _)| *id) else {
            return Ok(());
        };
        let rendered: Vec<_> = rows
            .into_iter()
            .map(|(id, title, format)| {
                let rendered = render_title(&title, &format);
                (id, title, format, rendered)
            })
            .collect();
        #[cfg(test)]
        {
            control.rendered_chunks += 1;
            if control.stale_source == Some(BackfillTarget::Revisions) {
                sqlx::query(
                    "UPDATE post_revisions SET title = 'changed while rendering' WHERE revision_id = $1",
                )
                .bind_storage(rendered[0].0)
                .execute(pool)
                .await?;
                control.stale_source = None;
            }
        }
        install_revisions_chunk::<DB>(pool, rendered).await?;
        #[cfg(test)]
        if control.interrupt_after == Some(BackfillTarget::Revisions) {
            control.interrupt_after = None;
            return Err(sqlx::Error::Protocol(
                "interrupted after revisions chunk commit".into(),
            ));
        }
        after = last;
    }
}

/// Installs one pre-rendered Posts page through the factory-owned write capability.
/// A lost commit acknowledgement is deliberately safe: each update is conditional
/// and the outer loop's final validator determines whether another pass is needed.
async fn install_posts_chunk<DB>(
    pool: &Pool<DB>,
    rendered: Vec<(PostId, PostTitle, PostFormat, RenderedPostTitle)>,
) -> sqlx::Result<()>
where
    DB: Backend + WriteScopeFactoryBackend,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> PostTitle: Encode<'q, DB> + Type<DB>,
    for<'q> PostFormat: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedPostTitle: Encode<'q, DB> + Type<DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let scope = DB::write_scope(pool.clone());
    match scope
        .run(move |transaction| {
            Box::pin(async move {
                let connection = DB::write_connection(transaction)?;
                for (id, title, format, rendered_title) in rendered {
                    sqlx::query(
                        "UPDATE posts SET rendered_title = $1
                         WHERE post_id = $2 AND rendered_title IS NULL AND title = $3 AND format = $4",
                    )
                    .bind_storage(rendered_title)
                    .bind_storage(id)
                    .bind_storage(title)
                    .bind_storage(format)
                    .execute(&mut *connection)
                    .await?;
                }
                Ok(())
            })
        })
        .await
    {
        Ok(MutationOutcome::Confirmed(()) | MutationOutcome::CommitIndeterminate(())) => Ok(()),
        Err(WriteScopeError::Begin(error) | WriteScopeError::Operation(error)) => Err(error),
    }
}

/// Installs one pre-rendered Post Revision page through the factory-owned write capability.
/// A lost commit acknowledgement is deliberately safe: each update is conditional
/// and the outer loop's final validator determines whether another pass is needed.
async fn install_revisions_chunk<DB>(
    pool: &Pool<DB>,
    rendered: Vec<(RevisionId, PostTitle, PostFormat, RenderedPostTitle)>,
) -> sqlx::Result<()>
where
    DB: Backend + WriteScopeFactoryBackend,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'q> PostTitle: Encode<'q, DB> + Type<DB>,
    for<'q> PostFormat: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedPostTitle: Encode<'q, DB> + Type<DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let scope = DB::write_scope(pool.clone());
    match scope
        .run(move |transaction| {
            Box::pin(async move {
                let connection = DB::write_connection(transaction)?;
                for (id, title, format, rendered_title) in rendered {
                    sqlx::query(
                        "UPDATE post_revisions SET rendered_title = $1
                         WHERE revision_id = $2 AND rendered_title IS NULL AND title = $3 AND format = $4",
                    )
                    .bind_storage(rendered_title)
                    .bind_storage(id)
                    .bind_storage(title)
                    .bind_storage(format)
                    .execute(&mut *connection)
                    .await?;
                }
                Ok(())
            })
        })
        .await
    {
        Ok(MutationOutcome::Confirmed(()) | MutationOutcome::CommitIndeterminate(())) => Ok(()),
        Err(WriteScopeError::Begin(error) | WriteScopeError::Operation(error)) => Err(error),
    }
}
