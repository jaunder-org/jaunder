use super::markup;
use crate::auth;
use crate::avatar::Avatar;
use crate::icon::{Icon, Icons};
use crate::registration;
use common::{
    registration::RegistrationPolicy, root_relative_url::RootRelativeUrl, username::Username,
};
use leptos::prelude::*;
use leptos_router::hooks;

/// A single nav item in the sidebar.
#[component]
fn SidebarNavItem(
    test_key: &'static str,
    label: &'static str,
    icon_path: &'static str,
    active: bool,
    href: Option<&'static RootRelativeUrl>,
) -> impl IntoView {
    let class = if active {
        "j-nav-item is-active"
    } else {
        "j-nav-item"
    };
    let test_selector = (test_key == "history").then_some("history-nav-link");
    let inner = view! {
        <Icon path=icon_path size=16 />
        <span>{label}</span>
    };
    match href {
        Some(href) => view! {
            <a class=class href=href.to_string() data-test=test_selector>
                {inner}
            </a>
        }
        .into_any(),
        None => view! { <div class=class>{inner}</div> }.into_any(),
    }
}

/// The left navigation sidebar. Reads session and current location from context.
#[component]
pub fn Sidebar() -> impl IntoView {
    let location = hooks::use_location();
    let active_for_path = move || markup::active_key(&location.pathname.get()).unwrap_or("");

    // The shared session context (#591) is the single source: its `current` signal
    // is marker-seeded (flash-free for BOTH username and operator chrome —
    // `is_operator` rides in the marker) and the reconcile keeps it current. The
    // anonymous sidebar is the pure `markup::render_sidebar` (the SAME code the
    // projector server-renders) injected via `inner_html`, so a seeded first paint
    // and the reactive re-render coincide (flash-free). `display:contents` keeps the
    // host wrapper out of the aside's layout.
    let session = auth::use_session().current;
    let policy = Resource::new(
        move || session.get().is_some(),
        |is_authenticated| async move {
            if is_authenticated {
                Some(registration::get_policy().await)
            } else {
                None
            }
        },
    );
    view! {
        <aside class="j-sidebar">
            {move || {
                let active_key = active_for_path();
                match session.get() {
                    None => {
                        markup::render_sidebar(active_key)
                            .inject_into(leptos::html::div().class("j-contents"))
                            .into_any()
                    }
                    Some(user) => {
                        let policy = policy
                            .get()
                            .flatten()
                            .and_then(Result::ok)
                            .unwrap_or(RegistrationPolicy::Closed);
                        authed_sidebar(active_key, &user.username, user.is_operator, policy)
                            .into_any()
                    }
                }
            }}
        </aside>
    }
}

/// The authenticated sidebar chrome (brand, search, navigation, and footer avatar).
/// Shared by the marker-seeded initial render and the reconciled render (#181) so
/// both are byte-for-byte the same authed markup; only its inputs change from
/// awaited values to these params.
fn authed_sidebar(
    active_key: &str,
    username: &Username,
    is_operator: bool,
    policy: RegistrationPolicy,
) -> impl IntoView {
    let active_key = active_key.to_string();
    let username = username.clone();
    view! {
        <div class="j-contents">
            <a class="j-brand" href="/">
                <div class="j-brand-mark">"j"</div>
                <div class="j-brand-text">"Jaunder"</div>
            </a>
            <div class="j-search">
                <Icon path=Icons::SEARCH size=14 />
                <span>"Search"</span>
                <span class="j-kbd">"⌘K"</span>
            </div>
            <nav class="j-nav" data-jaunder-part="primary-navigation">
                {markup::nav_items(policy, is_operator)
                    .map(|item| {
                        let is_active = item.key == active_key.as_str();
                        view! {
                            <SidebarNavItem
                                test_key=item.key
                                label=item.label
                                icon_path=item.icon_path
                                active=is_active
                                href=item.href.as_ref()
                            />
                        }
                    })
                    .collect::<Vec<_>>()}
            </nav>
            <div class="j-sb-foot">
                <Avatar name=&username size=28 />
                <div class="j-sb-foot-body">
                    <div class="j-sb-foot-name">{username.to_string()}</div>
                </div>
                <a class="j-sign-out" href="/logout">
                    "Sign out"
                </a>
            </div>
        </div>
    }
}
