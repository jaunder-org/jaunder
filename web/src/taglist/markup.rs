use std::fmt::Write;

use common::seed::TagSummary;

use crate::render::{escape_html, TagCtx};

/// The footer tag chips: a `<span class="j-tag-list">` of `<span class="j-tag-cell">`
/// chips, each a `#display` link to `/tags/:slug`, plus the "· here" link under
/// [`TagCtx::ForUser`]. Byte-identical to the reactive [`TagList`]; keep their
/// markup coincident.
#[must_use]
pub(crate) fn render(tags: &[TagSummary], ctx: &TagCtx) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let mut out = String::from("<span class=\"j-tag-list\">");
    for tag in tags {
        let slug = escape_html(&tag.slug);
        let _ = write!(
            out,
            "<span class=\"j-tag-cell\"><a class=\"j-tag\" href=\"/tags/{slug}\">#{display}</a>",
            display = escape_html(&tag.display),
        );
        if let TagCtx::ForUser(username) = ctx {
            let _ = write!(
                out,
                "<a class=\"j-tag-here\" href=\"/~{user}/tags/{slug}\" title=\"On this blog\">\u{00b7} here</a>",
                user = escape_html(username),
            );
        }
        out.push_str("</span>");
    }
    out.push_str("</span>");
    out
}

#[cfg(test)]
mod tests {
    use super::render;
    use common::seed::TagSummary;
    use common::test_support::parse_username;

    use crate::render::TagCtx;

    #[test]
    fn tag_list_site_wide_has_hash_chip_and_no_here_link() {
        let tags = [TagSummary {
            slug: "rust".parse().unwrap(),
            display: "Rust".parse().unwrap(),
        }];
        let html = render(&tags, &TagCtx::SiteWide);
        assert_eq!(
            html,
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
        let html = render(&tags, &TagCtx::ForUser(parse_username("alice")));
        assert!(
            html.contains(
                "<a class=\"j-tag-here\" href=\"/~alice/tags/rust\" title=\"On this blog\">"
            ),
            "{html}"
        );
    }

    #[test]
    fn empty_tag_list_renders_nothing() {
        assert_eq!(render(&[], &TagCtx::SiteWide), "");
    }
}
