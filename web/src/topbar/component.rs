use leptos::prelude::*;

/// The reactive half of the twin: title + optional sub + optional right-slot
/// children. Twins [`render`] — keep their markup coincident.
#[component]
pub fn Topbar(
    #[prop(into)] title: TextProp,
    #[prop(optional, into)] sub: Option<TextProp>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let theme = crate::app::public_theme();
    let location = leptos_router::hooks::use_location();
    view! {
        <header class="j-topbar" data-jaunder-part="masthead">
            <div>
                <span class="j-site-identity" data-jaunder-part="site-title">
                    "Jaunder"
                </span>
                {move || {
                    common::theme::is_public_presentation_path(&location.pathname.get())
                        .then(|| {
                            crate::app::render_theme_logo(&theme.get())
                                .inject_into(leptos::html::div().class("j-contents"))
                        })
                }}
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
