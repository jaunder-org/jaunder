use async_trait::async_trait;

use common::ids::FeedEventId;
use common::pagination::RowLimit;
use common::time::UtcInstant;
use host::feed::FeedEventClaimLimit;
use sqlx::{Pool, Sqlite};

use crate::feed_events::{
    self, ClaimedFeedEventRow, ClaimedRow, FeedEventDialect, FeedEventError, FeedEventRecord,
    FeedEventStore, StoredFeedDiagnostic,
};

use crate::sql::RowCount;
/// SQLite-backed feed-event storage.
pub type SqliteFeedEventStorage = FeedEventStore<Sqlite>;

fn placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

fn finish_purge(
    primary: Vec<FeedEventRecord>,
    purge: Result<(), sqlx::Error>,
) -> Vec<FeedEventRecord> {
    feed_events::finish_corrupt_purge(primary, purge, "storage.sqlite.feed_events.purge_corrupt")
}

/// Deletes claimed rows whose `feed_url` cannot decode. Partitioning reports
/// the aggregate decode failure; only a failed cleanup is reported here.
async fn purge_corrupt(
    connection: &mut sqlx::SqliteConnection,
    ids: &[FeedEventId],
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query("SAVEPOINT feed_event_purge")
        .execute(&mut *connection)
        .await?;
    let ph = placeholders(ids.len());
    let sql = format!("DELETE FROM feed_events WHERE id IN ({ph})");
    let mut query = sqlx::query(&sql);
    for id in ids {
        query = query.bind(*id);
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
        connection: &mut sqlx::SqliteConnection,
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
             RETURNING id, feed_url, status, attempts, last_error, next_attempt_at, claimed_at, terminal_at, \
                       created_at, regenerated_at, pinged_at",
        )
        .bind(now)
        .bind(now)
        .bind(lease_cutoff)
        .bind(limit)
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
        .bind(now)
        .bind(lease_cutoff)
        .fetch_one(pool)
        .await?;
        Ok(count.into_u64())
    }

    async fn mark_regenerated(
        connection: &mut sqlx::SqliteConnection,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError> {
        let now = UtcInstant::now();
        let ph = placeholders(ids.len());
        let sql = format!("UPDATE feed_events SET regenerated_at = ? WHERE id IN ({ph})");
        let mut q = sqlx::query(&sql).bind(now);
        for id in ids {
            q = q.bind(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn mark_pinged(
        connection: &mut sqlx::SqliteConnection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'done', pinged_at = ?, terminal_at = ? WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(&sql).bind(now).bind(now);
        for id in ids {
            q = q.bind(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn mark_failed(
        connection: &mut sqlx::SqliteConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        next_attempt_at: UtcInstant,
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
        q.execute(&mut *connection).await?;
        Ok(())
    }

    async fn mark_exhausted(
        connection: &mut sqlx::SqliteConnection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        let ph = placeholders(ids.len());
        let sql = format!(
            "UPDATE feed_events SET status = 'failed', last_error = ?, terminal_at = ? WHERE id IN ({ph})"
        );
        let mut q = sqlx::query(&sql).bind(error).bind(now);
        for id in ids {
            q = q.bind(*id);
        }
        q.execute(&mut *connection).await?;
        Ok(())
    }
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
        .bind(now)
        .bind(failed_cutoff)
        .bind(limit)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
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

    use super::finish_purge;
    use crate::FeedEventRecord;
    use crate::test_support::{Backend, fp, sqlite_only};
    use chrono::Duration;
    use common::{ids::FeedEventId, time::UtcInstant};
    use host::feed::FeedEventStatus;

    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn continuation_reporting_corrupt_purge_failure_preserves_valid_batch_and_reports_once() {
        let now = UtcInstant::now();
        let valid = vec![FeedEventRecord {
            id: FeedEventId::from(17),
            feed_path: fp("/feed.rss"),
            status: FeedEventStatus::Claimed,
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
            "storage.sqlite.feed_events.purge_corrupt",
        );
    }

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
        let write_scope = env.state.write_scope.clone();

        // Seed a populated queue with distinct valid feed paths.
        for i in 0..200 {
            let feed_events = Arc::clone(&feed_events);
            let url = fp(&format!("/tags/t{i}/feed.rss"));
            write_scope
                .run(move |transaction| {
                    Box::pin(async move { feed_events.enqueue(transaction, &url).await })
                })
                .await
                .expect("enqueue");
        }

        // Many concurrent claimers re-contending the same rows (zero lease keeps
        // every row claimable each pass → maximal UPDATE-upgrade contention).
        let mut handles = Vec::new();
        for _ in 0..16 {
            let feed_events = Arc::clone(&feed_events);
            let write_scope = write_scope.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..50 {
                    let feed_events = Arc::clone(&feed_events);
                    write_scope
                        .run(move |transaction| {
                            Box::pin(async move {
                                feed_events
                                    .claim_pending_batch(transaction, 200, Duration::zero())
                                    .await
                            })
                        })
                        .await?;
                }
                Ok::<(), anyhow::Error>(())
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
