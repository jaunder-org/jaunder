use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Row, Sqlite};

use crate::feed_events::{
    parse_status, FeedEventDialect, FeedEventError, FeedEventRecord, FeedEventStore,
};

/// SQLite-backed feed-event storage.
pub type SqliteFeedEventStorage = FeedEventStore<Sqlite>;

fn placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

#[async_trait]
impl FeedEventDialect for Sqlite {
    async fn claim_pending_batch(
        pool: &Pool<Sqlite>,
        now: DateTime<Utc>,
        lease_cutoff: DateTime<Utc>,
        limit_i: i64,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        // Single autocommit statement: SQLite takes the write lock immediately,
        // so there is no deferred read-then-write lock upgrade (ADR-0021) and the
        // 5s busy_timeout applies cleanly. Mirrors the Postgres CTE claim.
        let rows = sqlx::query(
            "UPDATE feed_events SET status = 'claimed', claimed_at = $1 \
             WHERE id IN ( \
                 SELECT id FROM feed_events \
                 WHERE (status = 'pending' AND next_attempt_at <= $2) \
                    OR (status = 'claimed' AND claimed_at < $3) \
                 ORDER BY next_attempt_at ASC \
                 LIMIT $4 \
             ) \
             RETURNING id, feed_url, status, attempts, last_error, next_attempt_at, claimed_at, \
                       created_at, regenerated_at, pinged_at",
        )
        .bind(now)
        .bind(now)
        .bind(lease_cutoff)
        .bind(limit_i)
        .fetch_all(pool)
        .await?;

        let records = rows
            .into_iter()
            .map(|r| {
                let attempts: i64 = r.get("attempts");
                FeedEventRecord {
                    id: r.get("id"),
                    feed_url: r.get("feed_url"),
                    status: parse_status(r.get::<&str, _>("status")),
                    attempts: i32::try_from(attempts).unwrap_or(i32::MAX),
                    last_error: r.get("last_error"),
                    next_attempt_at: r.get("next_attempt_at"),
                    claimed_at: r.get("claimed_at"),
                    created_at: r.get("created_at"),
                    regenerated_at: r.get("regenerated_at"),
                    pinged_at: r.get("pinged_at"),
                }
            })
            .collect();
        Ok(records)
    }

    async fn mark_regenerated(pool: &Pool<Sqlite>, ids: &[i64]) -> Result<(), FeedEventError> {
        let now = Utc::now();
        let ph = placeholders(ids.len());
        let sql = format!("UPDATE feed_events SET regenerated_at = ? WHERE id IN ({ph})");
        let mut q = sqlx::query(&sql).bind(now);
        for id in ids {
            q = q.bind(*id);
        }
        q.execute(pool).await?;
        Ok(())
    }

    async fn mark_pinged(pool: &Pool<Sqlite>, ids: &[i64]) -> Result<(), FeedEventError> {
        let now = Utc::now();
        let ph = placeholders(ids.len());
        let sql =
            format!("UPDATE feed_events SET status = 'done', pinged_at = ? WHERE id IN ({ph})");
        let mut q = sqlx::query(&sql).bind(now);
        for id in ids {
            q = q.bind(*id);
        }
        q.execute(pool).await?;
        Ok(())
    }

    async fn mark_failed(
        pool: &Pool<Sqlite>,
        ids: &[i64],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events \
             SET status = 'pending', attempts = attempts + 1, last_error = ?, next_attempt_at = ?, claimed_at = NULL \
             WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(&sql).bind(error).bind(next_attempt_at);
        for id in ids {
            q = q.bind(*id);
        }
        q.execute(pool).await?;
        Ok(())
    }

    async fn mark_exhausted(
        pool: &Pool<Sqlite>,
        ids: &[i64],
        error: &str,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql =
            format!("UPDATE feed_events SET status = 'failed', last_error = ? WHERE id IN ({ph})");
        let mut q = sqlx::query(&sql).bind(error);
        for id in ids {
            q = q.bind(*id);
        }
        q.execute(pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    use super::*;
    use crate::feed_events::FeedEventStatus;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn enqueue_creates_pending_row() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        let id = s.enqueue("/feed.rss").await.unwrap();
        assert!(id > 0);
    }

    #[tokio::test]
    async fn claim_returns_eligible_pending_row() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        s.enqueue("/feed.rss").await.unwrap();
        let claimed = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].status, FeedEventStatus::Claimed);
        assert!(claimed[0].claimed_at.is_some());
    }

    #[tokio::test]
    async fn double_claim_returns_no_rows_within_lease() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        s.enqueue("/feed.rss").await.unwrap();
        let first = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        let second = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 0);
    }

    #[tokio::test]
    async fn lease_expired_rows_are_reclaimable() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        s.enqueue("/feed.rss").await.unwrap();
        let _first = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        // With a zero lease, the just-claimed row is immediately re-eligible.
        let second = s
            .claim_pending_batch(10, chrono::Duration::zero())
            .await
            .unwrap();
        assert_eq!(second.len(), 1);
    }

    #[tokio::test]
    async fn mark_pinged_marks_done_and_removes_from_queue() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        s.enqueue("/feed.rss").await.unwrap();
        let claimed = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        let ids: Vec<i64> = claimed.iter().map(|r| r.id).collect();
        s.mark_regenerated(&ids).await.unwrap();
        s.mark_pinged(&ids).await.unwrap();
        let next = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert!(next.is_empty());
    }

    #[tokio::test]
    async fn mark_failed_increments_attempts_and_reschedules() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        let id = s.enqueue("/feed.rss").await.unwrap();
        let _ = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        let future = Utc::now() + chrono::Duration::minutes(1);
        s.mark_failed(&[id], "boom", future).await.unwrap();
        // Not eligible until `future`.
        let now = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert!(now.is_empty());
    }

    #[tokio::test]
    async fn mark_exhausted_marks_failed_terminal() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        let id = s.enqueue("/feed.rss").await.unwrap();
        s.mark_exhausted(&[id], "gave up").await.unwrap();
        // Failed rows are never eligible.
        let next = s
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert!(next.is_empty());
    }

    #[tokio::test]
    async fn empty_id_arrays_are_noops() {
        use crate::feed_events::FeedEventStorage;
        let s = SqliteFeedEventStorage::new(pool().await);
        s.mark_regenerated(&[]).await.unwrap();
        s.mark_pinged(&[]).await.unwrap();
        s.mark_failed(&[], "x", Utc::now()).await.unwrap();
        s.mark_exhausted(&[], "x").await.unwrap();
    }
}
