use crate::icon::{self, Icons};
use std::fmt::Write as _;

/// Sidebar nav items: `(key, label, icon_path, href, auth_required)`. Shared by
/// [`render_sidebar`] (anonymous → the `href.is_some() && !auth_required` subset)
/// and the reactive authed sidebar in [`crate::sidebar::Sidebar`].
pub(crate) const NAV_ITEMS: &[(&str, &str, &str, Option<&'static str>, bool)] = &[
    ("home", "Home", Icons::HOME, Some("/"), false),
    // The authed-only cockpit (#181, ADR-0044 D6): the owner's personalized feed at
    // /app. `auth_required = true` keeps it out of the cacheable anonymous sidebar
    // (`render_sidebar` filters `href.is_some() && !auth_required`) — it appears
    // only in the authed sidebar, so the projector's anonymous paint is unchanged.
    ("app", "Feed", Icons::HOME, Some("/app"), true),
    ("local", "Local", Icons::LOCAL, None, true),
    ("federated", "Federated", Icons::FED, None, true),
    ("replies", "Replies", Icons::REPLY, None, true),
    ("bookmarks", "Bookmarks", Icons::BOOKMARK, None, true),
    ("drafts", "Drafts", Icons::EDIT, Some("/drafts"), true),
    ("media", "Media", Icons::MEDIA, Some("/media"), true),
    (
        "audiences",
        "Audiences",
        Icons::BOOKMARK,
        Some("/audiences"),
        true,
    ),
    ("settings", "Settings", Icons::COG, None, true),
];

/// The static demo "Sources" rows in the sidebar: `(proto, name, sub)`.
pub(crate) const SIDEBAR_SOURCES: &[(&str, &str, &str)] = &[
    ("atproto", "Bluesky", "mara.bsky.social"),
    ("activitypub", "Mastodon", "@mara@hachyderm.io"),
    ("rss", "Ivy Chen", "weeknotes"),
    ("jsonfeed", "Manton", "manton.org"),
];

/// The inner HTML of the **anonymous** `<aside class="j-sidebar">`: brand, search,
/// the public nav (items with an href and no auth requirement — just "Home"),
/// the sources section, and an empty footer. The reactive [`crate::sidebar::Sidebar`]
/// injects this verbatim via `inner_html` for the anonymous viewer, so a seeded
/// first paint and the reactive re-render coincide; authed users get the reactive
/// build (extra nav, footer avatar) layered on top (#181).
#[must_use]
pub(crate) fn render_sidebar(active_key: &str) -> String {
    let mut out = String::from(
        "<a class=\"j-brand\" href=\"/\" style=\"text-decoration:none;color:inherit\">\
         <div class=\"j-brand-mark\">j</div><div class=\"j-brand-text\">Jaunder</div></a>",
    );
    let _ = write!(
        out,
        "<div class=\"j-search\">{}<span>Search</span><span class=\"j-kbd\">\u{2318}K</span></div>",
        icon::render(Icons::SEARCH, 14),
    );
    out.push_str("<nav class=\"j-nav\">");
    for &(key, label, icon_path, href, auth_required) in NAV_ITEMS {
        let Some(href) = href else { continue };
        if auth_required {
            continue;
        }
        let active = if key == active_key { " is-active" } else { "" };
        let _ = write!(
            out,
            "<a class=\"j-nav-item{active}\" href=\"{href}\">{icon}<span>{label}</span></a>",
            icon = icon::render(icon_path, 16),
        );
    }
    out.push_str("</nav><div><div class=\"j-sb-head\"><span>Sources</span><span class=\"j-sb-add\">+</span></div>");
    for &(proto, name, sub) in SIDEBAR_SOURCES {
        let _ = write!(
            out,
            "<div class=\"j-source\"><span class=\"j-dot\" style=\"width:8px;height:8px;border-radius:4px;background:var(--c-{proto})\"></span>\
             <div style=\"flex:1;min-width:0\"><div class=\"j-source-name\">{name}</div><div class=\"j-source-sub\">{sub}</div></div></div>",
        );
    }
    out.push_str("</div><div class=\"j-sb-foot\"></div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_renders_brand_public_nav_sources_and_empty_foot() {
        let html = render_sidebar("home");
        assert!(
            html.contains("<div class=\"j-brand-text\">Jaunder</div>"),
            "{html}"
        );
        // Public nav = Home only; active class applied for the matching key.
        assert!(
            html.contains("<a class=\"j-nav-item is-active\" href=\"/\">"),
            "{html}"
        );
        assert!(html.contains("<span>Home</span>"), "{html}");
        // Auth-required items must NOT appear for the anonymous sidebar.
        assert!(!html.contains(">Drafts<"), "{html}");
        assert!(!html.contains(">Settings<"), "{html}");
        // Sources section + empty footer.
        assert!(
            html.contains("<div class=\"j-source-name\">Bluesky</div>"),
            "{html}"
        );
        assert!(html.ends_with("<div class=\"j-sb-foot\"></div>"), "{html}");
    }

    #[test]
    fn sidebar_active_class_absent_for_non_home_route() {
        let html = render_sidebar("tags");
        assert!(
            html.contains("<a class=\"j-nav-item\" href=\"/\">"),
            "{html}"
        );
    }
}
