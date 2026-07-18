//! Topbar — the chrome bar. The pure `render` (server projector) and the
//! reactive `Topbar` (CSR client) are twins: the same `<div class="j-topbar">`
//! markup produced two ways. Co-located per ADR-0056.

use leptos::prelude::*;

/// The `<div class="j-topbar">` bar, mirroring the reactive [`Topbar`].
/// `right` is trusted HTML for the `j-topbar-right` slot (e.g. the home Sign-in /
/// Register buttons); `title`/`sub` are escaped.
#[must_use]
pub(crate) fn render(title: &str, sub: Option<&str>, right: &str) -> String {
    let sub_html = sub.map_or_else(String::new, |s| {
        format!(
            "<div class=\"j-sub\">{}</div>",
            crate::render::escape_html(s)
        )
    });
    format!(
        "<div class=\"j-topbar\"><div><h1>{title}</h1>{sub_html}</div><div class=\"j-topbar-right\">{right}</div></div>",
        title = crate::render::escape_html(title),
    )
}

/// The reactive half of the twin: title + optional sub + optional right-slot
/// children. Twins [`render`] — keep their markup coincident.
#[component]
pub fn Topbar(
    #[prop(into)] title: TextProp,
    #[prop(optional, into)] sub: Option<TextProp>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="j-topbar">
            <div>
                <h1>{move || title.get()}</h1>
                {sub
                    .map(|s| {
                        view! { <div class="j-sub">{move || s.get()}</div> }
                    })}
            </div>
            <div class="j-topbar-right">{children.map(|c| c())}</div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn topbar_with_sub_matches_reactive_component_markup() {
        // Must stay byte-identical to the reactive `Topbar`.
        let html = render("Title", Some("Subtitle"), "");
        assert_eq!(
            html,
            "<div class=\"j-topbar\"><div><h1>Title</h1>\
             <div class=\"j-sub\">Subtitle</div></div>\
             <div class=\"j-topbar-right\"></div></div>"
        );
    }

    #[test]
    fn topbar_without_sub_matches_reactive_component_markup() {
        // Must stay byte-identical to the reactive `Topbar`.
        let html = render("Title", None, "");
        assert_eq!(
            html,
            "<div class=\"j-topbar\"><div><h1>Title</h1></div>\
             <div class=\"j-topbar-right\"></div></div>"
        );
    }
}
