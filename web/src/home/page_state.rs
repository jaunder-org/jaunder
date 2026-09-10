//! Host-compiled public-navigation decision for the home page.
//!
//! The site timeline response is a route presentation, not merely rows: the
//! destination theme and page must commit together when CSR navigation resolves.

use common::{
    seed::{Page, PageSeed, PublicPresentation, RenderedPost, TimelineCursor, TimelineOrder},
    theme::PublishedThemePresentation,
};

/// Splits the server-owned site destination into the two values the reactive
/// commit needs, preserving the theme carried by the response.
#[must_use]
pub fn site_destination(
    presentation: PublicPresentation<Page<RenderedPost, TimelineCursor>>,
) -> (
    PublishedThemePresentation,
    Page<RenderedPost, TimelineCursor>,
) {
    (presentation.theme, presentation.page)
}

/// Selects the matching projector page and its order for the public home timeline.
///
/// A seed from any other route is not adoptable; without a matching projector seed,
/// CSR starts in the default newest-first order.
#[must_use]
pub fn site_timeline_seed(
    seed: Option<PageSeed>,
) -> (TimelineOrder, Option<Page<RenderedPost, TimelineCursor>>) {
    match seed {
        Some(PageSeed::SiteTimeline { order, page }) => (order, Some(page)),
        _ => (TimelineOrder::default(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::{site_destination, site_timeline_seed};
    use common::{
        seed::{Page, PageSeed, PublicPresentation, TimelineOrder},
        theme::{PublishedThemePresentation, Theme},
    };

    #[test]
    fn destination_keeps_the_server_resolved_theme() {
        let (theme, page) = site_destination(PublicPresentation {
            theme: PublishedThemePresentation::built_in(Theme::Reader),
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        });

        assert_eq!(theme, PublishedThemePresentation::built_in(Theme::Reader));
        assert!(page.posts.is_empty());
    }

    #[test]
    fn site_timeline_seed_preserves_oldest_order_and_rejects_other_routes() {
        let (order, page) = site_timeline_seed(Some(PageSeed::SiteTimeline {
            order: TimelineOrder::Oldest,
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        }));
        assert_eq!(order, TimelineOrder::Oldest);
        assert!(page.is_some());

        let (order, page) = site_timeline_seed(Some(PageSeed::Profile {
            username: "alice".parse().expect("valid username"),
            order: TimelineOrder::Oldest,
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        }));
        assert_eq!(order, TimelineOrder::Newest);
        assert!(page.is_none());
    }
}
