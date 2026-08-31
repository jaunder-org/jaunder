use common::ids::FeedEventId;
use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, backends, fp};

#[apply(backends)]
#[tokio::test]
async fn feed_events_marks_run(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let feed_events_for_enqueue = state.feed_events.clone();

    // Enqueue + claim to obtain real ids, then exercise every
    // FeedEventDialect mark_* method on this backend. Each is an independent
    // `UPDATE … WHERE id IN (…)`, so they all run regardless of row state.
    let feed_path = fp("/feed.rss");
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                feed_events_for_enqueue
                    .enqueue(transaction, &feed_path)
                    .await
            })
        })
        .await
        .unwrap();
    let feed_events_for_claim = state.feed_events.clone();
    let claim_limit = 50;
    let claim_lease = chrono::Duration::minutes(5);
    let claimed = storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    feed_events_for_claim
                        .claim_pending_batch(transaction, claim_limit, claim_lease)
                        .await
                })
            })
            .await
            .unwrap(),
        "claim acknowledgement",
    );
    let ids: Vec<FeedEventId> = claimed.iter().map(|r| r.id).collect();
    assert!(!ids.is_empty());

    let feed_events_for_regeneration = state.feed_events.clone();
    let ids_for_regeneration = ids.clone();
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                feed_events_for_regeneration
                    .mark_regenerated(transaction, &ids_for_regeneration)
                    .await
            })
        })
        .await
        .unwrap();
    let feed_events_for_ping = state.feed_events.clone();
    let ids_for_ping = ids.clone();
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                feed_events_for_ping
                    .mark_pinged(transaction, &ids_for_ping)
                    .await
            })
        })
        .await
        .unwrap();
    let feed_events_for_failure = state.feed_events.clone();
    let ids_for_failure = ids.clone();
    let failure_reason = "boom";
    let retry_at = UtcInstant::from(chrono::Utc::now() + chrono::Duration::minutes(1));
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                feed_events_for_failure
                    .mark_failed(transaction, &ids_for_failure, failure_reason, retry_at)
                    .await
            })
        })
        .await
        .unwrap();
    let feed_events_for_exhaustion = state.feed_events.clone();
    let ids_for_exhaustion = ids;
    let exhaustion_reason = "gave up";
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                feed_events_for_exhaustion
                    .mark_exhausted(transaction, &ids_for_exhaustion, exhaustion_reason)
                    .await
            })
        })
        .await
        .unwrap();
}
