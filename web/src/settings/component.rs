use leptos::prelude::*;
use leptos_router::hooks::use_location;

use common::registration::RegistrationPolicy;

use super::catalog::{AreaGroup, SettingsArea, account_groups, settings_groups};
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
pub fn AreaControls() -> impl IntoView {
    let session = crate::auth::use_session().current;
    let location = use_location();
    move || {
        session.get().map(|user| {
            let current = SettingsArea::owning(&location.pathname.get());
            let aria_current = move |area| (current == Some(area)).then_some("page");
            view! {
                <div class="j-area-controls">
                    <a
                        class="j-area-control"
                        href=SettingsArea::Settings.path()
                        aria-label=SettingsArea::Settings.label()
                        aria-current=aria_current(SettingsArea::Settings)
                    >
                        <Icon path=Icons::COG size=18 />
                    </a>
                    <a
                        class="j-area-control"
                        href=SettingsArea::Account.path()
                        aria-label=SettingsArea::Account.label()
                        aria-current=aria_current(SettingsArea::Account)
                    >
                        <Avatar name=&user.username size=24 />
                    </a>
                </div>
            }
        })
    }
}

/// The grouped, linked rows of one settings area page.
fn area_groups(groups: Vec<AreaGroup>) -> impl IntoView {
    groups
        .into_iter()
        .map(|group| {
            view! {
                <section class="j-card j-area-group">
                    <div class="j-card-head">
                        <h2>{group.title}</h2>
                    </div>
                    <ul class="j-area-rows">
                        {group
                            .links
                            .into_iter()
                            .map(|link| {
                                view! {
                                    <li>
                                        <a class="j-area-row" href=link.href.to_string()>
                                            <span class="j-area-row-text">
                                                <span class="j-area-row-label">{link.label}</span>
                                                <span class="j-area-row-description">
                                                    {link.description}
                                                </span>
                                            </span>
                                            <span class="j-area-row-chevron" aria-hidden="true">
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

/// Settings page — the User's publishing preferences, Invitations where the
/// Registration Policy allows, and site administration for operators.
///
/// If the Registration Policy cannot be loaded, the page still lists every
/// other row, treating the policy as Closed (no Invitations row), and says that
/// Invitations could not be checked.
#[component]
pub fn SettingsPage() -> impl IntoView {
    let session = crate::auth::use_session().current;
    let policy = Resource::new(|| (), |()| crate::registration::get_policy());
    view! {
        <Topbar title=SettingsArea::Settings.label() sub="Preferences and site administration" />
        <div class="j-scroll">
            <div class="j-settings">
                {move || {
                    let (policy, notice) = match policy.get() {
                        None => {
                            return view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any();
                        }
                        Some(Ok(policy)) => (policy, None),
                        Some(Err(error)) => {
                            let notice = format!(
                                "Invitation availability could not be loaded: {error}",
                            );
                            (RegistrationPolicy::Closed, Some(notice))
                        }
                    };
                    let is_operator = session
                        .with(|user| user.as_ref().is_some_and(|user| user.is_operator));
                    view! {
                        {notice.map(|notice| view! { <p class="error">{notice}</p> })}
                        {area_groups(settings_groups(policy, is_operator))}
                    }
                        .into_any()
                }}
            </div>
        </div>
    }
}

/// Account page — who is signed in, sign-in and security pages, and Sign out.
#[component]
pub fn AccountPage() -> impl IntoView {
    let session = crate::auth::use_session().current;
    view! {
        <Topbar title=SettingsArea::Account.label() sub="Sign-in and security" />
        <div class="j-scroll">
            <div class="j-settings">
                {move || {
                    session
                        .get()
                        .map(|user| {
                            view! {
                                <div class="j-area-identity">
                                    <Avatar name=&user.username size=44 />
                                    <span class="j-area-identity-name">
                                        {user.username.to_string()}
                                    </span>
                                </div>
                            }
                        })
                }} {area_groups(account_groups())} <a class="j-btn" href="/logout">
                    "Sign out"
                </a>
            </div>
        </div>
    }
}
