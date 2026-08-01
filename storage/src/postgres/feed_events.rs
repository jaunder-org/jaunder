use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::feed::FeedPath;
use common::ids::FeedEventId;
use sqlx::{Pool, Postgres, Row};

use crate::feed_events::{
    FeedEventDialect, FeedEventError, FeedEventRecord, FeedEventStatus, FeedEventStore,
};

/// Postgres-backed feed-event storage.
pub type PostgresFeedEventStorage = FeedEventStore<Postgres>;

/// Deletes `feed_events` rows whose `feed_url` could not be parsed into a
/// [`FeedPath`] — only reachable via DB tampering/corruption, since `enqueue`
/// takes a validated `FeedPath`. Such a row names no identifiable feed, so it is
/// purged to keep the worker draining rather than wedged on it forever; any real
/// regeneration need re-enqueues via the write/go-live path. Best-effort: a
/// delete failure is logged, never propagated — the corrupt ids are already
/// excluded from the returned batch, so the worker proceeds regardless.
async fn purge_corrupt(pool: &Pool<Postgres>, ids: &[FeedEventId]) {
    if ids.is_empty() {
        return;
    }
    tracing::warn!("feed_events: purging rows with an unparseable feed_url");
    if let Err(e) = sqlx::query("DELETE FROM feed_events WHERE id = ANY($1)")
        .bind(raw_ids(ids))
        .execute(pool)
        .await
    {
        // cov:ignore-start — defensive log; the delete is best-effort and the
        // corrupt ids are already excluded from the batch, so this never blocks.
        tracing::warn!(error = %e, "feed_events: purge of corrupt rows failed");
        // cov:ignore-stop
    }
}

/// Unwrap an id batch to the raw `i64` slice Postgres binds as a single
/// `= ANY($n)` array parameter — the sqlx seam takes `&[i64]`, not the newtype.
///
/// Forced, not chosen: the ADR-0071 bridge emits `Type`/`Encode`/`Decode` but no
/// `PgHasArrayType`, so `.bind(&[FeedEventId])` has no array encoding. #715 typed
/// `purge_corrupt`'s parameter and routed it through here, which added a fourth
/// caller — so this helper now launders *more* strips past both gates, not fewer.
/// That is #716's shape (a strip in one function, the bind in another), and closing
/// it means giving `IdNewtype` a `PgHasArrayType` rather than deleting this helper.
fn raw_ids(ids: &[FeedEventId]) -> Vec<i64> {
    ids.iter().map(|id| i64::from(*id)).collect()
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

        let mut records = Vec::with_capacity(rows.len());
        let mut corrupt = Vec::new();
        for r in rows {
            let id: FeedEventId = r.get("id");
            // A feed_url that won't parse can only come from DB tampering/corruption
            // (enqueue takes a validated FeedPath). Such a row is an unactionable
            // work item, so collect it for purge rather than failing the whole batch
            // (which would wedge the worker on the corrupt row forever).
            let Ok(feed_path) = r.try_get::<FeedPath, _>("feed_url") else {
                corrupt.push(id);
                continue;
            };
            records.push(FeedEventRecord {
                id,
                feed_path,
                status: r.get::<FeedEventStatus, _>("status"),
                attempts: r.get("attempts"),
                last_error: r.get("last_error"),
                next_attempt_at: r.get("next_attempt_at"),
                claimed_at: r.get("claimed_at"),
                created_at: r.get("created_at"),
                regenerated_at: r.get("regenerated_at"),
                pinged_at: r.get("pinged_at"),
            });
        }
        purge_corrupt(pool, &corrupt).await;
        Ok(records)
    }

    async fn mark_regenerated(
        pool: &Pool<Postgres>,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError> {
        let now = Utc::now();
        let raw = raw_ids(ids);
        sqlx::query("UPDATE feed_events SET regenerated_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(&raw)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn mark_pinged(pool: &Pool<Postgres>, ids: &[FeedEventId]) -> Result<(), FeedEventError> {
        let now = Utc::now();
        let raw = raw_ids(ids);
        sqlx::query("UPDATE feed_events SET status = 'done', pinged_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(&raw)
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
        let raw = raw_ids(ids);
        sqlx::query(
            "UPDATE feed_events \
             SET status = 'pending', attempts = attempts + 1, \
                 last_error = $1, next_attempt_at = $2, claimed_at = NULL \
             WHERE id = ANY($3)",
        )
        .bind(error)
        .bind(next_attempt_at)
        .bind(&raw)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn mark_exhausted(
        pool: &Pool<Postgres>,
        ids: &[FeedEventId],
        error: &str,
    ) -> Result<(), FeedEventError> {
        let raw = raw_ids(ids);
        sqlx::query("UPDATE feed_events SET status = 'failed', last_error = $1 WHERE id = ANY($2)")
            .bind(error)
            .bind(&raw)
            .execute(pool)
            .await?;
        Ok(())
    }
}
