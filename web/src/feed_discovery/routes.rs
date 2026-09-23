//! Typed route context shared by the public rail and the discovery destination.
//! The parser accepts only feed-bearing timelines; a Post permalink or private
//! route never acquires a Syndication Feed marker by matching a path prefix.

use crate::posts::ListingRoute;
use common::{
    feed::FeedSurface,
    seed::{PageSeed, TimelineOrder},
};

/// Existing listing policy for a discovery destination; Local uses its own
/// timeline endpoint, and every other context uses the shared listing route.
#[must_use]
pub fn listing_route_for_discovery(surface: &FeedSurface) -> Option<ListingRoute> {
    match surface {
        FeedSurface::Site => None,
        FeedSurface::SiteTag { tag } => Some(ListingRoute::SiteTag(
            Some(tag.clone()),
            TimelineOrder::Newest,
        )),
        FeedSurface::User { username } => Some(ListingRoute::Profile(
            Some(username.clone()),
            TimelineOrder::Newest,
        )),
        FeedSurface::UserTag { username, tag } => Some(ListingRoute::UserTag(
            Some(username.clone()),
            Some(tag.clone()),
            TimelineOrder::Newest,
        )),
    }
}

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

/// A User-tag shell is not a feed-bearing timeline until its User resolves.
/// Other valid timeline contexts do not require that extra existence check.
#[must_use]
pub fn timeline_marker_surface(
    path: &str,
    confirmed_user_tag: Option<&FeedSurface>,
) -> Option<FeedSurface> {
    let surface = timeline_surface(path)?;
    if matches!(surface, FeedSurface::UserTag { .. }) && confirmed_user_tag != Some(&surface) {
        return None;
    }
    Some(surface)
}

/// Revoke a resolved User-tag when navigation leaves that exact timeline.
#[must_use]
pub fn confirmed_user_tag_on_path(
    confirmed: Option<FeedSurface>,
    path: &str,
) -> Option<FeedSurface> {
    confirmed.filter(|current| timeline_surface(path).as_ref() == Some(current))
}

/// Keep an index's prior context out of the paint during route transitions.
#[must_use]
pub fn visible_discovery_surface(surface: Option<FeedSurface>, path: &str) -> Option<FeedSurface> {
    surface.filter(|current| discovery_surface(path).as_ref() == Some(current))
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
    fn discovery_uses_each_existing_listing_policy() {
        let username: common::username::Username = "alice".parse().unwrap();
        let tag: common::tag::Tag = "rust".parse().unwrap();
        assert_eq!(listing_route_for_discovery(&FeedSurface::Site), None);
        assert_eq!(
            listing_route_for_discovery(&FeedSurface::SiteTag { tag: tag.clone() }),
            Some(ListingRoute::SiteTag(
                Some(tag.clone()),
                TimelineOrder::Newest
            ))
        );
        assert_eq!(
            listing_route_for_discovery(&FeedSurface::User {
                username: username.clone()
            }),
            Some(ListingRoute::Profile(
                Some(username.clone()),
                TimelineOrder::Newest
            ))
        );
        assert_eq!(
            listing_route_for_discovery(&FeedSurface::UserTag {
                username: username.clone(),
                tag: tag.clone()
            }),
            Some(ListingRoute::UserTag(
                Some(username),
                Some(tag),
                TimelineOrder::Newest
            ))
        );
    }

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
    fn unknown_user_tag_has_no_marker_and_stale_index_cannot_paint() {
        let user_tag = FeedSurface::UserTag {
            username: "alice".parse().unwrap(),
            tag: "rust".parse().unwrap(),
        };
        assert_eq!(timeline_marker_surface("/~alice/tags/rust", None), None);
        assert_eq!(
            timeline_marker_surface("/~alice/tags/rust", Some(&user_tag)),
            Some(user_tag.clone())
        );
        assert_eq!(timeline_marker_surface("/", None), Some(FeedSurface::Site));
        assert_eq!(
            confirmed_user_tag_on_path(Some(user_tag.clone()), "/~alice/tags/rust"),
            Some(user_tag.clone())
        );
        assert_eq!(confirmed_user_tag_on_path(Some(user_tag), "/feeds"), None);
        assert_eq!(
            visible_discovery_surface(Some(FeedSurface::Site), "/feeds"),
            Some(FeedSurface::Site)
        );
        assert_eq!(
            visible_discovery_surface(Some(FeedSurface::Site), "/~alice/feeds"),
            None
        );
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
