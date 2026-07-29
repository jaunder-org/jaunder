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

/// One fully-decoded permalink route: the author, the date, and the slug, all three
/// present and typed. Produced only by [`parse_permalink_route`], so holding one is
/// proof the URL names a post that could exist — the caller fetches with it and never
/// re-checks the parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermalinkRoute {
    /// The `~`-stripped author of the permalink.
    pub username: Username,
    /// The `year`/`month`/`day` segments as one calendar date.
    pub date: PermalinkDate,
    /// The post's slug.
    pub slug: Slug,
}

/// Decode the `~username`/`year`/`month`/`day`/`slug` permalink route params into a
/// typed [`PermalinkRoute`], mirroring the client-side parse `PostPage` performs
/// before it fetches (ADR-0063 §4).
///
/// **All or nothing**: every failure mode names no post that could exist, and the
/// caller answers all of them identically (404 client-side, no round-trip), so this
/// returns one `Option` rather than a triple of them — the caller writes a single
/// guard instead of one per segment (#306). The modes are: a segment that is not a
/// `~username`, or one whose username won't parse; an absent, non-numeric, or
/// impossible date (e.g. month 13); and a slug that won't parse.
pub fn parse_permalink_route(
    username: Option<&str>,
    year: Option<&str>,
    month: Option<&str>,
    day: Option<&str>,
    slug: Option<&str>,
) -> Option<PermalinkRoute> {
    let username = username?.strip_prefix('~')?.parse::<Username>().ok()?;
    // Present at all three segments, each numeric, and together a real calendar date.
    // `.parse().ok()?` bridges each segment's parse to the `Option` `from_ymd` returns;
    // the target int types are inferred from it.
    let date = PermalinkDate::from_ymd(
        year?.parse().ok()?,
        month?.parse().ok()?,
        day?.parse().ok()?,
    )?;
    let slug = slug?.parse::<Slug>().ok()?;
    Some(PermalinkRoute {
        username,
        date,
        slug,
    })
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
    fn parses_valid_permalink_route() {
        let route = parse_permalink_route(
            Some("~alice"),
            Some("2026"),
            Some("01"),
            Some("02"),
            Some("hello"),
        )
        .expect("every segment parses");
        assert_eq!(route.username, parse_username("alice"));
        assert_eq!(
            route.date,
            PermalinkDate::from_ymd(2026, 1, 2).expect("a real date")
        );
        assert_eq!(route.slug, parse_slug("hello"));
    }

    #[test]
    fn absent_or_untilded_username_is_no_route() {
        // A segment that isn't a `~username` (e.g. a server-handled URL) is not a
        // permalink author, so the whole route decodes to `None` and the caller 404s.
        assert_eq!(
            parse_permalink_route(
                Some("alice"),
                Some("2026"),
                Some("01"),
                Some("02"),
                Some("hello")
            ),
            None
        );
        // A `~` with nothing parseable after it is no author either.
        assert_eq!(
            parse_permalink_route(
                Some("~not a username"),
                Some("2026"),
                Some("01"),
                Some("02"),
                Some("hello")
            ),
            None
        );
        // A missing segment altogether.
        assert_eq!(
            parse_permalink_route(None, Some("2026"), Some("01"), Some("02"), Some("hello")),
            None
        );
    }

    #[test]
    fn unparseable_or_impossible_date_is_no_route() {
        // A non-numeric segment can't form a date.
        assert_eq!(
            parse_permalink_route(Some("~a"), Some("x"), Some("01"), Some("02"), Some("s")),
            None
        );
        // An impossible date (month 13) is rejected by construction.
        assert_eq!(
            parse_permalink_route(Some("~a"), Some("2026"), Some("13"), Some("02"), Some("s")),
            None
        );
        // A missing segment leaves no date.
        assert_eq!(
            parse_permalink_route(Some("~a"), None, Some("01"), Some("02"), Some("s")),
            None
        );
    }

    #[test]
    fn unparseable_slug_is_no_route() {
        // A '~'-prefixed permalink with an invalid slug names no real post, so the
        // route is `None` even though the username and date decoded fine.
        assert_eq!(
            parse_permalink_route(
                Some("~alice"),
                Some("2026"),
                Some("01"),
                Some("02"),
                Some("Not A Slug!"),
            ),
            None
        );
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
