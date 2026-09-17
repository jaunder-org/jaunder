//! Host-compiled public-navigation decision for Local.
//!
//! The site timeline response is a route presentation, not merely rows: the
//! destination theme and page must commit together when CSR navigation resolves.

use leptos::prelude::{RwSignal, Set};

use common::{
    root_relative_url::RootRelativeUrl,
    seed::{
        LocalTimelinePresentation, Page, PageSeed, PublicPresentation, RenderedPost,
        TimelineCursor, TimelineOrder,
    },
    site::SiteIdentity,
    theme::PublishedThemePresentation,
};

/// A fully resolved Local destination, held until its theme is safe to paint.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocalDestination {
    pub theme: PublishedThemePresentation,
    pub identity: SiteIdentity,
    pub page: Page<RenderedPost, TimelineCursor>,
}

/// Splits the server-owned site destination into the values its reactive commit
/// needs, preserving the theme carried by the response.
#[must_use]
pub fn site_destination(
    presentation: PublicPresentation<LocalTimelinePresentation>,
) -> LocalDestination {
    LocalDestination {
        theme: presentation.theme,
        identity: presentation.page.identity,
        page: presentation.page.page,
    }
}

/// Commits identity and rows together after theme adoption succeeds.
///
/// Until this point Local has no resolved identity: an unseeded route must not
/// paint a fabricated title, tagline, or associated metadata while its destination
/// is in flight.
pub fn commit_destination(
    state: crate::timeline::TimelineState,
    identity: RwSignal<Option<SiteIdentity>>,
    destination: LocalDestination,
) {
    identity.set(Some(destination.identity));
    state.adopt(destination.page);
}

/// Local's fixed bare route, kept in the host-compiled navigation seam so the wasm
/// component only wires it to router navigation.
#[must_use]
pub fn site_timeline_base_url() -> RootRelativeUrl {
    let Ok(url) = "/".parse() else {
        unreachable!("home route is root-relative");
    };
    url
}

/// Selects the matching projector page and its order for public Local.
///
/// A seed from any other route is not adoptable; without a matching projector seed,
/// CSR starts in the default newest-first order.
#[must_use]
pub fn site_timeline_seed(
    seed: Option<PageSeed>,
) -> (
    TimelineOrder,
    Option<SiteIdentity>,
    Option<Page<RenderedPost, TimelineCursor>>,
) {
    match seed {
        Some(PageSeed::SiteTimeline {
            identity,
            order,
            page,
        }) => (order, Some(identity), Some(page)),
        _ => (TimelineOrder::default(), None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::{commit_destination, site_destination, site_timeline_base_url, site_timeline_seed};
    use crate::timeline::TimelineState;
    use common::{
        seed::{LocalTimelinePresentation, Page, PageSeed, PublicPresentation, TimelineOrder},
        site::SiteIdentity,
        theme::{PublishedThemePresentation, Theme},
    };
    use leptos::prelude::*;

    fn identity() -> SiteIdentity {
        SiteIdentity {
            title: "Jaunder".parse().unwrap(),
            tagline: None,
            base_url: None,
        }
    }

    #[test]
    fn site_timeline_uses_the_bare_root_url() {
        let url = site_timeline_base_url();
        let url: &str = url.as_ref();
        assert_eq!(url, "/");
    }

    #[test]
    fn destination_keeps_the_server_resolved_theme_and_identity() {
        let destination = site_destination(PublicPresentation {
            theme: PublishedThemePresentation::built_in(Theme::Reader),
            page: LocalTimelinePresentation {
                identity: identity(),
                page: Page {
                    posts: vec![],
                    next_cursor: None,
                    has_more: false,
                },
            },
        });

        assert_eq!(
            destination.theme,
            PublishedThemePresentation::built_in(Theme::Reader)
        );
        Owner::new().with(|| {
            let state = TimelineState::default();
            let resolved_identity = RwSignal::new(None);
            commit_destination(state, resolved_identity, destination);
            assert_eq!(resolved_identity.get(), Some(identity()));
            assert!(state.rows.get().is_empty());
        });
    }

    #[test]
    fn local_identity_is_absent_without_a_matching_projector_seed() {
        let (_, identity, _) = site_timeline_seed(None);
        assert_eq!(identity, None);
    }

    #[test]
    fn site_timeline_seed_preserves_identity_oldest_order_and_rejects_other_routes() {
        let (order, returned_identity, page) = site_timeline_seed(Some(PageSeed::SiteTimeline {
            identity: identity(),
            order: TimelineOrder::Oldest,
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        }));
        assert_eq!(order, TimelineOrder::Oldest);
        assert_eq!(returned_identity, Some(identity()));
        assert!(page.is_some());

        let (order, returned_identity, page) = site_timeline_seed(Some(PageSeed::Profile {
            username: "alice".parse().expect("valid username"),
            order: TimelineOrder::Oldest,
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        }));
        assert_eq!(order, TimelineOrder::Newest);
        assert!(returned_identity.is_none());
        assert!(page.is_none());
    }
}
