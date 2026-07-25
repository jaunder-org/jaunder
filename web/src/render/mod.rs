//! Shared pure, non-reactive HTML leaf primitives for the web crate.
//!
//! Plain-string building only — no leptos reactivity, like `common::feed` — so
//! `reactive_graph` never sits on the public request path (the #173 escape). These
//! leaves are shared across verticals: HTML escaping, SVG icon path data, tag-link
//! context, the home hero/masthead, the projector "Load more" placeholder, and a
//! byte-size formatter. Several are the pure twins the projector
//! (`server::projector`, via `crate::posts::render`) and the reactive components
//! both render, so their output coincides byte-for-byte (ADR-0041). See
//! `docs/adr/0041` and `docs/inbound-data-handling.md` §4.
//!
//! The page-frame shell projector (`render_shell`/`render_head` + the shell
//! constants) moved to `crate::app::render` with the reactive shell it twins
//! (#330); the remaining primitives here are dissolved onto their co-located homes
//! by #658.

use common::username::Username;

/// Linking context for a post's footer tag chips — imported by the reactive post
/// view (`crate::posts`) as `TagContext`. `SiteWide` links each
/// chip to `/tags/:slug` only; `ForUser` also renders the "· here" link to
/// `/~:username/tags/:slug`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagCtx {
    SiteWide,
    ForUser(Username),
}

/// Escape text for safe interpolation into HTML element or attribute content.
pub(crate) fn escape_html<S: AsRef<str>>(input: S) -> String {
    let input = input.as_ref();
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The home page hero block (constant copy). Composed into
/// [`render_home_masthead`] — the one source the projector and the reactive
/// `home::HomePage` both render (ADR-0041 §2), so there is no `view!` twin.
#[must_use]
pub(crate) fn render_hero() -> String {
    "<div class=\"j-hero\"><h1>One timeline. Every protocol.</h1><p>Jaunder is a self-hosted \
     social client that reads from ActivityPub, AT Protocol, RSS, Atom, and JSON Feed \u{2014} and \
     publishes back out to the ones you choose. Below: what\u{2019}s been posted from this \
     instance.</p></div>"
        .to_string()
}

/// The home page masthead — the topbar (with the anonymous Sign-in / Register
/// links) then the hero. The single source both the projector
/// (`crate::posts::render::render_body`) and the reactive `home::HomePage` render,
/// so coincidence holds by construction (ADR-0041 §2) — no `view!` twin to drift.
/// The links carry `j-anon-only` so the authed owner's pre-painted masthead hides
/// them (ADR-0044); an anonymous viewer (no `html.authed`) still sees them.
#[must_use]
pub(crate) fn render_home_masthead() -> String {
    format!(
        "{topbar}{hero}",
        topbar = crate::topbar::render(
            "jaunder.local",
            Some("Read-only \u{00b7} posts originating on this instance"),
            "<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>\
             <a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>",
        ),
        hero = render_hero(),
    )
}

/// The non-functional "Load more" button the projector paints so the reactive
/// button (which replaces it on boot) doesn't reflow. Rendered only when there is
/// a next page, matching the reactive `has_more` guard.
#[must_use]
pub(crate) fn render_load_more(has_more: bool) -> String {
    if has_more {
        "<button>Load more</button>".to_string()
    } else {
        String::new()
    }
}

/// SVG path `d` attribute strings for all Jaunder icons. Shared by the reactive
/// [`crate::icon::Icon`] component and the pure [`crate::icon::render`].
pub struct Icons;

impl Icons {
    pub const HOME: &'static str = "M3 10l7-6 7 6v7a1 1 0 0 1-1 1h-4v-5H8v5H4a1 1 0 0 1-1-1z";
    pub const LOCAL: &'static str = "M4 5h12v10H4z M4 9h12";
    pub const FED: &'static str =
        "M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zM3 10h14 M10 3c2 3 2 11 0 14 M10 3c-2 3-2 11 0 14";
    pub const REPLY: &'static str = "M4 4h12v9H7l-3 3z";
    pub const BOOKMARK: &'static str = "M5 3h10v14l-5-3-5 3z";
    pub const BOOST: &'static str =
        "M5 8l4-4 4 4 M4 7v4a3 3 0 0 0 3 3h9 M15 12l-4 4-4-4 M16 13V9a3 3 0 0 0-3-3H4";
    pub const HEART: &'static str =
        "M10 17s-7-4.5-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 17 7c0 5.5-7 10-7 10z";
    pub const SEARCH: &'static str = "M8 3a6 6 0 1 0 0 12a6 6 0 0 0 0-12z M17 17l-4-4";
    pub const PLUS: &'static str = "M10 4v12 M4 10h12";
    pub const COG: &'static str = "M10 6v2 M10 12v2 M6 10H4 M16 10h-2 M6.5 6.5l-1.5-1.5 M14 14l1.5 1.5 M6.5 13.5L5 15 M14 6l1.5-1.5 M10 13a3 3 0 1 0 0-6a3 3 0 0 0 0 6z";
    pub const EDIT: &'static str = "M3 17l4 0 9-9a2.83 2.83 0 0 0-4-4l-9 9 0 4 M12 5l3 3";
    pub const SHIELD: &'static str = "M10 3l6 2v4c0 4-2.4 7.1-6 8-3.6-.9-6-4-6-8V5l6-2z";
    pub const MEDIA: &'static str =
        "M3 5h14v10H3z M7 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2z M5 13l3-3 2 2 3-3 5 5H3z";
    pub const REFRESH: &'static str = "M15.5 8A6 6 0 1 0 16 11.5 M15.5 4v4h-4";
}

/// Formats a byte count as a human-readable size (`B` / `KB` / `MB` / `GB`, one
/// decimal). Shared display formatter, host-tested here.
#[expect(
    clippy::cast_precision_loss,
    reason = "byte counts < 2^52 convert to f64 exactly; larger values only affect a \
              human-readable one-decimal display, so any loss is immaterial"
)]
pub fn format_bytes(bytes: impl Into<i64>) -> String {
    const KB: i64 = 1_024;
    const MB: i64 = 1_024 * KB;
    const GB: i64 = 1_024 * MB;

    // Generic over the byte-ish newtypes (`ByteSize`, `MaxFileSize`, `UserQuota` — each
    // `Into<i64>` via `NumNewtype`) as well as a bare `i64`, so call sites pass the typed
    // value without spelling `.value()`.
    let bytes: i64 = bytes.into();

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_displays_bytes_below_kb() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_displays_kb_range() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn format_bytes_displays_mb_range() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 2), "2.0 MB");
    }

    #[test]
    fn format_bytes_displays_gb_range() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn escape_replaces_markup_metacharacters() {
        assert_eq!(escape_html("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
    }

    #[test]
    fn home_masthead_has_topbar_hero_and_anon_only_cta() {
        let html = render_home_masthead();
        assert!(html.contains("<h1>jaunder.local</h1>"), "{html}");
        assert!(
            html.contains("<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>"
            ),
            "{html}"
        );
        assert!(html.contains("<div class=\"j-hero\">"), "{html}");
    }
}
