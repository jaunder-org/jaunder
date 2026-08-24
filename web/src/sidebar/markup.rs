use maud::html;

use crate::html::Markup;
use crate::icon::{self, Icons};

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
    (
        "scheduled",
        "Scheduled",
        Icons::EDIT,
        Some("/scheduled"),
        true,
    ),
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
pub(crate) fn render_sidebar(active_key: &str) -> Markup {
    Markup::new(html! {
        a class="j-brand" href="/" style="text-decoration:none;color:inherit" {
            div class="j-brand-mark" { "j" }
            div class="j-brand-text" { "Jaunder" }
        }
        div class="j-search" {
            (icon::render(Icons::SEARCH, 14))
            span { "Search" }
            span class="j-kbd" { "\u{2318}K" }
        }
        nav class="j-nav" {
            @for &(key, label, icon_path, href, auth_required) in NAV_ITEMS {
                @if let Some(href) = href {
                    @if !auth_required {
                        a class={ "j-nav-item" @if key == active_key { " is-active" } }
                            href=(href)
                        {
                            (icon::render(icon_path, 16))
                            span { (label) }
                        }
                    }
                }
            }
        }
        div {
            div class="j-sb-head" {
                span { "Sources" }
                span class="j-sb-add" { "+" }
            }
            @for &(proto, name, sub) in SIDEBAR_SOURCES {
                div class="j-source" {
                    span class="j-dot"
                        style={
                            "width:8px;height:8px;border-radius:4px;background:var(--c-" (proto) ")"
                        } {}
                    div style="flex:1;min-width:0" {
                        div class="j-source-name" { (name) }
                        div class="j-source-sub" { (sub) }
                    }
                }
            }
        }
        div class="j-sb-foot" {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_renders_brand_public_nav_sources_and_empty_foot() {
        let markup = render_sidebar("home");
        let html = markup.as_str();
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
        assert!(!html.contains(">Scheduled<"), "{html}");
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
        let markup = render_sidebar("tags");
        let html = markup.as_str();
        assert!(
            html.contains("<a class=\"j-nav-item\" href=\"/\">"),
            "{html}"
        );
    }
}
