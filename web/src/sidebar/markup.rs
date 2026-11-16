use std::sync::LazyLock;

use common::{feed::FeedSurface, root_relative_url::RootRelativeUrl};
use maud::html;

use crate::html::Markup;
use crate::icon::{self, Icons};

/// A sidebar destination and its visibility policy. Shared by [`render_sidebar`]
/// and the reactive authenticated sidebar.
///
/// The sidebar holds only the places where a User reads and writes Posts.
/// Configuration destinations live on the Settings and Account hubs reached
/// from the page-header corner controls (`crate::hub`, #1733).
pub(super) struct NavItem {
    pub(super) key: &'static str,
    pub(super) label: &'static str,
    pub(super) icon_path: &'static str,
    pub(super) href: Option<RootRelativeUrl>,
    pub(super) requires_auth: bool,
}

pub(super) static NAV_ITEMS: LazyLock<[NavItem; 12]> = LazyLock::new(|| {
    [
        NavItem {
            key: "local",
            label: "Local",
            icon_path: Icons::LOCAL,
            href: Some(root_relative_url("/")),
            requires_auth: false,
        },
        // The authenticated cockpit is deliberately absent from the public
        // projector's navigation; authenticated chrome exposes it as Home.
        NavItem {
            key: "home",
            label: "Home",
            icon_path: Icons::HOME,
            href: Some(root_relative_url("/app")),
            requires_auth: true,
        },
        NavItem {
            key: "compose",
            label: "Compose",
            icon_path: Icons::EDIT,
            href: Some(root_relative_url("/posts/new")),
            requires_auth: true,
        },
        NavItem {
            key: "federated",
            label: "Federated",
            icon_path: Icons::FED,
            href: None,
            requires_auth: true,
        },
        NavItem {
            key: "replies",
            label: "Replies",
            icon_path: Icons::REPLY,
            href: None,
            requires_auth: true,
        },
        NavItem {
            key: "bookmarks",
            label: "Bookmarks",
            icon_path: Icons::BOOKMARK,
            href: None,
            requires_auth: true,
        },
        NavItem {
            key: "manage-posts",
            label: "Manage Posts",
            icon_path: Icons::EDIT,
            href: Some(root_relative_url("/posts/manage")),
            requires_auth: true,
        },
        NavItem {
            key: "drafts",
            label: "Drafts",
            icon_path: Icons::EDIT,
            href: Some(root_relative_url("/drafts")),
            requires_auth: true,
        },
        NavItem {
            key: "scheduled",
            label: "Scheduled",
            icon_path: Icons::EDIT,
            href: Some(root_relative_url("/scheduled")),
            requires_auth: true,
        },
        NavItem {
            key: "history",
            label: "History",
            icon_path: Icons::REFRESH,
            href: Some(root_relative_url("/history")),
            requires_auth: true,
        },
        NavItem {
            key: "media",
            label: "Media",
            icon_path: Icons::MEDIA,
            href: Some(root_relative_url("/media")),
            requires_auth: true,
        },
        NavItem {
            key: "audiences",
            label: "Audiences",
            icon_path: Icons::BOOKMARK,
            href: Some(root_relative_url("/audiences")),
            requires_auth: true,
        },
    ]
});

/// Parses a catalog literal at initialization; a failed parse would make this source
/// invalid rather than represent a runtime route condition.
fn root_relative_url(path: &'static str) -> RootRelativeUrl {
    let Ok(url) = path.parse() else {
        unreachable!("sidebar catalog contains only valid root-relative paths");
    };
    url
}

/// Returns the linked items visible to a viewer in the projected
/// authentication state.
pub(super) fn nav_items(is_authenticated: bool) -> impl Iterator<Item = &'static NavItem> {
    NAV_ITEMS
        .iter()
        .filter(move |item| item.requires_auth == is_authenticated && item.href.is_some())
}

/// Resolves the sole exact-match sidebar selection from the navigation catalog.
///
/// Linked destinations only are selectable: editor and other nested routes
/// intentionally leave the sidebar inactive rather than inheriting a parent item.
pub(crate) fn active_key(pathname: &str) -> Option<&'static str> {
    NAV_ITEMS
        .iter()
        .find(|item| item.href.as_deref() == Some(pathname))
        .map(|item| item.key)
}
/// Returns the stable browser-test selector for navigation contracts that need one.
pub(super) fn test_selector(key: &str) -> Option<&'static str> {
    match key {
        "history" => Some("history-nav-link"),
        _ => None,
    }
}

/// The rail footer's syndication link for the current timeline, if any.
#[must_use]
pub(crate) fn feed_link(surface: Option<&FeedSurface>) -> Markup {
    Markup::new(html! {
        @if let Some(surface) = surface {
            a class="j-feed-discovery-link" href=(surface.discovery_path()) aria-label="Syndication feeds" {
                (icon::render(Icons::RSS, 18))
            }
        }
    })
}

/// The inner HTML of the **anonymous** `<aside class="j-sidebar">`: brand, search,
/// the public nav (just "Local"), and a footer holding only the current
/// timeline's RSS marker. The reactive [`crate::sidebar::Sidebar`] injects this
/// verbatim for the anonymous viewer, so a seeded first paint and the reactive
/// re-render coincide; authenticated users get the reactive build (#181).
#[must_use]
pub(crate) fn render_sidebar(active_key: &str, surface: Option<&FeedSurface>) -> Markup {
    Markup::new(html! {
        a class="j-brand" href="/" data-jaunder-part="site-brand" {
            div class="j-brand-mark" { "j" }
            div class="j-brand-text" { "Jaunder" }
        }
        div class="j-search" data-jaunder-part="navigation-search" {
            (icon::render(Icons::SEARCH, 14))
            span { "Search" }
            span class="j-kbd" { "\u{2318}K" }
        }
        nav class="j-nav" data-jaunder-part="primary-navigation" {
            @for item in nav_items(false) {
                @if let Some(href) = &item.href {
                    a class={ "j-nav-item" @if item.key == active_key { " is-active" } }
                        href=(href)
                        data-test=[test_selector(item.key)]
                    {
                        (icon::render(item.icon_path, 16))
                        span { (item.label) }
                    }
                }
            }
        }
        div class="j-sb-foot" { (feed_link(surface)) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_renders_local_as_the_sole_anonymous_destination() {
        let markup = render_sidebar("local", None);
        let html = markup.as_str();
        assert!(
            html.contains("<div class=\"j-brand-text\">Jaunder</div>"),
            "{html}"
        );
        assert!(
            html.contains("<a class=\"j-nav-item is-active\" href=\"/\">"),
            "{html}"
        );
        assert!(html.contains("<span>Local</span>"), "{html}");
        assert!(!html.contains(">Home<"), "{html}");
        assert!(!html.contains(">Feed<"), "{html}");
        assert!(!html.contains(">Compose<"), "{html}");
        assert!(!html.contains(">Drafts<"), "{html}");
        assert!(!html.contains(">Scheduled<"), "{html}");
        assert!(!html.contains(">History<"), "{html}");
        assert!(!html.contains(">Sessions<"), "{html}");
        assert!(!html.contains(">Invites<"), "{html}");
        assert!(!html.contains(">Configure Backups<"), "{html}");
        assert!(!html.contains(">Site Settings<"), "{html}");
        assert!(!html.contains(">WebSub<"), "{html}");
        assert_eq!(
            html.matches("data-jaunder-part=\"primary-navigation\"")
                .count(),
            1,
            "{html}"
        );
        assert!(html.ends_with("<div class=\"j-sb-foot\"></div>"), "{html}");
    }

    #[test]
    fn public_rail_footer_links_only_the_current_syndication_context() {
        for (surface, path) in [
            (FeedSurface::Site, "/feeds"),
            (
                FeedSurface::SiteTag {
                    tag: "rust".parse().unwrap(),
                },
                "/tags/rust/feeds",
            ),
            (
                FeedSurface::User {
                    username: "alice".parse().unwrap(),
                },
                "/~alice/feeds",
            ),
            (
                FeedSurface::UserTag {
                    username: "alice".parse().unwrap(),
                    tag: "rust".parse().unwrap(),
                },
                "/~alice/tags/rust/feeds",
            ),
        ] {
            let html = render_sidebar("", Some(&surface)).into_string();
            assert!(
                html.contains(&format!("href=\"{path}\" aria-label=\"Syndication feeds\"")),
                "{html}"
            );
            assert_eq!(html.matches("aria-label=\"Syndication feeds\"").count(), 1);
        }
        assert!(
            !render_sidebar("", None)
                .as_str()
                .contains("Syndication feeds")
        );
    }

    #[test]
    fn sidebar_active_class_absent_for_non_local_route() {
        let markup = render_sidebar("tags", None);
        let html = markup.as_str();
        assert!(
            html.contains("<a class=\"j-nav-item\" href=\"/\">"),
            "{html}"
        );
    }

    #[test]
    fn nav_catalog_defines_order_and_authenticated_destinations() {
        let destinations = NAV_ITEMS
            .iter()
            .filter_map(|item| item.href.as_deref().map(|href| (item.key, href)))
            .collect::<Vec<_>>();
        assert_eq!(
            destinations,
            [
                ("local", "/"),
                ("home", "/app"),
                ("compose", "/posts/new"),
                ("manage-posts", "/posts/manage"),
                ("drafts", "/drafts"),
                ("scheduled", "/scheduled"),
                ("history", "/history"),
                ("media", "/media"),
                ("audiences", "/audiences"),
            ]
        );

        let authenticated = nav_items(true).map(|item| item.key).collect::<Vec<_>>();
        assert_eq!(
            authenticated,
            [
                "home",
                "compose",
                "manage-posts",
                "drafts",
                "scheduled",
                "history",
                "media",
                "audiences"
            ]
        );
        let anonymous = render_sidebar("", None).into_string();
        assert!(anonymous.contains(">Local<"), "{anonymous}");
        assert!(!anonymous.contains(">Home<"), "{anonymous}");
    }

    #[test]
    fn active_key_matches_every_linked_catalog_path_exactly() {
        for item in NAV_ITEMS.iter().filter(|item| item.href.is_some()) {
            let Some(href) = item.href.as_deref() else {
                unreachable!("filter retains only linked catalog items");
            };
            assert_eq!(active_key(href), Some(item.key), "{href}");
        }
        assert_eq!(active_key("/"), Some("local"));
        assert_eq!(active_key("/posts/new"), Some("compose"));
        assert_eq!(active_key("/posts/new/revisions"), None);
        assert_eq!(active_key("/unknown"), None);
    }

    #[test]
    fn browser_test_selectors_exist_only_for_navigation_contracts() {
        assert_eq!(test_selector("history"), Some("history-nav-link"));
        assert_eq!(test_selector("home"), None);
    }

    #[test]
    fn configuration_destinations_live_on_hubs_not_in_the_sidebar() {
        for path in [
            "/themes",
            "/sessions",
            "/passkeys",
            "/profile",
            "/invites",
            "/admin/backups",
            "/admin/site",
            "/admin/smtp",
            "/admin/websub",
        ] {
            assert_eq!(active_key(path), None, "{path}");
            assert!(crate::hub::Hub::owning(path).is_some(), "{path}");
        }
    }
}
