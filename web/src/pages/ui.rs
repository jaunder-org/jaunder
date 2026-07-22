// `current_user` is the sidebar's background reconcile (#181), used only in the
// wasm-only correction Effect.
use crate::auth::current_user;
use crate::backup::current_user_is_operator;
use common::username::Username;
use leptos::prelude::*;
use leptos_router::hooks::use_location;

/// Linking context for a post's footer tag chips — re-exported from the pure
/// `render` layer (`SiteWide` / `ForUser`) so the reactive components and the
/// projector share one type. See [`crate::render::TagCtx`]. Anonymous posts get
/// their chips from the pure [`crate::taglist::render`] (byte-coincident with
/// the projector, injected via `inner_html`); the authored post view — which the
/// projector never renders — uses the reactive [`crate::taglist::TagList`].
pub use crate::render::TagCtx as TagContext;

// ─── re-exported from the top-level leaf modules (avatar, icon, taglist, topbar) ─────────────────

pub use crate::{
    avatar::Avatar,
    icon::{Icon, Icons},
    taglist::TagList,
    topbar::Topbar,
};

// ─── 3.8 Sidebar ──────────────────────────────────────────────

/// A single nav item in the sidebar.
#[component]
fn SidebarNavItem(
    label: &'static str,
    icon_path: &'static str,
    active: bool,
    href: Option<&'static str>,
) -> impl IntoView {
    let class = if active {
        "j-nav-item is-active"
    } else {
        "j-nav-item"
    };
    let inner = view! {
        <Icon path=icon_path size=16 />
        <span>{label}</span>
    };
    match href {
        Some(href) => view! {
            <a class=class href=href>
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

    let location = use_location();
    let operator = Resource::new(
        move || location.pathname.get(),
        |_| current_user_is_operator(),
    );

    // Synchronous boot source (#181, ADR-0044): the auth marker decides authed vs.
    // anon at mount, so there is NO async <Suspense> swap on first paint. The
    // anonymous sidebar is the pure `render::render_sidebar` (the SAME code the
    // projector server-renders) injected via `inner_html`, so a seeded first paint
    // and the reactive re-render coincide (flash-free). `display:contents` keeps
    // the host wrapper out of the aside's layout.
    // TRANSITIONAL (#591 Task 1): seed the username out of the richer marker; Task 3
    // replaces this whole boot/reconcile block with the shared session context.
    let owner = RwSignal::new(crate::auth::marker_storage::get().map(|s| s.username));

    // Background reconcile / correctness backstop (D3): confirm the marker against
    // the real session and correct a stale one without gating first paint — a dead
    // session clears the marker (toward anon, the safe direction); a live session
    // with a missing marker sets it. wasm-only: the marker lives in localStorage.
    let reconcile = Resource::new(move || location.pathname.get(), |_| current_user());
    Effect::new(move |_| {
        if let Some(res) = reconcile.get() {
            match res {
                Ok(Some(u)) => {
                    // TRANSITIONAL (#591 Task 1): marker `is_operator` is unread until
                    // the shared session context seeds from it (Task 3, which deletes
                    // this Effect). `false` is harmless here.
                    crate::auth::marker_storage::set(&crate::auth::SessionUser {
                        username: u.clone(),
                        is_operator: false,
                    });
                    if owner.get_untracked().as_ref() != Some(&u) {
                        owner.set(Some(u));
                    }
                }
                Ok(None) => {
                    crate::auth::marker_storage::remove();
                    if owner.get_untracked().is_some() {
                        owner.set(None);
                    }
                }
                Err(_) => {}
            }
        }
    });

    let anon_html = crate::render::render_sidebar(&active_key);
    view! {
        <aside class="j-sidebar">
            {move || match owner.get() {
                None => {
                    view! { <div style="display:contents" inner_html=anon_html.clone()></div> }
                        .into_any()
                }
                Some(username) => {
                    authed_sidebar(&active_key, &username, matches!(operator.get(), Some(Ok(true))))
                        .into_any()
                }
            }}
        </aside>
    }
}

/// The authenticated sidebar chrome (brand, search, nav + operator admin links,
/// sources, footer avatar). Shared by the marker-seeded initial render and the
/// reconciled render (#181) so both are byte-for-byte the same authed markup —
/// only its inputs change from awaited values to these params.
// cov:ignore-start
fn authed_sidebar(active_key: &str, username: &Username, is_operator: bool) -> impl IntoView {
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
                {crate::render::NAV_ITEMS
                    .iter()
                    .filter(|&&(_, _, _, href, _)| href.is_some())
                    .map(|&(key, label, icon_path, href, _)| {
                        let is_active = key == active_key.as_str();
                        view! {
                            <SidebarNavItem
                                label=label
                                icon_path=icon_path
                                active=is_active
                                href=href
                            />
                        }
                    })
                    .collect::<Vec<_>>()}
                {if is_operator {
                    view! {
                        <SidebarNavItem
                            label="Configure Backups"
                            icon_path=Icons::SHIELD
                            active=active_key == "admin-backups"
                            href=Some("/admin/backups")
                        />
                        <SidebarNavItem
                            label="Site Settings"
                            icon_path=Icons::SHIELD
                            active=active_key == "admin-site"
                            href=Some("/admin/site")
                        />
                    }
                        .into_any()
                } else {
                    ().into_any()
                }}
            </nav>
            <div>
                <div class="j-sb-head">
                    <span>"Sources"</span>
                    <span class="j-sb-add">"+"</span>
                </div>
                {crate::render::SIDEBAR_SOURCES
                    .iter()
                    .map(|&(proto, name, sub)| {
                        view! { <SidebarSource proto=proto name=name sub=sub /> }
                    })
                    .collect::<Vec<_>>()}
            </div>
            <div class="j-sb-foot">
                <Avatar name=username.clone() size=28 />
                <div style="font-size:13px;flex:1;min-width:0">
                    <div style="font-weight:500">{username.to_string()}</div>
                </div>
                <a href="/logout" style="font-size:11px;color:var(--muted)">
                    "Sign out"
                </a>
            </div>
        </div>
    }
    // cov:ignore-stop
} // cov:ignore
