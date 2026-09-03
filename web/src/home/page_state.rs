//! Host-compiled public-navigation decision for the home page.
//!
//! The site timeline response is a route presentation, not merely rows: the
//! destination theme and page must commit together when CSR navigation resolves.

use common::{
    seed::{Page, PublicPresentation, RenderedPost},
    theme::Theme,
};

/// Splits the server-owned site destination into the two values the reactive
/// commit needs, preserving the theme carried by the response.
#[must_use]
pub fn site_destination(
    presentation: PublicPresentation<Page<RenderedPost>>,
) -> (Theme, Page<RenderedPost>) {
    (presentation.theme, presentation.page)
}

#[cfg(test)]
mod tests {
    use super::site_destination;
    use common::{
        seed::{Page, PublicPresentation},
        theme::Theme,
    };

    #[test]
    fn destination_keeps_the_server_resolved_theme() {
        let (theme, page) = site_destination(PublicPresentation {
            theme: Theme::Reader,
            page: Page {
                posts: vec![],
                next_cursor: None,
                has_more: false,
            },
        });

        assert_eq!(theme, Theme::Reader);
        assert!(page.posts.is_empty());
    }
}
