use std::sync::Arc;

use common::ids::FeedEventId;
use common::pagination::PageSize;
use common::time::UtcInstant;
use host::feed::FeedEventPhase;
use storage::{FeedEventDeadLetterCursor, FeedEventDeadLetterPage, FeedEventStorage, WriteScope};

use crate::cli::StorageArgs;

use super::support;

/// List one bounded, stable page of terminal `WebSub` work.
pub(super) async fn cmd_dead_letters_list(
    storage: &StorageArgs,
    phase: FeedEventPhase,
    cursor: Option<FeedEventDeadLetterCursor>,
    page_size: PageSize,
) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    let page = state
        .feed_events
        .dead_letters(phase, cursor, page_size)
        .await?;
    println!("{}", format_dead_letter_page(&page)?);
    Ok(())
}

/// Atomically redrive the exact terminal selection.
pub(super) async fn cmd_dead_letters_redrive(
    storage: &StorageArgs,
    ids: &[FeedEventId],
) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    redrive_selected(
        Arc::clone(&state.feed_events),
        &state.write_scope,
        ids.to_vec(),
    )
    .await?;
    println!("redriven={}", ids.len());
    Ok(())
}

/// Uses the storage transaction seam so a rejected selection cannot partially redrive.
async fn redrive_selected(
    feed_events: Arc<dyn FeedEventStorage>,
    write_scope: &WriteScope,
    ids: Vec<FeedEventId>,
) -> anyhow::Result<()> {
    let now = UtcInstant::now();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                feed_events
                    .redrive_dead_letters(transaction, &ids, now)
                    .await
            })
        })
        .await?;
    support::require_confirmed_mutation(outcome, "dead-letter redrive")?;
    Ok(())
}

/// Formats a page as one deterministic JSON document, suitable for scripts.
fn format_dead_letter_page(page: &FeedEventDeadLetterPage) -> anyhow::Result<String> {
    let events = page
        .events
        .iter()
        .map(|event| DeadLetterOutput {
            id: i64::from(event.id),
            feed_path: event.feed_path.to_string(),
            phase: event.phase.as_ref(),
            attempts: event.attempts,
            terminal_at: event.terminal_at.to_string(),
            diagnostic: event.diagnostic.as_deref(),
        })
        .collect();
    let output = DeadLetterPageOutput {
        events,
        next_cursor: page.next_cursor.map(format_cursor),
    };
    Ok(serde_json::to_string(&output)?)
}

#[derive(serde::Serialize)]
struct DeadLetterPageOutput<'a> {
    events: Vec<DeadLetterOutput<'a>>,
    next_cursor: Option<String>,
}

#[derive(serde::Serialize)]
struct DeadLetterOutput<'a> {
    id: i64,
    feed_path: String,
    phase: &'a str,
    attempts: i32,
    terminal_at: String,
    diagnostic: Option<&'a str>,
}

fn format_cursor(cursor: FeedEventDeadLetterCursor) -> String {
    format!("{},{}", cursor.terminal_at, i64::from(cursor.id))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{TimeZone as _, Utc};
    use rstest::*;
    use rstest_reuse::*;
    use storage::{
        FeedEventDeadLetter, FeedEventDeadLetterPage, FeedEventRedriveError,
        test_support::{Backend, TestEnv, backends, confirmed},
    };

    use super::super::test_support::assert_command_source;
    use super::*;

    async fn terminal_event(env: &TestEnv, phase: FeedEventPhase, suffix: &str) -> FeedEventId {
        let path = format!("/~operator-{suffix}/feed.rss")
            .parse()
            .expect("feed path");
        let feed_events = Arc::clone(&env.state.feed_events);
        let id = confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { feed_events.enqueue(transaction, &path).await })
                })
                .await
                .expect("enqueue terminal fixture"),
        );
        let feed_events = Arc::clone(&env.state.feed_events);
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        match phase {
                            FeedEventPhase::Regeneration => {
                                feed_events
                                    .dead_letter_regeneration(
                                        transaction,
                                        &[id],
                                        "regeneration terminal fixture",
                                        UtcInstant::now(),
                                    )
                                    .await
                            }
                            FeedEventPhase::Publication => {
                                feed_events
                                    .dead_letter_publication(
                                        transaction,
                                        &[id],
                                        "publication terminal fixture",
                                        UtcInstant::now(),
                                    )
                                    .await
                            }
                        }
                    })
                })
                .await
                .expect("terminalize fixture"),
        );
        id
    }

    #[test]
    fn list_output_includes_every_field_and_stable_next_cursor() {
        let terminal_at = UtcInstant::from(Utc.with_ymd_and_hms(2026, 9, 3, 12, 0, 0).unwrap());
        let id = FeedEventId::from(42);
        let page = FeedEventDeadLetterPage {
            events: vec![FeedEventDeadLetter {
                id,
                feed_path: "/~operator/feed.rss".parse().unwrap(),
                phase: FeedEventPhase::Publication,
                attempts: 10,
                terminal_at,
                diagnostic: Some("hub returned 410".to_owned()),
            }],
            next_cursor: Some(FeedEventDeadLetterCursor { terminal_at, id }),
        };

        assert_eq!(
            format_dead_letter_page(&page).unwrap(),
            r#"{"events":[{"id":42,"feed_path":"/~operator/feed.rss","phase":"publication","attempts":10,"terminal_at":"2026-09-03T12:00:00Z","diagnostic":"hub returned 410"}],"next_cursor":"2026-09-03T12:00:00Z,42"}"#,
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn redrive_rejects_stale_or_invalid_ids_without_partial_mutation(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let regeneration = terminal_event(&env, FeedEventPhase::Regeneration, "regeneration").await;
        let publication = terminal_event(&env, FeedEventPhase::Publication, "publication").await;

        let error = redrive_selected(
            Arc::clone(&env.state.feed_events),
            &env.state.write_scope,
            vec![regeneration, FeedEventId::from(-1)],
        )
        .await
        .expect_err("an invalid selection rejects every id");
        assert_command_source::<FeedEventRedriveError>(
            &error,
            "write operation failed: one or more feed events are absent, expired, or not dead-lettered",
        );

        for phase in [FeedEventPhase::Regeneration, FeedEventPhase::Publication] {
            assert_eq!(
                env.state
                    .feed_events
                    .dead_letters(phase, None, PageSize::default())
                    .await
                    .unwrap()
                    .events
                    .len(),
                1,
                "invalid selection did not partially redrive {phase:?}",
            );
        }

        redrive_selected(
            Arc::clone(&env.state.feed_events),
            &env.state.write_scope,
            vec![regeneration, publication],
        )
        .await
        .expect("the exact terminal selection redrives atomically");
        for phase in [FeedEventPhase::Regeneration, FeedEventPhase::Publication] {
            assert!(
                env.state
                    .feed_events
                    .dead_letters(phase, None, PageSize::default())
                    .await
                    .unwrap()
                    .events
                    .is_empty(),
                "exact selection redrove {phase:?}",
            );
        }
        let error = redrive_selected(
            Arc::clone(&env.state.feed_events),
            &env.state.write_scope,
            vec![regeneration],
        )
        .await
        .expect_err("an already-redriven id is stale");
        assert_command_source::<FeedEventRedriveError>(
            &error,
            "write operation failed: one or more feed events are absent, expired, or not dead-lettered",
        );
    }
}
