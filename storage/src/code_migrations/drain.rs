//! Queue dispatch and per-operation transaction boundary shared by both backends.

use sqlx::{Connection, Encode, Executor, FromRow, Pool, Type};

#[cfg(test)]
tokio::task_local! { static COMMIT_PAUSE: std::sync::Arc<CommitPause>; }

/// Bounded fixture-only observation at the two real COMMIT boundaries. A task
/// cancelled after COMMIT has lost acknowledgment, not a rollback result.
#[cfg(test)]
pub(crate) struct CommitPause {
    pub(crate) after_commit: bool,
    pub(crate) reached: tokio::sync::Notify,
    pub(crate) resume: tokio::sync::Notify,
}

#[cfg(test)]
impl CommitPause {
    pub(crate) fn new(after_commit: bool) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            after_commit,
            reached: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        })
    }
}

#[cfg(test)]
pub(crate) async fn with_commit_pause<F: std::future::Future>(
    pause: std::sync::Arc<CommitPause>,
    future: F,
) -> F::Output {
    COMMIT_PAUSE.scope(pause, future).await
}

#[cfg(test)]
async fn pause_commit(after_commit: bool) {
    if let Ok(pause) = COMMIT_PAUSE.try_with(std::sync::Arc::clone)
        && pause.after_commit == after_commit
    {
        pause.reached.notify_one();
        pause.resume.notified().await;
    }
}

use super::types::{CodeMigrationOperation, CodeMigrationQueueId};
use super::{media_references, rendered_posts};
use crate::posts::models::RenderedHtml;
use crate::sql::QueryStorageExt;
use crate::{PostDialect, helpers};
use common::ids::PostId;
use common::render::RenderedPostTitle;
use common::tag::Tag;
use common::time::UtcInstant;
use common::username::Username;
use host::feed::FeedPath;

/// `SQLite` starts an immediate writer; `PostgreSQL`'s regular transaction is
/// sufficient under the same-directory offline lock and runtime exclusion.
pub(crate) trait CodeMigrationDialect: PostDialect {
    const BEGIN_CODE_MIGRATION: &'static str;

    fn normalize_comparable_instants(
        conn: &mut Self::Connection,
    ) -> impl std::future::Future<Output = sqlx::Result<()>> + Send;
}

impl CodeMigrationDialect for sqlx::Sqlite {
    const BEGIN_CODE_MIGRATION: &'static str = "BEGIN IMMEDIATE";

    async fn normalize_comparable_instants(conn: &mut Self::Connection) -> sqlx::Result<()> {
        super::comparable_instants::normalize_sqlite(conn).await
    }
}

impl CodeMigrationDialect for sqlx::Postgres {
    const BEGIN_CODE_MIGRATION: &'static str = "BEGIN";

    async fn normalize_comparable_instants(conn: &mut Self::Connection) -> sqlx::Result<()> {
        super::comparable_instants::normalize_postgres(conn).await
    }
}

/// Consumes pending operations in insertion order. The caller owns the
/// storage-directory lock; `authorize` is invoked only if work is pending.
pub(crate) async fn drain_pending<DB>(
    pool: &Pool<DB>,
    authorize: &(dyn Fn() -> sqlx::Result<()> + Sync),
) -> sqlx::Result<()>
where
    DB: CodeMigrationDialect,
    for<'r> (CodeMigrationQueueId, CodeMigrationOperation): FromRow<'r, DB::Row>,
    for<'q> CodeMigrationQueueId: Encode<'q, DB> + Type<DB>,
    (PostId, RenderedHtml): for<'r> FromRow<'r, DB::Row>,
    rendered_posts::CurrentPostRendering: for<'r> FromRow<'r, DB::Row>,
    for<'r> (Username,): FromRow<'r, DB::Row>,
    for<'r> Tag: sqlx::Decode<'r, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RenderedHtml: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q RenderedPostTitle>: Encode<'q, DB> + Type<DB>,
    for<'q> FeedPath: Encode<'q, DB> + Type<DB>,
    for<'q> &'q FeedPath: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    // Raw BEGIN is not tracked by SQLx's transaction manager. Own the physical
    // connection so cancellation or failed COMMIT can never pool an open writer.
    let mut conn = pool.acquire().await?.detach();
    while let Some((queue_id, operation)) =
        sqlx::query_as::<DB, (CodeMigrationQueueId, CodeMigrationOperation)>(
            "SELECT queue_id, operation FROM pending_code_migrations ORDER BY queue_id LIMIT 1",
        )
        .fetch_optional(&mut conn)
        .await?
    {
        authorize()?;
        sqlx::query(DB::BEGIN_CODE_MIGRATION)
            .execute(&mut conn)
            .await?;
        let result: sqlx::Result<()> = async {
            match operation.as_ref() {
                "backfill_post_media_references" => {
                    media_references::backfill_post_media_references::<DB>(&mut conn).await?;
                }
                "rebuild_rendered_posts" => {
                    rendered_posts::rebuild_rendered_posts::<DB>(&mut conn).await?;
                }
                "normalize_comparable_instants_v1" => {
                    DB::normalize_comparable_instants(&mut conn).await?;
                }
                other => {
                    return Err(sqlx::Error::Protocol(format!(
                        "unknown offline code migration operation: {other}"
                    )));
                }
            }
            sqlx::query("DELETE FROM pending_code_migrations WHERE queue_id = $1")
                .bind_storage(queue_id)
                .execute(&mut conn)
                .await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                #[cfg(test)]
                pause_commit(false).await;
                if let Err(error) = sqlx::query("COMMIT").execute(&mut conn).await {
                    return helpers::preserve_after_secondary(
                        Err(error),
                        conn.close().await,
                        host::error::ErrorKind::Storage,
                        host::error::ErrorClass::Transient,
                        "storage.code_migrations.commit_close",
                    );
                }
                #[cfg(test)]
                pause_commit(true).await;
            }
            Err(error) => {
                return helpers::preserve_after_secondary(
                    Err(error),
                    sqlx::query("ROLLBACK").execute(&mut conn).await.map(|_| ()),
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "storage.code_migrations.rollback",
                );
            }
        }
    }
    conn.close().await
}
