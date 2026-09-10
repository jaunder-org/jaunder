use common::MutationOutcome;
use common::post_body::PostBody;
use common::time::{self, UtcInstant};
use leptos::prelude::*;
use thiserror::Error;

use super::api::SavedPost;
use super::compose_state::{self, PublicationIntent};
use crate::forms::Field;

/// The publication state captured when the editor response was assembled.
///
/// Classification is immutable for the loaded editor: a Scheduled Post that
/// becomes due while the page is open remains a scheduled edit, avoiding a
/// browser-clock race in both controls and payload construction. Both published
/// variants retain the exact stored instant for lossless editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadedPublication {
    Draft,
    Scheduled(UtcInstant),
    Live(UtcInstant),
}

/// Classify a loaded Post against the server snapshot returned with it.
#[must_use]
pub fn loaded_publication(
    published_at: Option<UtcInstant>,
    fetched_at: UtcInstant,
) -> LoadedPublication {
    match published_at {
        None => LoadedPublication::Draft,
        Some(published_at) if published_at.value() > fetched_at.value() => {
            LoadedPublication::Scheduled(published_at)
        }
        Some(published_at) => LoadedPublication::Live(published_at),
    }
}

/// Returns the scheduled publication instant, if the server's fetch snapshot
/// classified the authored post as scheduled.
#[must_use]
pub fn scheduled_publication_at(
    published_at: Option<UtcInstant>,
    fetched_at: UtcInstant,
) -> Option<UtcInstant> {
    match loaded_publication(published_at, fetched_at) {
        LoadedPublication::Scheduled(at) => Some(at),
        LoadedPublication::Draft | LoadedPublication::Live(_) => None,
    }
}

/// A published editor's local display value and exact original UTC instant.
///
/// The original remains authoritative until the author edits the control. This
/// preserves seconds, nanoseconds, and the selected instant in a repeated DST
/// wall-clock interval.
#[derive(Clone, Copy)]
pub struct PublicationTimeEditState {
    pub value: RwSignal<String>,
    original: UtcInstant,
    edited: RwSignal<bool>,
}

/// The complete publication branch and branch-specific signals for one loaded editor.
///
/// Keeping the publication-time state inside both non-Draft variants makes an
/// impossible `published-without-publication-time` combination unrepresentable.
#[derive(Clone, Copy)]
pub enum EditPublicationState {
    Draft(RwSignal<String>),
    Scheduled(PublicationTimeEditState),
    Live(PublicationTimeEditState),
}

/// Last confirmed publication transition for the mounted editor.
///
/// Failed and commit-indeterminate settlements deliberately leave this state
/// untouched, so a confirmed pullback to Draft cannot regress to the stale
/// publication branch loaded when the editor mounted.
#[derive(Clone, Copy)]
pub struct EditLifecycleState {
    confirmed_draft: RwSignal<bool>,
}

impl EditLifecycleState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            confirmed_draft: RwSignal::new(false),
        }
    }

    pub fn adopt_settlement<E>(&self, settlement: &Result<MutationOutcome<SavedPost>, E>) {
        if let Ok(MutationOutcome::Confirmed(saved)) = settlement {
            self.confirmed_draft.set(saved.published_at.is_none());
        }
    }

    #[must_use]
    pub fn current_publication(
        self,
        loaded: EditPublicationState,
        draft_publish_at: RwSignal<String>,
    ) -> EditPublicationState {
        if self.confirmed_draft.get() {
            EditPublicationState::Draft(draft_publish_at)
        } else {
            loaded
        }
    }
}

impl Default for EditLifecycleState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Enter a valid local date and time")]
pub struct InvalidSchedule;

impl PublicationTimeEditState {
    #[must_use]
    pub fn new(original: UtcInstant, display_value: String) -> Self {
        Self {
            value: RwSignal::new(display_value),
            original,
            edited: RwSignal::new(false),
        }
    }

    pub fn set_input(&self, value: String) {
        self.value.set(value);
        self.edited.set(true);
    }

    /// # Errors
    ///
    /// Returns [`InvalidSchedule`] after the author changes the control to a
    /// local wall-clock value that does not identify a real instant. Empty input
    /// is invalid here; pulling a Post back to Draft is an explicit action.
    pub fn publication(&self) -> Result<PublicationIntent, InvalidSchedule> {
        if !self.edited.get() {
            return Ok(PublicationIntent::PublishAt(self.original));
        }

        time::strict_utc_instant_from_local(&self.value.get())
            .map(PublicationIntent::PublishAt)
            .ok_or(InvalidSchedule)
    }
}

impl EditPublicationState {
    #[must_use]
    pub fn from_loaded(loaded: LoadedPublication, draft_publish_at: RwSignal<String>) -> Self {
        let edit_time = |original| {
            PublicationTimeEditState::new(original, time::local_datetime_from_utc(original))
        };
        match loaded {
            LoadedPublication::Draft => Self::Draft(draft_publish_at),
            LoadedPublication::Scheduled(original) => Self::Scheduled(edit_time(original)),
            LoadedPublication::Live(original) => Self::Live(edit_time(original)),
        }
    }

    #[must_use]
    pub fn loaded(self) -> LoadedPublication {
        match self {
            Self::Draft(_) => LoadedPublication::Draft,
            Self::Scheduled(state) => LoadedPublication::Scheduled(state.original),
            Self::Live(state) => LoadedPublication::Live(state.original),
        }
    }

    #[must_use]
    pub fn publication_time(self) -> Option<PublicationTimeEditState> {
        match self {
            Self::Scheduled(state) | Self::Live(state) => Some(state),
            Self::Draft(_) => None,
        }
    }
}

/// Bind every loaded editor branch to derived Save and Unpublish payloads.
///
/// The two disabled signals differ for a published Post: an invalid publication
/// time blocks Save, while explicit Unpublish ignores that irrelevant field but
/// still requires every field it persists to be valid and loaded.
#[must_use]
pub fn edit_submit_gate(
    body: Field<PostBody>,
    also_blocked: Signal<bool>,
    publication: EditPublicationState,
    on_submit: Callback<(PostBody, PublicationIntent)>,
) -> (
    Signal<bool>,
    Signal<bool>,
    Signal<Option<InvalidSchedule>>,
    Callback<bool>,
) {
    match publication {
        EditPublicationState::Draft(publish_at) => {
            let (disabled, on_click) = compose_state::submit_gate(
                body,
                also_blocked,
                Callback::new(move |(body, publish): (PostBody, bool)| {
                    on_submit.run((
                        body,
                        compose_state::publication_from_local(publish, &publish_at.get()),
                    ));
                }),
            );
            (
                disabled,
                disabled,
                Signal::derive(|| None::<InvalidSchedule>),
                on_click,
            )
        }
        EditPublicationState::Scheduled(state) | EditPublicationState::Live(state) => {
            published_submit_gate(body, also_blocked, state, on_submit)
        }
    }
}

/// The shared Scheduled/live arm of [`edit_submit_gate`].
fn published_submit_gate(
    body: Field<PostBody>,
    also_blocked: Signal<bool>,
    publication_time: PublicationTimeEditState,
    on_submit: Callback<(PostBody, PublicationIntent)>,
) -> (
    Signal<bool>,
    Signal<bool>,
    Signal<Option<InvalidSchedule>>,
    Callback<bool>,
) {
    let publication = Memo::new(move |_| publication_time.publication());
    let schedule_error = Signal::derive(move || publication.get().err());
    let unpublish_disabled = Signal::derive(move || also_blocked.get() || body.parsed().is_none());
    let save_disabled =
        Signal::derive(move || unpublish_disabled.get() || publication.get().is_err());
    let on_click = Callback::new(move |publish: bool| {
        if unpublish_disabled.get() {
            return;
        }
        let Some(body) = body.parsed() else {
            return;
        };
        let intent = if publish {
            let Ok(intent) = publication.get() else {
                return;
            };
            intent
        } else {
            PublicationIntent::Draft
        };
        on_submit.run((body, intent));
    });

    (save_disabled, unpublish_disabled, schedule_error, on_click)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forms::Field;
    use crate::posts::compose_state::PublicationIntent;
    use common::post_body::PostBody;
    use common::time::UtcInstant;

    fn instant(value: &str) -> UtcInstant {
        value.parse().unwrap()
    }

    fn saved(published_at: Option<UtcInstant>) -> SavedPost {
        SavedPost {
            post_id: 1_i64.into(),
            slug: "post".parse().unwrap(),
            published_at,
            permalink: "/~alice/2026/01/01/post".parse().unwrap(),
        }
    }

    #[test]
    fn loaded_publication_uses_the_server_snapshot_and_retains_the_instant() {
        let fetched_at = instant("2026-08-13T12:00:00Z");
        let live_at = instant("2026-08-13T11:59:59.123456789Z");
        assert_eq!(
            loaded_publication(None, fetched_at),
            LoadedPublication::Draft
        );
        assert_eq!(
            loaded_publication(Some(instant("2026-08-13T12:00:01Z")), fetched_at),
            LoadedPublication::Scheduled(instant("2026-08-13T12:00:01Z")),
        );
        assert_eq!(
            loaded_publication(Some(fetched_at), fetched_at),
            LoadedPublication::Live(fetched_at),
        );
        assert_eq!(
            loaded_publication(Some(live_at), fetched_at),
            LoadedPublication::Live(live_at),
        );
    }

    #[test]
    fn scheduled_publication_uses_the_fetch_snapshot_and_only_returns_future_instants() {
        let fetched_at = instant("2026-09-10T12:00:00Z");
        let scheduled_at = instant("2026-09-10T12:00:01Z");

        assert_eq!(scheduled_publication_at(None, fetched_at), None);
        assert_eq!(
            scheduled_publication_at(Some(fetched_at), fetched_at),
            None,
            "an instant due at fetch time is already live",
        );
        assert_eq!(
            scheduled_publication_at(Some(scheduled_at), fetched_at),
            Some(scheduled_at)
        );
    }

    #[test]
    fn lifecycle_state_changes_only_on_confirmed_settlements() {
        Owner::new().with(|| {
            let original = instant("2026-09-10T12:00:00Z");
            let loaded = EditPublicationState::from_loaded(
                LoadedPublication::Live(original),
                RwSignal::new(String::new()),
            );
            let draft_publish_at = RwSignal::new(String::new());
            let state = EditLifecycleState::new();

            state.adopt_settlement::<()>(&Ok(MutationOutcome::CommitIndeterminate(saved(None))));
            assert!(matches!(
                state.current_publication(loaded, draft_publish_at),
                EditPublicationState::Live(_)
            ));

            state.adopt_settlement::<()>(&Ok(MutationOutcome::Confirmed(saved(None))));
            assert!(matches!(
                state.current_publication(loaded, draft_publish_at),
                EditPublicationState::Draft(_)
            ));

            state.adopt_settlement(&Err::<MutationOutcome<SavedPost>, _>(()));
            assert!(matches!(
                state.current_publication(loaded, draft_publish_at),
                EditPublicationState::Draft(_)
            ));

            state.adopt_settlement::<()>(&Ok(MutationOutcome::Confirmed(saved(Some(original)))));
            assert!(matches!(
                state.current_publication(loaded, draft_publish_at),
                EditPublicationState::Live(_)
            ));
        });
    }

    #[test]
    fn draft_gate_retains_create_form_publication_choices() {
        Owner::new().with(|| {
            let body = Field::<PostBody>::new();
            body.set_value("body");
            let seen = RwSignal::new(None);
            let publication = EditPublicationState::from_loaded(
                LoadedPublication::Draft,
                RwSignal::new("2999-02-03T10:15".to_owned()),
            );
            let (save_disabled, _, schedule_error, click) = edit_submit_gate(
                body,
                Signal::derive(|| false),
                publication,
                Callback::new(move |(_, intent)| seen.set(Some(intent))),
            );

            assert!(!save_disabled.get());
            assert_eq!(schedule_error.get(), None);
            click.run(false);
            assert_eq!(seen.get(), Some(PublicationIntent::Draft));
            click.run(true);
            assert!(matches!(seen.get(), Some(PublicationIntent::PublishAt(_))));
        });
    }

    #[test]
    fn untouched_live_time_preserves_the_exact_original_instant() {
        Owner::new().with(|| {
            let original = instant("2026-11-01T05:30:00.123456789Z");
            let publication = EditPublicationState::from_loaded(
                LoadedPublication::Live(original),
                RwSignal::new(String::new()),
            );
            let state = publication
                .publication_time()
                .expect("live Posts have editable publication time");

            assert_eq!(state.value.get(), time::local_datetime_from_utc(original));
            assert_eq!(
                state.publication(),
                Ok(PublicationIntent::PublishAt(original))
            );
            assert_eq!(publication.loaded(), LoadedPublication::Live(original));
        });
    }

    #[test]
    fn edited_publication_time_accepts_backdates_and_rejects_empty_or_invalid_input() {
        Owner::new().with(|| {
            let state = PublicationTimeEditState::new(
                instant("2999-01-01T09:00:00Z"),
                "2999-01-01T09:00".into(),
            );
            state.set_input(String::new());
            assert_eq!(state.publication(), Err(InvalidSchedule));
            state.set_input("not-a-date".into());
            assert_eq!(state.publication(), Err(InvalidSchedule));
            state.set_input("2020-03-05T12:00".into());
            assert!(matches!(
                state.publication(),
                Ok(PublicationIntent::PublishAt(_))
            ));
        });
    }

    #[test]
    fn invalid_time_blocks_save_but_not_atomic_unpublish() {
        Owner::new().with(|| {
            let publication_time = PublicationTimeEditState::new(
                instant("2999-01-01T09:00:00Z"),
                "2999-01-01T09:00".into(),
            );
            publication_time.set_input(String::new());
            let body = Field::<PostBody>::new();
            body.set_value("edited body");
            let seen = RwSignal::new(None);
            let (save_disabled, unpublish_disabled, schedule_error, click) = published_submit_gate(
                body,
                Signal::derive(|| false),
                publication_time,
                Callback::new(move |(body, intent)| {
                    seen.set(Some((body, intent)));
                }),
            );

            assert!(save_disabled.get());
            assert!(!unpublish_disabled.get());
            assert_eq!(schedule_error.get(), Some(InvalidSchedule));
            click.run(true);
            assert!(seen.get().is_none(), "invalid Save must not dispatch");
            click.run(false);
            let (body, intent) = seen.get().expect("Unpublish must dispatch");
            assert_eq!(body.as_ref(), "edited body");
            assert_eq!(intent, PublicationIntent::Draft);
        });
    }

    #[test]
    fn published_gate_blocks_both_actions_when_persisted_fields_are_invalid() {
        Owner::new().with(|| {
            let publication_time = PublicationTimeEditState::new(
                instant("2999-01-01T09:00:00Z"),
                "2999-01-01T09:00".into(),
            );
            let body = Field::<PostBody>::new();
            let ran = RwSignal::new(false);
            let (save_disabled, unpublish_disabled, _, click) = published_submit_gate(
                body,
                Signal::derive(|| false),
                publication_time,
                Callback::new(move |_| ran.set(true)),
            );

            assert!(save_disabled.get());
            assert!(unpublish_disabled.get());
            click.run(true);
            click.run(false);
            assert!(!ran.get());
        });
    }
}
