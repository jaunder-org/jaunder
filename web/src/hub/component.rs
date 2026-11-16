use leptos::prelude::*;
use leptos_router::hooks::use_location;

use super::catalog::{Hub, HubGroup, account_groups, settings_groups};
use crate::avatar::Avatar;
use crate::icon::{Icon, Icons};
use crate::topbar::Topbar;

/// The Settings and Account corner controls at the end of every page header,
/// rendered only for a signed-in viewer from the shared session context.
///
/// The anonymous projector paint has no controls; `html.authed` reserves their
/// width before first paint (see `jaunder.css`), so filling them in does not
/// move the page's own header actions.
#[component]
pub fn HubControls() -> impl IntoView {
    let session = crate::auth::use_session().current;
    let location = use_location();
    move || {
        session.get().map(|user| {
            let current = Hub::owning(&location.pathname.get());
            let aria_current = move |hub| (current == Some(hub)).then_some("page");
            view! {
                <div class="j-hub-controls">
                    <a
                        class="j-hub-control"
                        href=Hub::Settings.path()
                        aria-label=Hub::Settings.label()
                        aria-current=aria_current(Hub::Settings)
                    >
                        <Icon path=Icons::COG size=18 />
                    </a>
                    <a
                        class="j-hub-control"
                        href=Hub::Account.path()
                        aria-label=Hub::Account.label()
                        aria-current=aria_current(Hub::Account)
                    >
                        <Avatar name=&user.username size=24 />
                    </a>
                </div>
            }
        })
    }
}

/// The grouped, linked rows of one hub.
fn hub_groups(groups: Vec<HubGroup>) -> impl IntoView {
    groups
        .into_iter()
        .map(|group| {
            view! {
                <section class="j-card j-hub-group">
                    <div class="j-card-head">
                        <h2>{group.title}</h2>
                    </div>
                    <ul class="j-hub-rows">
                        {group
                            .links
                            .into_iter()
                            .map(|link| {
                                view! {
                                    <li>
                                        <a class="j-hub-row" href=link.href.to_string()>
                                            <span class="j-hub-row-text">
                                                <span class="j-hub-row-label">{link.label}</span>
                                                <span class="j-hub-row-description">
                                                    {link.description}
                                                </span>
                                            </span>
                                            <span class="j-hub-row-chevron" aria-hidden="true">
                                                "\u{203a}"
                                            </span>
                                        </a>
                                    </li>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </ul>
                </section>
            }
        })
        .collect::<Vec<_>>()
}

/// Settings hub — the User's publishing preferences, Invitations where the
/// Registration Policy allows, and site administration for operators.
#[component]
pub fn SettingsPage() -> impl IntoView {
    let session = crate::auth::use_session().current;
    let policy = Resource::new(|| (), |()| crate::registration::get_policy());
    view! {
        <Topbar title=Hub::Settings.label() sub="Preferences and site administration" />
        <div class="j-scroll">
            <div class="j-settings">
                {move || match policy.get() {
                    None => view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any(),
                    Some(Err(error)) => {
                        view! { <p class="error">{error.to_string()}</p> }.into_any()
                    }
                    Some(Ok(policy)) => {
                        let is_operator = session
                            .with(|user| user.as_ref().is_some_and(|user| user.is_operator));
                        hub_groups(settings_groups(policy, is_operator)).into_any()
                    }
                }}
            </div>
        </div>
    }
}

/// Account hub — who is signed in, sign-in and security pages, and Sign out.
#[component]
pub fn AccountPage() -> impl IntoView {
    let session = crate::auth::use_session().current;
    view! {
        <Topbar title=Hub::Account.label() sub="Sign-in and security" />
        <div class="j-scroll">
            <div class="j-settings">
                {move || {
                    session
                        .get()
                        .map(|user| {
                            view! {
                                <div class="j-hub-identity">
                                    <Avatar name=&user.username size=44 />
                                    <span class="j-hub-identity-name">
                                        {user.username.to_string()}
                                    </span>
                                </div>
                            }
                        })
                }} {hub_groups(account_groups())} <a class="j-btn" href="/logout">
                    "Sign out"
                </a>
            </div>
        </div>
    }
}
