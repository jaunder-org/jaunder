//! Timeline pagination — the host-tested model (ADR-0070 §6): the
//! `TimelineCursor` newtype, the `LoadStatus` enum, and the reactive
//! `TimelineState` signal bundle that wraps them. Everything here is ungated and
//! coverage-measured; the signal bundle is exercised under a reactive `Owner`
//! (the `web::reactive` / `forms::Field` / `tags::input_state` convention), which
//! is what makes its transitions testable at all — they were invisible to the
//! coverage gate while the bundle lived in the wasm-only `component.rs` (#671).
//!
//! `component.rs` keeps only what cannot run on the host: `Effect::new` and
//! `spawn_local`.

use leptos::prelude::*;

use common::ids::PostId;
use common::seed::{TimelinePage, TimelinePostSummary};
use common::time::UtcInstant;

/// A keyset pagination cursor: the `(created_at, post_id)` pair a timeline page
/// hands back to fetch the next page. Bundling the two — which always move
/// together — makes "one set, the other not" unrepresentable (they were two
/// independent `Option` signals before #329).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineCursor {
    pub created_at: UtcInstant,
    pub post_id: PostId,
}

impl TimelineCursor {
    /// Build a cursor from a page's flat next-cursor fields: `Some` only when
    /// **both** components are present. A partial pair (which the server never
    /// emits) collapses to `None` rather than a half-cursor.
    #[must_use]
    pub fn from_page(page: &TimelinePage) -> Option<Self> {
        match (page.next_cursor_created_at, page.next_cursor_post_id) {
            (Some(created_at), Some(post_id)) => Some(Self {
                created_at,
                post_id,
            }),
            _ => None,
        }
    }

    /// Split an optional cursor into the `(created_at, post_id)` optionals a
    /// timeline list fn takes — `(None, None)` when there is no cursor. Keeps the
    /// pairing logic host-tested and out of the wasm-only paginator.
    #[must_use]
    pub fn into_query(cursor: Option<Self>) -> (Option<UtcInstant>, Option<PostId>) {
        match cursor {
            Some(c) => (Some(c.created_at), Some(c.post_id)),
            None => (None, None),
        }
    }
}

/// The load state of a timeline: idle, a load-more in flight, or a failed fetch
/// carrying its display message. Replaces the old `loading_more: bool` +
/// `error: Option<String>` pair, which admitted the illegal "loading *and*
/// errored" combination.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LoadStatus {
    #[default]
    Idle,
    InFlight,
    Failed(String),
}

impl LoadStatus {
    /// Whether a load-more is in flight (drives the button's disabled state).
    #[must_use]
    pub fn is_in_flight(&self) -> bool {
        matches!(self, Self::InFlight)
    }

    /// Consume the status into the failure message to display, if the last load
    /// failed. Owned (`self`) so the reactive callers — which hold a cloned
    /// `LoadStatus` from the status signal's `.get()` — can return the `String`
    /// directly instead of re-matching the `Failed` arm inline.
    #[must_use]
    pub fn into_failure(self) -> Option<String> {
        match self {
            Self::Failed(message) => Some(message),
            Self::Idle | Self::InFlight => None,
        }
    }
}

/// The reactive state of a cursor-paginated timeline, shared by the public Local
/// timeline (`home.rs`) and the authed `/app` cockpit (`cockpit.rs`).
///
/// Every field is an `RwSignal` (a `Copy` handle into the reactive runtime), so the
/// whole struct is `Copy` and can be handed to each event closure and child callback
/// without per-signal capture.
#[derive(Clone, Copy, Default)]
pub struct TimelineState {
    pub rows: RwSignal<Vec<TimelinePostSummary>>,
    pub cursor: RwSignal<Option<TimelineCursor>>,
    pub has_more: RwSignal<bool>,
    pub status: RwSignal<LoadStatus>,
}

impl TimelineState {
    /// Adopt a page's rows + cursor (a projector seed or a fresh fetch),
    /// replacing what's shown.
    pub fn adopt(&self, page: TimelinePage) {
        self.cursor.set(TimelineCursor::from_page(&page));
        self.has_more.set(page.has_more);
        self.rows.set(page.posts);
    }

    /// Resolve a re-fetch into the signals and settle to idle (clearing any prior
    /// failure). wasm-only: re-fetches resolve on the client, in the page's
    /// client-side `Effect`.
    pub fn resolve(&self, page: TimelinePage) {
        self.adopt(page);
        self.status.set(LoadStatus::Idle);
    }

    /// Record a fetch failure: empty the rows (don't show a stale page), clear
    /// the cursor + `has_more` so a failed timeline offers no "Load more", and
    /// mark the failure for display.
    pub fn fail(&self, message: String) {
        self.rows.set(Vec::new());
        self.cursor.set(None);
        self.has_more.set(false);
        self.status.set(LoadStatus::Failed(message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts::render::test_fixtures::sample_summary;

    fn instant() -> UtcInstant {
        "2026-07-19T10:30:00Z".parse().unwrap()
    }

    fn page_with(
        posts: Vec<TimelinePostSummary>,
        next_cursor_created_at: Option<UtcInstant>,
        next_cursor_post_id: Option<PostId>,
        has_more: bool,
    ) -> TimelinePage {
        TimelinePage {
            posts,
            next_cursor_created_at,
            next_cursor_post_id,
            has_more,
        }
    }

    fn page(
        next_cursor_created_at: Option<UtcInstant>,
        next_cursor_post_id: Option<PostId>,
        has_more: bool,
    ) -> TimelinePage {
        page_with(
            Vec::new(),
            next_cursor_created_at,
            next_cursor_post_id,
            has_more,
        )
    }

    /// Run `body` under a fresh reactive `Owner` (the `web::reactive` /
    /// `forms::Field` / `tags::input_state` convention), so `RwSignal`s work
    /// host-side without a browser.
    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    #[test]
    fn cursor_from_page_needs_both_components() {
        assert_eq!(
            TimelineCursor::from_page(&page(Some(instant()), Some(PostId::from(7)), true)),
            Some(TimelineCursor {
                created_at: instant(),
                post_id: PostId::from(7)
            }),
        );
        assert_eq!(TimelineCursor::from_page(&page(None, None, false)), None);
        assert_eq!(
            TimelineCursor::from_page(&page(Some(instant()), None, true)),
            None
        );
        assert_eq!(
            TimelineCursor::from_page(&page(None, Some(PostId::from(7)), true)),
            None
        );
    }

    #[test]
    fn cursor_into_query_splits_or_empties() {
        let cursor = TimelineCursor {
            created_at: instant(),
            post_id: PostId::from(7),
        };
        assert_eq!(
            TimelineCursor::into_query(Some(cursor)),
            (Some(instant()), Some(PostId::from(7))),
        );
        assert_eq!(TimelineCursor::into_query(None), (None, None));
    }

    #[test]
    fn default_state_is_empty_and_idle() {
        with_owner(|| {
            let state = TimelineState::default();
            assert!(state.rows.get().is_empty());
            assert_eq!(state.cursor.get(), None);
            assert!(!state.has_more.get());
            assert_eq!(state.status.get(), LoadStatus::Idle);
        });
    }

    #[test]
    fn adopt_replaces_rows_cursor_and_has_more() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(
                vec![sample_summary()],
                Some(instant()),
                Some(PostId::from(7)),
                true,
            ));
            assert_eq!(state.rows.get().len(), 1);
            assert_eq!(
                state.cursor.get(),
                Some(TimelineCursor {
                    created_at: instant(),
                    post_id: PostId::from(7)
                })
            );
            assert!(state.has_more.get());
        });
    }

    #[test]
    fn resolve_adopts_and_clears_a_prior_failure() {
        with_owner(|| {
            let state = TimelineState::default();
            state.fail("boom".to_owned());
            state.resolve(page_with(vec![sample_summary()], None, None, false));
            assert_eq!(state.rows.get().len(), 1);
            assert_eq!(state.status.get(), LoadStatus::Idle, "failure cleared");
        });
    }

    #[test]
    fn fail_empties_the_timeline_and_records_the_message() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(
                vec![sample_summary()],
                Some(instant()),
                Some(PostId::from(7)),
                true,
            ));
            state.fail("boom".to_owned());
            assert!(state.rows.get().is_empty(), "no stale page");
            assert_eq!(state.cursor.get(), None);
            assert!(
                !state.has_more.get(),
                "a failed timeline offers no Load more"
            );
            assert_eq!(state.status.get(), LoadStatus::Failed("boom".to_owned()));
        });
    }

    #[test]
    fn load_status_accessors_cover_each_arm() {
        assert!(!LoadStatus::Idle.is_in_flight());
        assert!(LoadStatus::InFlight.is_in_flight());
        assert!(!LoadStatus::Failed("boom".into()).is_in_flight());

        assert_eq!(LoadStatus::Idle.into_failure(), None);
        assert_eq!(LoadStatus::InFlight.into_failure(), None);
        assert_eq!(
            LoadStatus::Failed("boom".into()).into_failure(),
            Some("boom".to_owned())
        );
    }
}
