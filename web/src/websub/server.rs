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
    use std::io;
    use std::sync::Mutex;

    use common::ids::UserId;
    use common::test_support::parse_username;
    use common::time::UtcInstant;
    use host::feed::FeedPath;
    use leptos::prelude::{Owner, provide_context};
    use storage::{
        EmailVerified, FeedEventDeadLetter, FeedEventDeadLetterError, FeedEventDeadLetterPage,
        FeedEventRedriveError, FeedEventRedriveRejected, MockFeedEventStorage, MockUserStorage,
        OperatorStatus, UserRecord, UserStorage, test_support::mock_write_scope,
    };

    use crate::error::{ErrorKind, WebError};
    use crate::test_support::auth_parts;

    struct Publisher {
        hub: Result<Option<HubUrl>, &'static str>,
        mutation: Result<MutationOutcome<()>, &'static str>,
        seen_hub: Mutex<Vec<Option<HubUrl>>>,
    }

    #[async_trait]
    impl WebsubPublisher for Publisher {
        async fn hub_url(&self) -> Result<Option<HubUrl>, WebsubPublisherError> {
            self.hub
                .clone()
                .map_err(|message| WebsubPublisherError::new(Box::new(io::Error::other(message))))
        }

        async fn mutate_hub(
            &self,
            hub: Option<HubUrl>,
        ) -> Result<MutationOutcome<()>, WebsubPublisherError> {
            self.seen_hub
                .lock()
                .expect("publisher mutex must not poison")
                .push(hub);
            self.mutation
                .clone()
                .map_err(|message| WebsubPublisherError::new(Box::new(io::Error::other(message))))
        }
    }

    fn operator() -> UserRecord {
        UserRecord {
            user_id: UserId::from(1),
            username: parse_username("alice"),
            display_name: None,
            bio: None,
            created_at: UtcInstant::now(),
            last_authenticated_at: None,
            email: None,
            email_verified: EmailVerified::UNVERIFIED,
            is_operator: OperatorStatus::OPERATOR,
        }
    }

    fn operator_owner() -> Owner {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut users = MockUserStorage::new();
        users.expect_get_user().returning(|_| Ok(Some(operator())));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);
        owner
    }

    fn publisher(
        hub: Result<Option<HubUrl>, &'static str>,
        mutation: Result<MutationOutcome<()>, &'static str>,
    ) -> Arc<Publisher> {
        Arc::new(Publisher {
            hub,
            mutation,
            seen_hub: Mutex::new(Vec::new()),
        })
    }

    #[test]
    fn publisher_error_preserves_source_and_has_safe_display() {
        let error = WebsubPublisherError::new(Box::new(io::Error::other("upstream down")));

        assert_eq!(error.to_string(), "WebSub publisher operation failed");
        assert_eq!(
            error.source().expect("source must be retained").to_string(),
            "upstream down"
        );
    }

    #[test]
    fn phase_conversions_are_inverse_for_wire_phases() {
        assert_eq!(
            phase(WebsubPhase::Regeneration),
            FeedEventPhase::Regeneration
        );
        assert_eq!(phase(WebsubPhase::Publication), FeedEventPhase::Publication);
        assert_eq!(
            row_phase(FeedEventPhase::Regeneration),
            WebsubPhase::Regeneration
        );
        assert_eq!(
            row_phase(FeedEventPhase::Publication),
            WebsubPhase::Publication
        );
    }

    #[tokio::test]
    async fn settings_and_hub_update_delegate_to_publisher() {
        let owner = operator_owner();
        let hub: HubUrl = "https://hub.example.test/".parse().expect("valid hub URL");
        let publisher = publisher(
            Ok(Some(hub.clone())),
            Ok(MutationOutcome::CommitIndeterminate(())),
        );
        provide_context(Arc::clone(&publisher) as Arc<dyn WebsubPublisher>);

        assert_eq!(
            super::super::get_websub_settings()
                .await
                .expect("publisher returns settings"),
            WebsubSettings {
                hub_url: Some(hub.clone())
            }
        );
        assert_eq!(
            super::super::update_websub_hub(Some(hub.clone()))
                .await
                .expect("publisher accepts hub"),
            MutationOutcome::CommitIndeterminate(())
        );
        assert_eq!(
            publisher
                .seen_hub
                .lock()
                .expect("publisher mutex must not poison")
                .as_slice(),
            &[Some(hub)]
        );
        drop(owner);
    }

    #[tokio::test]
    async fn publisher_failures_and_missing_context_are_server_errors() {
        let owner = operator_owner();
        let publisher = publisher(Err("read failed"), Err("write failed"));
        provide_context(Arc::clone(&publisher) as Arc<dyn WebsubPublisher>);

        assert_eq!(
            get_settings_impl()
                .await
                .expect_err("publisher failure")
                .kind(),
            ErrorKind::Internal
        );
        assert_eq!(
            update_hub_impl(None)
                .await
                .expect_err("publisher failure")
                .kind(),
            ErrorKind::Internal
        );
        drop(owner);

        let owner = operator_owner();
        assert_eq!(
            get_settings_impl()
                .await
                .expect_err("publisher context is required")
                .kind(),
            ErrorKind::Internal
        );
        assert_eq!(
            update_hub_impl(None)
                .await
                .expect_err("publisher context is required")
                .kind(),
            ErrorKind::Internal
        );
        drop(owner);
    }

    #[tokio::test]
    async fn dead_letter_listing_projects_rows_cursor_and_storage_errors() {
        let owner = operator_owner();
        let terminal_at = UtcInstant::now();
        let id = FeedEventId::from(9);
        let next_id = FeedEventId::from(8);
        let mut events = MockFeedEventStorage::new();
        events
            .expect_dead_letters()
            .withf(move |phase, cursor, _page_size| {
                *phase == FeedEventPhase::Publication
                    && *cursor == Some(FeedEventDeadLetterCursor { terminal_at, id })
            })
            .returning(move |_, _, _| {
                Ok(FeedEventDeadLetterPage {
                    events: vec![FeedEventDeadLetter {
                        id,
                        feed_path: "/feed.rss".parse::<FeedPath>().expect("valid feed path"),
                        phase: FeedEventPhase::Publication,
                        attempts: 4,
                        terminal_at,
                        diagnostic: Some("publisher timed out".to_owned()),
                    }],
                    next_cursor: Some(FeedEventDeadLetterCursor {
                        terminal_at,
                        id: next_id,
                    }),
                })
            });
        provide_context(Arc::new(events) as Arc<dyn FeedEventStorage>);

        let page = super::super::list_dead_letters(
            WebsubPhase::Publication,
            Some(DeadLetterCursor { terminal_at, id }),
            PageSize::default(),
        )
        .await
        .expect("dead-letter page");
        assert_eq!(
            page,
            DeadLetterPage {
                events: vec![DeadLetterRow {
                    id,
                    feed_path: "/feed.rss".to_owned(),
                    phase: WebsubPhase::Publication,
                    attempts: 4,
                    terminal_at,
                    diagnostic: Some("publisher timed out".to_owned()),
                }],
                next_cursor: Some(DeadLetterCursor {
                    terminal_at,
                    id: next_id,
                }),
            }
        );
        drop(owner);

        let owner = operator_owner();
        let mut events = MockFeedEventStorage::new();
        events
            .expect_dead_letters()
            .returning(|_, _, _| Err(FeedEventDeadLetterError::CorruptRow));
        provide_context(Arc::new(events) as Arc<dyn FeedEventStorage>);
        assert_eq!(
            list_dead_letters_impl(WebsubPhase::Regeneration, None, PageSize::default())
                .await
                .expect_err("storage rejection")
                .kind(),
            ErrorKind::Storage
        );
        drop(owner);
    }

    #[tokio::test]
    async fn dead_letter_context_and_redrive_rejections_are_reported() {
        let owner = operator_owner();
        assert_eq!(
            list_dead_letters_impl(WebsubPhase::Regeneration, None, PageSize::default())
                .await
                .expect_err("storage context is required")
                .kind(),
            ErrorKind::Internal
        );
        assert_eq!(
            redrive_dead_letters_impl(Vec::new())
                .await
                .expect_err("selection is required")
                .kind(),
            ErrorKind::Validation
        );
        drop(owner);

        let owner = operator_owner();
        provide_context(Arc::new(MockFeedEventStorage::new()) as Arc<dyn FeedEventStorage>);
        assert_eq!(
            redrive_dead_letters_impl(vec![FeedEventId::from(3)])
                .await
                .expect_err("write scope context is required")
                .kind(),
            ErrorKind::Internal
        );
        drop(owner);

        let owner = operator_owner();
        let mut events = MockFeedEventStorage::new();
        events
            .expect_redrive_dead_letters()
            .returning(|_, _, _| Err(FeedEventRedriveRejected.into()));
        provide_context(Arc::new(events) as Arc<dyn FeedEventStorage>);
        provide_context(mock_write_scope());
        assert!(matches!(
            super::super::redrive_dead_letters(vec![FeedEventId::from(3)])
                .await
                .expect_err("rejected selection"),
            WebError::Conflict { .. }
        ));
        drop(owner);
    }

    #[tokio::test]
    async fn redrive_commits_selected_ids_and_maps_database_errors() {
        let owner = operator_owner();
        let mut events = MockFeedEventStorage::new();
        events
            .expect_redrive_dead_letters()
            .withf(|_, ids, _| ids == [FeedEventId::from(4), FeedEventId::from(5)])
            .returning(|_, _, _| Ok(()));
        provide_context(Arc::new(events) as Arc<dyn FeedEventStorage>);
        provide_context(mock_write_scope());
        assert_eq!(
            redrive_dead_letters_impl(vec![FeedEventId::from(4), FeedEventId::from(5)])
                .await
                .expect("valid selection is redriven"),
            MutationOutcome::Confirmed(())
        );
        drop(owner);

        let owner = operator_owner();
        let mut events = MockFeedEventStorage::new();
        events
            .expect_redrive_dead_letters()
            .returning(|_, _, _| Err(FeedEventRedriveError::Db(sqlx::Error::RowNotFound)));
        provide_context(Arc::new(events) as Arc<dyn FeedEventStorage>);
        provide_context(mock_write_scope());
        assert_eq!(
            redrive_dead_letters_impl(vec![FeedEventId::from(4)])
                .await
                .expect_err("database failure is surfaced as storage")
                .kind(),
            ErrorKind::Storage
        );
        drop(owner);
    }

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
