//! Host-compiled public-navigation decision for Local.
//!
//! The site timeline response is a route presentation, not merely rows: the
//! destination theme and page must commit together when CSR navigation resolves.

use leptos::prelude::{RwSignal, Set};

use common::{
    registration::RegistrationPolicy,
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
    pub registration_policy: RegistrationPolicy,
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
        registration_policy: presentation.page.registration_policy,
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
    registration_policy: RwSignal<Option<RegistrationPolicy>>,
    destination: LocalDestination,
) {
    identity.set(Some(destination.identity));
    registration_policy.set(Some(destination.registration_policy));
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

/// Adoptable Local presentation extracted from the projector seed.
#[derive(Debug, PartialEq, Eq)]
pub struct LocalSeedState {
    pub order: TimelineOrder,
    pub identity: Option<SiteIdentity>,
    pub registration_policy: Option<RegistrationPolicy>,
    pub page: Option<Page<RenderedPost, TimelineCursor>>,
}

/// Selects the matching projector presentation and its order for public Local.
///
/// A seed from any other route is not adoptable; without a matching projector seed,
/// CSR starts in the default newest-first order.
#[must_use]
pub fn site_timeline_seed(seed: Option<PageSeed>) -> LocalSeedState {
    match seed {
        Some(PageSeed::SiteTimeline {
            identity,
            registration_policy,
            order,
            page,
        }) => LocalSeedState {
            order,
            identity: Some(identity),
            registration_policy: Some(registration_policy),
            page: Some(page),
        },
        _ => LocalSeedState {
            order: TimelineOrder::default(),
            identity: None,
            registration_policy: None,
            page: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{commit_destination, site_destination, site_timeline_base_url, site_timeline_seed};
    use crate::timeline::TimelineState;
    use common::{
        registration::RegistrationPolicy,
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
                registration_policy: RegistrationPolicy::Open,
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
            let resolved_registration_policy = RwSignal::new(None);
            commit_destination(
                state,
                resolved_identity,
                resolved_registration_policy,
                destination,
            );
            assert_eq!(resolved_identity.get(), Some(identity()));
            assert_eq!(
                resolved_registration_policy.get(),
                Some(RegistrationPolicy::Open)
            );
            assert!(state.rows.get().is_empty());
        });
    }

    #[test]
    fn local_identity_is_absent_without_a_matching_projector_seed() {
        let seed = site_timeline_seed(None);
        assert_eq!(seed.order, TimelineOrder::Newest);
        assert_eq!(seed.identity, None);
        assert_eq!(seed.registration_policy, None);
        assert_eq!(seed.page, None);
    }

    #[test]
    fn site_timeline_seed_preserves_identity_oldest_order_and_rejects_other_routes() {
        let seed = site_timeline_seed(Some(PageSeed::SiteTimeline {
            identity: identity(),
            registration_policy: RegistrationPolicy::MemberInvites,
            order: TimelineOrder::Oldest,
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        }));
        assert_eq!(seed.order, TimelineOrder::Oldest);
        assert_eq!(seed.identity, Some(identity()));
        assert_eq!(
            seed.registration_policy,
            Some(RegistrationPolicy::MemberInvites)
        );
        assert!(seed.page.expect("matching Local seed").posts.is_empty());

        let seed = site_timeline_seed(Some(PageSeed::Profile {
            username: "alice".parse().expect("valid username"),
            order: TimelineOrder::Oldest,
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        }));
        assert_eq!(seed.order, TimelineOrder::Newest);
        assert!(seed.identity.is_none());
        assert!(seed.registration_policy.is_none());
        assert!(seed.page.is_none());
    }
}
