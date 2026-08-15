use common::ids::FeedEventId;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, backends, fp};

#[apply(backends)]
#[tokio::test]
async fn feed_events_marks_run(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let fe = &state.feed_events;

    // Enqueue + claim to obtain real ids, then exercise every
    // FeedEventDialect mark_* method on this backend. Each is an independent
    // `UPDATE … WHERE id IN (…)`, so they all run regardless of row state.
    fe.enqueue(&fp("/feed.rss")).await.unwrap();
    let claimed = fe
        .claim_pending_batch(50, chrono::Duration::minutes(5))
        .await
        .unwrap();
    let ids: Vec<FeedEventId> = claimed.iter().map(|r| r.id).collect();
    assert!(!ids.is_empty());

    fe.mark_regenerated(&ids).await.unwrap();
    fe.mark_pinged(&ids).await.unwrap();
    fe.mark_failed(
        &ids,
        "boom",
        chrono::Utc::now() + chrono::Duration::minutes(1),
    )
    .await
    .unwrap();
    fe.mark_exhausted(&ids, "gave up").await.unwrap();
}
