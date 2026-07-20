// `current_user` is the sidebar's background reconcile (#181), used only in the
// wasm-only correction Effect.
use crate::auth::current_user;
use crate::backup::current_user_is_operator;
use common::time::UtcInstant;
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

/// Converts a `datetime-local` input value — a naive local wall-clock such as
/// "2026-07-01T13:30" — into a UTC RFC3339 instant string for the server.
/// Returns `None` for an empty/whitespace input (i.e. publish-now).
///
/// The browser's `Date` does the local→UTC conversion so it honors the
/// author's timezone and DST. Form dispatch is client-only, so the non-wasm
/// build only needs this to compile (the stub is never executed there).
// Deliberate manual keep: this genuine helper (not a Leptos view) benefits from
// `#[must_use]`; the crate-wide `must_use_candidate = "allow"` (Cargo.toml, #94)
// means clippy no longer flags it, so we assert it by hand.
#[must_use]
fn local_datetime_to_utc_rfc3339(local: &str) -> Option<String> {
    let trimmed = local.trim();
    if trimmed.is_empty() {
        return None;
    }
    // `new Date("YYYY-MM-DDTHH:MM")` (time present, no offset) is parsed as
    // local time per ECMAScript; `toISOString()` re-renders it in UTC.
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(trimmed));
    if date.get_time().is_nan() {
        return None;
    }
    date.to_iso_string().as_string()
}

/// Parses a `datetime-local` control's raw value into the UTC [`UtcInstant`] used for the
/// `publish_at` wire arg — the browser local→UTC conversion
/// ([`local_datetime_to_utc_rfc3339`]) followed by the domain parse, in one place. `None`
/// for an empty/unparseable field (i.e. publish now).
#[must_use]
pub(crate) fn publish_at_from_local(local: &str) -> Option<UtcInstant> {
    local_datetime_to_utc_rfc3339(local).and_then(|s| s.parse::<UtcInstant>().ok())
}

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
    let operator = crate::server_resource(
        move || location.pathname.get(),
        |_| current_user_is_operator(),
    );

    // Synchronous boot source (#181, ADR-0044): the auth marker decides authed vs.
    // anon at mount, so there is NO async <Suspense> swap on first paint. The
    // anonymous sidebar is the pure `render::render_sidebar` (the SAME code the
    // projector server-renders) injected via `inner_html`, so a seeded first paint
    // and the reactive re-render coincide (flash-free). `display:contents` keeps
    // the host wrapper out of the aside's layout.
    let owner = RwSignal::new(marker_username_on_boot());

    // Background reconcile / correctness backstop (D3): confirm the marker against
    // the real session and correct a stale one without gating first paint — a dead
    // session clears the marker (toward anon, the safe direction); a live session
    // with a missing marker sets it. wasm-only: the marker lives in localStorage.
    let reconcile = crate::server_resource(move || location.pathname.get(), |_| current_user());
    Effect::new(move |_| {
        if let Some(res) = reconcile.get() {
            match res {
                Ok(Some(u)) => {
                    crate::auth::marker_storage::set(&u);
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

/// Boot-time marker read: `Some(username)` in the browser when the auth marker is
/// set, `None` on the host build (the sidebar only ever renders in wasm). Lets the
/// sidebar pick authed vs. anon synchronously at mount (#181), no async gate.
fn marker_username_on_boot() -> Option<Username> {
    crate::auth::marker_storage::get()
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
