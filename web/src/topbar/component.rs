use leptos::prelude::*;

use crate::hub::{Hub, HubControls};

/// The reactive half of the twin: title + optional sub + optional right-slot
/// children, followed by the Settings/Account corner controls. Twins [`render`] —
/// keep their markup coincident.
///
/// A page that a configuration hub lists also gets a link back to that hub,
/// derived from the hub catalog so no page has to declare its own parent.
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
                            theme
                                .get()
                                .map(|theme| {
                                    crate::app::render_theme_logo(&theme)
                                        .inject_into(leptos::html::div().class("j-contents"))
                                })
                        })
                }}
                {move || {
                    Hub::parent_of(&location.pathname.get())
                        .map(|hub| {
                            view! {
                                <a class="j-topbar-back" href=hub.path()>
                                    {format!("\u{2190} {}", hub.label())}
                                </a>
                            }
                        })
                }}
                <h1>{move || title.get()}</h1>
                {sub
                    .map(|s| {
                        view! { <div class="j-sub">{move || s.get()}</div> }
                    })}
            </div>
            <div class="j-topbar-right">{children.map(|c| c())} <HubControls /></div>
        </header>
    }
}
