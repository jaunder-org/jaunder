//! Typed route context shared by the public rail and the discovery destination.
//! The parser accepts only feed-bearing timelines; a Post permalink or private
//! route never acquires a Syndication Feed marker by matching a path prefix.

use common::{feed::FeedSurface, seed::PageSeed};

/// Adopt a projected index only when its typed context matches the current URL.
#[must_use]
pub fn seeded_discovery(seed: Option<PageSeed>, path: &str) -> Option<FeedSurface> {
    match (seed, discovery_surface(path)) {
        (Some(PageSeed::FeedDiscovery(seeded)), Some(route)) if seeded == route => Some(seeded),
        _ => None,
    }
}

/// Resolve a timeline's typed seed without promoting a Post or index page to a feed-bearing timeline.
#[must_use]
pub fn timeline_seed_surface(seed: &PageSeed) -> Option<FeedSurface> {
    match seed {
        PageSeed::SiteTimeline { .. } => Some(FeedSurface::Site),
        PageSeed::SiteTag { tag, .. } => Some(FeedSurface::SiteTag { tag: tag.clone() }),
        PageSeed::Profile { username, .. } => Some(FeedSurface::User {
            username: username.clone(),
        }),
        PageSeed::UserTag { username, tag, .. } => Some(FeedSurface::UserTag {
            username: username.clone(),
            tag: tag.clone(),
        }),
        PageSeed::Permalink(_) | PageSeed::FeedDiscovery(_) => None,
    }
}

/// Resolve an exact public timeline path, independent of viewer identity.
#[must_use]
pub fn timeline_surface(path: &str) -> Option<FeedSurface> {
    if path == "/" {
        return Some(FeedSurface::Site);
    }
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["tags", tag] => Some(FeedSurface::SiteTag {
            tag: tag.parse().ok()?,
        }),
        [user] => Some(FeedSurface::User {
            username: user.strip_prefix('~')?.parse().ok()?,
        }),
        [user, "tags", tag] => Some(FeedSurface::UserTag {
            username: user.strip_prefix('~')?.parse().ok()?,
            tag: tag.parse().ok()?,
        }),
        _ => None,
    }
}

/// Resolve only a nested feed-discovery destination, never a timeline itself.
#[must_use]
pub fn discovery_surface(path: &str) -> Option<FeedSurface> {
    let parent = path.strip_suffix("/feeds")?;
    timeline_surface(if parent.is_empty() { "/" } else { parent })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_adopted_only_for_its_exact_discovery_route() {
        let seed = PageSeed::FeedDiscovery(FeedSurface::Site);
        assert_eq!(
            seeded_discovery(Some(seed.clone()), "/feeds"),
            Some(FeedSurface::Site)
        );
        assert_eq!(seeded_discovery(Some(seed), "/~alice/feeds"), None);
        assert_eq!(seeded_discovery(None, "/feeds"), None);
    }

    #[test]
    fn route_context_is_exact_and_contextual() {
        for (timeline, destination, expected) in [
            ("/", "/feeds", FeedSurface::Site),
            (
                "/tags/rust",
                "/tags/rust/feeds",
                FeedSurface::SiteTag {
                    tag: "rust".parse().unwrap(),
                },
            ),
            (
                "/~alice",
                "/~alice/feeds",
                FeedSurface::User {
                    username: "alice".parse().unwrap(),
                },
            ),
            (
                "/~alice/tags/rust",
                "/~alice/tags/rust/feeds",
                FeedSurface::UserTag {
                    username: "alice".parse().unwrap(),
                    tag: "rust".parse().unwrap(),
                },
            ),
        ] {
            assert_eq!(timeline_surface(timeline), Some(expected.clone()));
            assert_eq!(discovery_surface(destination), Some(expected.clone()));
            assert_eq!(expected.discovery_path(), destination);
        }
        for path in [
            "/app",
            "/profile",
            "/~alice/2026/01/02/hello",
            "/feeds",
            "/~alice/tags/rust/feeds",
            "/tags/%20",
            "/~%20",
            "/tags/rust/more",
        ] {
            assert_eq!(timeline_surface(path), None, "{path}");
        }
        for path in [
            "/",
            "/app/feeds",
            "/~alice/2026/01/02/feeds",
            "/tags/%20/feeds",
        ] {
            assert_eq!(discovery_surface(path), None, "{path}");
        }
    }
}
