use super::markup;
use crate::auth;
use crate::avatar::Avatar;
use crate::icon::{Icon, Icons};
use crate::registration;
use common::{
    registration::RegistrationPolicy, root_relative_url::RootRelativeUrl, username::Username,
};
use leptos::prelude::*;

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

/// A static source row in the sidebar sources section.
#[component]
fn SidebarSource(proto: &'static str, name: &'static str, sub: &'static str) -> impl IntoView {
    let dot_style = format!("width:8px;height:8px;border-radius:4px;background:var(--c-{proto})");
    view! {
        <div class="j-source">
            <span class="j-dot" style=dot_style></span>
            <div style="flex:1;min-width:0">
                <div class="j-source-name">{name}</div>
                <div class="j-source-sub">{sub}</div>
            </div>
        </div>
    }
}

/// The left navigation sidebar. Reads theme and current-user from context.
/// `active`: the key of the currently active nav item (e.g. `"home"`).
#[component]
pub fn Sidebar(#[prop(optional)] active: Option<String>) -> impl IntoView {
    let active_key = active.unwrap_or_default();

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
    let anon_html = markup::render_sidebar(&active_key);
    view! {
        <aside class="j-sidebar">
            {move || match session.get() {
                None => {
                    anon_html
                        .clone()
                        .inject_into(leptos::html::div().style("display:contents"))
                        .into_any()
                }
                Some(user) => {
                    let policy = policy
                        .get()
                        .flatten()
                        .and_then(Result::ok)
                        .unwrap_or(RegistrationPolicy::Closed);
                    authed_sidebar(&active_key, &user.username, user.is_operator, policy).into_any()
                }
            }}
        </aside>
    }
}

/// The authenticated sidebar chrome (brand, search, nav + policy-authorized links,
/// sources, footer avatar). Shared by the marker-seeded initial render and the
/// reconciled render (#181) so both are byte-for-byte the same authed markup —
/// only its inputs change from awaited values to these params.
fn authed_sidebar(
    active_key: &str,
    username: &Username,
    is_operator: bool,
    policy: RegistrationPolicy,
) -> impl IntoView {
    let active_key = active_key.to_string();
    let username = username.clone();
    view! {
        <div style="display:contents">
            <a class="j-brand" href="/" style="text-decoration:none;color:inherit">
                <div class="j-brand-mark">"j"</div>
                <div class="j-brand-text">"Jaunder"</div>
            </a>
            <div class="j-search">
                <Icon path=Icons::SEARCH size=14 />
                <span>"Search"</span>
                <span class="j-kbd">"⌘K"</span>
            </div>
            <nav class="j-nav">
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
            <div>
                <div class="j-sb-head">
                    <span>"Sources"</span>
                    <span class="j-sb-add">"+"</span>
                </div>
                {markup::SIDEBAR_SOURCES
                    .iter()
                    .map(|&(proto, name, sub)| {
                        view! { <SidebarSource proto=proto name=name sub=sub /> }
                    })
                    .collect::<Vec<_>>()}
            </div>
            <div class="j-sb-foot">
                <Avatar name=&username size=28 />
                <div style="font-size:13px;flex:1;min-width:0">
                    <div style="font-weight:500">{username.to_string()}</div>
                </div>
                <a href="/logout" style="font-size:11px;color:var(--muted)">
                    "Sign out"
                </a>
            </div>
        </div>
    }
}
