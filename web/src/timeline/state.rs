//! Timeline pagination — the host-tested model (ADR-0070 §6): the
//! `TimelineCursor` newtype, the `LoadStatus` enum, and the reactive
//! `TimelineState` signal bundle that wraps them. Everything here is ungated and
//! coverage-measured; the signal bundle is exercised under a reactive `Owner`
//! (the `web::reactive` / `forms::Field` / `tags::input_state` convention), which
//! is what makes its transitions testable at all — they were invisible to the
//! coverage gate while the bundle lived in the wasm-only `component.rs` (#671).
//!
//! `component.rs` keeps only what cannot run on the host: `Effect::new` and
//! `spawn_local`.

use leptos::prelude::*;

use common::ids::PostId;
use common::seed::{TimelinePage, TimelinePostSummary};
use common::time::UtcInstant;

use crate::error::{WebError, WebResult};
use crate::taglist::TagCtx;

/// A keyset pagination cursor: the `(created_at, post_id)` pair a timeline page
/// hands back to fetch the next page. Bundling the two — which always move
/// together — makes "one set, the other not" unrepresentable (they were two
/// independent `Option` signals before #329).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineCursor {
    pub created_at: UtcInstant,
    pub post_id: PostId,
}

impl TimelineCursor {
    /// Build a cursor from a page's flat next-cursor fields: `Some` only when
    /// **both** components are present. A partial pair (which the server never
    /// emits) collapses to `None` rather than a half-cursor.
    #[must_use]
    pub fn from_page(page: &TimelinePage) -> Option<Self> {
        match (page.next_cursor_created_at, page.next_cursor_post_id) {
            (Some(created_at), Some(post_id)) => Some(Self {
                created_at,
                post_id,
            }),
            _ => None,
        }
    }

    /// Split an optional cursor into the `(created_at, post_id)` optionals a
    /// timeline list fn takes — `(None, None)` when there is no cursor. Keeps the
    /// pairing logic host-tested and out of the wasm-only paginator.
    #[must_use]
    pub fn into_query(cursor: Option<Self>) -> (Option<UtcInstant>, Option<PostId>) {
        match cursor {
            Some(c) => (Some(c.created_at), Some(c.post_id)),
            None => (None, None),
        }
    }
}

/// The load state of a timeline: idle, a load-more in flight, or a failed fetch
/// carrying the error itself. Replaces the old `loading_more: bool` +
/// `error: Option<String>` pair, which admitted the illegal "loading *and*
/// errored" combination.
///
/// `Failed` carries the typed [`WebError`], not a pre-rendered `String` (#671):
/// failure stays on `Result`'s error axis all the way to the render, which is the
/// only place that decides how to display it. Stringifying at the producer threw
/// the error *kind* away for no benefit.
/// `NeverLoaded` is the default so "loaded yet?" is a property of the status
/// rather than a parallel `RwSignal<bool>` each page carried alongside it (#671):
/// "idle but never loaded" is now unrepresentable, the same way `Failed` already
/// made "loading *and* errored" unrepresentable. `Unidentified` is the terminal
/// outcome of a load that resolved to *nobody* — the cockpit's anonymous/expired
/// session — which is neither a failure nor a page.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LoadStatus {
    #[default]
    NeverLoaded,
    Idle,
    InFlight,
    Failed(WebError),
    Unidentified,
}

impl LoadStatus {
    /// Whether a load-more is in flight (drives the button's disabled state).
    #[must_use]
    pub fn is_in_flight(&self) -> bool {
        matches!(self, Self::InFlight)
    }

    /// Consume the status into the error to display, if the last load failed.
    /// Owned (`self`) so the reactive callers — which hold a cloned `LoadStatus`
    /// from the status signal's `.get()` — can return the `WebError` directly
    /// instead of re-matching the `Failed` arm inline.
    #[must_use]
    pub fn into_failure(self) -> Option<WebError> {
        match self {
            Self::Failed(error) => Some(error),
            Self::NeverLoaded | Self::Idle | Self::InFlight | Self::Unidentified => None,
        }
    }
}

/// What the gate should paint right now — the one decision every timeline page
/// used to re-derive inside its own view closure, where the coverage gate could
/// not see it (#671).
///
/// Failure is **not** a variant here: it travels on `WebResult`'s error axis, so
/// the type keeps saying which outcomes are successes. `Unidentified` means there
/// is no viewer/subject to show a timeline for — the page decides whether that is
/// blank or a redirect, via [`NoIdentity`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelinePaint {
    Loading,
    Rows(TagCtx),
    Unidentified,
}

impl TimelinePaint {
    /// Whether the page's own chrome accompanies this paint. The gate renders
    /// chrome in a sibling region gated on this, rather than inside each arm, so
    /// the subtree survives a `Loading → Rows` transition instead of being torn
    /// down and rebuilt — which for `home` is projector-coincident markup
    /// (ADR-0041 §2, the #653 flash class).
    #[must_use]
    pub fn shows_chrome(&self) -> bool {
        match self {
            Self::Loading | Self::Rows(_) => true,
            Self::Unidentified => false,
        }
    }
}

/// How a page renders [`TimelinePaint::Unidentified`]. A **data** enum rather than
/// a closure prop: the choice stays host-testable and the gate body stays a bare
/// `match`, which is what the thin-component direction (#306) wants by
/// construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoIdentity {
    Blank,
    Redirect(&'static str),
}

/// The reactive state of a cursor-paginated timeline, shared by the public Local
/// timeline (`home.rs`) and the authed `/app` cockpit (`cockpit.rs`).
///
/// Every field is an `RwSignal` (a `Copy` handle into the reactive runtime), so the
/// whole struct is `Copy` and can be handed to each event closure and child callback
/// without per-signal capture.
#[derive(Clone, Copy, Default)]
pub struct TimelineState {
    pub rows: RwSignal<Vec<TimelinePostSummary>>,
    pub cursor: RwSignal<Option<TimelineCursor>>,
    pub has_more: RwSignal<bool>,
    pub status: RwSignal<LoadStatus>,
}

impl TimelineState {
    /// Adopt a page's rows + cursor — a projector seed or a fresh fetch —
    /// replacing what's shown and settling to idle.
    ///
    /// Settling to `Idle` is what lets this one method serve **both** the seed
    /// path and the fetch-resolve path; the separate `resolve()` it replaced
    /// differed only by that line (#671). It also clears any prior failure, so a
    /// successful refetch after an error recovers.
    pub fn adopt(&self, page: TimelinePage) {
        self.cursor.set(TimelineCursor::from_page(&page));
        self.has_more.set(page.has_more);
        self.rows.set(page.posts);
        self.status.set(LoadStatus::Idle);
    }

    /// Adopt a projector seed when the page was seeded for these params. `None`
    /// means the projector painted a different page (or none), so the timeline
    /// stays `NeverLoaded` and the reactive fetch fills it in.
    pub fn adopt_seed(&self, page: Option<TimelinePage>) {
        if let Some(page) = page {
            self.adopt(page);
        }
    }

    /// Apply an initial/refetch result: replace on success, reset on failure.
    pub fn apply(&self, result: WebResult<TimelinePage>) {
        match result {
            Ok(page) => self.adopt(page),
            Err(error) => self.fail(error),
        }
    }

    /// Record a fetch failure: empty the rows (don't show a stale page), clear
    /// the cursor + `has_more` so a failed timeline offers no "Load more", and
    /// mark the failure for display.
    pub fn fail(&self, error: WebError) {
        self.rows.set(Vec::new());
        self.cursor.set(None);
        self.has_more.set(false);
        self.status.set(LoadStatus::Failed(error));
    }

    /// Record that the load resolved to no viewer at all (anonymous / expired).
    /// Clears like a failure, but is not one — the page decides what to paint,
    /// which for the cockpit is a redirect to `/login`.
    pub fn unidentified(&self) {
        self.rows.set(Vec::new());
        self.cursor.set(None);
        self.has_more.set(false);
        self.status.set(LoadStatus::Unidentified);
    }

    /// Apply a load-more result: **extend** on success, and on failure mark the
    /// status *only*.
    ///
    /// Deliberately asymmetric with [`apply`](Self::apply), which clears: page 1
    /// succeeded and only page 2 failed, so throwing page 1 away would lose work
    /// the user already has.
    pub fn append(&self, result: WebResult<TimelinePage>) {
        match result {
            Ok(page) => {
                self.cursor.set(TimelineCursor::from_page(&page));
                self.has_more.set(page.has_more);
                self.rows.update(|rows| rows.extend(page.posts));
                self.status.set(LoadStatus::Idle);
            }
            Err(error) => self.status.set(LoadStatus::Failed(error)),
        }
    }

    /// Claim the load-more slot: `None` when there is nothing to fetch or a fetch
    /// is already in flight, else the current cursor as the `(created_at, post_id)`
    /// query pair, having marked the status `InFlight`.
    ///
    /// Returning the query pair rather than a bare `bool` keeps the cursor read
    /// and its split host-tested, leaving the wasm caller a six-line shell.
    pub fn begin_load_more(&self) -> Option<(Option<UtcInstant>, Option<PostId>)> {
        if self.status.get_untracked().is_in_flight() || !self.has_more.get_untracked() {
            return None;
        }
        self.status.set(LoadStatus::InFlight);
        Some(TimelineCursor::into_query(self.cursor.get_untracked()))
    }

    /// Fold the status and the page's row context into the one thing the gate
    /// needs to know: what to paint.
    ///
    /// `context` is the page's *route-derived* identity — `None` when its URL
    /// segment has not resolved to a user. That is a different source from
    /// `LoadStatus::Unidentified`, which is *fetch-derived* (the cockpit's session
    /// resolving to nobody), but both mean the same thing to the renderer, so they
    /// converge on one arm.
    ///
    /// # Errors
    ///
    /// Returns the [`WebError`] the last load failed with, so failure reaches the
    /// render on `Result`'s error axis instead of as an anonymous `Option`. It
    /// outranks `context`: an errored timeline paints its banner whether or not the
    /// route identity resolved.
    pub fn paint(&self, context: Option<TagCtx>) -> WebResult<TimelinePaint> {
        match self.status.get() {
            LoadStatus::Failed(error) => Err(error),
            LoadStatus::NeverLoaded => Ok(TimelinePaint::Loading),
            LoadStatus::Unidentified => Ok(TimelinePaint::Unidentified),
            LoadStatus::Idle | LoadStatus::InFlight => match context {
                Some(context) => Ok(TimelinePaint::Rows(context)),
                None => Ok(TimelinePaint::Unidentified),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts::render::test_fixtures::sample_summary;
    use common::test_support::parse_username;

    fn instant() -> UtcInstant {
        "2026-07-19T10:30:00Z".parse().unwrap()
    }

    fn for_user() -> TagCtx {
        TagCtx::ForUser(parse_username("bob"))
    }

    fn page_with(
        posts: Vec<TimelinePostSummary>,
        next_cursor_created_at: Option<UtcInstant>,
        next_cursor_post_id: Option<PostId>,
        has_more: bool,
    ) -> TimelinePage {
        TimelinePage {
            posts,
            next_cursor_created_at,
            next_cursor_post_id,
            has_more,
        }
    }

    fn page(
        next_cursor_created_at: Option<UtcInstant>,
        next_cursor_post_id: Option<PostId>,
        has_more: bool,
    ) -> TimelinePage {
        page_with(
            Vec::new(),
            next_cursor_created_at,
            next_cursor_post_id,
            has_more,
        )
    }

    /// Run `body` under a fresh reactive `Owner` (the `web::reactive` /
    /// `forms::Field` / `tags::input_state` convention), so `RwSignal`s work
    /// host-side without a browser.
    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    #[test]
    fn cursor_from_page_needs_both_components() {
        assert_eq!(
            TimelineCursor::from_page(&page(Some(instant()), Some(PostId::from(7)), true)),
            Some(TimelineCursor {
                created_at: instant(),
                post_id: PostId::from(7)
            }),
        );
        assert_eq!(TimelineCursor::from_page(&page(None, None, false)), None);
        assert_eq!(
            TimelineCursor::from_page(&page(Some(instant()), None, true)),
            None
        );
        assert_eq!(
            TimelineCursor::from_page(&page(None, Some(PostId::from(7)), true)),
            None
        );
    }

    #[test]
    fn cursor_into_query_splits_or_empties() {
        let cursor = TimelineCursor {
            created_at: instant(),
            post_id: PostId::from(7),
        };
        assert_eq!(
            TimelineCursor::into_query(Some(cursor)),
            (Some(instant()), Some(PostId::from(7))),
        );
        assert_eq!(TimelineCursor::into_query(None), (None, None));
    }

    #[test]
    fn default_status_is_never_loaded() {
        with_owner(|| {
            let state = TimelineState::default();
            assert!(state.rows.get().is_empty());
            assert_eq!(state.cursor.get(), None);
            assert!(!state.has_more.get());
            assert_eq!(state.status.get(), LoadStatus::NeverLoaded);
        });
    }

    #[test]
    fn adopt_replaces_rows_cursor_and_has_more() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(
                vec![sample_summary()],
                Some(instant()),
                Some(PostId::from(7)),
                true,
            ));
            assert_eq!(state.rows.get().len(), 1);
            assert_eq!(
                state.cursor.get(),
                Some(TimelineCursor {
                    created_at: instant(),
                    post_id: PostId::from(7)
                })
            );
            assert!(state.has_more.get());
        });
    }

    #[test]
    fn adopt_settles_to_idle_and_clears_a_prior_failure() {
        with_owner(|| {
            let state = TimelineState::default();
            state.fail(WebError::validation("boom"));
            state.adopt(page_with(vec![sample_summary()], None, None, false));
            assert_eq!(state.rows.get().len(), 1);
            assert_eq!(
                state.status.get(),
                LoadStatus::Idle,
                "adopt IS the old resolve"
            );
        });
    }

    #[test]
    fn adopt_seed_adopts_only_when_seeded() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt_seed(None);
            assert!(state.rows.get().is_empty());
            assert_eq!(
                state.status.get(),
                LoadStatus::NeverLoaded,
                "not seeded, not loaded"
            );

            state.adopt_seed(Some(page_with(vec![sample_summary()], None, None, false)));
            assert_eq!(state.rows.get().len(), 1);
            assert_eq!(state.status.get(), LoadStatus::Idle);
        });
    }

    #[test]
    fn apply_ok_adopts_and_apply_err_empties() {
        with_owner(|| {
            let state = TimelineState::default();
            state.apply(Ok(page_with(
                vec![sample_summary()],
                Some(instant()),
                Some(PostId::from(7)),
                true,
            )));
            assert_eq!(state.rows.get().len(), 1);
            assert!(state.has_more.get());

            state.apply(Err(WebError::validation("boom")));
            assert!(
                state.rows.get().is_empty(),
                "no stale page on a refetch failure"
            );
            assert_eq!(state.cursor.get(), None);
            assert!(!state.has_more.get());
            assert_eq!(
                state.status.get(),
                LoadStatus::Failed(WebError::validation("boom"))
            );
        });
    }

    #[test]
    fn unidentified_empties_the_timeline_and_marks_the_status() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(vec![sample_summary()], None, None, true));
            state.unidentified();
            assert!(state.rows.get().is_empty());
            assert_eq!(state.cursor.get(), None);
            assert!(!state.has_more.get());
            assert_eq!(state.status.get(), LoadStatus::Unidentified);
        });
    }

    // All four effects asserted, so an `append` that forgets the cursor — and
    // therefore refetches page 1 forever — cannot pass.
    #[test]
    fn append_ok_extends_rows_and_advances_the_cursor() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(
                vec![sample_summary()],
                Some(instant()),
                Some(PostId::from(7)),
                true,
            ));
            state.status.set(LoadStatus::InFlight);

            let later: UtcInstant = "2026-07-20T10:30:00Z".parse().unwrap();
            state.append(Ok(page_with(
                vec![sample_summary(), sample_summary()],
                Some(later),
                Some(PostId::from(9)),
                false,
            )));

            assert_eq!(state.rows.get().len(), 3, "extends, does not replace");
            assert_eq!(
                state.cursor.get(),
                Some(TimelineCursor {
                    created_at: later,
                    post_id: PostId::from(9)
                }),
                "cursor advances to the new page"
            );
            assert!(!state.has_more.get(), "has_more is overwritten");
            assert_eq!(state.status.get(), LoadStatus::Idle);
        });
    }

    // A load-more failure keeps the pages already fetched — unlike `apply`, which
    // clears. The asymmetry is deliberate: page 1 succeeded, only page 2 failed.
    #[test]
    fn append_err_marks_the_status_and_retains_the_rows() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(
                vec![sample_summary()],
                Some(instant()),
                Some(PostId::from(7)),
                true,
            ));
            state.append(Err(WebError::validation("boom")));

            assert_eq!(
                state.rows.get().len(),
                1,
                "page 1 survives a page-2 failure"
            );
            assert_eq!(
                state.cursor.get(),
                Some(TimelineCursor {
                    created_at: instant(),
                    post_id: PostId::from(7)
                }),
                "cursor untouched"
            );
            assert!(state.has_more.get(), "has_more untouched");
            assert_eq!(
                state.status.get(),
                LoadStatus::Failed(WebError::validation("boom"))
            );
        });
    }

    #[test]
    fn begin_load_more_guards_then_marks_in_flight() {
        with_owner(|| {
            let state = TimelineState::default();

            state.has_more.set(false);
            assert_eq!(state.begin_load_more(), None, "nothing more to fetch");

            state.has_more.set(true);
            state.status.set(LoadStatus::InFlight);
            assert_eq!(state.begin_load_more(), None, "already in flight");

            state.status.set(LoadStatus::Idle);
            state.cursor.set(Some(TimelineCursor {
                created_at: instant(),
                post_id: PostId::from(7),
            }));
            assert_eq!(
                state.begin_load_more(),
                Some((Some(instant()), Some(PostId::from(7)))),
                "hands back the cursor as a query pair"
            );
            assert_eq!(
                state.status.get(),
                LoadStatus::InFlight,
                "and marks it in flight"
            );
        });
    }

    #[test]
    fn begin_load_more_without_a_cursor_yields_an_empty_query() {
        with_owner(|| {
            let state = TimelineState::default();
            state.has_more.set(true);
            assert_eq!(state.begin_load_more(), Some((None, None)));
        });
    }

    #[test]
    fn fail_empties_the_timeline_and_records_the_message() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(
                vec![sample_summary()],
                Some(instant()),
                Some(PostId::from(7)),
                true,
            ));
            state.fail(WebError::validation("boom"));
            assert!(state.rows.get().is_empty(), "no stale page");
            assert_eq!(state.cursor.get(), None);
            assert!(
                !state.has_more.get(),
                "a failed timeline offers no Load more"
            );
            assert_eq!(
                state.status.get(),
                LoadStatus::Failed(WebError::validation("boom"))
            );
        });
    }

    #[test]
    fn paint_reports_failure_on_the_error_axis() {
        with_owner(|| {
            let state = TimelineState::default();
            state.fail(WebError::validation("boom"));
            assert_eq!(
                state.paint(Some(TagCtx::SiteWide)),
                Err(WebError::validation("boom")),
                "failure outranks context"
            );
            assert_eq!(state.paint(None), Err(WebError::validation("boom")));
        });
    }

    #[test]
    fn paint_reports_loading_before_the_first_load() {
        with_owner(|| {
            let state = TimelineState::default();
            assert_eq!(
                state.paint(Some(TagCtx::SiteWide)),
                Ok(TimelinePaint::Loading)
            );
            assert_eq!(state.paint(None), Ok(TimelinePaint::Loading));
        });
    }

    #[test]
    fn paint_reports_unidentified_when_the_fetch_found_nobody() {
        with_owner(|| {
            let state = TimelineState::default();
            state.unidentified();
            assert_eq!(
                state.paint(Some(TagCtx::SiteWide)),
                Ok(TimelinePaint::Unidentified),
                "a fetch-determined absence outranks a present context"
            );
        });
    }

    #[test]
    fn paint_reports_rows_once_loaded_with_a_context() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(vec![sample_summary()], None, None, false));
            assert_eq!(
                state.paint(Some(TagCtx::SiteWide)),
                Ok(TimelinePaint::Rows(TagCtx::SiteWide))
            );
            assert_eq!(
                state.paint(Some(for_user())),
                Ok(TimelinePaint::Rows(for_user()))
            );

            state.status.set(LoadStatus::InFlight);
            assert_eq!(
                state.paint(Some(TagCtx::SiteWide)),
                Ok(TimelinePaint::Rows(TagCtx::SiteWide)),
                "a load-more in flight keeps painting rows"
            );
        });
    }

    #[test]
    fn paint_reports_unidentified_when_the_route_context_is_absent() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt(page_with(vec![sample_summary()], None, None, false));
            assert_eq!(state.paint(None), Ok(TimelinePaint::Unidentified));

            state.status.set(LoadStatus::InFlight);
            assert_eq!(state.paint(None), Ok(TimelinePaint::Unidentified));
        });
    }

    // Projector coincidence, pinned at unit level: a seeded page must paint ROWS on
    // its very first `paint()`, never `Loading`. `adopt_seed` runs synchronously in
    // the component body before first render, so if this ever returned `Loading` a
    // projector-painted page would flash its loading placeholder over server-rendered
    // content — the #653 class. Fails in milliseconds; the CLS probes are the
    // browser-level backstop.
    #[test]
    fn a_seeded_timeline_paints_rows_immediately_never_loading() {
        with_owner(|| {
            let state = TimelineState::default();
            state.adopt_seed(Some(page_with(vec![sample_summary()], None, None, false)));
            assert_eq!(
                state.paint(Some(TagCtx::SiteWide)),
                Ok(TimelinePaint::Rows(TagCtx::SiteWide)),
                "a projector-seeded page must never paint Loading"
            );
        });
    }

    #[test]
    fn shows_chrome_for_every_paint() {
        assert!(TimelinePaint::Loading.shows_chrome());
        assert!(TimelinePaint::Rows(TagCtx::SiteWide).shows_chrome());
        assert!(!TimelinePaint::Unidentified.shows_chrome());
    }

    // `NoIdentity` is only *matched* in the wasm gate body, so without this its
    // derive line would be an uncovered host line — and the coverage gate forbids a
    // marker here. Same reason for `TimelinePaint`'s `Debug`: `assert_eq!` formats
    // only on FAILURE, so the tests above never invoke it.
    #[test]
    fn no_identity_variants_are_distinct_and_copyable() {
        let blank = NoIdentity::Blank;
        let redirect = NoIdentity::Redirect("/login");
        assert_ne!(blank, redirect);
        assert_eq!(redirect, redirect, "Copy + PartialEq");
        assert_eq!(format!("{blank:?}"), "Blank");
    }

    #[test]
    fn timeline_paint_is_debug_printable() {
        assert_eq!(format!("{:?}", TimelinePaint::Loading), "Loading");
        assert_eq!(format!("{:?}", TimelinePaint::Unidentified), "Unidentified");
        assert!(format!("{:?}", TimelinePaint::Rows(TagCtx::SiteWide)).contains("Rows"));
    }

    #[test]
    fn is_in_flight_covers_every_status() {
        assert!(!LoadStatus::NeverLoaded.is_in_flight());
        assert!(!LoadStatus::Idle.is_in_flight());
        assert!(LoadStatus::InFlight.is_in_flight());
        assert!(!LoadStatus::Failed(WebError::validation("boom")).is_in_flight());
        assert!(!LoadStatus::Unidentified.is_in_flight());
    }

    // The payload is the typed `WebError`, not a pre-rendered string: the error KIND
    // survives the round trip, so the render decides how to display it and nothing
    // stringifies eagerly at the producer (#671 D3).
    #[test]
    fn into_failure_covers_every_status() {
        assert_eq!(LoadStatus::NeverLoaded.into_failure(), None);
        assert_eq!(LoadStatus::Idle.into_failure(), None);
        assert_eq!(LoadStatus::InFlight.into_failure(), None);
        assert_eq!(LoadStatus::Unidentified.into_failure(), None);
        assert_eq!(
            LoadStatus::Failed(WebError::validation("boom")).into_failure(),
            Some(WebError::validation("boom")),
        );
    }
}
