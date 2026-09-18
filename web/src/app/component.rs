//! The app vertical's wasm-only reactive shell (ADR-0070): `App` (Router + route
//! table) and its private `AppShell` (the `j-root` frame). The pure projector twin
//! that must coincide byte-for-byte lives in the sibling `super::render` leaf. No
//! `#[cfg]` of its own — wasm-only via its `mod` line in `mod.rs`.

use super::PrivateDestination;
use crate::backup::BackupBanner;
use crate::sidebar::Sidebar;
use crate::site::SiteBaseUrlBanner;
use std::{cell::RefCell, rc::Rc};

use crate::error::{WebError, WebResult};
use common::theme::{PublishedThemePresentation, Theme};
use leptos::prelude::*;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::{
    NavigateOptions, StaticSegment,
    components::{Outlet, ParentRoute, Route, Router, Routes},
    hooks::{use_location, use_navigate},
};
use wasm_bindgen::JsCast;
#[must_use]
pub fn public_theme() -> RwSignal<PublishedThemePresentation> {
    use_context::<RwSignal<PublishedThemePresentation>>()
        .unwrap_or_else(|| RwSignal::new(PublishedThemePresentation::built_in(Theme::Studio)))
}

/// The reactive twin of [`super::render::render_theme_hero`].
///
/// Keeping the wrapper, header ordering, and semantic hook here makes every CSR
/// route share one hero seam; route components supply only their masthead.
#[component]
pub fn ThemeHero(theme: RwSignal<PublishedThemePresentation>, children: Children) -> impl IntoView {
    view! {
        <section data-jaunder-part=super::render::HERO_PART>
            {move || {
                super::render::render_theme_header(&theme.get())
                    .inject_into(leptos::html::div().class("j-contents"))
            }} {children()}
        </section>
    }
}

/// The outcome of waiting for a navigation presentation to settle.
///
/// A superseded navigation deliberately does not alter either the old or the
/// newer destination; its caller must likewise leave its page state alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeAdoption {
    Applied,
    Superseded,
}

type StylesheetCallback = wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>;
type StylesheetCallbacks = Rc<RefCell<(Option<StylesheetCallback>, Option<StylesheetCallback>)>>;

struct StagedThemeStylesheet {
    generation: u64,
    link: web_sys::HtmlLinkElement,
    sender: Rc<RefCell<Option<futures_channel::oneshot::Sender<bool>>>>,
    callbacks: StylesheetCallbacks,
}

thread_local! {
    static STAGED_THEME_STYLESHEET: RefCell<Option<StagedThemeStylesheet>> =
        const { RefCell::new(None) };
}

/// The one authority for committing a resolved public presentation in CSR.
///
/// A custom theme is staged under a non-matching media query. The coordinator
/// publishes it, its role images, and the route's page only after the browser
/// has loaded that stylesheet.
#[derive(Clone, Copy)]
pub struct ThemePresentationCoordinator {
    theme: RwSignal<PublishedThemePresentation>,
    generation: RwSignal<u64>,
}

impl ThemePresentationCoordinator {
    #[must_use]
    fn new(theme: RwSignal<PublishedThemePresentation>) -> Self {
        Self {
            theme,
            generation: RwSignal::new(0),
        }
    }

    /// Starts a route transition without changing the currently painted
    /// presentation. Any staged stylesheet is settled and removed immediately.
    pub fn begin_navigation(self) {
        self.generation.update(|generation| *generation += 1);
        Self::cancel_staged();
        remove_staged_theme_stylesheets();
    }

    /// Waits for the current navigation's presentation to become safe to paint.
    ///
    /// Built-in themes and private navigation settle synchronously. A newer
    /// navigation settles an older custom load as [`ThemeAdoption::Superseded`].
    ///
    /// # Errors
    ///
    /// Returns an error when the browser cannot create or load the custom stylesheet.
    pub async fn adopt(self, presentation: PublishedThemePresentation) -> WebResult<ThemeAdoption> {
        self.begin_navigation();
        let generation = self.generation.get_untracked();

        if !matches!(
            presentation.identity,
            common::theme::PublishedThemeIdentity::Custom(_)
        ) {
            remove_theme_stylesheet();
            self.theme.set(presentation);
            return Ok(ThemeAdoption::Applied);
        }

        let (link, receiver) = Self::stage_custom(generation, &presentation)?;

        let loaded = receiver.await.unwrap_or(false);
        if self.generation.get_untracked() != generation {
            return Ok(ThemeAdoption::Superseded);
        }
        if !loaded {
            return Err(WebError::server_message(
                "Custom theme stylesheet failed to load",
            ));
        }

        remove_theme_stylesheet();
        let _ = link.remove_attribute("data-jaunder-theme-staged");
        let _ = link.set_attribute(super::THEME_STYLESHEET_MARKER_ATTR, "");
        link.set_media("all");
        self.theme.set(presentation);
        Ok(ThemeAdoption::Applied)
    }

    fn stage_custom(
        generation: u64,
        presentation: &PublishedThemePresentation,
    ) -> WebResult<(
        web_sys::HtmlLinkElement,
        futures_channel::oneshot::Receiver<bool>,
    )> {
        let Some(document) = leptos::web_sys::window().and_then(|window| window.document()) else {
            return Err(WebError::server_message(
                "Unable to stage custom theme stylesheet",
            ));
        };
        let Ok(element) = document.create_element("link") else {
            return Err(WebError::server_message(
                "Unable to create custom theme stylesheet",
            ));
        };
        let Ok(link) = element.dyn_into::<web_sys::HtmlLinkElement>() else {
            return Err(WebError::server_message(
                "Unable to stage custom theme stylesheet",
            ));
        };
        let _ = link.set_attribute("rel", "stylesheet");
        let _ = link.set_attribute("data-jaunder-theme-staged", "");
        // Download without making the destination cascade part of the current
        // page. `all` is restored in the single settlement below.
        link.set_media("not all");
        link.set_href(presentation.stylesheet_url.as_ref());

        let (sender, receiver) = futures_channel::oneshot::channel();
        let sender = Rc::new(RefCell::new(Some(sender)));
        let callbacks = Rc::new(RefCell::new((None, None)));
        let onload = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            Self::settle_staged(generation, true);
        });
        let onerror = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            Self::settle_staged(generation, false);
        });
        link.set_onload(Some(onload.as_ref().unchecked_ref()));
        link.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        callbacks.borrow_mut().0 = Some(onload);
        callbacks.borrow_mut().1 = Some(onerror);
        STAGED_THEME_STYLESHEET.with(|staged| {
            *staged.borrow_mut() = Some(StagedThemeStylesheet {
                generation,
                link: link.clone(),
                sender,
                callbacks,
            });
        });
        if document
            .head()
            .and_then(|head| head.append_child(&link).ok())
            .is_none()
        {
            Self::settle_staged(generation, false);
        }
        Ok((link, receiver))
    }

    fn settle_staged(generation: u64, loaded: bool) {
        let staged = STAGED_THEME_STYLESHEET.with(|current| {
            let mut current = current.borrow_mut();
            if current
                .as_ref()
                .is_some_and(|current| current.generation == generation)
            {
                current.take()
            } else {
                None
            }
        });
        let Some(staged) = staged else {
            return;
        };
        staged.link.set_onload(None);
        staged.link.set_onerror(None);
        *staged.callbacks.borrow_mut() = (None, None);
        if !loaded {
            staged.link.remove();
        }
        if let Some(sender) = staged.sender.borrow_mut().take() {
            let _ = sender.send(loaded);
        }
    }

    fn cancel_staged() {
        let staged = STAGED_THEME_STYLESHEET.with(|current| current.borrow_mut().take());
        let Some(staged) = staged else {
            return;
        };
        staged.link.set_onload(None);
        staged.link.set_onerror(None);
        *staged.callbacks.borrow_mut() = (None, None);
        staged.link.remove();
        if let Some(sender) = staged.sender.borrow_mut().take() {
            let _ = sender.send(false);
        }
    }

    fn clear_for_private_route(self) {
        self.generation.update(|generation| *generation += 1);
        Self::cancel_staged();
        remove_theme_stylesheet();
        remove_staged_theme_stylesheets();
        self.theme
            .set(PublishedThemePresentation::built_in(Theme::Studio));
    }

    fn adopt_initial(self) {
        reconcile_theme_stylesheet(&self.theme.get_untracked());
    }
}

/// Obtain the public-navigation presentation authority.
#[must_use]
pub fn theme_presentation() -> ThemePresentationCoordinator {
    use_context::<ThemePresentationCoordinator>()
        .unwrap_or_else(|| ThemePresentationCoordinator::new(public_theme()))
}

fn remove_theme_stylesheet() {
    let Some(document) = leptos::web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let selector = format!("link[{}]", super::THEME_STYLESHEET_MARKER_ATTR);
    let Ok(nodes) = document.query_selector_all(&selector) else {
        return;
    };
    for index in 0..nodes.length() {
        if let Some(node) = nodes.item(index)
            && let Some(parent) = node.parent_node()
        {
            let _ = parent.remove_child(&node);
        }
    }
}

fn remove_staged_theme_stylesheets() {
    let Some(document) = leptos::web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(nodes) = document.query_selector_all("link[data-jaunder-theme-staged]") else {
        return;
    };
    for index in 0..nodes.length() {
        if let Some(node) = nodes.item(index)
            && let Some(parent) = node.parent_node()
        {
            let _ = parent.remove_child(&node);
        }
    }
}

fn reconcile_theme_stylesheet(presentation: &PublishedThemePresentation) {
    if !matches!(
        presentation.identity,
        common::theme::PublishedThemeIdentity::Custom(_)
    ) {
        remove_theme_stylesheet();
        return;
    }
    let Some(document) = leptos::web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let selector = format!("link[{}]", super::THEME_STYLESHEET_MARKER_ATTR);
    let existing = document
        .query_selector(&selector)
        .ok()
        .flatten()
        .or_else(|| {
            let element = document.create_element("link").ok()?;
            element
                .set_attribute(super::THEME_STYLESHEET_MARKER_ATTR, "")
                .ok()?;
            element.set_attribute("rel", "stylesheet").ok()?;
            document.head()?.append_child(&element).ok()?;
            Some(element)
        });
    if let Some(link) = existing {
        let _ = link.set_attribute("href", presentation.stylesheet_url.as_ref());
    }
}

#[component]
fn AppShell() -> impl IntoView {
    let site_warning =
        crate::warning_revalidation::SiteBaseUrlWarning(crate::reactive::Invalidator::new());
    let backup_warning =
        crate::warning_revalidation::BackupWarning(crate::reactive::Invalidator::new());
    provide_context(site_warning);
    provide_context(backup_warning);

    // The shared session context lives here, not in `App`: it reads `use_location`
    // (per-navigation reconcile), which requires the `<Router>` context, and every
    // consumer renders under this shell (#591).
    crate::auth::provide_session_context();

    let theme = public_theme();
    let location = use_location();
    let presentation = ThemePresentationCoordinator::new(theme);
    presentation.adopt_initial();
    provide_context(presentation);
    Effect::new(move |_| {
        if !common::theme::is_public_presentation_path(&location.pathname.get()) {
            presentation.clear_for_private_route();
        }
    });
    // `data-theme` must be a plain dynamic attribute, NOT `attr:data-theme`: the
    // Leptos `attr:` directive prefix is only for spreading onto a component; on a
    // plain element it leaks a literal `attr:data-theme` attribute into the mounted
    // DOM and the `.j-root[data-theme=...]` theme selector stops matching (#22).
    view! {
        <div
            class="j-root"
            data-theme=move || {
                if common::theme::is_public_presentation_path(&location.pathname.get()) {
                    theme.get().data_theme()
                } else {
                    Theme::Studio.token().to_owned()
                }
            }
        >
            {move || {
                if common::theme::is_public_presentation_path(&location.pathname.get()) {
                    view! {
                        <div class="j-theme-clip" data-jaunder-theme-clip>
                            <div
                                class="j-shell"
                                data-jaunder-theme-surface
                                data-jaunder-style-contract=common::theme::STYLE_CONTRACT_VERSION
                            >
                                <Sidebar />
                                <div class="j-main-region">
                                    <main class="j-main" data-jaunder-part="main">
                                        <Outlet />
                                    </main>
                                </div>
                            </div>
                        </div>
                        <div id="j-trusted-post-actions" class="j-trusted-post-actions"></div>
                        <div id="j-trusted-chrome" class="j-trusted-chrome">
                            <BackupBanner />
                            <SiteBaseUrlBanner />
                        </div>
                    }
                        .into_any()
                } else {
                    view! {
                        <div class="j-shell">
                            <Sidebar />
                            <div class="j-main-region">
                                <BackupBanner />
                                <SiteBaseUrlBanner />
                                <main class="j-main">
                                    <Outlet />
                                </main>
                            </div>
                        </div>
                        <div id="j-trusted-post-actions" class="j-trusted-post-actions"></div>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// Withholds a private route's view until the shared session reconcile confirms it.
///
/// The marker-backed `current` session remains appropriate for chrome, but it is
/// advisory; only this Resource's cookie-checked value can admit a private page.
#[component]
fn PrivateRoute(private: bool, children: ChildrenFn) -> impl IntoView {
    let session = crate::auth::use_session();
    let location = use_location();
    let navigate = use_navigate();
    let retry = RwSignal::new(0_u64);
    let confirmation = Resource::new(
        move || (location.pathname.get(), retry.get()),
        move |_| async move { session.reconcile.await },
    );
    let destination = move || {
        PrivateDestination::from_location(
            &location.pathname.get(),
            &location.search.get(),
            &location.hash.get(),
        )
    };

    Effect::new(move |_| {
        if private
            && matches!(confirmation.get(), Some(Ok(None)))
            && let Some(destination) = destination()
        {
            navigate(
                &destination.login_path(),
                NavigateOptions {
                    replace: true,
                    ..NavigateOptions::default()
                },
            );
        }
    });

    view! {
        {move || {
            if !private {
                return children().into_any();
            }
            match confirmation.get() {
                None | Some(Ok(None)) => {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any()
                }
                Some(Err(error)) => {
                    view! {
                        <p class="error">{error.to_string()}</p>
                        <button
                            type="button"
                            class="j-btn"
                            on:click=move |_| {
                                session.reconcile.refetch();
                                retry.update(|value| *value += 1);
                            }
                        >
                            "Retry"
                        </button>
                    }
                        .into_any()
                }
                Some(Ok(Some(_))) => children().into_any(),
            }
        }}
    }
}

/// Supplies the historic application fallback title everywhere except Local.
///
/// This subscriber mounts after the route tree so nested route parameters settle
/// before fallback metadata reacts to the same navigation. Local owns its title only
/// after its coherent destination identity resolves.
#[component]
fn AppDefaultTitle() -> impl IntoView {
    let location = use_location();
    move || (location.pathname.get() != "/").then(|| view! { <Title text="Jaunder" /> })
}

macro_rules! app_router {
    ($(($name:ident, $access:ident, $pattern:literal, $path:expr, $view:path))*) => {
        view! {
            <Router>
                <Routes fallback=|| "Page not found.".into_view()>
                    <ParentRoute path=StaticSegment("") view=AppShell>
                        $(<Route
                            path=$path
                            view=move || view! {
                                <PrivateRoute private=matches!(super::route_policy::Access::$access, super::route_policy::Access::Private)>
                                    { $view() }
                                </PrivateRoute>
                            }
                        />)*
                    </ParentRoute>
                </Routes>
                <AppDefaultTitle />
            </Router>
        }
    };
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // No server-fn redirect-hook override (#591): `<Router>` installs a same-origin
    // `use_navigate` hook into the first-caller-wins `OnceLock` before any auth action
    // can redirect, so login/logout/register use client-side pushState with no full
    // document reload. Chrome updates reactively via the shared session context, which
    // those components set/clear on success.
    crate::app_routes!(app_router)
}
