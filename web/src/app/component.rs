//! The app vertical's wasm-only reactive shell (ADR-0070): `App` (Router + route
//! table) and its private `AppShell` (the `j-root` frame). The pure projector twin
//! that must coincide byte-for-byte lives in the sibling `super::render` leaf. No
//! `#[cfg]` of its own — wasm-only via its `mod` line in `mod.rs`.

use crate::audiences::AudiencesPage;
use crate::auth::{LoginPage, LogoutPage};
use crate::backup::{BackupBanner, BackupSettingsPage};
use crate::cockpit::CockpitPage;
use crate::email::{EmailPage, VerifyEmailPage};
use crate::home::HomePage;
use crate::invites::InvitesPage;
use crate::media::MediaPage;
use crate::password_reset::{ForgotPasswordPage, ResetPasswordPage};
use crate::posts::{
    CreatePostPage, DraftsPage, EditPostPage, HistoryPage, PostHistoryPage, PostPage,
    RevisionHistoryDetailPage, ScheduledPage, SiteTagPage, UserTagPage, UserTimelinePage,
};
use crate::profile::ProfilePage;
use crate::registration::RegisterPage;
use crate::route_segments::TildeUsername;
use crate::sessions::SessionsPage;
use crate::sidebar::Sidebar;
use crate::site::{SiteBaseUrlBanner, SiteSettingsPage};
use crate::smtp::SmtpSettingsPage;
use crate::websub::WebsubPage;
use std::{cell::RefCell, rc::Rc};

use crate::error::{WebError, WebResult};
use common::theme::{PublishedThemePresentation, Theme};
use leptos::prelude::*;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::{
    ParamSegment, StaticSegment,
    components::{Outlet, ParentRoute, Route, Router, Routes},
    hooks::use_location,
};
use wasm_bindgen::JsCast;
#[must_use]
pub fn public_theme() -> RwSignal<PublishedThemePresentation> {
    use_context::<RwSignal<PublishedThemePresentation>>()
        .unwrap_or_else(|| RwSignal::new(PublishedThemePresentation::built_in(Theme::Studio)))
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
                        <div id="j-trusted-chrome" class="j-trusted-chrome">
                            <BackupBanner />
                            <SiteBaseUrlBanner />
                        </div>
                        <div class="j-theme-clip" data-jaunder-theme-clip>
                            <div
                                class="j-shell"
                                data-jaunder-theme-surface
                                data-jaunder-style-contract="1"
                            >
                                <Sidebar />
                                <div class="j-main-region">
                                    <main class="j-main" data-jaunder-part="main">
                                        <Outlet />
                                    </main>
                                </div>
                            </div>
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
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // No server-fn redirect-hook override (#591): `<Router>` installs a same-origin
    // `use_navigate` hook into the first-caller-wins `OnceLock` before any auth action
    // can redirect, so login/logout/register use client-side pushState with no full
    // document reload. Chrome updates reactively via the shared session context, which
    // those components set/clear on success.

    view! {
        <Title text="Jaunder" />

        <Router>
            <Routes fallback=|| "Page not found.".into_view()>
                <ParentRoute path=StaticSegment("") view=AppShell>
                    <Route path=StaticSegment("") view=HomePage />
                    // The authed-only cockpit (#181, ADR-0044 D6): the relocated
                    // home Feed. Static "app" wins over the ParamSegment username route.
                    <Route path=StaticSegment("app") view=CockpitPage />
                    <Route path=StaticSegment("register") view=RegisterPage />
                    <Route path=StaticSegment("login") view=LoginPage />
                    <Route path=StaticSegment("logout") view=LogoutPage />
                    <Route path=(StaticSegment("profile"), StaticSegment("email")) view=EmailPage />
                    <Route path=StaticSegment("profile") view=ProfilePage />
                    <Route path=StaticSegment("sessions") view=SessionsPage />
                    <Route path=StaticSegment("audiences") view=AudiencesPage />
                    <Route path=StaticSegment("invites") view=InvitesPage />
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("backups"))
                        view=BackupSettingsPage
                    />
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("site"))
                        view=SiteSettingsPage
                    />
                    <Route
                        path=(StaticSegment("admin"), StaticSegment("smtp"))
                        view=SmtpSettingsPage
                    />
                    <Route path=(StaticSegment("admin"), StaticSegment("websub")) view=WebsubPage />
                    <Route
                        path=(StaticSegment("posts"), StaticSegment("new"))
                        view=CreatePostPage
                    />
                    <Route path=StaticSegment("drafts") view=DraftsPage />
                    <Route path=StaticSegment("scheduled") view=ScheduledPage />
                    <Route path=StaticSegment("media") view=MediaPage />
                    <Route path=StaticSegment("history") view=HistoryPage />
                    <Route
                        path=(
                            StaticSegment("posts"),
                            ParamSegment("post_id"),
                            StaticSegment("history"),
                        )
                        view=PostHistoryPage
                    />
                    <Route
                        path=(
                            StaticSegment("posts"),
                            ParamSegment("post_id"),
                            StaticSegment("history"),
                            ParamSegment("revision_id"),
                        )
                        view=RevisionHistoryDetailPage
                    />
                    <Route
                        path=(
                            StaticSegment("posts"),
                            ParamSegment("post_id"),
                            StaticSegment("edit"),
                        )
                        view=EditPostPage
                    />
                    <Route path=StaticSegment("verify-email") view=VerifyEmailPage />
                    <Route path=StaticSegment("forgot-password") view=ForgotPasswordPage />
                    <Route path=StaticSegment("reset-password") view=ResetPasswordPage />
                    <Route path=(StaticSegment("tags"), ParamSegment("tag")) view=SiteTagPage />
                    <Route
                        path=(ParamSegment("username"), StaticSegment("tags"), ParamSegment("tag"))
                        view=UserTagPage
                    />
                    <Route path=ParamSegment("username") view=UserTimelinePage />
                    <Route
                        path=(
                            TildeUsername("username"),
                            ParamSegment("year"),
                            ParamSegment("month"),
                            ParamSegment("day"),
                            ParamSegment("slug"),
                        )
                        view=PostPage
                    />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
