pub mod email;
pub mod invites;
pub mod password_reset;
pub mod profile;
pub mod sessions;
pub(crate) mod signal_read;
pub mod site;
pub mod ui;
pub use ui::{Avatar, Icon, Icons, Sidebar, Topbar};

/// Default theme identifier (the CSS variable pack applied via `data-theme` on the
/// root element). Defined in `crate::render` (the shell layer) so the projector's
/// server-painted shell and this reactive `AppShell` share one value.
pub use crate::render::DEFAULT_THEME;

/// The localStorage key holding the persisted theme. Local to `web` — unlike the
/// auth marker's key, it is not shared with the pre-paint script or any other layer.
const THEME_KEY: &str = "jaunder_theme";

use crate::audiences::AudiencesPage;
use crate::auth::{LoginPage, LogoutPage};
use crate::backup::{BackupBanner, BackupSettingsPage};
use crate::cockpit::CockpitPage;
use crate::home::HomePage;
use crate::media::MediaPage;
use crate::pages::email::{EmailPage, VerifyEmailPage};
use crate::pages::invites::InvitesPage;
use crate::pages::password_reset::{ForgotPasswordPage, ResetPasswordPage};
use crate::pages::profile::ProfilePage;
use crate::pages::sessions::SessionsPage;
use crate::pages::site::SiteSettingsPage;
use crate::posts::{
    CreatePostPage, DraftsPage, EditPostPage, PostPage, SiteTagPage, UserTagPage, UserTimelinePage,
};
use crate::registration::RegisterPage;
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Title};
use leptos_router::{
    components::{Outlet, ParentRoute, Route, Router, Routes},
    ParamSegment, StaticSegment,
};

#[component]
fn AppShell() -> impl IntoView {
    let theme = use_context::<RwSignal<String>>()
        .unwrap_or_else(|| RwSignal::new(DEFAULT_THEME.to_string()));
    // `data-theme` must be a plain dynamic attribute, NOT `attr:data-theme`: the
    // Leptos `attr:` directive prefix is only for spreading onto a component; on a
    // plain element it leaks a literal `attr:data-theme` attribute into the hydrated
    // DOM and the `.j-root[data-theme=...]` theme selector stops matching (#22).
    view! {
        <div class="j-root" data-theme=move || theme.get()>
            <div class="j-shell">
                <Sidebar />
                <div class="j-main-region">
                    <BackupBanner />
                    <main class="j-main">
                        <Outlet />
                    </main>
                </div>
            </div>
        </div>
    }
}

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    // Override the router's SPA-navigation redirect hook with a full-page reload.
    // The Router component installs a hook via the same OnceLock (first caller wins),
    // so we must register ours here — before the view! tree renders and instantiates
    // Router.  Using window.location.replace() instead of use_navigate() ensures:
    // - the browser performs a real page load, refreshing all server-rendered state
    //   (including the auth header that reads from the `user` Resource), and
    // - Playwright's waitForURL() reliably detects the navigation in all browsers.
    let _ = leptos::server_fn::redirect::set_redirect_hook(|loc: &str| {
        if let Some(window) = web_sys::window() {
            let _ = window.location().replace(loc);
        }
    });

    let theme = RwSignal::new(DEFAULT_THEME.to_string());

    // On WASM: restore theme from localStorage on startup. A read failure (storage
    // unavailable) falls back to the default theme — cosmetic, nothing to recover.
    if let Ok(Some(val)) = client::storage::get(THEME_KEY) {
        if !val.is_empty() {
            theme.set(val);
        }
    }

    provide_context(theme);

    // On WASM: persist theme to localStorage whenever it changes. Theme persistence
    // is cosmetic, so a write failure (e.g. quota) is deliberately ignored at this
    // caller rather than surfaced — the primitive reports it; we choose not to act.
    Effect::new(move |_| {
        let _ = client::storage::set(THEME_KEY, &theme.get());
    });

    view! {
        // sets the document title
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
                        path=(StaticSegment("posts"), StaticSegment("new"))
                        view=CreatePostPage
                    />
                    <Route path=StaticSegment("drafts") view=DraftsPage />
                    <Route path=StaticSegment("media") view=MediaPage />
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
                            ParamSegment("username"),
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
