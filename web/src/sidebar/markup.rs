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

pub(super) static NAV_ITEMS: LazyLock<[NavItem; 19]> = LazyLock::new(|| {
    [
        NavItem {
            key: "local",
            label: "Local",
            icon_path: Icons::LOCAL,
            href: Some(root_relative_url("/")),
            requires_auth: false,
            requires_operator: false,
        },
        // The authenticated cockpit is deliberately absent from the public
        // projector's navigation; authenticated chrome exposes it as Home.
        NavItem {
            key: "home",
            label: "Home",
            icon_path: Icons::HOME,
            href: Some(root_relative_url("/app")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "compose",
            label: "Compose",
            icon_path: Icons::EDIT,
            href: Some(root_relative_url("/posts/new")),
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
            key: "themes",
            label: "Themes",
            icon_path: Icons::COG,
            href: Some(root_relative_url("/themes")),
            requires_auth: true,
            requires_operator: false,
        },
        NavItem {
            key: "passkeys",
            label: "Passkeys",
            icon_path: Icons::COG,
            href: Some(root_relative_url("/passkeys")),
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
            key: "admin-smtp",
            label: "SMTP Relay",
            icon_path: Icons::SHIELD,
            href: Some(root_relative_url("/admin/smtp")),
            requires_auth: true,
            requires_operator: true,
        },
        NavItem {
            key: "admin-websub",
            label: "WebSub",
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

/// Returns linked items visible to a viewer for the projected authentication,
/// registration-policy, and operator state.
pub(super) fn nav_items(
    policy: RegistrationPolicy,
    is_operator: bool,
    is_authenticated: bool,
) -> impl Iterator<Item = &'static NavItem> {
    NAV_ITEMS.iter().filter(move |item| {
        let has_access = if is_authenticated {
            item.requires_auth && (!item.requires_operator || is_operator)
        } else {
            !item.requires_auth && !item.requires_operator
        };
        has_access
            && item.href.is_some()
            && (item.key != "invites" || policy.may_issue_invitation(is_operator))
    })
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
        "passkeys" => Some("passkeys-nav-link"),
        _ => None,
    }
}

/// The inner HTML of the **anonymous** `<aside class="j-sidebar">`: brand, search,
/// the public nav (items with an href and no auth requirement — just "Local"), and
/// an empty footer. The reactive [`crate::sidebar::Sidebar`] injects this verbatim
/// via `inner_html` for the anonymous viewer, so a seeded first paint and the
/// reactive re-render coincide; authenticated users get the reactive build (extra
/// nav, footer avatar) layered on top (#181).
#[must_use]
pub(crate) fn render_sidebar(active_key: &str) -> Markup {
    Markup::new(html! {
        a class="j-brand" href="/" {
            div class="j-brand-mark" { "j" }
            div class="j-brand-text" { "Jaunder" }
        }
        div class="j-search" {
            (icon::render(Icons::SEARCH, 14))
            span { "Search" }
            span class="j-kbd" { "\u{2318}K" }
        }
        nav class="j-nav" data-jaunder-part="primary-navigation" {
            @for item in nav_items(RegistrationPolicy::Closed, false, false) {
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
        div class="j-sb-foot" {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_renders_local_as_the_sole_anonymous_destination() {
        let markup = render_sidebar("local");
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
    fn sidebar_active_class_absent_for_non_local_route() {
        let markup = render_sidebar("tags");
        let html = markup.as_str();
        assert!(
            html.contains("<a class=\"j-nav-item\" href=\"/\">"),
            "{html}"
        );
    }

    #[test]
    fn nav_catalog_exposes_home_only_to_authenticated_navigation() {
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
                ("drafts", "/drafts"),
                ("scheduled", "/scheduled"),
                ("history", "/history"),
                ("media", "/media"),
                ("audiences", "/audiences"),
                ("themes", "/themes"),
                ("passkeys", "/passkeys"),
                ("settings", "/profile"),
                ("invites", "/invites"),
                ("admin-backups", "/admin/backups"),
                ("admin-site", "/admin/site"),
                ("admin-smtp", "/admin/smtp"),
                ("admin-websub", "/admin/websub"),
            ]
        );

        let authenticated = nav_items(RegistrationPolicy::Closed, false, true)
            .map(|item| item.key)
            .collect::<Vec<_>>();
        assert!(authenticated.contains(&"home"));
        assert!(!authenticated.contains(&"local"));
        let anonymous = render_sidebar("").into_string();
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
        assert_eq!(test_selector("passkeys"), Some("passkeys-nav-link"));
        assert_eq!(test_selector("home"), None);
    }

    #[test]
    fn operator_websub_destination_uses_page_title() {
        let websub = nav_items(RegistrationPolicy::Closed, true, true)
            .find(|item| item.key == "admin-websub")
            .map(|item| (item.label, item.href.as_deref()));

        assert_eq!(websub, Some(("WebSub", Some("/admin/websub"))));
    }

    #[test]
    fn operator_destinations_are_visible_only_to_operators() {
        let viewer_items = nav_items(RegistrationPolicy::Closed, false, true)
            .map(|item| item.key)
            .collect::<Vec<_>>();
        assert!(!viewer_items.contains(&"admin-backups"));
        assert!(!viewer_items.contains(&"admin-site"));
        assert!(!viewer_items.contains(&"admin-smtp"));
        assert!(!viewer_items.contains(&"admin-websub"));

        let operator_items = nav_items(RegistrationPolicy::Closed, true, true)
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
        assert!(
            operator_items.contains(&("admin-smtp", "/admin/smtp")),
            "{operator_items:?}"
        );
        assert!(
            operator_items.contains(&("admin-websub", "/admin/websub")),
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
            let visible = nav_items(policy, is_operator, true).any(|item| item.key == "invites");
            assert_eq!(visible, expected, "{policy:?}, operator={is_operator}");
        }
    }
}
