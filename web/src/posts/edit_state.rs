use common::post_body::PostBody;
use common::time::{self, UtcInstant};
use leptos::prelude::*;
use thiserror::Error;

use crate::forms::Field;
use crate::posts::compose_state::{PublicationIntent, publication_from_local, submit_gate};

/// The publication state captured when the editor response was assembled.
///
/// Classification is immutable for the loaded editor: a scheduled Post that
/// becomes due while the page is open remains a scheduled edit, avoiding a
/// browser-clock race in both controls and payload construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadedPublication {
    Draft,
    Scheduled(UtcInstant),
    Live,
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
        Some(_) => LoadedPublication::Live,
    }
}

/// A scheduled editor's local display value and exact original UTC instant.
///
/// The original remains authoritative until the author edits the control. This
/// preserves seconds, nanoseconds, and the selected instant in a repeated DST
/// wall-clock interval.
#[derive(Clone, Copy)]
pub struct ScheduledEditState {
    pub value: RwSignal<String>,
    original: UtcInstant,
    edited: RwSignal<bool>,
}

/// The complete publication branch and branch-specific signals for one loaded editor.
///
/// Keeping the schedule signal inside the enum makes an impossible
/// `Scheduled-without-ScheduledEditState` combination unrepresentable.
#[derive(Clone, Copy)]
pub enum EditPublicationState {
    Draft(RwSignal<String>),
    Scheduled(ScheduledEditState),
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Enter a valid local date and time")]
pub struct InvalidSchedule;

impl ScheduledEditState {
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

    pub fn clear(&self) {
        self.set_input(String::new());
    }

    /// # Errors
    ///
    /// Returns [`InvalidSchedule`] after the author changes the control to a
    /// non-empty local wall-clock value that does not identify a real instant.
    pub fn publication(&self) -> Result<PublicationIntent, InvalidSchedule> {
        if !self.edited.get() {
            return Ok(PublicationIntent::PublishAt(self.original));
        }

        let value = self.value.get();
        if value.trim().is_empty() {
            return Ok(PublicationIntent::Draft);
        }

        time::strict_utc_instant_from_local(&value)
            .map(PublicationIntent::PublishAt)
            .ok_or(InvalidSchedule)
    }
}

impl EditPublicationState {
    #[must_use]
    pub fn from_loaded(loaded: LoadedPublication, draft_publish_at: RwSignal<String>) -> Self {
        match loaded {
            LoadedPublication::Draft => Self::Draft(draft_publish_at),
            LoadedPublication::Scheduled(original) => Self::Scheduled(ScheduledEditState::new(
                original,
                time::local_datetime_from_utc(original),
            )),
            LoadedPublication::Live => Self::Live,
        }
    }

    #[must_use]
    pub fn loaded(self) -> LoadedPublication {
        match self {
            Self::Draft(_) => LoadedPublication::Draft,
            Self::Scheduled(schedule) => LoadedPublication::Scheduled(schedule.original),
            Self::Live => LoadedPublication::Live,
        }
    }

    #[must_use]
    pub fn scheduled(self) -> Option<ScheduledEditState> {
        match self {
            Self::Scheduled(schedule) => Some(schedule),
            Self::Draft(_) | Self::Live => None,
        }
    }
}

/// Bind every loaded editor branch to one derived payload and dispatch callback.
///
/// Draft buttons map through the existing create-form converter, live Save always
/// maps to `PublishNow`, and scheduled Save uses the exact-preserving strict state.
/// The component receives one uniform callback and never selects persisted intent.
#[must_use]
pub fn edit_submit_gate(
    body: Field<PostBody>,
    also_blocked: Signal<bool>,
    publication: EditPublicationState,
    on_submit: Callback<(PostBody, PublicationIntent)>,
) -> (
    Signal<bool>,
    Signal<Option<InvalidSchedule>>,
    Callback<bool>,
) {
    match publication {
        EditPublicationState::Draft(publish_at) => {
            let (disabled, on_click) = submit_gate(
                body,
                also_blocked,
                Callback::new(move |(body, publish): (PostBody, bool)| {
                    on_submit.run((body, publication_from_local(publish, &publish_at.get())));
                }),
            );
            (
                disabled,
                Signal::derive(|| None::<InvalidSchedule>),
                on_click,
            )
        }
        EditPublicationState::Scheduled(schedule) => {
            let (disabled, schedule_error, on_click) =
                scheduled_submit_gate(body, also_blocked, schedule, on_submit);
            (
                disabled,
                schedule_error,
                Callback::new(move |_: bool| on_click.run(())),
            )
        }
        EditPublicationState::Live => {
            let (disabled, on_click) = submit_gate(
                body,
                also_blocked,
                Callback::new(move |(body, _): (PostBody, bool)| {
                    on_submit.run((body, PublicationIntent::PublishNow));
                }),
            );
            (
                disabled,
                Signal::derive(|| None::<InvalidSchedule>),
                on_click,
            )
        }
    }
}

/// The scheduled arm of [`edit_submit_gate`].
fn scheduled_submit_gate(
    body: Field<PostBody>,
    also_blocked: Signal<bool>,
    schedule: ScheduledEditState,
    on_submit: Callback<(PostBody, PublicationIntent)>,
) -> (Signal<bool>, Signal<Option<InvalidSchedule>>, Callback<()>) {
    let publication = Memo::new(move |_| schedule.publication());
    let schedule_error = Signal::derive(move || publication.get().err());
    let payload = Memo::new(move |_| {
        if also_blocked.get() {
            return None;
        }
        Some((body.parsed()?, publication.get().ok()?))
    });
    let disabled = Signal::derive(move || payload.get().is_none());
    let on_click = Callback::new(move |()| {
        if let Some(payload) = payload.get() {
            on_submit.run(payload);
        }
    });

    (disabled, schedule_error, on_click)
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

    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    #[test]
    fn loaded_publication_uses_the_server_snapshot() {
        let fetched_at = instant("2026-08-13T12:00:00Z");
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
            LoadedPublication::Live,
        );
        assert_eq!(
            loaded_publication(Some(instant("2026-08-13T11:59:59Z")), fetched_at),
            LoadedPublication::Live,
        );
    }
    #[test]
    fn edit_submit_gate_routes_all_loaded_publication_states() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            body.set_input("body");
            let seen = RwSignal::new(None);
            let draft_schedule = RwSignal::new("2999-02-03T10:15".to_owned());
            let draft = EditPublicationState::from_loaded(LoadedPublication::Draft, draft_schedule);
            assert_eq!(draft.loaded(), LoadedPublication::Draft);
            assert!(draft.scheduled().is_none());
            let (disabled, schedule_error, click) = edit_submit_gate(
                body,
                Signal::derive(|| false),
                draft,
                Callback::new(move |(_, intent)| seen.set(Some(intent))),
            );
            assert!(!disabled.get());
            assert_eq!(schedule_error.get(), None);
            click.run(false);
            assert_eq!(seen.get(), Some(PublicationIntent::Draft));
            click.run(true);
            assert!(matches!(seen.get(), Some(PublicationIntent::PublishAt(_))));

            let original = instant("2999-11-03T05:30:00.123456789Z");
            let scheduled = EditPublicationState::from_loaded(
                LoadedPublication::Scheduled(original),
                RwSignal::new(String::new()),
            );
            assert_eq!(scheduled.loaded(), LoadedPublication::Scheduled(original));
            assert!(scheduled.scheduled().is_some());
            let (_, _, click) = edit_submit_gate(
                body,
                Signal::derive(|| false),
                scheduled,
                Callback::new(move |(_, intent)| seen.set(Some(intent))),
            );
            click.run(true);
            assert_eq!(seen.get(), Some(PublicationIntent::PublishAt(original)));

            let live = EditPublicationState::from_loaded(
                LoadedPublication::Live,
                RwSignal::new(String::new()),
            );
            assert_eq!(live.loaded(), LoadedPublication::Live);
            assert!(live.scheduled().is_none());
            let (_, _, click) = edit_submit_gate(
                body,
                Signal::derive(|| false),
                live,
                Callback::new(move |(_, intent)| seen.set(Some(intent))),
            );
            click.run(false);
            assert_eq!(seen.get(), Some(PublicationIntent::PublishNow));
        });
    }

    #[test]
    fn untouched_schedule_preserves_the_exact_original_instant() {
        with_owner(|| {
            let original = instant("2026-11-01T05:30:00.123456789Z");
            let state = ScheduledEditState::new(original, "2026-11-01T01:30".into());
            assert_eq!(state.value.get(), "2026-11-01T01:30");
            assert_eq!(
                state.publication(),
                Ok(PublicationIntent::PublishAt(original))
            );
        });
    }

    #[test]
    fn clear_is_local_and_maps_only_to_draft() {
        with_owner(|| {
            let state =
                ScheduledEditState::new(instant("2999-01-01T09:00:00Z"), "2999-01-01T09:00".into());
            state.clear();
            assert_eq!(state.value.get(), "");
            assert_eq!(state.publication(), Ok(PublicationIntent::Draft));
        });
    }

    #[test]
    fn edited_schedule_uses_the_parser_and_rejects_invalid_nonempty_input() {
        with_owner(|| {
            let state =
                ScheduledEditState::new(instant("2999-01-01T09:00:00Z"), "2999-01-01T09:00".into());
            state.set_input("not-a-date".into());
            assert_eq!(state.publication(), Err(InvalidSchedule));
            state.set_input("2999-02-03T10:15".into());
            assert!(matches!(
                state.publication(),
                Ok(PublicationIntent::PublishAt(_))
            ));
            state.set_input("2020-03-05T12:00".into());
            assert!(
                matches!(state.publication(), Ok(PublicationIntent::PublishAt(_))),
                "a valid past value is a backdate, not an editor validation error",
            );
        });
    }

    #[test]
    fn scheduled_gate_dispatches_the_untouched_original_without_reparsing() {
        with_owner(|| {
            let original = instant("2026-11-01T05:30:00.123456789Z");
            let schedule = ScheduledEditState::new(original, "2026-11-01T01:30".into());
            let body = Field::<PostBody>::new();
            body.set_input("body");
            let seen = RwSignal::new(None);
            let (disabled, schedule_error, click) = scheduled_submit_gate(
                body,
                Signal::derive(|| false),
                schedule,
                Callback::new(move |(_, intent)| seen.set(Some(intent))),
            );

            assert!(!disabled.get());
            assert_eq!(schedule_error.get(), None);
            click.run(());
            assert_eq!(seen.get(), Some(PublicationIntent::PublishAt(original)));
        });
    }

    #[test]
    fn scheduled_gate_dispatches_draft_after_clear() {
        with_owner(|| {
            let schedule =
                ScheduledEditState::new(instant("2999-01-01T09:00:00Z"), "2999-01-01T09:00".into());
            schedule.clear();
            let body = Field::<PostBody>::new();
            body.set_input("body");
            let seen = RwSignal::new(None);
            let (disabled, schedule_error, click) = scheduled_submit_gate(
                body,
                Signal::derive(|| false),
                schedule,
                Callback::new(move |(_, intent)| seen.set(Some(intent))),
            );

            assert!(!disabled.get());
            assert_eq!(schedule_error.get(), None);
            click.run(());
            assert_eq!(seen.get(), Some(PublicationIntent::Draft));
        });
    }

    #[test]
    fn scheduled_gate_blocks_invalid_input_and_dispatches_nothing() {
        with_owner(|| {
            let schedule =
                ScheduledEditState::new(instant("2999-01-01T09:00:00Z"), "2999-01-01T09:00".into());
            schedule.set_input("not-a-date".into());
            let body = Field::<PostBody>::new();
            body.set_input("body");
            let ran = RwSignal::new(false);
            let (disabled, schedule_error, click) = scheduled_submit_gate(
                body,
                Signal::derive(|| false),
                schedule,
                Callback::new(move |_| ran.set(true)),
            );

            assert!(disabled.get());
            assert_eq!(schedule_error.get(), Some(InvalidSchedule));
            click.run(());
            assert!(!ran.get());
        });
    }

    #[test]
    fn scheduled_gate_blocks_body_and_caller_predicate() {
        with_owner(|| {
            let schedule =
                ScheduledEditState::new(instant("2999-01-01T09:00:00Z"), "2999-01-01T09:00".into());
            let body = Field::<PostBody>::new();
            let blocked = RwSignal::new(false);
            let ran = RwSignal::new(0_u32);
            let (disabled, schedule_error, click) = scheduled_submit_gate(
                body,
                Signal::derive(move || blocked.get()),
                schedule,
                Callback::new(move |_| ran.update(|count| *count += 1)),
            );

            assert!(disabled.get(), "blank body blocks");
            click.run(());
            assert_eq!(ran.get(), 0);

            body.set_input("body");
            blocked.set(true);
            assert!(disabled.get(), "the caller predicate blocks");
            click.run(());
            assert_eq!(ran.get(), 0);

            blocked.set(false);
            assert!(!disabled.get());
            assert_eq!(schedule_error.get(), None);
            click.run(());
            assert_eq!(ran.get(), 1);
        });
    }
}
