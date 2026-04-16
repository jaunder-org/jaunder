pub mod auth;
pub mod email;
pub mod home;
pub mod invites;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod sessions;

use crate::pages::email::{EmailPage, VerifyEmailPage};
use crate::pages::home::HomePage;
use crate::pages::invites::InvitesPage;
use crate::pages::password_reset::{ForgotPasswordPage, ResetPasswordPage};
use crate::pages::posts::{CreatePostPage, DraftPreviewPage, DraftsPage, EditPostPage, PostPage};
use crate::pages::profile::ProfilePage;
use crate::pages::sessions::SessionsPage;
use crate::{
    auth::current_user,
    pages::auth::{LoginPage, LogoutPage, RegisterPage},
};
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    ParamSegment, StaticSegment,
};

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

    let user = Resource::new(|| (), |_| current_user());

    view! {
        <Stylesheet id="leptos" href="/pkg/jaunder.css" />

        // sets the document title
        <Title text="Jaunder" />

        // content for this welcome page
        <Router>
            <header>
                <Suspense fallback=|| {
                    view! {
                        <nav>
                            <a href="/login">"Login"</a>
                            " "
                            <a href="/register">"Register"</a>
                        </nav>
                    }
                }>
                    {move || Suspend::new(async move {
                        let user = user.await;
                        match user {
                            Ok(Some(username)) => {
                                view! {
                                    <nav>
                                        <span>"Logged in as " {username}</span>
                                        " "
                                        <a href="/logout">"Logout"</a>
                                    </nav>
                                }
                                    .into_any()
                            }
                            Ok(None) | Err(_) => {
                                view! {
                                    <nav>
                                        <a href="/login">"Login"</a>
                                        " "
                                        <a href="/register">"Register"</a>
                                    </nav>
                                }
                                    .into_any()
                            }
                        }
                    })}
                </Suspense>
            </header>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
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
                </Routes>
            </main>
        </Router>
    }
}
