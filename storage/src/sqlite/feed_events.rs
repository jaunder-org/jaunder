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

// Reproduction harness for issue #18: the SQLite claim_pending_batch lock
// flake. With the old SELECT->UPDATE->SELECT deferred transaction, concurrent
// claimers upgrade a shared lock to a reserved lock against a stale snapshot
// and SQLite returns "database is locked" (busy_timeout cannot rescue an
// upgrade). With the single-statement UPDATE ... RETURNING (ADR-0021) the
// writes serialize cleanly under busy_timeout.
//
// Timing-based, so it is #[ignore]d -- excluded from CI to avoid being a
// flake source itself. Run on demand:
//   cargo nextest run -p storage -- --ignored claim_pending_batch_no_lock_contention
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::test_support::{sqlite_only, Backend};
    use crate::FeedEventError;
    use chrono::Duration;

    use rstest::*;
    use rstest_reuse::*;

    #[apply(sqlite_only)]
    // reason: reproduces the SQLite-specific issue #18 claim_pending_batch lock flake
    // (reserved-lock upgrade under busy_timeout); Postgres MVCC cannot exhibit it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "timing-based #18 reproduction; run manually with --ignored"]
    async fn claim_pending_batch_no_lock_contention(#[case] backend: Backend) {
        // cov:ignore-start — #[ignore]d manual #18 repro; its body never runs in the
        // automated coverage suite, so these lines are accepted-uncovered.
        let env = backend.setup().await;
        let feed_events = env.state.feed_events.clone();

        // Seed a populated queue.
        for i in 0..200 {
            feed_events
                .enqueue(&format!("/feed-{i}.rss"))
                .await
                .expect("enqueue");
        }

        // Many concurrent claimers re-contending the same rows (zero lease keeps
        // every row claimable each pass → maximal UPDATE-upgrade contention).
        let mut handles = Vec::new();
        for _ in 0..16 {
            let fe = Arc::clone(&feed_events);
            handles.push(tokio::spawn(async move {
                for _ in 0..50 {
                    fe.claim_pending_batch(200, Duration::zero()).await?;
                }
                Ok::<(), FeedEventError>(())
            }));
        }

        for h in handles {
            h.await
                .expect("task panicked")
                .expect("no database-is-locked error");
        }
        // cov:ignore-stop
    }
}
