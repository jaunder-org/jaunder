// Reproduction harness for issue #18: the SQLite claim_pending_batch lock
// flake. With the old SELECT->UPDATE->SELECT deferred transaction, concurrent
// claimers upgrade a shared lock to a reserved lock against a stale snapshot
// and SQLite returns "database is locked" (busy_timeout cannot rescue an
// upgrade). With the single-statement UPDATE ... RETURNING (ADR-0021) the
// writes serialize cleanly under busy_timeout.
//
// Timing-based, so it is #[ignore]d -- excluded from CI to avoid being a
// flake source itself. Run on demand:
//   cargo test -p jaunder --test feed -- --ignored claim_pending_batch_no_lock_contention
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use crate::helpers::{sqlite_only, Backend};
use chrono::Duration;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

#[apply(sqlite_only)]
// reason: reproduces the SQLite-specific issue #18 claim_pending_batch lock flake
// (reserved-lock upgrade under busy_timeout); Postgres MVCC cannot exhibit it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "timing-based #18 reproduction; run manually with --ignored"]
async fn claim_pending_batch_no_lock_contention(#[case] backend: Backend) {
    let _ = backend; // sqlite_only template supplies Backend::Sqlite
    let env = Backend::Sqlite.setup().await;
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
            Ok::<(), storage::FeedEventError>(())
        }));
    }

    for h in handles {
        h.await
            .expect("task panicked")
            .expect("no database-is-locked error");
    }
}
