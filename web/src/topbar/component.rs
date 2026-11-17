use leptos::prelude::*;

use crate::settings::{AreaControls, SettingsArea};

/// The reactive half of the twin: title + optional sub + optional right-slot
/// children, followed by the Settings/Account corner controls. Twins [`render`],
/// with an intentional asymmetry: this reactive twin additionally renders the
/// signed-in-only corner controls (whose width `html.authed` reserves on the
/// projector paint) and a back link that appears only on private pages a
/// settings area lists. Everything else stays coincident with `topbar::render`.
///
/// The back link is derived from the settings area catalog, so no page has to
/// declare its own parent.
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
                    SettingsArea::parent_of(&location.pathname.get())
                        .map(|area| {
                            view! {
                                <a class="j-topbar-back" href=area.path()>
                                    {format!("\u{2190} {}", area.label())}
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
            <div class="j-topbar-right">{children.map(|c| c())} <AreaControls /></div>
        </header>
    }
}
