use leptos::prelude::*;

/// The reactive half of the twin: title + optional sub + optional right-slot
/// children. Twins [`render`] — keep their markup coincident.
#[component]
pub fn Topbar(
    #[prop(into)] title: TextProp,
    #[prop(optional, into)] sub: Option<TextProp>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <header class="j-topbar" data-jaunder-part="masthead">
            <div>
                <span class="j-site-identity" data-jaunder-part="site-title">
                    "Jaunder"
                </span>
                <h1>{move || title.get()}</h1>
                {sub
                    .map(|s| {
                        view! { <div class="j-sub">{move || s.get()}</div> }
                    })}
            </div>
            <div class="j-topbar-right">{children.map(|c| c())}</div>
        </header>
    }
}
