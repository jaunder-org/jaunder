use async_trait::async_trait;

use common::ids::FeedEventId;
use common::pagination::RowLimit;
use common::time::UtcInstant;
use host::feed::FeedEventClaimLimit;
use sqlx::{Pool, Postgres};

use crate::feed_events::{
    self, ClaimedFeedEventRow, ClaimedRow, FeedEventDialect, FeedEventError, FeedEventRecord,
    FeedEventStore, StoredFeedDiagnostic,
};
use crate::sql::QueryStorageExt;
use crate::sql::RowCount;

/// Postgres-backed feed-event storage.
pub type PostgresFeedEventStorage = FeedEventStore<Postgres>;

fn finish_purge(
    primary: Vec<FeedEventRecord>,
    purge: Result<(), sqlx::Error>,
) -> Vec<FeedEventRecord> {
    feed_events::finish_corrupt_purge(primary, purge, "storage.postgres.feed_events.purge_corrupt")
}

/// Deletes claimed rows whose `feed_url` cannot decode. Partitioning reports
/// the aggregate decode failure; only a failed cleanup is reported here.
async fn purge_corrupt(
    connection: &mut sqlx::PgConnection,
    ids: &[FeedEventId],
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query("SAVEPOINT feed_event_purge")
        .execute(&mut *connection)
        .await?;
    let result = sqlx::query("DELETE FROM feed_events WHERE id = ANY($1)")
        .bind_storage(ids)
        .execute(&mut *connection)
        .await;
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
impl FeedEventDialect for Postgres {
    async fn claim_pending_batch(
        connection: &mut sqlx::PgConnection,
        now: UtcInstant,
        lease_cutoff: UtcInstant,
        limit: FeedEventClaimLimit,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        let rows = sqlx::query_as::<_, ClaimedFeedEventRow>(
            "WITH eligible AS ( \
                SELECT id FROM feed_events \
                WHERE (status = 'pending' AND next_attempt_at <= $1) \
                   OR (status = 'claimed' AND claimed_at < $2) \
                ORDER BY next_attempt_at ASC \
                LIMIT $3 \
                FOR UPDATE SKIP LOCKED \
             ) \
             UPDATE feed_events SET status = 'claimed', claimed_at = $1 \
             WHERE id IN (SELECT id FROM eligible) \
             RETURNING id, feed_url, status, attempts, last_error, next_attempt_at, claimed_at, terminal_at, \
                       created_at, regenerated_at, pinged_at",
        )
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
        pool: &Pool<Postgres>,
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

    async fn mark_regenerated(
        connection: &mut sqlx::PgConnection,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError> {
        let now = UtcInstant::now();
        sqlx::query("UPDATE feed_events SET regenerated_at = $1 WHERE id = ANY($2)")
            .bind_storage(now)
            .bind_storage(ids)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn mark_pinged(
        connection: &mut sqlx::PgConnection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        sqlx::query(
            "UPDATE feed_events SET status = 'done', pinged_at = $1, terminal_at = $1 WHERE id = ANY($2)",
        )
        .bind_storage(now)
        .bind_storage(ids)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    async fn mark_failed(
        connection: &mut sqlx::PgConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError> {
        sqlx::query(
            "UPDATE feed_events \
             SET status = 'pending', attempts = attempts + 1, \
                 last_error = $1, next_attempt_at = $2, claimed_at = NULL \
             WHERE id = ANY($3)",
        )
        .bind_storage(error)
        .bind_storage(next_attempt_at)
        .bind_storage(ids)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    async fn mark_exhausted(
        connection: &mut sqlx::PgConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        sqlx::query(
            "UPDATE feed_events SET status = 'failed', last_error = $1, terminal_at = $2 WHERE id = ANY($3)",
        )
        .bind_storage(error)
        .bind_storage(now)
        .bind_storage(ids)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }
    async fn prune_terminal_events(
        pool: &Pool<Postgres>,
        now: UtcInstant,
        failed_cutoff: UtcInstant,
        limit: RowLimit,
    ) -> Result<u64, FeedEventError> {
        let result = sqlx::query(
            "WITH eligible AS ( \
                SELECT id FROM feed_events \
                WHERE (status = 'done' AND terminal_at <= $1) \
                   OR (status = 'failed' AND terminal_at <= $2) \
                ORDER BY terminal_at ASC \
                LIMIT $3 \
             ) \
             DELETE FROM feed_events WHERE id IN (SELECT id FROM eligible)",
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
    use super::*;

    #[test]
    fn continuation_reporting_corrupt_purge_failure_preserves_valid_batch_and_reports_once() {
        let now = UtcInstant::now();
        let valid = vec![FeedEventRecord {
            id: FeedEventId::from(17),
            feed_path: "/feed.rss".parse().expect("valid feed path"),
            status: host::feed::FeedEventStatus::Claimed,
            attempts: 0,
            last_error: None,
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
            "storage.postgres.feed_events.purge_corrupt",
        );
    }
}
