use leptos::prelude::*;

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

/// Shared icon-and-tooltip content for compact buttons with an external accessible name.
#[component]
pub fn IconButtonContent(path: &'static str, tooltip: &'static str) -> impl IntoView {
    view! {
        <span aria-hidden="true">
            <Icon path=path />
        </span>
        <span class="j-button-tooltip" role="tooltip">
            {tooltip}
        </span>
    }
}
