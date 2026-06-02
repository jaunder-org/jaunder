use common::feed::{canonicalize, FeedFormat, FeedSurface};
use leptos::prelude::*;
use leptos_meta::Link;

/// Renders feed auto-discovery link tags for RSS, Atom, and JSON Feed.
/// The component itself is invisible; it hoists `<link>` tags into the document head.
#[allow(clippy::must_use_candidate)]
#[component]
#[allow(clippy::needless_pass_by_value)]
pub fn FeedDiscovery(surface: FeedSurface) -> impl IntoView {
    // Build a human-readable label for the feed based on the surface.
    let label = surface_label(&surface);

    view! {
        <Link
            rel="alternate"
            type_="application/rss+xml"
            title=format!("{label} (RSS)")
            href=canonicalize(&surface, FeedFormat::Rss)
        />
        <Link
            rel="alternate"
            type_="application/atom+xml"
            title=format!("{label} (Atom)")
            href=canonicalize(&surface, FeedFormat::Atom)
        />
        <Link
            rel="alternate"
            type_="application/feed+json"
            title=format!("{label} (JSON Feed)")
            href=canonicalize(&surface, FeedFormat::Json)
        />
    }
}

/// Renders the `RSD` (`EditURI`) autodiscovery link for a user's `AtomPub`
/// publishing endpoint. Like [`FeedDiscovery`], it is invisible and only hoists
/// a `<link>` into the document head; editors such as `MarsEdit` follow it.
#[allow(clippy::must_use_candidate)]
#[component]
#[allow(clippy::needless_pass_by_value)]
pub fn RsdDiscovery(username: String) -> impl IntoView {
    view! {
        <Link
            rel="EditURI"
            type_="application/rsd+xml"
            title="AtomPub (RSD)"
            href=rsd_href(&username)
        />
    }
}

/// Returns the `RSD` discovery URL for a user's page.
fn rsd_href(username: &str) -> String {
    format!("/~{username}/rsd.xml")
}

/// Generate a human-readable label for the feed based on the surface.
fn surface_label(surface: &FeedSurface) -> String {
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

    #[test]
    fn labels_site_surface() {
        assert_eq!(surface_label(&FeedSurface::Site), "Site feed");
    }

    #[test]
    fn rsd_href_targets_user_discovery_doc() {
        assert_eq!(rsd_href("alice"), "/~alice/rsd.xml");
    }

    #[test]
    fn labels_site_tag_surface() {
        assert_eq!(
            surface_label(&FeedSurface::SiteTag { tag: "rust".into() }),
            "#rust feed"
        );
    }

    #[test]
    fn labels_user_surface() {
        assert_eq!(
            surface_label(&FeedSurface::User {
                username: "alice".into()
            }),
            "@alice feed"
        );
    }

    #[test]
    fn labels_user_tag_surface() {
        assert_eq!(
            surface_label(&FeedSurface::UserTag {
                username: "bob".into(),
                tag: "leptos".into()
            }),
            "@bob #leptos feed"
        );
    }
}
