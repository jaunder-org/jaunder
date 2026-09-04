use std::sync::LazyLock;

use common::{registration::RegistrationPolicy, root_relative_url::RootRelativeUrl};
use maud::html;

use crate::html::Markup;
use crate::icon::{self, Icons};

/// A sidebar destination and its visibility policy. Shared by [`render_sidebar`]
/// and the reactive authenticated sidebar.
pub(super) struct NavItem {
    pub(super) key: &'static str,
    pub(super) label: &'static str,
    pub(super) icon_path: &'static str,
    pub(super) href: Option<RootRelativeUrl>,
    pub(super) requires_auth: bool,
    pub(super) requires_operator: bool,
}

pub(super) static NAV_ITEMS: LazyLock<[NavItem; 16]> = LazyLock::new(|| {
    [
        NavItem {
            key: "home",
            label: "Home",
            icon_path: Icons::HOME,
            href: Some(root_relative_url("/")),
            requires_auth: false,
            requires_operator: false,
        },
        // The authed-only cockpit (#181, ADR-0044 D6): the owner's personalized feed at
        // /app. `requires_auth` keeps it out of the cacheable anonymous sidebar
        // (`render_sidebar` filters `href.is_some() && !requires_auth`) — it appears
        // only in the authed sidebar, so the projector's anonymous paint is unchanged.
        NavItem {
            key: "app",
            label: "Feed",
            icon_path: Icons::HOME,
            href: Some(root_relative_url("/app")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "local",
            label: "Local",
            icon_path: Icons::LOCAL,
            href: None,
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "federated",
            label: "Federated",
            icon_path: Icons::FED,
            href: None,
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "replies",
            label: "Replies",
            icon_path: Icons::REPLY,
            href: None,
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "bookmarks",
            label: "Bookmarks",
            icon_path: Icons::BOOKMARK,
            href: None,
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "drafts",
            label: "Drafts",
            icon_path: Icons::EDIT,
            href: Some(root_relative_url("/drafts")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "scheduled",
            label: "Scheduled",
            icon_path: Icons::EDIT,
            href: Some(root_relative_url("/scheduled")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "history",
            label: "History",
            icon_path: Icons::REFRESH,
            href: Some(root_relative_url("/history")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "media",
            label: "Media",
            icon_path: Icons::MEDIA,
            href: Some(root_relative_url("/media")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "audiences",
            label: "Audiences",
            icon_path: Icons::BOOKMARK,
            href: Some(root_relative_url("/audiences")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "settings",
            label: "Settings",
            icon_path: Icons::COG,
            href: Some(root_relative_url("/profile")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "invites",
            label: "Invites",
            icon_path: Icons::PLUS,
            href: Some(root_relative_url("/invites")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "admin-backups",
            label: "Configure Backups",
            icon_path: Icons::SHIELD,
            href: Some(root_relative_url("/admin/backups")),
            requires_auth: true,
            requires_operator: true,
        },
        NavItem {
            key: "admin-site",
            label: "Site Settings",
            icon_path: Icons::SHIELD,
            href: Some(root_relative_url("/admin/site")),
            requires_auth: true,
            requires_operator: true,
        },
        NavItem {
            key: "admin-websub",
            label: "WebSub Recovery",
            icon_path: Icons::SHIELD,
            href: Some(root_relative_url("/admin/websub")),
            requires_auth: true,
            requires_operator: true,
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

/// Returns linked items visible to an authenticated viewer for the projected policy and role.
pub(super) fn nav_items(
    policy: RegistrationPolicy,
    is_operator: bool,
) -> impl Iterator<Item = &'static NavItem> {
    NAV_ITEMS.iter().filter(move |item| {
        item.href.is_some()
            && (!item.requires_operator || is_operator)
            && (item.key != "invites" || policy.may_issue_invitation(is_operator))
    })
}

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
            @for item in nav_items(RegistrationPolicy::Closed, false) {
                @if let Some(href) = &item.href {
                    @if !item.requires_auth {
                        a class={ "j-nav-item" @if item.key == active_key { " is-active" } }
                            href=(href)
                        {
                            (icon::render(item.icon_path, 16))
                            span { (item.label) }
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
        // Auth-required items and non-link placeholders must NOT appear for the
        // anonymous sidebar.
        assert!(!html.contains(">Feed<"), "{html}");
        assert!(!html.contains(">Drafts<"), "{html}");
        assert!(!html.contains(">Scheduled<"), "{html}");
        assert!(!html.contains(">History<"), "{html}");
        assert!(!html.contains(">Invites<"), "{html}");
        assert!(!html.contains(">Configure Backups<"), "{html}");
        assert!(!html.contains(">Site Settings<"), "{html}");
        assert!(!html.contains(">WebSub Recovery<"), "{html}");
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

    #[test]
    fn nav_catalog_preserves_destinations_and_non_link_placeholders() {
        let destinations = NAV_ITEMS
            .iter()
            .filter_map(|item| item.href.as_deref().map(|href| (item.key, href)))
            .collect::<Vec<_>>();
        assert_eq!(
            destinations,
            [
                ("home", "/"),
                ("app", "/app"),
                ("drafts", "/drafts"),
                ("scheduled", "/scheduled"),
                ("history", "/history"),
                ("media", "/media"),
                ("audiences", "/audiences"),
                ("settings", "/profile"),
                ("invites", "/invites"),
                ("admin-backups", "/admin/backups"),
                ("admin-site", "/admin/site"),
                ("admin-websub", "/admin/websub"),
            ]
        );

        let placeholders = NAV_ITEMS
            .iter()
            .filter_map(|item| item.href.is_none().then_some(item.key))
            .collect::<Vec<_>>();
        assert_eq!(placeholders, ["local", "federated", "replies", "bookmarks"]);
    }

    #[test]
    fn operator_destinations_are_visible_only_to_operators() {
        let viewer_items = nav_items(RegistrationPolicy::Closed, false)
            .map(|item| item.key)
            .collect::<Vec<_>>();
        assert!(!viewer_items.contains(&"admin-backups"));
        assert!(!viewer_items.contains(&"admin-site"));

        let operator_items = nav_items(RegistrationPolicy::Closed, true)
            .map(|item| {
                let Some(href) = item.href.as_ref() else {
                    unreachable!("nav_items returns linked items");
                };
                let href: &str = href;
                (item.key, href)
            })
            .collect::<Vec<_>>();
        assert!(
            operator_items.contains(&("admin-backups", "/admin/backups")),
            "{operator_items:?}"
        );
        assert!(
            operator_items.contains(&("admin-site", "/admin/site")),
            "{operator_items:?}"
        );
    }

    #[test]
    fn invitation_destination_matches_policy_and_role_authority() {
        let cases = [
            (RegistrationPolicy::Closed, false, false),
            (RegistrationPolicy::Closed, true, false),
            (RegistrationPolicy::OperatorInvites, false, false),
            (RegistrationPolicy::OperatorInvites, true, true),
            (RegistrationPolicy::MemberInvites, false, true),
            (RegistrationPolicy::MemberInvites, true, true),
            (RegistrationPolicy::Open, false, false),
            (RegistrationPolicy::Open, true, false),
        ];

        for (policy, is_operator, expected) in cases {
            let visible = nav_items(policy, is_operator).any(|item| item.key == "invites");
            assert_eq!(visible, expected, "{policy:?}, operator={is_operator}");
        }
    }
}
