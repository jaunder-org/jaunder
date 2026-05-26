use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Row};

use crate::feed_events::{FeedEventError, FeedEventRecord, FeedEventStatus, FeedEventStorage};

pub struct PostgresFeedEventStorage {
    pool: PgPool,
}

impl PostgresFeedEventStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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

#[async_trait]
impl FeedEventStorage for PostgresFeedEventStorage {
    #[tracing::instrument(name = "storage.postgres.feed_events.enqueue", skip(self))]
    async fn enqueue(&self, feed_url: &str) -> Result<i64, FeedEventError> {
        let id: i64 =
            sqlx::query_scalar("INSERT INTO feed_events (feed_url) VALUES ($1) RETURNING id")
                .bind(feed_url)
                .fetch_one(&self.pool)
                .await?;
        Ok(id)
    }

    #[tracing::instrument(name = "storage.postgres.feed_events.claim_pending_batch", skip(self))]
    async fn claim_pending_batch(
        &self,
        limit: usize,
        lease_timeout: Duration,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        let now = Utc::now();
        let lease_cutoff = now - lease_timeout;
        let limit_i = i64::try_from(limit).unwrap_or(i64::MAX);

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
        .fetch_all(&self.pool)
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

    #[tracing::instrument(name = "storage.postgres.feed_events.mark_regenerated", skip(self))]
    async fn mark_regenerated(&self, ids: &[i64]) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query("UPDATE feed_events SET regenerated_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(ids)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(name = "storage.postgres.feed_events.mark_pinged", skip(self))]
    async fn mark_pinged(&self, ids: &[i64]) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query("UPDATE feed_events SET status = 'done', pinged_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(ids)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(name = "storage.postgres.feed_events.mark_failed", skip(self))]
    async fn mark_failed(
        &self,
        ids: &[i64],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE feed_events \
             SET status = 'pending', attempts = attempts + 1, \
                 last_error = $1, next_attempt_at = $2, claimed_at = NULL \
             WHERE id = ANY($3)",
        )
        .bind(error)
        .bind(next_attempt_at)
        .bind(ids)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(name = "storage.postgres.feed_events.mark_exhausted", skip(self))]
    async fn mark_exhausted(&self, ids: &[i64], error: &str) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query("UPDATE feed_events SET status = 'failed', last_error = $1 WHERE id = ANY($2)")
            .bind(error)
            .bind(ids)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
