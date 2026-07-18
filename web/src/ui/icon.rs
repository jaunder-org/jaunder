//! Icon — one inline SVG glyph. The pure [`render`] (server projector) and the
//! reactive [`Icon`] (CSR client) are twins: the same `<svg class="j-icon">`
//! markup produced two ways. Co-located per ADR-0056.

use leptos::prelude::*;

/// SVG path `d` strings — re-exported from the pure `render` layer so the reactive
/// [`Icon`] component and the pure [`render`] twin share one source of truth.
pub use crate::render::Icons;

/// One inline icon `<svg class="j-icon">`, matching the reactive [`Icon`].
#[must_use]
pub(crate) fn render(path: &str, size: u32) -> String {
    format!(
        concat!(
            "<svg class=\"j-icon\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 20 20\" ",
            "fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" ",
            "stroke-linejoin=\"round\"><path d=\"{path}\"></path></svg>",
        ),
        size = size,
        path = path,
    )
}

/// The reactive half of the twin: one inline `<svg class="j-icon">` from a `path`.
/// Twins [`render`] — keep their markup coincident.
#[component]
pub fn Icon(path: &'static str, #[prop(default = 16)] size: u32) -> impl IntoView {
    view! {
        <svg
            class="j-icon"
            width=size
            height=size
            viewBox="0 0 20 20"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
        >
            <path d=path />
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::{render, Icons};

    #[test]
    fn icon_matches_reactive_component_markup() {
        assert_eq!(
            render(Icons::HOME, 16),
            format!(
                "<svg class=\"j-icon\" width=\"16\" height=\"16\" viewBox=\"0 0 20 20\" \
                 fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" \
                 stroke-linejoin=\"round\"><path d=\"{}\"></path></svg>",
                Icons::HOME
            )
        );
    }
}
