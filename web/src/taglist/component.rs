use leptos::prelude::*;

use crate::render::TagCtx;
use crate::tags::TagSummary;

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
