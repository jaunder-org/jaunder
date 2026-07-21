//! Pure, host-testable logic for the **posts** vertical (ADR-0070 §6, ADR-0055):
//! the permalink route-param decoder and the draft-row title/schedule-badge
//! computation, extracted out of the wasm-only [`super::component`] page
//! components so they stay host-compiled, host-tested, and coverage-measured (an
//! "extra leaf" beside `mod`/`api`/`server`/`component`, like [`super::render`]).
//! The components call these fns and wrap the returned plain data in `view!`
//! markup; the `#[cfg(test)] mod tests` below pin the valid and edge cases.

use crate::posts::DraftSummary;
use common::slug::Slug;
use common::username::Username;

/// Decode the `~username`/`year`/`month`/`day`/`slug` permalink route params into
/// typed values, mirroring the client-side parse `PostPage` performs before it
/// fetches (ADR-0063 §4). A segment that is not a `~username` yields `None` (a
/// non-permalink URL the caller reloads for the server to handle); a `~`-prefixed
/// URL whose slug won't parse names no real post, so `slug` is `None` and the
/// caller 404s client-side without a round-trip. `year`/`month`/`day` fall back to
/// `0` on absence or parse failure, as the original inline closure did.
pub fn parse_permalink_params(
    username: Option<&str>,
    year: Option<&str>,
    month: Option<&str>,
    day: Option<&str>,
    slug: Option<&str>,
) -> (Option<Username>, i32, u32, u32, Option<Slug>) {
    let username = username
        .unwrap_or_default()
        .strip_prefix('~')
        .and_then(|s| s.parse::<Username>().ok());
    let year = year.and_then(|v| v.parse::<i32>().ok()).unwrap_or_default();
    let month = month
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or_default();
    let day = day.and_then(|v| v.parse::<u32>().ok()).unwrap_or_default();
    let slug = slug.and_then(|s| s.parse::<Slug>().ok());
    (username, year, month, day, slug)
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
/// scheduled post (future `published_at`) carries `scheduled_at` and gets a badge
/// marking it distinctly from a true draft on this shared "not-yet-live" surface.
pub fn draft_row_display(draft: &DraftSummary) -> DraftRowDisplay {
    let label = draft
        .title
        .clone()
        .map_or_else(|| draft.summary_label.to_string(), String::from);
    let scheduled_badge = draft
        .scheduled_at
        .map(|when| format!("Scheduled for {when}"));
    DraftRowDisplay {
        label,
        scheduled_badge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::ids::PostId;
    use common::test_support::{
        parse_post_summary, parse_post_title, parse_slug, parse_username, parse_utc_instant,
    };

    #[test]
    fn parses_valid_permalink_params() {
        let (username, year, month, day, slug) = parse_permalink_params(
            Some("~alice"),
            Some("2026"),
            Some("01"),
            Some("02"),
            Some("hello"),
        );
        assert_eq!(username, Some(parse_username("alice")));
        assert_eq!(year, 2026);
        assert_eq!(month, 1);
        assert_eq!(day, 2);
        assert_eq!(slug, Some(parse_slug("hello")));
    }

    #[test]
    fn username_without_tilde_is_none() {
        // A segment that isn't a `~username` (e.g. a server-handled URL) is not a
        // permalink author, so the caller reloads for the server to handle it.
        let (username, ..) = parse_permalink_params(
            Some("alice"),
            Some("2026"),
            Some("01"),
            Some("02"),
            Some("hello"),
        );
        assert_eq!(username, None);
    }

    #[test]
    fn missing_segments_are_absent_or_zero() {
        let (username, year, month, day, slug) =
            parse_permalink_params(None, None, None, None, None);
        assert_eq!(username, None);
        assert_eq!(year, 0);
        assert_eq!(month, 0);
        assert_eq!(day, 0);
        assert_eq!(slug, None);
    }

    #[test]
    fn unparseable_date_segments_default_to_zero() {
        let (_, year, month, day, _) = parse_permalink_params(
            Some("~alice"),
            Some("nope"),
            None,
            Some("xx"),
            Some("hello"),
        );
        assert_eq!(year, 0);
        assert_eq!(month, 0);
        assert_eq!(day, 0);
    }

    #[test]
    fn unparseable_slug_is_none() {
        // A '~'-prefixed permalink with an invalid slug names no real post.
        let (username, _, _, _, slug) = parse_permalink_params(
            Some("~alice"),
            Some("2026"),
            Some("01"),
            Some("02"),
            Some("Not A Slug!"),
        );
        assert_eq!(username, Some(parse_username("alice")));
        assert_eq!(slug, None);
    }

    fn draft(title: Option<&str>, scheduled: Option<&str>) -> DraftSummary {
        DraftSummary {
            post_id: PostId::from(1),
            title: title.map(parse_post_title),
            summary_label: parse_post_summary("fallback label"),
            slug: parse_slug("my-post"),
            created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            updated_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            scheduled_at: scheduled.map(parse_utc_instant),
            preview_url: "/draft/1/preview".to_string(),
            edit_url: "/posts/1/edit".to_string(),
            permalink: "/~alice/2026/01/01/my-post".to_string(),
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
