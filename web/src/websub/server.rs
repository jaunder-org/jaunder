//! Host-only authorization, publisher, and recovery-storage helpers.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use common::MutationOutcome;
use common::ids::FeedEventId;
use common::pagination::PageSize;
use common::tagged_url::HubUrl;
use common::time::UtcInstant;
use host::feed::FeedEventPhase;
use leptos::prelude::*;
use storage::{FeedEventDeadLetterCursor, FeedEventRedriveError, FeedEventStorage, WriteScope};

use crate::auth;
use crate::error::{InternalError, InternalResult, from_write_scope_error};

use super::{DeadLetterCursor, DeadLetterPage, DeadLetterRow, WebsubPhase, WebsubSettings};

/// Erased publisher-service failure at the web/server composition seam.
#[derive(Debug)]
pub struct WebsubPublisherError {
    source: Box<dyn Error + Send + Sync>,
}

impl WebsubPublisherError {
    #[must_use]
    pub fn new(source: Box<dyn Error + Send + Sync>) -> Self {
        Self { source }
    }
}

impl fmt::Display for WebsubPublisherError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WebSub publisher operation failed")
    }
}

impl Error for WebsubPublisherError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Publisher operation boundary supplied by the composition root. Keeping this
/// trait in `web` prevents the web crate from depending on the server's gate
/// implementation while ensuring hub changes cannot bypass that gate.
#[async_trait]
pub trait WebsubPublisher: Send + Sync {
    async fn hub_url(&self) -> Result<Option<HubUrl>, WebsubPublisherError>;
    async fn mutate_hub(
        &self,
        hub: Option<HubUrl>,
    ) -> Result<MutationOutcome<()>, WebsubPublisherError>;
}

fn phase(phase: WebsubPhase) -> FeedEventPhase {
    match phase {
        WebsubPhase::Regeneration => FeedEventPhase::Regeneration,
        WebsubPhase::Publication => FeedEventPhase::Publication,
    }
}

fn row_phase(phase: FeedEventPhase) -> WebsubPhase {
    match phase {
        FeedEventPhase::Regeneration => WebsubPhase::Regeneration,
        FeedEventPhase::Publication => WebsubPhase::Publication,
    }
}

pub(super) async fn get_settings_impl() -> InternalResult<WebsubSettings> {
    auth::require_operator().await?;
    let publisher = use_context::<Arc<dyn WebsubPublisher>>()
        .ok_or_else(|| InternalError::server_message("WebSub publisher is unavailable"))?;
    let hub_url = publisher.hub_url().await.map_err(InternalError::server)?;
    Ok(WebsubSettings { hub_url })
}

pub(super) async fn update_hub_impl(
    hub_url: Option<HubUrl>,
) -> InternalResult<MutationOutcome<()>> {
    auth::require_operator().await?;
    let publisher = use_context::<Arc<dyn WebsubPublisher>>()
        .ok_or_else(|| InternalError::server_message("WebSub publisher is unavailable"))?;
    publisher
        .mutate_hub(hub_url)
        .await
        .map_err(InternalError::server)
}

pub(super) async fn list_dead_letters_impl(
    selected_phase: WebsubPhase,
    cursor: Option<DeadLetterCursor>,
    page_size: PageSize,
) -> InternalResult<DeadLetterPage> {
    auth::require_operator().await?;
    let feed_events = use_context::<Arc<dyn FeedEventStorage>>()
        .ok_or_else(|| InternalError::server_message("feed-event storage is unavailable"))?;
    let cursor = cursor.map(|cursor| FeedEventDeadLetterCursor {
        terminal_at: cursor.terminal_at,
        id: cursor.id,
    });
    let page = feed_events
        .dead_letters(phase(selected_phase), cursor, page_size)
        .await
        .map_err(InternalError::storage)?;
    Ok(DeadLetterPage {
        events: page
            .events
            .into_iter()
            .map(|event| DeadLetterRow {
                id: event.id,
                feed_path: event.feed_path.to_string(),
                phase: row_phase(event.phase),
                attempts: event.attempts,
                terminal_at: event.terminal_at,
                diagnostic: event.diagnostic,
            })
            .collect(),
        next_cursor: page.next_cursor.map(|cursor| DeadLetterCursor {
            terminal_at: cursor.terminal_at,
            id: cursor.id,
        }),
    })
}

pub(super) async fn redrive_dead_letters_impl(
    ids: Vec<FeedEventId>,
) -> InternalResult<MutationOutcome<()>> {
    auth::require_operator().await?;
    if ids.is_empty() {
        return Err(InternalError::validation(
            "select at least one dead-letter event",
        ));
    }
    let feed_events = use_context::<Arc<dyn FeedEventStorage>>()
        .ok_or_else(|| InternalError::server_message("feed-event storage is unavailable"))?;
    let write_scope = use_context::<WriteScope>()
        .ok_or_else(|| InternalError::server_message("write scope is unavailable"))?;
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                feed_events
                    .redrive_dead_letters(transaction, &ids, UtcInstant::now())
                    .await
                    .map_err(|error| match error {
                        FeedEventRedriveError::Rejected(_) => InternalError::conflict(
                            "one or more selected events are no longer dead-lettered",
                        ),
                        FeedEventRedriveError::Db(error) => InternalError::storage(error),
                    })
            })
        })
        .await
        .map_err(from_write_scope_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::ids::UserId;
    use leptos::prelude::{Owner, provide_context};
    use storage::{MockUserStorage, UserStorage};

    use crate::test_support::auth_parts;

    // guard:no-backend — missing operator user and deliberately absent feed-event
    // context prove authorization rejects before this endpoint reads storage.
    #[tokio::test]
    async fn dead_letter_listing_authorizes_before_storage_lookup() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "not-an-operator"));
        let mut users = MockUserStorage::new();
        users.expect_get_user().returning(|_| Ok(None));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);

        let error = list_dead_letters_impl(WebsubPhase::Regeneration, None, PageSize::default())
            .await
            .expect_err("unauthorized request must fail before feed-event lookup");
        drop(owner);
        assert_eq!(error.kind(), crate::error::ErrorKind::Auth);
    }
}
