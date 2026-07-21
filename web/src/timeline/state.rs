//! Timeline pagination — the pure, host-tested value model (ADR-0070 §6): the
//! `TimelineCursor` newtype, the `LoadStatus` enum, and the row-fold helper. The
//! reactive `TimelineState` that wraps these in signals lives in the wasm-only
//! `component.rs`; everything here is ungated and coverage-measured.

use common::ids::PostId;
use common::time::UtcInstant;

use crate::posts::TimelinePage;

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
    /// `LoadStatus` from `read_signal!` — can return the `String` directly
    /// instead of re-matching the `Failed` arm inline.
    #[must_use]
    pub fn into_failure(self) -> Option<String> {
        match self {
            Self::Failed(message) => Some(message),
            Self::Idle | Self::InFlight => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant() -> UtcInstant {
        "2026-07-19T10:30:00Z".parse().unwrap()
    }

    fn page(
        next_cursor_created_at: Option<UtcInstant>,
        next_cursor_post_id: Option<PostId>,
        has_more: bool,
    ) -> TimelinePage {
        TimelinePage {
            posts: Vec::new(),
            next_cursor_created_at,
            next_cursor_post_id,
            has_more,
        }
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
