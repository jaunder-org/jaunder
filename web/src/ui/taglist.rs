//! `TagList` — a post's footer tag chips. The pure [`render`] (server projector,
//! injected via `inner_html`) and the reactive [`TagList`] (CSR client, the
//! authored post view the projector never renders) are twins: the same
//! `<span class="j-tag-list">` markup produced two ways. Co-located per ADR-0056.

use std::fmt::Write;

use leptos::prelude::*;

use crate::render::{escape_html, TagCtx};
use crate::tags::TagSummary;

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

/// The reactive half of the twin: a post's tags as clickable chips for the
/// authored post view. Twins [`render`] — keep their markup coincident. See
/// [`TagCtx`] for the linking behavior.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
#[component]
pub fn TagList(tags: Vec<TagSummary>, context: TagCtx) -> impl IntoView {
    if tags.is_empty() {
        return ().into_any();
    }
    let chips: Vec<_> = tags
        .into_iter()
        .map(|tag| {
            let slug = tag.slug.clone();
            let here = match &context {
                TagCtx::ForUser(username) => {
                    let here_href = format!("/~{username}/tags/{slug}");
                    Some(view! {
                        <a class="j-tag-here" href=here_href title="On this blog">
                            "\u{00b7} here"
                        </a>
                    })
                }
                TagCtx::SiteWide => None,
            };
            let chip_href = format!("/tags/{slug}");
            // TagLabel isn't IntoRender/IntoAttributeValue — stringify for the view.
            view! {
                <span class="j-tag-cell">
                    <a class="j-tag" href=chip_href>
                        "#"
                        {tag.display.to_string()}
                    </a>
                    {here}
                </span>
            }
        })
        .collect();
    view! { <span class="j-tag-list">{chips}</span> }.into_any()
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::render::TagCtx;
    use crate::tags::TagSummary;
    use common::username::Username;

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
        let html = render(
            &tags,
            &TagCtx::ForUser("alice".parse::<Username>().unwrap()),
        );
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
