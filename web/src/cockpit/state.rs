//! Host-compiled, host-tested decision logic for the cockpit page (#306, ADR-0083).
//!
//! The cockpit's wasm-only component body folded several already-resolved values into
//! what it should paint. That fold is not browser wiring, so it lives here where it is
//! coverage-measured and assertable, and the component keeps only the wiring an
//! `Effect` genuinely requires.

use std::future::Future;

use leptos::prelude::*;

use common::seed::TimelinePage;
use common::username::Username;

use crate::auth::SessionUser;
use crate::error::WebResult;
use crate::timeline::TimelineState;

/// One resolved cockpit load: the session-confirmed viewer paired with the feed page
/// fetched for them, or `None` when the session resolved to nobody — anonymous or
/// expired (ADR-0044 D6), which the page turns into the `/login` bounce.
pub type CockpitLoad = Option<(Username, TimelinePage)>;

/// Resolve the cockpit's initial payload: gate the feed fetch on the session's
/// server-confirmed reconcile, and pair the page with the identity that reconcile
/// carries.
///
/// `fetch_feed` is a parameter rather than a direct `list_home_feed` call so this
/// fold is host-testable without a server: the wasm caller passes the real server fn,
/// a test passes a stub — and the stub is what proves an anonymous or failed
/// reconcile never issues the fetch at all.
///
/// # Errors
///
/// Propagates the reconcile's error unchanged, or the feed fetch's when the viewer
/// was confirmed but their feed could not be read.
pub async fn resolve_initial_page<F, Fut>(
    reconcile: WebResult<Option<SessionUser>>,
    fetch_feed: F,
) -> WebResult<CockpitLoad>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = WebResult<TimelinePage>>,
{
    match reconcile {
        Ok(Some(user)) => fetch_feed().await.map(|page| Some((user.username, page))),
        Ok(None) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Whether publishing `next` is a real identity change.
///
/// The guard exists to protect the chrome: a spurious `set` re-runs the chrome
/// closure and REMOUNTS `InlineComposer`, wiping its publish/draft flash — and a
/// refetch fires on every publish, so the same username arrives repeatedly.
fn is_new_identity(current: Option<&Username>, next: &Username) -> bool {
    current != Some(next)
}

/// The cockpit's reactive state: the timeline it paints plus the session-confirmed
/// viewer its chrome reads.
///
/// Both fields are `Copy` handles into the reactive runtime, so the bundle is `Copy`
/// and moves into the page's `Effect` and callbacks whole.
#[derive(Clone, Copy, Default)]
pub struct CockpitState {
    pub timeline: TimelineState,
    /// The viewer confirmed by the reconcile — `None` until the first load resolves,
    /// which is the state the topbar-only chrome renders.
    pub username: RwSignal<Option<Username>>,
}

impl CockpitState {
    /// Publish the session-confirmed identity, skipping a write that would not change
    /// it (see [`is_new_identity`]).
    pub fn adopt_username(&self, user: Username) {
        if is_new_identity(self.username.get_untracked().as_ref(), &user) {
            self.username.set(Some(user));
        }
    }

    /// Fold one resolved load into the page's reactive state: adopt the identity and
    /// the page, mark the timeline unidentified so the gate bounces to `/login`, or
    /// record the failure so the gate paints its banner.
    pub fn apply(&self, result: WebResult<CockpitLoad>) {
        match result {
            Ok(Some((user, page))) => {
                self.adopt_username(user);
                self.timeline.adopt(page);
            }
            Ok(None) => self.timeline.unidentified(),
            Err(error) => self.timeline.fail(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::{ready, Ready};

    use super::*;
    use crate::error::WebError;
    use crate::posts::render::test_fixtures::sample_summary;
    use crate::timeline::LoadStatus;
    use common::test_support::parse_username;

    /// Run `body` under a fresh reactive `Owner` (the `timeline::state` convention),
    /// so `RwSignal`s work host-side without a browser.
    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    fn viewer(name: &str) -> SessionUser {
        SessionUser {
            username: parse_username(name),
            is_operator: false,
        }
    }

    fn page() -> TimelinePage {
        TimelinePage {
            posts: vec![sample_summary()],
            next_cursor: None,
            has_more: false,
        }
    }

    /// A feed fetcher that records whether it ran. **One** helper shared by the
    /// positive and the negative cases rather than a stub inline per test: an inline
    /// stub in a "must not fetch" test is by construction never executed, so its own
    /// body would sit uncovered — which the coverage gate correctly reports and which
    /// no marker should paper over. Sharing it also makes the negative assertions
    /// mean something, since the same instrumented fetcher is demonstrably capable of
    /// running.
    fn recording_fetch(
        fetched: &Cell<bool>,
    ) -> impl FnOnce() -> Ready<WebResult<TimelinePage>> + '_ {
        move || {
            fetched.set(true);
            ready(Ok(page()))
        }
    }

    #[tokio::test]
    async fn a_confirmed_viewer_gets_their_feed_paired_with_their_name() {
        let fetched = Cell::new(false);
        let resolved =
            resolve_initial_page(Ok(Some(viewer("bob"))), recording_fetch(&fetched)).await;
        let (name, feed) = resolved.unwrap().expect("a confirmed viewer yields a page");
        assert_eq!(name, parse_username("bob"));
        assert_eq!(feed.posts.len(), 1);
        assert!(
            fetched.get(),
            "a confirmed viewer DOES fetch — which is what makes the \
             `!fetched.get()` assertions below meaningful"
        );
    }

    #[tokio::test]
    async fn an_anonymous_reconcile_resolves_to_none_without_fetching() {
        let fetched = Cell::new(false);
        let resolved = resolve_initial_page(Ok(None), recording_fetch(&fetched)).await;
        assert_eq!(resolved, Ok(None));
        assert!(!fetched.get(), "an anonymous session must not fetch a feed");
    }

    #[tokio::test]
    async fn a_failed_reconcile_propagates_without_fetching() {
        let fetched = Cell::new(false);
        let resolved = resolve_initial_page(
            Err(WebError::validation("no session")),
            recording_fetch(&fetched),
        )
        .await;
        assert_eq!(resolved, Err(WebError::validation("no session")));
        assert!(!fetched.get(), "a failed reconcile must not fetch a feed");
    }

    #[tokio::test]
    async fn a_failed_feed_fetch_propagates_its_own_error() {
        let resolved = resolve_initial_page(Ok(Some(viewer("bob"))), || async {
            Err(WebError::server_message("feed down"))
        })
        .await;
        assert_eq!(resolved, Err(WebError::server_message("feed down")));
    }

    #[test]
    fn is_new_identity_only_for_a_different_or_absent_name() {
        let bob = parse_username("bob");
        let ada = parse_username("ada");
        assert!(is_new_identity(None, &bob), "first identity is new");
        assert!(is_new_identity(Some(&ada), &bob), "a different name is new");
        assert!(
            !is_new_identity(Some(&bob), &bob),
            "the same name must not be republished"
        );
    }

    #[test]
    fn adopt_username_publishes_and_then_holds_steady() {
        with_owner(|| {
            let state = CockpitState::default();
            assert_eq!(state.username.get(), None);

            state.adopt_username(parse_username("bob"));
            assert_eq!(state.username.get(), Some(parse_username("bob")));

            state.adopt_username(parse_username("bob"));
            assert_eq!(state.username.get(), Some(parse_username("bob")));

            state.adopt_username(parse_username("ada"));
            assert_eq!(state.username.get(), Some(parse_username("ada")));
        });
    }

    #[test]
    fn apply_ok_some_adopts_both_the_identity_and_the_page() {
        with_owner(|| {
            let state = CockpitState::default();
            state.apply(Ok(Some((parse_username("bob"), page()))));
            assert_eq!(state.username.get(), Some(parse_username("bob")));
            assert_eq!(state.timeline.rows.get().len(), 1);
            assert_eq!(state.timeline.status.get(), LoadStatus::Idle);
        });
    }

    #[test]
    fn apply_ok_none_marks_the_timeline_unidentified_for_the_login_bounce() {
        with_owner(|| {
            let state = CockpitState::default();
            state.apply(Ok(Some((parse_username("bob"), page()))));
            state.apply(Ok(None));
            assert_eq!(state.timeline.status.get(), LoadStatus::Unidentified);
            assert!(state.timeline.rows.get().is_empty(), "no stale feed");
            assert_eq!(
                state.username.get(),
                Some(parse_username("bob")),
                "the bounce travels on the timeline status, not by clearing the name"
            );
        });
    }

    #[test]
    fn apply_err_records_the_failure_on_the_timeline() {
        with_owner(|| {
            let state = CockpitState::default();
            state.apply(Err(WebError::validation("boom")));
            assert_eq!(
                state.timeline.status.get(),
                LoadStatus::Failed(WebError::validation("boom"))
            );
            assert!(state.timeline.rows.get().is_empty());
        });
    }

    // The bundle is handed to an `Effect` and to callbacks by value; `Copy` is what
    // makes that work without per-signal capture, and a `Clone` that stopped being a
    // bitwise copy would silently split the two views of the same signals.
    #[test]
    fn the_bundle_is_copy_and_both_copies_share_one_signal() {
        with_owner(|| {
            let state = CockpitState::default();
            let alias = state;
            alias.adopt_username(parse_username("bob"));
            assert_eq!(state.username.get(), Some(parse_username("bob")));
        });
    }
}
