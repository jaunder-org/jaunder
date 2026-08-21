use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::ids::FeedEventId;
use sqlx::{Pool, Postgres};

use crate::feed_events::{
    ClaimedRow, FeedEventDialect, FeedEventError, FeedEventRecord, FeedEventStore,
    partition_claimed,
};

/// Postgres-backed feed-event storage.
pub type PostgresFeedEventStorage = FeedEventStore<Postgres>;

fn finish_purge(
    primary: Vec<FeedEventRecord>,
    purge: Result<(), sqlx::Error>,
) -> Vec<FeedEventRecord> {
    crate::feed_events::finish_corrupt_purge(
        primary,
        purge,
        "storage.postgres.feed_events.purge_corrupt",
    )
}

/// Deletes claimed rows whose `feed_url` cannot decode. Partitioning reports
/// the aggregate decode failure; only a failed cleanup is reported here.
async fn purge_corrupt(pool: &Pool<Postgres>, ids: &[FeedEventId]) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM feed_events WHERE id = ANY($1)")
        .bind(ids)
        .execute(pool)
        .await?;
    Ok(())
}

#[async_trait]
impl FeedEventDialect for Postgres {
    async fn claim_pending_batch(
        pool: &Pool<Postgres>,
        now: DateTime<Utc>,
        lease_cutoff: DateTime<Utc>,
        limit_i: i64,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        // Postgres can express the whole claim atomically with FOR UPDATE
        // SKIP LOCKED + UPDATE … RETURNING in a single statement.
        let rows = sqlx::query_as::<_, ClaimedRow>(
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
             RETURNING id, feed_url, status, attempts, last_error, next_attempt_at, claimed_at, \
                       created_at, regenerated_at, pinged_at",
        )
        .bind(now)
        .bind(lease_cutoff)
        .bind(limit_i)
        .fetch_all(pool)
        .await?;

        let (records, corrupt) = partition_claimed(rows);
        let purge = purge_corrupt(pool, &corrupt).await;
        Ok(finish_purge(records, purge))
    }

    async fn claimable_count(
        pool: &Pool<Postgres>,
        now: DateTime<Utc>,
        lease_cutoff: DateTime<Utc>,
    ) -> Result<u64, FeedEventError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM feed_events \
             WHERE (status = 'pending' AND next_attempt_at <= $1) \
                OR (status = 'claimed' AND claimed_at < $2)",
        )
        .bind(now)
        .bind(lease_cutoff)
        .fetch_one(pool)
        .await?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    async fn mark_regenerated(
        pool: &Pool<Postgres>,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError> {
        let now = Utc::now();
        sqlx::query("UPDATE feed_events SET regenerated_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(ids)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn mark_pinged(pool: &Pool<Postgres>, ids: &[FeedEventId]) -> Result<(), FeedEventError> {
        let now = Utc::now();
        sqlx::query("UPDATE feed_events SET status = 'done', pinged_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(ids)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn mark_failed(
        pool: &Pool<Postgres>,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError> {
        sqlx::query(
            "UPDATE feed_events \
             SET status = 'pending', attempts = attempts + 1, \
                 last_error = $1, next_attempt_at = $2, claimed_at = NULL \
             WHERE id = ANY($3)",
        )
        .bind(error)
        .bind(next_attempt_at)
        .bind(ids)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn mark_exhausted(
        pool: &Pool<Postgres>,
        ids: &[FeedEventId],
        error: &str,
    ) -> Result<(), FeedEventError> {
        sqlx::query("UPDATE feed_events SET status = 'failed', last_error = $1 WHERE id = ANY($2)")
            .bind(error)
            .bind(ids)
            .execute(pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_reporting_corrupt_purge_failure_preserves_valid_batch_and_reports_once() {
        let now = Utc::now();
        let valid = vec![FeedEventRecord {
            id: FeedEventId::from(17),
            feed_path: "/feed.rss".parse().expect("valid feed path"),
            status: common::feed::FeedEventStatus::Claimed,
            attempts: 0,
            last_error: None,
            next_attempt_at: now,
            claimed_at: Some(now),
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
