//! The Settings and Account hub catalogs: which configuration destinations each
//! hub lists, who may see them, and which hub owns a given page. Pure and
//! host-tested; the reactive hub pages and the `Topbar` corner controls render
//! from it, so visibility rules live in exactly one place.

use std::sync::LazyLock;

use common::{registration::RegistrationPolicy, root_relative_url::RootRelativeUrl};

/// One of the two configuration hubs reached from the page-header controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hub {
    Settings,
    Account,
}

impl Hub {
    /// The hub page's own path.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Settings => "/settings",
            Self::Account => "/account",
        }
    }

    /// The hub's visible name, used for its page title and back links.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Settings => "Settings",
            Self::Account => "Account",
        }
    }

    /// The hub that owns `pathname`: the hub page itself or any destination it
    /// lists. Matching is exact, like the sidebar's, so nested routes (e.g. a
    /// Post editor) are never claimed by a hub.
    #[must_use]
    pub fn owning(pathname: &str) -> Option<Self> {
        [Self::Settings, Self::Account].into_iter().find(|hub| {
            hub.path() == pathname
                || HUB_LINKS
                    .iter()
                    .any(|link| link.hub == *hub && link.href.as_ref() == pathname)
        })
    }

    /// The hub a sub-page should link back to; `None` on the hub page itself
    /// and on pages no hub owns.
    #[must_use]
    pub fn parent_of(pathname: &str) -> Option<Self> {
        Self::owning(pathname).filter(|hub| hub.path() != pathname)
    }
}

/// Who may see a hub row, beyond being signed in (both hubs are private).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Everyone,
    /// Shown only when the Registration Policy lets this viewer issue Invitations.
    InvitationIssuers,
    Operators,
}

/// A linked configuration destination listed on a hub.
pub struct HubLink {
    hub: Hub,
    pub group: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub href: RootRelativeUrl,
    visibility: Visibility,
}

/// Group names, in display order within their hub.
const PUBLISHING: &str = "Publishing";
const INVITATIONS: &str = "Invitations";
const SITE_ADMINISTRATION: &str = "Site administration";
const SIGN_IN: &str = "Sign-in and security";

static HUB_LINKS: LazyLock<[HubLink; 10]> = LazyLock::new(|| {
    let link = |hub, group, label, description, href, visibility| HubLink {
        hub,
        group,
        label,
        description,
        href: root_relative_url(href),
        visibility,
    };
    [
        // `/profile` is a Settings destination: beyond the username card it
        // holds the Default Audience, Content License and Default Post Format.
        link(
            Hub::Settings,
            PUBLISHING,
            "Profile and posting defaults",
            "Display name, bio, default audience, license and post format",
            "/profile",
            Visibility::Everyone,
        ),
        link(
            Hub::Settings,
            PUBLISHING,
            "Themes",
            "How your public pages look",
            "/themes",
            Visibility::Everyone,
        ),
        link(
            Hub::Settings,
            INVITATIONS,
            "Invites",
            "Invite people to join",
            "/invites",
            Visibility::InvitationIssuers,
        ),
        link(
            Hub::Settings,
            SITE_ADMINISTRATION,
            "Configure Backups",
            "Scheduled backups and restore",
            "/admin/backups",
            Visibility::Operators,
        ),
        link(
            Hub::Settings,
            SITE_ADMINISTRATION,
            "Site Settings",
            "Site identity, base URL and Registration Policy",
            "/admin/site",
            Visibility::Operators,
        ),
        link(
            Hub::Settings,
            SITE_ADMINISTRATION,
            "SMTP Relay",
            "Outgoing email delivery",
            "/admin/smtp",
            Visibility::Operators,
        ),
        link(
            Hub::Settings,
            SITE_ADMINISTRATION,
            "WebSub",
            "Hub publishing for syndication feeds",
            "/admin/websub",
            Visibility::Operators,
        ),
        link(
            Hub::Account,
            SIGN_IN,
            "Email",
            "Your address and its verification",
            "/profile/email",
            Visibility::Everyone,
        ),
        link(
            Hub::Account,
            SIGN_IN,
            "Passkeys",
            "Sign in securely with this device",
            "/passkeys",
            Visibility::Everyone,
        ),
        link(
            Hub::Account,
            SIGN_IN,
            "Sessions",
            "Where you are signed in, and app passwords",
            "/sessions",
            Visibility::Everyone,
        ),
    ]
});

/// Parses a catalog literal at initialization; a failed parse would make this
/// source invalid rather than represent a runtime route condition.
fn root_relative_url(path: &'static str) -> RootRelativeUrl {
    let Ok(url) = path.parse() else {
        unreachable!("hub catalog contains only valid root-relative paths");
    };
    url
}

/// A titled group of hub rows, in display order.
pub struct HubGroup {
    pub title: &'static str,
    pub links: Vec<&'static HubLink>,
}

/// The Settings hub's groups for a signed-in viewer with this authority. Groups
/// with no visible rows are omitted rather than rendered empty.
#[must_use]
pub fn settings_groups(policy: RegistrationPolicy, is_operator: bool) -> Vec<HubGroup> {
    groups(Hub::Settings, |visibility| match visibility {
        Visibility::Everyone => true,
        Visibility::InvitationIssuers => policy.may_issue_invitation(is_operator),
        Visibility::Operators => is_operator,
    })
}

/// The Account hub's groups; every signed-in viewer sees the same rows.
#[must_use]
pub fn account_groups() -> Vec<HubGroup> {
    groups(Hub::Account, |visibility| {
        visibility == Visibility::Everyone
    })
}

fn groups(hub: Hub, visible: impl Fn(Visibility) -> bool) -> Vec<HubGroup> {
    let mut groups: Vec<HubGroup> = Vec::new();
    for link in HUB_LINKS.iter().filter(|link| link.hub == hub) {
        if !visible(link.visibility) {
            continue;
        }
        match groups.last_mut() {
            Some(group) if group.title == link.group => group.links.push(link),
            _ => groups.push(HubGroup {
                title: link.group,
                links: vec![link],
            }),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hrefs(groups: &[HubGroup]) -> Vec<(&'static str, &str)> {
        groups
            .iter()
            .flat_map(|group| {
                group
                    .links
                    .iter()
                    .map(move |link| (group.title, link.href.as_ref()))
            })
            .collect()
    }

    #[test]
    fn settings_hub_shows_members_only_their_own_preferences() {
        let groups = settings_groups(RegistrationPolicy::Closed, false);
        assert_eq!(
            hrefs(&groups),
            [(PUBLISHING, "/profile"), (PUBLISHING, "/themes")]
        );
    }

    #[test]
    fn settings_hub_adds_site_administration_for_operators() {
        let groups = settings_groups(RegistrationPolicy::Closed, true);
        assert_eq!(
            hrefs(&groups),
            [
                (PUBLISHING, "/profile"),
                (PUBLISHING, "/themes"),
                (SITE_ADMINISTRATION, "/admin/backups"),
                (SITE_ADMINISTRATION, "/admin/site"),
                (SITE_ADMINISTRATION, "/admin/smtp"),
                (SITE_ADMINISTRATION, "/admin/websub"),
            ]
        );
    }

    #[test]
    fn invitation_row_matches_policy_and_role_authority() {
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
            let groups = settings_groups(policy, is_operator);
            let visible = hrefs(&groups).contains(&(INVITATIONS, "/invites"));
            assert_eq!(visible, expected, "{policy:?}, operator={is_operator}");
        }
    }

    #[test]
    fn account_hub_lists_sign_in_and_security() {
        assert_eq!(
            hrefs(&account_groups()),
            [
                (SIGN_IN, "/profile/email"),
                (SIGN_IN, "/passkeys"),
                (SIGN_IN, "/sessions"),
            ]
        );
    }

    #[test]
    fn owning_hub_is_exact_for_hubs_and_their_destinations() {
        assert_eq!(Hub::owning("/settings"), Some(Hub::Settings));
        assert_eq!(Hub::owning("/account"), Some(Hub::Account));
        assert_eq!(Hub::owning("/profile"), Some(Hub::Settings));
        assert_eq!(Hub::owning("/profile/email"), Some(Hub::Account));
        assert_eq!(Hub::owning("/admin/websub"), Some(Hub::Settings));
        assert_eq!(Hub::owning("/passkeys"), Some(Hub::Account));
        assert_eq!(Hub::owning("/app"), None);
        assert_eq!(Hub::owning("/settings/extra"), None);
        assert_eq!(Hub::owning("/~alice"), None);
    }

    #[test]
    fn hubs_are_private_routes() {
        for hub in [Hub::Settings, Hub::Account] {
            assert!(crate::app::PrivateDestination::parse(hub.path()).is_some());
        }
    }

    #[test]
    fn back_link_parent_excludes_the_hub_page_itself() {
        assert_eq!(Hub::parent_of("/settings"), None);
        assert_eq!(Hub::parent_of("/account"), None);
        assert_eq!(Hub::parent_of("/themes"), Some(Hub::Settings));
        assert_eq!(Hub::parent_of("/sessions"), Some(Hub::Account));
        assert_eq!(Hub::parent_of("/drafts"), None);
    }

    #[test]
    fn every_row_has_copy_and_belongs_to_a_routed_private_page() {
        for link in HUB_LINKS.iter() {
            assert!(!link.label.is_empty() && !link.description.is_empty());
            assert!(
                crate::app::PrivateDestination::parse(link.href.as_ref()).is_some(),
                "{}",
                link.href
            );
        }
    }
}
