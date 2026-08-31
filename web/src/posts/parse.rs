//! Pure, host-testable logic for the **posts** vertical (ADR-0070 §6, ADR-0055):
//! the permalink route-param decoder and the draft-row title/schedule-badge
//! computation, extracted out of the wasm-only [`super::component`] page
//! components so they stay host-compiled, host-tested, and coverage-measured (an
//! "extra leaf" beside `mod`/`api`/`server`/`component`, like [`super::render`]).
//! The components call these fns and wrap the returned plain data in `view!`
//! markup; the `#[cfg(test)] mod tests` below pin the valid and edge cases.

use crate::posts::UnpublishedPost;
use common::permalink_route::PermalinkRoute;

/// Normalizes Leptos's optional route captures before delegating semantic parsing
/// to [`PermalinkRoute`]. The `~` is router syntax, not part of the username.
#[must_use]
pub fn parse_permalink_route(
    username: Option<&str>,
    year: Option<&str>,
    month: Option<&str>,
    day: Option<&str>,
    slug: Option<&str>,
) -> Option<PermalinkRoute> {
    PermalinkRoute::parse(username?.strip_prefix('~')?, year?, month?, day?, slug?)
}

/// Presentational data for one draft row, computed by [`draft_row_display`] so the
/// wasm-only component keeps only its `view!` markup.
pub struct DraftRowDisplay {
    /// The row's displayed title: the post title if present, else the summary label.
    pub label: String,
    /// "Scheduled for …" badge text when the post is scheduled (a future
    /// `published_at`); `None` for a true draft.
    pub scheduled_badge: Option<String>,
}

/// Compute the displayed title and the scheduled-badge text for a draft row. A
/// scheduled post gets a badge marking it distinctly from a true draft on this
/// shared "not-yet-live" surface.
pub fn draft_row_display(draft: &UnpublishedPost) -> DraftRowDisplay {
    let label = draft
        .title
        .clone()
        .map_or_else(|| draft.summary_label.to_string(), String::from);
    // `list_drafts` only returns true drafts (`published_at` NULL) and scheduled
    // posts (`published_at` in the future), so a `Some` here is necessarily a
    // scheduled time — that is what makes the badge text correct.
    let scheduled_badge = draft
        .post
        .published_at
        .map(|when| format!("Scheduled for {when}"));
    DraftRowDisplay {
        label,
        scheduled_badge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts::SavedPost;
    use common::ids::PostId;
    use common::test_support::{
        parse_post_summary, parse_post_title, parse_root_relative_url, parse_slug,
        parse_utc_instant,
    };

    #[test]
    fn strips_the_client_permalink_marker_once() {
        let route = parse_permalink_route(
            Some("~alice"),
            Some("2026"),
            Some("01"),
            Some("02"),
            Some("hello"),
        )
        .expect("the adapter removes the router marker before common parsing");

        assert_eq!(route.username, "alice");
        assert_eq!(
            parse_permalink_route(
                Some("~~alice"),
                Some("2026"),
                Some("01"),
                Some("02"),
                Some("hello"),
            ),
            None
        );
    }

    #[test]
    fn requires_every_router_capture() {
        assert_eq!(
            parse_permalink_route(
                Some("~alice"),
                Some("2026"),
                None,
                Some("02"),
                Some("hello")
            ),
            None
        );
        assert_eq!(
            parse_permalink_route(
                Some("alice"),
                Some("2026"),
                Some("01"),
                Some("02"),
                Some("hello"),
            ),
            None
        );
    }

    fn draft(title: Option<&str>, scheduled: Option<&str>) -> UnpublishedPost {
        UnpublishedPost {
            post: SavedPost {
                post_id: PostId::from(1),
                slug: parse_slug("my-post"),
                published_at: scheduled.map(parse_utc_instant),
                permalink: parse_root_relative_url("/~alice/2026/01/01/my-post"),
            },
            title: title.map(parse_post_title),
            summary_label: parse_post_summary("fallback label"),
            edit_url: parse_root_relative_url("/posts/1/edit"),
        }
    }

    #[test]
    fn draft_row_uses_title_when_present() {
        let row = draft_row_display(&draft(Some("My Title"), None));
        assert_eq!(row.label, "My Title");
        assert_eq!(row.scheduled_badge, None);
    }

    #[test]
    fn draft_row_falls_back_to_summary_label_when_untitled() {
        let row = draft_row_display(&draft(None, None));
        assert_eq!(row.label, "fallback label");
        assert_eq!(row.scheduled_badge, None);
    }

    #[test]
    fn draft_row_scheduled_post_gets_badge_text() {
        let row = draft_row_display(&draft(Some("Scheduled Post"), Some("2099-06-15T12:00:00Z")));
        let badge = row
            .scheduled_badge
            .expect("a scheduled post carries a badge");
        assert!(badge.starts_with("Scheduled for "), "badge text: {badge}");
    }
}
