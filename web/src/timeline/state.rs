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

    /// The failure message to display, if the last load failed.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message),
            _ => None,
        }
    }
}

/// Whether a fetched page replaces the current rows (a seed or re-fetch) or is
/// appended to them (load-more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMode {
    Replace,
    Append,
}

/// Fold a fetched page's rows into the rows already shown. `Replace` swaps them
/// (first paint / re-fetch); `Append` extends them (load-more). Generic so the
/// merge is host-testable without constructing `TimelinePostSummary` fixtures.
#[must_use]
pub fn apply_rows<T>(current: Vec<T>, incoming: Vec<T>, mode: PageMode) -> Vec<T> {
    match mode {
        PageMode::Replace => incoming,
        PageMode::Append => {
            let mut current = current;
            current.extend(incoming);
            current
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
    fn load_status_accessors_cover_each_arm() {
        assert!(!LoadStatus::Idle.is_in_flight());
        assert!(LoadStatus::InFlight.is_in_flight());
        assert!(!LoadStatus::Failed("boom".into()).is_in_flight());

        assert_eq!(LoadStatus::Idle.error_message(), None);
        assert_eq!(LoadStatus::InFlight.error_message(), None);
        assert_eq!(
            LoadStatus::Failed("boom".into()).error_message(),
            Some("boom")
        );
    }

    #[test]
    fn apply_rows_replaces_or_appends() {
        assert_eq!(
            apply_rows(vec![1, 2], vec![3, 4], PageMode::Replace),
            vec![3, 4]
        );
        assert_eq!(
            apply_rows(vec![1, 2], vec![3, 4], PageMode::Append),
            vec![1, 2, 3, 4]
        );
    }
}
