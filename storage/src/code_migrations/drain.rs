//! Queue dispatch and per-operation transaction boundary shared by both backends.

use sqlx::{Encode, Executor, FromRow, Pool, Type};

use super::media_references;
use super::types::{CodeMigrationOperation, CodeMigrationQueueId};
use crate::posts::models::RenderedHtml;
use crate::sql::QueryStorageExt;
use crate::{PostDialect, helpers};
use common::ids::PostId;

/// `SQLite` starts an immediate writer; `PostgreSQL`'s regular transaction is
/// sufficient under the same-directory offline lock and runtime exclusion.
pub(crate) trait CodeMigrationDialect: PostDialect {
    const BEGIN_CODE_MIGRATION: &'static str;
}

impl CodeMigrationDialect for sqlx::Sqlite {
    const BEGIN_CODE_MIGRATION: &'static str = "BEGIN IMMEDIATE";
}

impl CodeMigrationDialect for sqlx::Postgres {
    const BEGIN_CODE_MIGRATION: &'static str = "BEGIN";
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
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let mut conn = pool.acquire().await?;
    while let Some((queue_id, operation)) =
        sqlx::query_as::<DB, (CodeMigrationQueueId, CodeMigrationOperation)>(
            "SELECT queue_id, operation FROM pending_code_migrations ORDER BY queue_id LIMIT 1",
        )
        .fetch_optional(&mut *conn)
        .await?
    {
        authorize()?;
        sqlx::query(DB::BEGIN_CODE_MIGRATION)
            .execute(&mut *conn)
            .await?;
        let result: sqlx::Result<()> = async {
            match operation.as_ref() {
                "backfill_post_media_references" => {
                    media_references::backfill_post_media_references::<DB>(&mut *conn).await?;
                }
                other => {
                    return Err(sqlx::Error::Protocol(format!(
                        "unknown offline code migration operation: {other}"
                    )));
                }
            }
            sqlx::query("DELETE FROM pending_code_migrations WHERE queue_id = $1")
                .bind_storage(queue_id)
                .execute(&mut *conn)
                .await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
            }
            Err(error) => {
                return helpers::preserve_after_secondary(
                    Err(error),
                    sqlx::query("ROLLBACK")
                        .execute(&mut *conn)
                        .await
                        .map(|_| ()),
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "storage.code_migrations.rollback",
                );
            }
        }
    }
    Ok(())
}
