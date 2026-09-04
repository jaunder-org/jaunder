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
use crate::websub::WebsubPage;
use common::theme::Theme;
use leptos::prelude::*;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::{
    ParamSegment, StaticSegment,
    components::{Outlet, ParentRoute, Route, Router, Routes},
    hooks::use_location,
};
#[must_use]
pub fn public_theme() -> RwSignal<Theme> {
    use_context::<RwSignal<Theme>>().unwrap_or_else(|| RwSignal::new(Theme::Studio))
}

#[component]
fn AppShell() -> impl IntoView {
    // The shared session context lives here, not in `App`: it reads `use_location`
    // (per-navigation reconcile), which requires the `<Router>` context, and every
    // consumer renders under this shell (#591).
    crate::auth::provide_session_context();

    let theme = public_theme();
    let location = use_location();
    // `data-theme` must be a plain dynamic attribute, NOT `attr:data-theme`: the
    // Leptos `attr:` directive prefix is only for spreading onto a component; on a
    // plain element it leaks a literal `attr:data-theme` attribute into the mounted
    // DOM and the `.j-root[data-theme=...]` theme selector stops matching (#22).
    view! {
        <div
            class="j-root"
            data-theme=move || {
                if common::theme::is_public_presentation_path(&location.pathname.get()) {
                    theme.get()
                } else {
                    Theme::Studio
                }
                    .token()
            }
        >
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
