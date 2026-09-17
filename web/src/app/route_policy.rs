//! The one application-route catalog, including access classification and safe
//! private return destinations. The router expands this catalog too, so adding a
//! route cannot bypass either policy.

use common::root_relative_url::RootRelativeUrl;
use url::Url;

/// Browser access required by an application route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteDeclaration {
    name: &'static str,
    access: Access,
    pattern: &'static str,
}

/// A destination already proven to be a private application route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateDestination(RootRelativeUrl);

impl PrivateDestination {
    /// Parses a same-origin private route while preserving its query and fragment.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        if !value.starts_with('/')
            || value.starts_with("//")
            || value.contains('\\')
            || value.contains(|character: char| character.is_whitespace() || character.is_control())
            || !has_valid_percent_encoding(value)
        {
            return None;
        }

        let parsed = Url::parse("https://jaunder.invalid")
            .ok()?
            .join(value)
            .ok()
            .filter(|url| url.origin().ascii_serialization() == "https://jaunder.invalid")?;
        let supplied_path = value.split_once(['?', '#']).map_or(value, |(path, _)| path);
        if parsed.path() != supplied_path {
            return None;
        }

        let destination = value.parse::<RootRelativeUrl>().ok()?;
        private_route_matches(parsed.path()).then_some(Self(destination))
    }
}

fn has_valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if bytes
                .get(index + 1)
                .is_none_or(|byte| !byte.is_ascii_hexdigit())
                || bytes
                    .get(index + 2)
                    .is_none_or(|byte| !byte.is_ascii_hexdigit())
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

impl AsRef<str> for PrivateDestination {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

fn private_route_matches(path: &str) -> bool {
    route_catalog()
        .iter()
        .filter(|route| route.access == Access::Private)
        .any(|route| pattern_matches(route.pattern, path))
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern_segments = pattern.trim_start_matches('/').split('/');
    let path_segments = path.trim_start_matches('/').split('/');
    pattern_segments.zip(path_segments).all(|(pattern, path)| {
        (!pattern.starts_with(':') && pattern == path)
            || (pattern.starts_with(':') && !path.is_empty())
    }) && pattern.trim_start_matches('/').split('/').count()
        == path.trim_start_matches('/').split('/').count()
}

/// The exact private acceptance snapshot named by the issue specification.
#[cfg(test)]
const APPROVED_PRIVATE_ROUTES: &[(&str, &str)] = &[
    ("AdminBackups", "/admin/backups"),
    ("AdminSite", "/admin/site"),
    ("AdminSmtp", "/admin/smtp"),
    ("AdminWebsub", "/admin/websub"),
    ("App", "/app"),
    ("Audiences", "/audiences"),
    ("Drafts", "/drafts"),
    ("History", "/history"),
    ("Invites", "/invites"),
    ("Media", "/media"),
    ("Passkeys", "/passkeys"),
    ("PostEdit", "/posts/:post_id/edit"),
    ("PostHistory", "/posts/:post_id/history"),
    ("PostsNew", "/posts/new"),
    ("Profile", "/profile"),
    ("ProfileEmail", "/profile/email"),
    ("RevisionHistory", "/posts/:post_id/history/:revision_id"),
    ("Scheduled", "/scheduled"),
    ("Sessions", "/sessions"),
    ("Themes", "/themes"),
];

#[cfg(test)]
fn private_route_snapshot(routes: &[RouteDeclaration]) -> Vec<(&'static str, &'static str)> {
    let mut snapshot: Vec<_> = routes
        .iter()
        .filter(|route| route.access == Access::Private)
        .map(|route| (route.name, route.pattern))
        .collect();
    snapshot.sort_unstable();
    snapshot
}

#[cfg(test)]
fn private_inventory_conforms(routes: &[RouteDeclaration]) -> bool {
    private_route_snapshot(routes) == APPROVED_PRIVATE_ROUTES
}

macro_rules! catalog_entry {
    ($name:ident, $access:ident, $pattern:literal, $path:expr, $view:path) => {
        RouteDeclaration {
            name: stringify!($name),
            access: Access::$access,
            pattern: $pattern,
        }
    };
}

/// Expands the complete application route declarations for a local consumer.
///
/// Each entry carries its access policy, path matcher, and routed component. The
/// host catalog and wasm router invoke this same source; there is no runtime
/// private-route list to maintain separately.
#[doc(hidden)]
#[macro_export]
macro_rules! app_routes {
    ($consumer:ident) => {
        $consumer! {
            (Local, Public, "/", leptos_router::StaticSegment(""), $crate::local::LocalPage)
            (App, Private, "/app", leptos_router::StaticSegment("app"), $crate::cockpit::CockpitPage)
            (Register, Public, "/register", leptos_router::StaticSegment("register"), $crate::registration::RegisterPage)
            (Login, Public, "/login", leptos_router::StaticSegment("login"), $crate::auth::LoginPage)
            (Logout, Public, "/logout", leptos_router::StaticSegment("logout"), $crate::auth::LogoutPage)
            (ProfileEmail, Private, "/profile/email", (leptos_router::StaticSegment("profile"), leptos_router::StaticSegment("email")), $crate::email::EmailPage)
            (Profile, Private, "/profile", leptos_router::StaticSegment("profile"), $crate::profile::ProfilePage)
            (Sessions, Private, "/sessions", leptos_router::StaticSegment("sessions"), $crate::sessions::SessionsPage)
            (Passkeys, Private, "/passkeys", leptos_router::StaticSegment("passkeys"), $crate::passkeys::PasskeysPage)
            (Audiences, Private, "/audiences", leptos_router::StaticSegment("audiences"), $crate::audiences::AudiencesPage)
            (Invites, Private, "/invites", leptos_router::StaticSegment("invites"), $crate::invites::InvitesPage)
            (AdminBackups, Private, "/admin/backups", (leptos_router::StaticSegment("admin"), leptos_router::StaticSegment("backups")), $crate::backup::BackupSettingsPage)
            (AdminSite, Private, "/admin/site", (leptos_router::StaticSegment("admin"), leptos_router::StaticSegment("site")), $crate::site::SiteSettingsPage)
            (AdminSmtp, Private, "/admin/smtp", (leptos_router::StaticSegment("admin"), leptos_router::StaticSegment("smtp")), $crate::smtp::SmtpSettingsPage)
            (AdminWebsub, Private, "/admin/websub", (leptos_router::StaticSegment("admin"), leptos_router::StaticSegment("websub")), $crate::websub::WebsubPage)
            (PostsNew, Private, "/posts/new", (leptos_router::StaticSegment("posts"), leptos_router::StaticSegment("new")), $crate::posts::CreatePostPage)
            (Drafts, Private, "/drafts", leptos_router::StaticSegment("drafts"), $crate::posts::DraftsPage)
            (Scheduled, Private, "/scheduled", leptos_router::StaticSegment("scheduled"), $crate::posts::ScheduledPage)
            (Media, Private, "/media", leptos_router::StaticSegment("media"), $crate::media::MediaPage)
            (Themes, Private, "/themes", leptos_router::StaticSegment("themes"), $crate::themes::ThemesPage)
            (History, Private, "/history", leptos_router::StaticSegment("history"), $crate::posts::HistoryPage)
            (PostHistory, Private, "/posts/:post_id/history", (leptos_router::StaticSegment("posts"), leptos_router::ParamSegment("post_id"), leptos_router::StaticSegment("history")), $crate::posts::PostHistoryPage)
            (RevisionHistory, Private, "/posts/:post_id/history/:revision_id", (leptos_router::StaticSegment("posts"), leptos_router::ParamSegment("post_id"), leptos_router::StaticSegment("history"), leptos_router::ParamSegment("revision_id")), $crate::posts::RevisionHistoryDetailPage)
            (PostEdit, Private, "/posts/:post_id/edit", (leptos_router::StaticSegment("posts"), leptos_router::ParamSegment("post_id"), leptos_router::StaticSegment("edit")), $crate::posts::EditPostPage)
            (VerifyEmail, Public, "/verify-email", leptos_router::StaticSegment("verify-email"), $crate::email::VerifyEmailPage)
            (ForgotPassword, Public, "/forgot-password", leptos_router::StaticSegment("forgot-password"), $crate::password_reset::ForgotPasswordPage)
            (ResetPassword, Public, "/reset-password", leptos_router::StaticSegment("reset-password"), $crate::password_reset::ResetPasswordPage)
            (SiteTag, Public, "/tags/:tag", (leptos_router::StaticSegment("tags"), leptos_router::ParamSegment("tag")), $crate::posts::SiteTagPage)
            (UserTag, Public, "/:username/tags/:tag", (leptos_router::ParamSegment("username"), leptos_router::StaticSegment("tags"), leptos_router::ParamSegment("tag")), $crate::posts::UserTagPage)
            (UserTimeline, Public, "/:username", leptos_router::ParamSegment("username"), $crate::posts::UserTimelinePage)
            (Post, Public, "/~:username/:year/:month/:day/:slug", ($crate::route_segments::TildeUsername("username"), leptos_router::ParamSegment("year"), leptos_router::ParamSegment("month"), leptos_router::ParamSegment("day"), leptos_router::ParamSegment("slug")), $crate::posts::PostPage)
        }
    };
}

macro_rules! route_catalog_entries {
    ($(($name:ident, $access:ident, $pattern:literal, $path:expr, $view:path))*) => {
        &[$(catalog_entry!($name, $access, $pattern, $path, $view)),*]
    };
}

fn route_catalog() -> &'static [RouteDeclaration] {
    app_routes!(route_catalog_entries)
}

#[cfg(test)]
mod tests {
    use super::{Access, PrivateDestination, private_inventory_conforms, route_catalog};

    #[test]
    fn route_catalog_conforms_to_the_approved_private_inventory() {
        assert_eq!(
            route_catalog().len(),
            31,
            "every declared route is classified"
        );
        assert!(private_inventory_conforms(route_catalog()));
    }

    #[test]
    fn unexpected_private_classification_fails_conformance() {
        let mut routes = route_catalog().to_vec();
        routes
            .iter_mut()
            .find(|route| route.name == "Register")
            .expect("Register is declared")
            .access = Access::Private;

        assert!(!private_inventory_conforms(&routes));
    }

    #[test]
    fn private_destination_round_trips_path_query_and_fragment() {
        let destination = PrivateDestination::parse("/posts/42/history/7?order=oldest#revision")
            .expect("a private route is a valid destination");
        assert_eq!(
            destination.as_ref(),
            "/posts/42/history/7?order=oldest#revision"
        );
    }

    #[test]
    fn parameterized_private_routes_require_their_complete_shape() {
        for target in ["/posts/42/history", "/posts/42/history/7", "/posts/42/edit"] {
            assert!(PrivateDestination::parse(target).is_some(), "{target}");
        }
        for target in ["/posts/history", "/posts/42", "/posts/42/history/7/extra"] {
            assert!(PrivateDestination::parse(target).is_none(), "{target}");
        }
    }

    #[test]
    fn rejects_unsafe_or_non_private_destinations() {
        for target in [
            "https://evil.example/app",
            "//evil.example/app",
            "/\\evil.example/app",
            "/login",
            "/logout",
            "/media/file.png",
            "/unknown",
            "/posts/%",
            "/posts/%/edit",
            "/posts/../edit",
            "/posts/%2e%2e/edit",
            "/app with-space",
        ] {
            assert!(PrivateDestination::parse(target).is_none(), "{target}");
        }
    }
}
