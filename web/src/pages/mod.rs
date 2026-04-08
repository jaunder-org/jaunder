pub mod auth;
pub mod email;
pub mod home;
pub mod invites;
pub mod profile;
pub mod sessions;

use crate::pages::auth::{LoginPage, LogoutPage, RegisterPage};
use crate::pages::email::{EmailPage, VerifyEmailPage};
use crate::pages::home::HomePage;
use crate::pages::invites::InvitesPage;
use crate::pages::profile::ProfilePage;
use crate::pages::sessions::SessionsPage;
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/jaunder.css" />

        // sets the document title
        <Title text="Welcome to Leptos" />

        // content for this welcome page
        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("") view=HomePage />
                    <Route path=StaticSegment("register") view=RegisterPage />
                    <Route path=StaticSegment("login") view=LoginPage />
                    <Route path=StaticSegment("logout") view=LogoutPage />
                    <Route path=StaticSegment("profile") view=ProfilePage />
                    <Route path=StaticSegment("sessions") view=SessionsPage />
                    <Route path=StaticSegment("invites") view=InvitesPage />
                    <Route path=(StaticSegment("profile"), StaticSegment("email")) view=EmailPage />
                    <Route path=StaticSegment("verify-email") view=VerifyEmailPage />
                </Routes>
            </main>
        </Router>
    }
}
