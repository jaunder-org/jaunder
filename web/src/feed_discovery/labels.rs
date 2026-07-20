//! Pure URL/label helpers for the feed-discovery `<link>` tags. Host-tested;
//! consumed by the wasm-only [`super::component`].

use common::feed::FeedSurface;
use common::username::Username;

/// Returns the `RSD` discovery URL for a user's page.
pub(crate) fn rsd_href(username: &Username) -> String {
    format!("/~{username}/rsd.xml")
}

/// Generate a human-readable label for the feed based on the surface.
pub(crate) fn surface_label(surface: &FeedSurface) -> String {
    match surface {
        FeedSurface::Site => "Site feed".to_string(),
        FeedSurface::SiteTag { tag } => format!("#{tag} feed"),
        FeedSurface::User { username } => format!("@{username} feed"),
        FeedSurface::UserTag { username, tag } => format!("@{username} #{tag} feed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_username;

    #[test]
    fn labels_site_surface() {
        assert_eq!(surface_label(&FeedSurface::Site), "Site feed");
    }

    #[test]
    fn rsd_href_targets_user_discovery_doc() {
        assert_eq!(rsd_href(&parse_username("alice")), "/~alice/rsd.xml");
    }

    #[test]
    fn labels_site_tag_surface() {
        assert_eq!(
            surface_label(&FeedSurface::SiteTag {
                tag: "rust".parse().unwrap()
            }),
            "#rust feed"
        );
    }

    #[test]
    fn labels_user_surface() {
        assert_eq!(
            surface_label(&FeedSurface::User {
                username: "alice".parse().unwrap()
            }),
            "@alice feed"
        );
    }

    #[test]
    fn labels_user_tag_surface() {
        assert_eq!(
            surface_label(&FeedSurface::UserTag {
                username: "bob".parse().unwrap(),
                tag: "leptos".parse().unwrap()
            }),
            "@bob #leptos feed"
        );
    }
}
