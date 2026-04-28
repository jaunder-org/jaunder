pub mod auth;
pub mod email;
pub mod home;
pub mod invites;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod sessions;
pub(crate) mod signal_read;
pub mod ui;
pub use ui::{
    Avatar, Chip, Dot, Icon, Icons, InlineComposer, PostCard, PostDisplay, Sidebar, Topbar,
};

/// Default theme identifier. This selects the CSS variable pack applied via
/// `data-theme` on the root element. "studio" is the pragmatic default chosen
/// during initial UI import; to make this per-server-configurable, replace this
/// constant with a value from server configuration.
pub const DEFAULT_THEME: &str = "studio";

use crate::pages::auth::{LoginPage, LogoutPage, RegisterPage};
use crate::pages::email::{EmailPage, VerifyEmailPage};
use crate::pages::home::HomePage;
use crate::pages::invites::InvitesPage;
use crate::pages::password_reset::{ForgotPasswordPage, ResetPasswordPage};
use crate::pages::posts::{
    CreatePostPage, DraftPreviewPage, DraftsPage, EditPostPage, PostPage, UserTimelinePage,
};
use crate::pages::profile::ProfilePage;
use crate::pages::sessions::SessionsPage;
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Stylesheet, Title};
use leptos_router::{
    components::{Outlet, ParentRoute, Route, Router, Routes},
    ParamSegment, StaticSegment,
};

#[component]
fn AppShell() -> impl IntoView {
    let theme = use_context::<RwSignal<String>>().expect("theme context missing");
    view! {
        <div class="j-root" attr:data-theme=move || theme.get()>
            <div class="j-shell">
                <Sidebar />
                <main class="j-main">
                    <Outlet />
                </main>
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
    #[cfg(target_arch = "wasm32")]
    {
        let _ = leptos::server_fn::redirect::set_redirect_hook(|loc: &str| {
            if let Some(window) = web_sys::window() {
                let _ = window.location().replace(loc);
            }
        });
    }

    let theme = RwSignal::new(DEFAULT_THEME.to_string());

    // On WASM: restore theme from localStorage on startup.
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            if let Ok(Some(val)) = storage.get_item("jaunder_theme") {
                if !val.is_empty() {
                    theme.set(val);
                }
            }
        }
    }

    provide_context(theme);

    // On WASM: persist theme to localStorage whenever it changes.
    #[cfg(target_arch = "wasm32")]
    {
        Effect::new(move |_| {
            let val = theme.get();
            if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            {
                let _ = storage.set_item("jaunder_theme", &val);
            }
        });
    }

    view! {
        <Stylesheet id="leptos" href="/pkg/jaunder.css" />

        // sets the document title
        <Title text="Jaunder" />

        <Router>
            <Routes fallback=|| "Page not found.".into_view()>
                <ParentRoute path=StaticSegment("") view=AppShell>
                    <Route path=StaticSegment("") view=HomePage />
                    <Route path=StaticSegment("register") view=RegisterPage />
                    <Route path=StaticSegment("login") view=LoginPage />
                    <Route path=StaticSegment("logout") view=LogoutPage />
                    <Route path=(StaticSegment("profile"), StaticSegment("email")) view=EmailPage />
                    <Route path=StaticSegment("profile") view=ProfilePage />
                    <Route path=StaticSegment("sessions") view=SessionsPage />
                    <Route path=StaticSegment("invites") view=InvitesPage />
                    <Route
                        path=(StaticSegment("posts"), StaticSegment("new"))
                        view=CreatePostPage
                    />
                    <Route path=StaticSegment("drafts") view=DraftsPage />
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
                    <Route
                        path=(
                            StaticSegment("draft"),
                            ParamSegment("post_id"),
                            StaticSegment("preview"),
                        )
                        view=DraftPreviewPage
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_is_nonempty() {
        assert!(!DEFAULT_THEME.is_empty());
    }
}
