use async_trait::async_trait;

use common::ids::FeedEventId;
use common::pagination::RowLimit;
use common::time::UtcInstant;
use host::feed::{FeedEventClaimLimit, FeedEventPhase};
use sqlx::{AssertSqlSafe, Error, Pool, Sqlite, SqliteConnection};

use crate::feed_events::{
    self, ClaimedFeedEventRow, ClaimedRow, DeadLetterRow, FeedEventDeadLetterCursor,
    FeedEventDeadLetterError, FeedEventDialect, FeedEventError, FeedEventRecord,
    FeedEventRedriveError, FeedEventStore, StoredFeedDiagnostic,
};

use crate::sql::QueryStorageExt;
use crate::sql::RowCount;
/// SQLite-backed feed-event storage.
pub type SqliteFeedEventStorage = FeedEventStore<Sqlite>;

// Every caller interpolates only this locally generated sequence of `?`
// placeholders; event data is bound through `bind_storage` below.
fn placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

fn finish_purge(primary: Vec<FeedEventRecord>, purge: Result<(), Error>) -> Vec<FeedEventRecord> {
    feed_events::finish_corrupt_purge(primary, purge, "storage.sqlite.feed_events.purge_corrupt")
}

/// Deletes claimed rows whose `feed_url` cannot decode. Partitioning reports
/// the aggregate decode failure; only a failed cleanup is reported here.
async fn purge_corrupt(
    connection: &mut SqliteConnection,
    ids: &[FeedEventId],
) -> Result<(), Error> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query("SAVEPOINT feed_event_purge")
        .execute(&mut *connection)
        .await?;
    let ph = placeholders(ids.len());
    let sql = format!("DELETE FROM feed_events WHERE id IN ({ph})");
    let mut query = sqlx::query(AssertSqlSafe(sql));
    for id in ids {
        query = query.bind_storage(*id);
    }
    let result = query.execute(&mut *connection).await;
    if let Err(error) = result {
        sqlx::query("ROLLBACK TO SAVEPOINT feed_event_purge")
            .execute(&mut *connection)
            .await?;
        sqlx::query("RELEASE SAVEPOINT feed_event_purge")
            .execute(&mut *connection)
            .await?;
        return Err(error);
    }
    sqlx::query("RELEASE SAVEPOINT feed_event_purge")
        .execute(&mut *connection)
        .await?;
    Ok(())
}

#[async_trait]
impl FeedEventDialect for Sqlite {
    async fn claim_pending_batch(
        connection: &mut SqliteConnection,
        now: UtcInstant,
        lease_cutoff: UtcInstant,
        limit: FeedEventClaimLimit,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        let rows = sqlx::query_as::<_, ClaimedFeedEventRow>(
            "UPDATE feed_events SET status = 'claimed', claimed_at = $1 \
             WHERE id IN ( \
                 SELECT id FROM feed_events \
                 WHERE (status = 'pending' AND next_attempt_at <= $2) \
                    OR (status = 'claimed' AND claimed_at < $3) \
                 ORDER BY next_attempt_at ASC \
                 LIMIT $4 \
             ) \
             RETURNING id, feed_url, status, phase, regeneration_attempts, publication_attempts, \
                       regeneration_diagnostic, publication_diagnostic, next_attempt_at, claimed_at, terminal_at, \
                       created_at, regenerated_at, pinged_at",
        )
        .bind_storage(now)
        .bind_storage(now)
        .bind_storage(lease_cutoff)
        .bind_storage(limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(ClaimedRow::from)
        .collect();

        let (records, corrupt) = feed_events::partition_claimed(rows);
        let purge = purge_corrupt(connection, &corrupt).await;
        Ok(finish_purge(records, purge))
    }

    async fn claimable_count(
        pool: &Pool<Sqlite>,
        now: UtcInstant,
        lease_cutoff: UtcInstant,
    ) -> Result<u64, FeedEventError> {
        let count = sqlx::query_scalar::<_, RowCount>(
            "SELECT COUNT(*) FROM feed_events \
             WHERE (status = 'pending' AND next_attempt_at <= $1) \
                OR (status = 'claimed' AND claimed_at < $2)",
        )
        .bind_storage(now)
        .bind_storage(lease_cutoff)
        .fetch_one(pool)
        .await?;
        Ok(count.into_u64())
    }

    async fn dead_letters(
        pool: &Pool<Sqlite>,
        phase: FeedEventPhase,
        cursor: Option<FeedEventDeadLetterCursor>,
        limit: RowLimit,
    ) -> Result<Vec<DeadLetterRow>, FeedEventDeadLetterError> {
        let (terminal_at, id) = cursor.map_or((None, None), |cursor| {
            (Some(cursor.terminal_at), Some(cursor.id))
        });
        Ok(sqlx::query_as(
            "SELECT id, feed_url, phase, \
                    CASE WHEN phase = 'regeneration' THEN regeneration_attempts ELSE publication_attempts END AS attempts, \
                    terminal_at, \
                    CASE WHEN phase = 'regeneration' THEN regeneration_diagnostic ELSE publication_diagnostic END AS diagnostic \
             FROM feed_events \
             WHERE status = 'failed' AND phase = $1 \
               AND ($2 IS NULL OR terminal_at < $2 OR (terminal_at = $2 AND id < $3)) \
             ORDER BY terminal_at DESC, id DESC LIMIT $4",
        )
        .bind_storage(phase)
        .bind_storage(terminal_at)
        .bind_storage(id)
        .bind_storage(limit)
        .fetch_all(pool)
        .await?)
    }

    async fn redrive_dead_letters(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        now: UtcInstant,
        failed_cutoff: UtcInstant,
    ) -> Result<bool, FeedEventRedriveError> {
        // The placeholder list is generated solely from the slice length; all
        // event IDs and timestamps remain bound values.
        let ph = placeholders(ids.len());
        let count_sql = format!(
            "SELECT COUNT(*) FROM feed_events \
             WHERE status = 'failed' AND terminal_at > ? AND id IN ({ph})"
        );
        let mut count =
            sqlx::query_scalar::<_, RowCount>(AssertSqlSafe(count_sql)).bind_storage(failed_cutoff);
        for id in ids {
            count = count.bind_storage(*id);
        }
        if count.fetch_one(&mut *connection).await?.into_u64() != ids.len() as u64 {
            return Ok(false);
        }
        let sql = format!(
            "UPDATE feed_events SET status = 'pending', \
             regeneration_attempts = CASE WHEN phase = 'regeneration' THEN 0 ELSE regeneration_attempts END, \
             publication_attempts = CASE WHEN phase = 'publication' THEN 0 ELSE publication_attempts END, \
             regeneration_diagnostic = CASE WHEN phase = 'regeneration' THEN NULL ELSE regeneration_diagnostic END, \
             publication_diagnostic = CASE WHEN phase = 'publication' THEN NULL ELSE publication_diagnostic END, \
             terminal_at = NULL, claimed_at = NULL, next_attempt_at = ? WHERE id IN ({ph})"
        );
        let mut update = sqlx::query(AssertSqlSafe(sql)).bind_storage(now);
        for id in ids {
            update = update.bind_storage(*id);
        }
        let result = update.execute(&mut *connection).await?;
        Ok(result.rows_affected() == ids.len() as u64)
    }

    async fn mark_regenerated(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError> {
        let now = UtcInstant::now();
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET regenerated_at = ?, phase = 'publication' WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql)).bind_storage(now);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn mark_pinged(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'done', pinged_at = ?, terminal_at = ? WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql))
            .bind_storage(now)
            .bind_storage(now);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn retry_regeneration(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'pending', phase = 'regeneration', \
             regeneration_attempts = regeneration_attempts + 1, regeneration_diagnostic = ?, \
             terminal_at = NULL, claimed_at = NULL, next_attempt_at = ? WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql))
            .bind_storage(error)
            .bind_storage(next_attempt_at);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn dead_letter_regeneration(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'failed', phase = 'regeneration', \
             regeneration_attempts = regeneration_attempts + 1, regeneration_diagnostic = ?, \
             terminal_at = ?, claimed_at = NULL WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql))
            .bind_storage(error)
            .bind_storage(now);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn retry_publication(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'pending', phase = 'publication', \
             publication_attempts = publication_attempts + 1, publication_diagnostic = ?, \
             terminal_at = NULL, claimed_at = NULL, next_attempt_at = ? WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql))
            .bind_storage(error)
            .bind_storage(next_attempt_at);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn dead_letter_publication(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'failed', phase = 'publication', \
             publication_attempts = publication_attempts + 1, publication_diagnostic = ?, \
             terminal_at = ?, claimed_at = NULL WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql))
            .bind_storage(error)
            .bind_storage(now);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn restart_regeneration(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'pending', phase = 'regeneration', \
             regeneration_attempts = 0, regeneration_diagnostic = NULL, terminal_at = NULL, \
             claimed_at = NULL, next_attempt_at = ? WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql)).bind_storage(now);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn reset_regeneration(
        connection: &mut SqliteConnection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'pending', phase = 'regeneration', \
             regeneration_attempts = 0, regeneration_diagnostic = NULL, terminal_at = NULL, \
             claimed_at = NULL, next_attempt_at = ? WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql)).bind_storage(now);
        for id in ids {
            q = q.bind_storage(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    } // cov:ignore: LLVM leaves this async helper's closing edge unmarked after shared success and database-error behavior.
    async fn prune_terminal_events(
        pool: &Pool<Sqlite>,
        now: UtcInstant,
        failed_cutoff: UtcInstant,
        limit: RowLimit,
    ) -> Result<u64, FeedEventError> {
        let result = sqlx::query(
            "DELETE FROM feed_events WHERE id IN ( \
                SELECT id FROM feed_events \
                WHERE (status = 'done' AND terminal_at <= $1) \
                   OR (status = 'failed' AND terminal_at <= $2) \
                ORDER BY terminal_at ASC \
                LIMIT $3 \
             )",
        )
        .bind_storage(now)
        .bind_storage(failed_cutoff)
        .bind_storage(limit)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::finish_purge;
    use crate::FeedEventRecord;
    use crate::test_support::fp;
    use common::{ids::FeedEventId, time::UtcInstant};
    use host::feed::FeedEventStatus;

    #[test]
    fn continuation_reporting_corrupt_purge_failure_preserves_valid_batch_and_reports_once() {
        let now = UtcInstant::now();
        let valid = vec![FeedEventRecord {
            id: FeedEventId::from(17),
            feed_path: fp("/feed.rss"),
            status: FeedEventStatus::Claimed,
            phase: host::feed::FeedEventPhase::Regeneration,
            regeneration_attempts: 0,
            publication_attempts: 0,
            regeneration_diagnostic: None,
            publication_diagnostic: None,
            next_attempt_at: now,
            claimed_at: Some(now),
            terminal_at: None,
            created_at: now,
            regenerated_at: None,
            pinged_at: None,
        }];
        let (records, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_purge(valid.clone(), Err(sqlx::Error::PoolClosed))
        });
        assert_eq!(records, valid);
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.feed_events.purge_corrupt",
        );
    }

}
