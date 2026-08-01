use maud::html;

use common::seed::TagSummary;

use crate::html::Markup;
use crate::taglist::TagCtx;

/// The footer tag chips: a `<span class="j-tag-list">` of `<span class="j-tag-cell">`
/// chips, each a `#display` link to `/tags/:slug`, plus the "· here" link under
/// [`TagCtx::ForUser`]. Mirrors the reactive [`TagList`]; keep their markup
/// coincident.
#[must_use]
pub(crate) fn render(tags: &[TagSummary], ctx: &TagCtx) -> Markup {
    if tags.is_empty() {
        return Markup::empty();
    }
    Markup::new(html! {
        span class="j-tag-list" {
            @for tag in tags {
                span class="j-tag-cell" {
                    a class="j-tag" href={ "/tags/" (tag.slug) } { "#" (tag.display) }
                    @if let TagCtx::ForUser(username) = ctx {
                        a class="j-tag-here"
                            href={ "/~" (username) "/tags/" (tag.slug) }
                            title="On this blog"
                        {
                            "\u{00b7} here"
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::render;
    use common::seed::TagSummary;
    use common::test_support::parse_username;

    use crate::taglist::TagCtx;

    #[test]
    fn tag_list_site_wide_has_hash_chip_and_no_here_link() {
        let tags = [TagSummary {
            slug: "rust".parse().unwrap(),
            display: "Rust".parse().unwrap(),
        }];
        let html = render(&tags, &TagCtx::SiteWide);
        assert_eq!(
            html.as_str(),
            "<span class=\"j-tag-list\"><span class=\"j-tag-cell\">\
             <a class=\"j-tag\" href=\"/tags/rust\">#Rust</a></span></span>"
        );
    }

    #[test]
    fn tag_list_for_user_adds_here_link() {
        let tags = [TagSummary {
            slug: "rust".parse().unwrap(),
            display: "Rust".parse().unwrap(),
        }];
        let markup = render(&tags, &TagCtx::ForUser(parse_username("alice")));
        let html = markup.as_str();
        assert!(
            html.contains(
                "<a class=\"j-tag-here\" href=\"/~alice/tags/rust\" title=\"On this blog\">"
            ),
            "{html}"
        );
    }

    #[test]
    fn empty_tag_list_renders_nothing() {
        assert_eq!(render(&[], &TagCtx::SiteWide).as_str(), "");
    }
}
