//! Pure, host-testable logic for the **posts** vertical (ADR-0070 §6, ADR-0055):
//! the permalink route-param decoder and the draft-row title/schedule-badge
//! computation, extracted out of the wasm-only [`super::component`] page
//! components so they stay host-compiled, host-tested, and coverage-measured (an
//! "extra leaf" beside `mod`/`api`/`server`/`component`, like [`super::render`]).
//! The components call these fns and wrap the returned plain data in `view!`
//! markup; the `#[cfg(test)] mod tests` below pin the valid and edge cases.

use crate::posts::DraftSummary;
use common::slug::Slug;
use common::time::PermalinkDate;
use common::username::Username;

/// Decode the `~username`/`year`/`month`/`day`/`slug` permalink route params into
/// typed values, mirroring the client-side parse `PostPage` performs before it
/// fetches (ADR-0063 §4). A segment that is not a `~username` yields `None` (a
/// non-permalink URL the caller reloads for the server to handle); a `~`-prefixed
/// URL whose slug won't parse names no real post, so `slug` is `None` and the
/// caller 404s client-side without a round-trip. The three date segments are
/// assembled into one [`PermalinkDate`]; an absent, non-numeric, or impossible
/// date (e.g. month 13) yields `None`, so the caller 404s client-side rather than
/// fetching a date that can name no post.
pub fn parse_permalink_params(
    username: Option<&str>,
    year: Option<&str>,
    month: Option<&str>,
    day: Option<&str>,
    slug: Option<&str>,
) -> (Option<Username>, Option<PermalinkDate>, Option<Slug>) {
    let username = username
        .unwrap_or_default()
        .strip_prefix('~')
        .and_then(|s| s.parse::<Username>().ok());
    // Present at all three segments, each numeric, and together a real calendar date —
    // else `None` (the caller 404s client-side). `.parse().ok()?` bridges each segment's
    // parse to the `Option` `from_ymd` returns; the target int types are inferred from it.
    let date = year.zip(month).zip(day).and_then(|((y, m), d)| {
        PermalinkDate::from_ymd(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
    });
    let slug = slug.and_then(|s| s.parse::<Slug>().ok());
    (username, date, slug)
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
        parse_post_summary, parse_post_title, parse_root_relative_url, parse_slug, parse_username,
        parse_utc_instant,
    };

    #[test]
    fn parses_valid_permalink_params() {
        let (username, date, slug) = parse_permalink_params(
            Some("~alice"),
            Some("2026"),
            Some("01"),
            Some("02"),
            Some("hello"),
        );
        assert_eq!(username, Some(parse_username("alice")));
        assert_eq!(date, PermalinkDate::from_ymd(2026, 1, 2));
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
    fn unparseable_or_impossible_date_is_none() {
        // A non-numeric segment can't form a date.
        let (_, d1, _) =
            parse_permalink_params(Some("~a"), Some("x"), Some("01"), Some("02"), Some("s"));
        assert_eq!(d1, None);
        // An impossible date (month 13) is rejected by construction.
        let (_, d2, _) =
            parse_permalink_params(Some("~a"), Some("2026"), Some("13"), Some("02"), Some("s"));
        assert_eq!(d2, None);
        // A missing segment leaves no date.
        let (_, d3, _) =
            parse_permalink_params(Some("~a"), None, Some("01"), Some("02"), Some("s"));
        assert_eq!(d3, None);
    }

    #[test]
    fn unparseable_slug_is_none() {
        // A '~'-prefixed permalink with an invalid slug names no real post.
        let (username, _, slug) = parse_permalink_params(
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
            edit_url: parse_root_relative_url("/posts/1/edit"),
            permalink: parse_root_relative_url("/~alice/2026/01/01/my-post"),
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
