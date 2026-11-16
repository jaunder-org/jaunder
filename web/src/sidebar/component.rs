use super::markup;
use crate::auth;
use crate::icon::{Icon, Icons};
use common::root_relative_url::RootRelativeUrl;
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
    let test_selector = markup::test_selector(test_key);
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
    let confirmed_user_tag = crate::feed_discovery::ConfirmedUserTag::current();
    let active_for_path = move || markup::active_key(&location.pathname.get()).unwrap_or("");

    // The shared session context (#591) is the single source: its `current` signal
    // is marker-seeded (flash-free) and the reconcile keeps it current. The
    // anonymous sidebar is the pure `markup::render_sidebar` (the SAME code the
    // projector server-renders) injected via `inner_html`, so a seeded first paint
    // and the reactive re-render coincide (flash-free). `display:contents` keeps the
    // host wrapper out of the aside's layout.
    let session = auth::use_session().current;
    view! {
        <aside class="j-sidebar" data-jaunder-part="navigation-rail">
            {move || {
                let active_key = active_for_path();
                let confirmed = confirmed_user_tag.0.get();
                let surface = crate::feed_discovery::routes::timeline_marker_surface(
                    &location.pathname.get(),
                    confirmed.as_ref(),
                );
                if session.with(Option::is_some) {
                    authed_sidebar(active_key, surface.as_ref()).into_any()
                } else {
                    markup::render_sidebar(active_key, surface.as_ref())
                        .inject_into(leptos::html::div().class("j-contents"))
                        .into_any()
                }
            }}
        </aside>
    }
}

/// The authenticated sidebar chrome (brand, search, Post navigation, and the
/// syndication footer). Account and configuration destinations are not here:
/// they are reached from the page-header corner controls (`crate::hub`, #1733).
fn authed_sidebar(active_key: &str, surface: Option<&common::feed::FeedSurface>) -> impl IntoView {
    let active_key = active_key.to_string();
    view! {
        <div class="j-contents">
            <a class="j-brand" href="/app" data-jaunder-part="site-brand">
                <div class="j-brand-mark">"j"</div>
                <div class="j-brand-text">"Jaunder"</div>
            </a>
            <div class="j-search" data-jaunder-part="navigation-search">
                <Icon path=Icons::SEARCH size=14 />
                <span>"Search"</span>
                <span class="j-kbd">"⌘K"</span>
            </div>
            <nav class="j-nav" data-jaunder-part="primary-navigation">
                {markup::nav_items(true)
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
                {markup::feed_link(surface).inject_into(leptos::html::div().class("j-contents"))}
            </div>
        </div>
    }
}
