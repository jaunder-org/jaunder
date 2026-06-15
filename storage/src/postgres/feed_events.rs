use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Postgres, Row};

use crate::feed_events::{
    FeedEventDialect, FeedEventError, FeedEventRecord, FeedEventStatus, FeedEventStore,
};

/// Postgres-backed feed-event storage.
pub type PostgresFeedEventStorage = FeedEventStore<Postgres>;

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
        let rows = sqlx::query(
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

        let records = rows
            .into_iter()
            .map(|r| FeedEventRecord {
                id: r.get("id"),
                feed_url: r.get("feed_url"),
                status: parse_status(r.get::<&str, _>("status")),
                attempts: r.get("attempts"),
                last_error: r.get("last_error"),
                next_attempt_at: r.get("next_attempt_at"),
                claimed_at: r.get("claimed_at"),
                created_at: r.get("created_at"),
                regenerated_at: r.get("regenerated_at"),
                pinged_at: r.get("pinged_at"),
            })
            .collect();
        Ok(records)
    }

    async fn mark_regenerated(pool: &Pool<Postgres>, ids: &[i64]) -> Result<(), FeedEventError> {
        let now = Utc::now();
        sqlx::query("UPDATE feed_events SET regenerated_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(ids)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn mark_pinged(pool: &Pool<Postgres>, ids: &[i64]) -> Result<(), FeedEventError> {
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
        ids: &[i64],
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
        ids: &[i64],
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

fn parse_status(s: &str) -> FeedEventStatus {
    match s {
        "pending" => FeedEventStatus::Pending,
        "claimed" => FeedEventStatus::Claimed,
        "done" => FeedEventStatus::Done,
        _ => FeedEventStatus::Failed,
    }
}
