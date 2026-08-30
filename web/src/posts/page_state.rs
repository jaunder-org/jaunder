//! Host-compiled, host-tested decision logic for the posts vertical's pages
//! (#306/#58, ADR-0083).
//!
//! The tag and user-timeline pages each carried the same shaped setup in their
//! wasm-only component bodies: decide whether a projector seed applies to the route
//! this render is for, and decide whether a route parameter is usable before
//! fetching. Neither is browser wiring — both are folds over already-resolved values
//! — so they belong here, where `nextest` can assert them and the coverage gate can
//! see them, leaving the component with the `Effect`/`Resource` wiring that genuinely
//! cannot run on the host.
//!
//! The post editor's two folds live here for the same reason: [`publish_redirect`]
//! (did this update publish, and where does the browser go?) and [`with_post_id`]
//! (short-circuit an absent/unparseable `post_id` to a client-side not-found instead
//! of paying a round-trip, #487).
//!
//! `PostCard`'s parent-callback plumbing ([`notify`], [`notify_with_fallback`]) is
//! here too. Firing an `Option<Callback>` is not browser wiring — ADR-0083 §1 grants
//! only `Effect::new` and `spawn_local` permanent wasm-only status — so it is
//! exercised under a reactive `Owner` exactly as [`crate::media::UploadCallbacks`]'s
//! twin is.
//!
//! The named-audience picker's load and submit decision is another pure fold:
//! [`NamedAudienceState`] keeps unresolved, genuinely loaded-empty, populated,
//! and failed results distinct so only a real successful load can authorize a
//! create or update.

use std::future::Future;

use leptos::prelude::*;

use common::ids::PostId;
use common::revision_history::{
    RevisionHistoryAudience, RevisionHistoryDetail, RevisionHistoryTag,
};
use common::root_relative_url::RootRelativeUrl;
use common::seed::{Page, PageSeed, RenderedPost};
use common::tag::Tag;
use common::username::Username;
use common::visibility::AudienceSelection;

use crate::audiences;
use crate::error::{WebError, WebResult};
use crate::posts::{
    CurrentPostHistory, RevisionHistoryCursor, RevisionHistoryMetadata, RevisionHistoryPage,
    RevisionLifecycle, SavedPost,
};

/// Resolution state for the named audiences offered by the post editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedAudienceState {
    /// The audience request has not settled.
    Loading,
    /// The server returned the author's audiences, including a genuine empty list.
    Ready(Vec<audiences::Summary>),
    /// The audience request failed.
    Failed,
}

impl NamedAudienceState {
    /// Fold the audience resource's unresolved/resolved shape into page state.
    #[must_use]
    pub fn resolve(result: Option<Result<Vec<audiences::Summary>, WebError>>) -> Self {
        match result {
            None => Self::Loading,
            Some(Ok(audiences)) => Self::Ready(audiences),
            Some(Err(_)) => Self::Failed,
        }
    }

    /// Borrow the current selection only after a successful audience load.
    ///
    /// A loaded-empty list is still successful: the base selection remains a
    /// real author choice. Loading and failure return no payload, so callers
    /// cannot dispatch either as an invented empty named selection.
    #[must_use]
    pub const fn selection_for_submit<'a>(
        &self,
        selection: &'a AudienceSelection,
    ) -> Option<&'a AudienceSelection> {
        match self {
            Self::Loading | Self::Failed => None,
            Self::Ready(_) => Some(selection),
        }
    }
}

/// Which listing page is rendering, carrying the route segments it has **already**
/// parsed (`None` = the segment was absent or would not parse).
///
/// A data enum rather than one seed-matching fn per page (ADR-0083 §3): the three
/// pages differ only in which parts of the URL identify them, so the difference
/// travels as a value and [`seeded_page`] stays one host-tested fold.
pub enum ListingRoute {
    /// `/~:username` — the user timeline.
    Profile(Option<Username>),
    /// `/tags/:tag` — the site-wide tag listing.
    SiteTag(Option<Tag>),
    /// `/~:username/tags/:tag` — the per-user tag listing.
    UserTag(Option<Username>, Option<Tag>),
}

/// The projector seed this render may adopt (#178/#179): `Some(page)` only when the
/// seed the server left in context is the seed for **this** route.
///
/// The guard is what makes a client-side navigation safe. The seed is whatever the
/// initial URL was painted with and it never changes, so a nav from `/~alice` to
/// `/~bob` would otherwise adopt alice's posts under bob's heading. A non-matching
/// seed, a seed for a different page kind, and no seed at all all mean the same
/// thing — nothing to adopt — and the reactive fetch fills the page in.
#[must_use]
pub fn seeded_page(seed: Option<PageSeed>, route: &ListingRoute) -> Option<Page<RenderedPost>> {
    match (seed?, route) {
        (PageSeed::Profile { username, page }, ListingRoute::Profile(wanted))
            if wanted.as_ref() == Some(&username) =>
        {
            Some(page)
        }
        (PageSeed::SiteTag { tag, page }, ListingRoute::SiteTag(wanted))
            if wanted.as_ref() == Some(&tag) =>
        {
            Some(page)
        }
        (
            PageSeed::UserTag {
                username,
                tag,
                page,
            },
            ListingRoute::UserTag(wanted_username, wanted_tag),
        ) if wanted_username.as_ref() == Some(&username) && wanted_tag.as_ref() == Some(&tag) => {
            Some(page)
        }
        _ => None,
    }
}

/// The username a user-scoped listing fetch needs.
///
/// # Errors
///
/// [`WebError::validation`] when the `~username` segment did not parse — the page
/// paints that error instead of fetching, since no such user can exist.
pub fn user_query(username: Option<Username>) -> WebResult<Username> {
    username.ok_or_else(|| WebError::validation("Invalid username"))
}

/// The tag a tag listing fetch needs.
///
/// # Errors
///
/// [`WebError::validation`] when the `:tag` segment did not parse.
pub fn tag_query(tag: Option<Tag>) -> WebResult<Tag> {
    tag.ok_or_else(|| WebError::validation("Invalid tag"))
}

/// Both route values the per-user tag listing needs, at once.
///
/// # Errors
///
/// The username's error outranks the tag's, so a page whose *both* segments are
/// broken names the first one — the order the two separate guards had.
pub fn user_tag_query(username: Option<Username>, tag: Option<Tag>) -> WebResult<(Username, Tag)> {
    Ok((user_query(username)?, tag_query(tag)?))
}

/// Where an update settles the browser, shaped for `on_settled_ok`'s read closure:
/// `Some(Ok(permalink))` only when the update **published** the post.
///
/// Editor → permalink is always a route change, so a fresh `PostPage` mount refetches
/// — no explicit invalidation needed (#592). A settled-but-still-draft update, a
/// failed one, and "not settled yet" all mean *nothing to navigate to*, and
/// `on_settled_ok` skips all three identically, so collapsing them here leaves the
/// component with no branch at all.
///
/// The permalink stays a [`RootRelativeUrl`] all the way to `use_navigate`, which takes
/// `&str` by deref — unwrapping it here would trade the type for an allocation.
#[must_use]
pub fn publish_redirect<E>(
    settled: Option<Result<SavedPost, E>>,
) -> Option<Result<RootRelativeUrl, E>> {
    let updated = settled?.ok()?;
    let published = updated.published_at.is_some();
    published.then_some(updated.permalink).map(Ok)
}

/// Fire an optional parent callback, when the caller supplied one.
///
/// Every lifecycle hook in the posts vertical spelled out the same `if let Some(cb)`
/// — caller plumbing, not component logic (#306), and nothing about it is
/// browser-bound, so it lives in this host-compiled module rather than in the
/// wasm-only `component.rs` where no test could reach it.
pub fn notify(callback: Option<Callback<()>>) {
    if let Some(callback) = callback {
        callback.run(());
    }
}

/// Fire `preferred`, falling back to `shared` when the caller supplied only the
/// shared one.
///
/// `PostCard`'s unpublish policy: a caller that wants to tell unpublish apart from the
/// other mutations passes `on_unpublish`; one that treats them alike passes only
/// `on_mutate` and still gets told. That is a real per-caller rule — which of two
/// callbacks wins — so it is asserted here rather than left as an `.or()` inside the
/// component.
pub fn notify_with_fallback(preferred: Option<Callback<()>>, shared: Option<Callback<()>>) {
    notify(preferred.or(shared));
}

/// Await `fetch` with the route's post id, or short-circuit to a client-side
/// not-found.
///
/// A missing or unparseable `post_id` is honest absence, not a real id: answering it
/// here rather than minting a sentinel id avoids a round-trip that could only ever
/// return not-found (#487). Both of the editor's resources fetch through this, so the
/// short-circuit is written — and asserted — once.
///
/// # Errors
///
/// [`WebError::not_found`] for an absent id; otherwise whatever `fetch` returns.
pub async fn with_post_id<T, F, Fut>(post_id: Option<PostId>, fetch: F) -> WebResult<T>
where
    F: FnOnce(PostId) -> Fut,
    Fut: Future<Output = WebResult<T>>,
{
    match post_id {
        Some(post_id) => fetch(post_id).await,
        None => Err(WebError::not_found("Post")),
    }
}

/// One scalar value shown in a history metadata card.
#[derive(Debug, PartialEq, Eq)]
pub struct HistoryDisplayRow {
    /// Human-readable field name.
    pub label: &'static str,
    /// Already-formatted immutable field value.
    pub value: String,
    /// Stable selector attached to the value, when the field has one.
    pub data_test: Option<&'static str>,
}

/// One child collection shown in an immutable revision snapshot.
#[derive(Debug, PartialEq, Eq)]
pub struct HistoryCollectionDisplay {
    /// Human-readable section heading.
    pub heading: &'static str,
    /// Heading ID used by the section's accessible name.
    pub heading_id: &'static str,
    /// Stable end-to-end selector.
    pub data_test: &'static str,
    /// Preformatted collection source, or the collection's useful empty message.
    pub value: String,
}

fn display_row(label: &'static str, value: String) -> HistoryDisplayRow {
    HistoryDisplayRow {
        label,
        value,
        data_test: None,
    }
}

fn optional_display(value: Option<impl ToString>, absent: &'static str) -> String {
    value.map_or_else(|| absent.to_owned(), |value| value.to_string())
}

fn lifecycle_label(lifecycle: &RevisionLifecycle) -> &'static str {
    match lifecycle {
        RevisionLifecycle::Draft => "Draft",
        RevisionLifecycle::Scheduled => "Scheduled",
        RevisionLifecycle::Published => "Published",
        RevisionLifecycle::Deleted => "Deleted",
    }
}

/// Flatten the current Post state into the single metadata-row shape the UI paints.
#[must_use]
pub fn current_history_rows(current: CurrentPostHistory) -> Vec<HistoryDisplayRow> {
    vec![
        display_row("Post ID", i64::from(current.post_id).to_string()),
        display_row("Title", optional_display(current.title, "No title")),
        display_row("Slug", current.slug.to_string()),
        display_row("Format", current.format.to_string()),
        HistoryDisplayRow {
            label: "Lifecycle",
            value: lifecycle_label(&current.lifecycle).to_owned(),
            data_test: Some("history-current-lifecycle"),
        },
        display_row("Created", current.created_at.to_string()),
        display_row("Updated", current.updated_at.to_string()),
        display_row(
            "Published",
            optional_display(current.published_at, "Not set"),
        ),
        display_row("Deleted", optional_display(current.deleted_at, "Not set")),
    ]
}

/// Flatten immutable revision scalars into the same metadata-row shape as current state.
#[must_use]
pub fn revision_history_rows(detail: &RevisionHistoryDetail) -> Vec<HistoryDisplayRow> {
    vec![
        display_row("Revision ID", i64::from(detail.revision_id).to_string()),
        display_row("Post ID", i64::from(detail.post_id).to_string()),
        display_row("Title", optional_display(detail.title.as_ref(), "No title")),
        display_row("Slug", detail.slug.to_string()),
        display_row("Format", detail.format.to_string()),
        display_row(
            "Summary",
            optional_display(detail.summary.as_ref(), "Not set"),
        ),
        display_row("Created", detail.created_at.to_string()),
        display_row("Updated", detail.updated_at.to_string()),
        display_row(
            "Published",
            optional_display(detail.published_at, "Not set"),
        ),
        display_row("Deleted", optional_display(detail.deleted_at, "Not set")),
        display_row("Captured", detail.captured_at.to_string()),
    ]
}

/// Project revision child DTOs to immutable source text before the wasm view is built.
#[must_use]
pub fn revision_collection_displays(
    tags: Vec<RevisionHistoryTag>,
    audiences: Vec<RevisionHistoryAudience>,
    media: &[String],
) -> Vec<HistoryCollectionDisplay> {
    let tags = if tags.is_empty() {
        "No tags in this snapshot.".to_owned()
    } else {
        tags.into_iter()
            .map(|tag| format!("{} ({})", tag.display, tag.tag))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let audiences = if audiences.is_empty() {
        "No audiences in this snapshot.".to_owned()
    } else {
        audiences
            .into_iter()
            .map(|audience| {
                audience
                    .audience_id
                    .map_or(audience.kind, |id| format!("named ({})", i64::from(id)))
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let media = if media.is_empty() {
        "No media references in this snapshot.".to_owned()
    } else {
        media.join("\n")
    };

    vec![
        HistoryCollectionDisplay {
            heading: "Tags",
            heading_id: "revision-tags-heading",
            data_test: "history-tags",
            value: tags,
        },
        HistoryCollectionDisplay {
            heading: "Audiences",
            heading_id: "revision-audiences-heading",
            data_test: "history-audiences",
            value: audiences,
        },
        HistoryCollectionDisplay {
            heading: "Media references",
            heading_id: "revision-media-heading",
            data_test: "history-media",
            value: media,
        },
    ]
}

/// Paint state for an authenticated history route after its serializable resource
/// result resolves.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthenticatedHistoryState<T> {
    /// The route parameters were absent or invalid.
    NotFound,
    /// Reconciliation confirmed there is no authenticated viewer.
    AuthRequired,
    /// The authenticated route fetch completed successfully.
    // rendered-html-from-trusted:allow history paint state may carry server-sanitized revision DTO HTML (#1055)
    Ready(T),
    /// Session reconciliation or the route fetch failed.
    Failed(WebError),
}

impl<T> AuthenticatedHistoryState<T> {
    /// Project only the successful payload while preserving every non-ready state.
    pub fn map_ready<U>(self, project: impl FnOnce(T) -> U) -> AuthenticatedHistoryState<U> {
        match self {
            Self::NotFound => AuthenticatedHistoryState::NotFound,
            Self::AuthRequired => AuthenticatedHistoryState::AuthRequired,
            Self::Ready(value) => AuthenticatedHistoryState::Ready(project(value)),
            Self::Failed(error) => AuthenticatedHistoryState::Failed(error),
        }
    }
}

/// Resolve one authenticated history resource without putting a UI-only state enum
/// into Leptos's serializable [`Resource`] payload.
///
/// `reconcile` is lazy so an invalid route remains a client-side not-found and does
/// not issue even the session request. `fetch` is likewise called only after the
/// session confirms an authenticated viewer.
///
/// # Errors
///
/// Propagates the reconcile error unchanged, or the route fetch error after
/// authentication succeeds.
pub async fn load_authenticated_history<R, U, T, RF, RFut, F, Fut>(
    route: Option<R>,
    reconcile: RF,
    fetch: F,
) -> WebResult<Option<T>>
where
    RF: FnOnce() -> RFut,
    RFut: Future<Output = WebResult<Option<U>>>,
    F: FnOnce(R) -> Fut,
    Fut: Future<Output = WebResult<T>>,
{
    let Some(route) = route else {
        return Ok(None);
    };
    let Some(_) = reconcile().await? else {
        return Ok(None);
    };
    fetch(route).await.map(Some)
}

/// Project a serializable history resource result into the page's four paint states.
///
/// The loader returns `Ok(None)` for both an invalid route and an anonymous session
/// so its payload stays the existing serde-friendly `WebResult<Option<T>>`; the
/// already-parsed route presence disambiguates those states after resolution.
pub fn authenticated_history_state<T>(
    route_present: bool,
    result: WebResult<Option<T>>,
) -> AuthenticatedHistoryState<T> {
    match result {
        Err(error) => AuthenticatedHistoryState::Failed(error),
        Ok(Some(value)) => AuthenticatedHistoryState::Ready(value),
        Ok(None) if route_present => AuthenticatedHistoryState::AuthRequired,
        Ok(None) => AuthenticatedHistoryState::NotFound,
    }
}

/// Reactive state and transition logic for a cursor-paginated history list.
#[derive(Clone, Copy)]
pub struct HistoryListState {
    /// All revision rows loaded so far.
    pub rows: RwSignal<Vec<RevisionHistoryMetadata>>,
    /// Cursor to request next, when the server exposed another page.
    pub cursor: RwSignal<Option<RevisionHistoryCursor>>,
    /// Whether the current page advertises another page.
    pub has_more: RwSignal<bool>,
    /// Whether a next-page request is in flight.
    pub loading_more: RwSignal<bool>,
    /// User-visible next-page failure, cleared before retrying.
    pub load_error: RwSignal<Option<String>>,
}

impl HistoryListState {
    /// Adopt the server-provided first page into reactive list state.
    #[must_use]
    pub fn new(initial: RevisionHistoryPage) -> Self {
        Self {
            rows: RwSignal::new(initial.revisions),
            cursor: RwSignal::new(initial.next_cursor),
            has_more: RwSignal::new(initial.has_more),
            loading_more: RwSignal::new(false),
            load_error: RwSignal::new(None),
        }
    }

    /// Start a next-page request when one is available and no request is in flight.
    ///
    /// Returning the cursor makes request dispatch conditional without duplicating
    /// the loading/cursor guards in the wasm-only component.
    #[must_use]
    pub fn begin_load_more(self) -> Option<RevisionHistoryCursor> {
        if self.loading_more.get_untracked() {
            return None;
        }
        let cursor = self.cursor.get_untracked()?;
        self.loading_more.set(true);
        self.load_error.set(None);
        Some(cursor)
    }

    /// Fold a completed next-page request into the rows and paging indicators.
    pub fn finish_load_more(self, result: WebResult<RevisionHistoryPage>) {
        match result {
            Ok(page) => {
                self.rows.update(|rows| rows.extend(page.revisions));
                self.cursor.set(page.next_cursor);
                self.has_more.set(page.has_more);
            }
            Err(error) => self.load_error.set(Some(error.to_string())),
        }
        self.loading_more.set(false);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::{Ready, ready};

    use super::*;
    use common::test_support::{
        parse_post_body, parse_root_relative_url, parse_slug, parse_tag, parse_tag_label,
        parse_username, parse_utc_instant,
    };
    use common::time::UtcInstant;

    fn page(has_more: bool) -> Page<RenderedPost> {
        Page {
            posts: Vec::new(),
            next_cursor: None,
            has_more,
        }
    }

    fn alice() -> Username {
        parse_username("alice")
    }

    fn rust() -> Tag {
        parse_tag("rust")
    }

    // --- seed adoption ---

    #[test]
    fn a_matching_profile_seed_is_adopted() {
        let seed = PageSeed::Profile {
            username: alice(),
            page: page(true),
        };
        let adopted = seeded_page(Some(seed), &ListingRoute::Profile(Some(alice())))
            .expect("the seed names this route");
        // Assert the seeded PAGE came through, not merely that something did: a fold
        // that returned a default page would satisfy `is_some()`.
        assert!(adopted.has_more);
    }

    #[test]
    fn a_profile_seed_for_a_different_user_is_ignored() {
        // The #178/#179 client-nav hazard: the seed is the INITIAL URL's, so without
        // the guard `/~alice` → `/~bob` would paint alice's posts under bob's heading.
        let seed = PageSeed::Profile {
            username: alice(),
            page: page(true),
        };
        assert!(
            seeded_page(
                Some(seed),
                &ListingRoute::Profile(Some(parse_username("bob")))
            )
            .is_none()
        );
    }

    #[test]
    fn a_profile_seed_is_ignored_when_the_route_segment_did_not_parse() {
        let seed = PageSeed::Profile {
            username: alice(),
            page: page(true),
        };
        assert!(seeded_page(Some(seed), &ListingRoute::Profile(None)).is_none());
    }

    #[test]
    fn a_matching_site_tag_seed_is_adopted() {
        let seed = PageSeed::SiteTag {
            tag: rust(),
            page: page(true),
        };
        let adopted =
            seeded_page(Some(seed), &ListingRoute::SiteTag(Some(rust()))).expect("tag matches");
        assert!(adopted.has_more);
    }

    #[test]
    fn a_site_tag_seed_for_a_different_tag_is_ignored() {
        let seed = PageSeed::SiteTag {
            tag: rust(),
            page: page(true),
        };
        assert!(
            seeded_page(
                Some(seed),
                &ListingRoute::SiteTag(Some(parse_tag("leptos")))
            )
            .is_none()
        );
        let seed = PageSeed::SiteTag {
            tag: rust(),
            page: page(true),
        };
        assert!(seeded_page(Some(seed), &ListingRoute::SiteTag(None)).is_none());
    }

    #[test]
    fn a_user_tag_seed_needs_both_halves_to_match() {
        let matching = seeded_page(
            Some(PageSeed::UserTag {
                username: alice(),
                tag: rust(),
                page: page(true),
            }),
            &ListingRoute::UserTag(Some(alice()), Some(rust())),
        )
        .expect("both halves match");
        assert!(matching.has_more);

        // Half a match is no match — one `&&`, both directions asserted, so dropping
        // either conjunct fails.
        assert!(
            seeded_page(
                Some(PageSeed::UserTag {
                    username: alice(),
                    tag: rust(),
                    page: page(true),
                }),
                &ListingRoute::UserTag(Some(parse_username("bob")), Some(rust())),
            )
            .is_none()
        );
        assert!(
            seeded_page(
                Some(PageSeed::UserTag {
                    username: alice(),
                    tag: rust(),
                    page: page(true),
                }),
                &ListingRoute::UserTag(Some(alice()), Some(parse_tag("leptos"))),
            )
            .is_none()
        );
    }

    #[test]
    fn a_seed_of_the_wrong_variant_is_ignored() {
        // The projector painted a *different kind* of page — e.g. the site timeline or
        // a permalink — so this route has nothing to adopt even though a seed exists.
        assert!(
            seeded_page(
                Some(PageSeed::SiteTimeline(page(true))),
                &ListingRoute::Profile(Some(alice())),
            )
            .is_none()
        );
        assert!(
            seeded_page(
                Some(PageSeed::SiteTag {
                    tag: rust(),
                    page: page(true),
                }),
                &ListingRoute::UserTag(Some(alice()), Some(rust())),
            )
            .is_none()
        );
        assert!(
            seeded_page(
                Some(PageSeed::UserTag {
                    username: alice(),
                    tag: rust(),
                    page: page(true),
                }),
                &ListingRoute::SiteTag(Some(rust())),
            )
            .is_none()
        );
    }

    #[test]
    fn no_seed_at_all_adopts_nothing() {
        assert!(seeded_page(None, &ListingRoute::Profile(Some(alice()))).is_none());
        assert!(seeded_page(None, &ListingRoute::SiteTag(Some(rust()))).is_none());
        assert!(seeded_page(None, &ListingRoute::UserTag(Some(alice()), Some(rust()))).is_none());
    }

    // --- route-param guards ---

    #[test]
    fn user_query_passes_a_parsed_username_and_names_the_bad_segment() {
        assert_eq!(user_query(Some(alice())), Ok(alice()));
        assert_eq!(
            user_query(None),
            Err(WebError::validation("Invalid username"))
        );
    }

    #[test]
    fn tag_query_passes_a_parsed_tag_and_names_the_bad_segment() {
        assert_eq!(tag_query(Some(rust())), Ok(rust()));
        assert_eq!(tag_query(None), Err(WebError::validation("Invalid tag")));
    }

    #[test]
    fn user_tag_query_reports_the_username_first() {
        assert_eq!(
            user_tag_query(Some(alice()), Some(rust())),
            Ok((alice(), rust()))
        );
        assert_eq!(
            user_tag_query(None, Some(rust())),
            Err(WebError::validation("Invalid username"))
        );
        assert_eq!(
            user_tag_query(Some(alice()), None),
            Err(WebError::validation("Invalid tag"))
        );
        // Both broken: the username's error wins.
        assert_eq!(
            user_tag_query(None, None),
            Err(WebError::validation("Invalid username"))
        );
    }

    // --- the editor's folds ---

    fn saved_post(published_at: Option<UtcInstant>) -> SavedPost {
        SavedPost {
            post_id: PostId::from(7),
            slug: parse_slug("hello"),
            published_at,
            permalink: parse_root_relative_url("/~alice/2026/01/02/hello"),
        }
    }

    #[test]
    fn a_published_update_redirects_to_its_typed_permalink() {
        assert_eq!(
            publish_redirect::<WebError>(Some(Ok(saved_post(Some(
                "2026-01-02T00:00:00Z".parse().expect("a real instant")
            )))))
            .expect("a published update navigates"),
            Ok(parse_root_relative_url("/~alice/2026/01/02/hello")),
        );
    }

    #[test]
    fn a_still_unpublished_update_stays_put() {
        // The editor must not navigate away when the author saved a draft — the
        // invariant the `published_at.is_some()` gate exists to keep.
        assert_eq!(
            publish_redirect::<WebError>(Some(Ok(saved_post(None)))),
            None
        );
    }

    #[test]
    fn an_unsettled_or_failed_update_navigates_nowhere() {
        assert_eq!(publish_redirect::<WebError>(None), None);
        assert_eq!(
            publish_redirect(Some(Err(WebError::validation("boom")))),
            None
        );
    }

    /// A fetch that records the id it was called with and hands it straight back.
    ///
    /// **One helper, not a stub inline per test.** The "must not fetch" case asserts
    /// precisely that this body never executes, so an inline stub there would be an
    /// uncovered region by construction — the coverage gate reports that, correctly,
    /// and a `cov:ignore` would be papering over it. Sharing the closure with the case
    /// where the fetch *does* run covers the body once, and strengthens the negative
    /// assertion into a real observation: the very same instrumented closure is
    /// demonstrably capable of recording, so an empty `seen` means it was not called
    /// rather than that it could not have been.
    fn recording_fetch(
        seen: &Cell<Option<PostId>>,
    ) -> impl FnOnce(PostId) -> Ready<WebResult<PostId>> + '_ {
        move |post_id| {
            seen.set(Some(post_id));
            ready(Ok(post_id))
        }
    }

    #[tokio::test]
    async fn with_post_id_fetches_when_the_route_named_one() {
        // The id reaches the fetch unchanged — a fold that dropped it would still
        // return `Ok` from a fetch that ignored its argument.
        let seen = Cell::new(None);
        let fetched = with_post_id(Some(PostId::from(7)), recording_fetch(&seen)).await;
        assert_eq!(
            seen.get(),
            Some(PostId::from(7)),
            "the fetch ran, with the route's id"
        );
        assert_eq!(fetched, Ok(PostId::from(7)));
    }

    #[tokio::test]
    async fn with_post_id_short_circuits_an_absent_id_without_fetching() {
        // Asserting the fetch never RAN is the point (#487): a version that called it
        // with a sentinel id would still return an error and pass a message-only check.
        let seen = Cell::new(None);
        let fetched = with_post_id(None, recording_fetch(&seen)).await;
        assert_eq!(fetched, Err(WebError::not_found("Post")));
        assert_eq!(seen.get(), None, "an absent id costs no round-trip");
    }

    #[tokio::test]
    async fn with_post_id_propagates_the_fetch_error() {
        let fetched = with_post_id(Some(PostId::from(7)), |_| async {
            Err::<String, _>(WebError::validation("boom"))
        })
        .await;
        assert_eq!(fetched, Err(WebError::validation("boom")));
    }

    fn history_reconcile(
        seen: &Cell<bool>,
        result: WebResult<Option<()>>,
    ) -> impl FnOnce() -> Ready<WebResult<Option<()>>> + '_ {
        move || {
            seen.set(true);
            ready(result)
        }
    }

    fn history_fetch(
        seen: &Cell<Option<PostId>>,
        result: WebResult<PostId>,
    ) -> impl FnOnce(PostId) -> Ready<WebResult<PostId>> + '_ {
        move |post_id| {
            seen.set(Some(post_id));
            ready(result)
        }
    }

    #[tokio::test]
    async fn authenticated_history_fetches_the_resolved_route() {
        let reconciled = Cell::new(false);
        let fetched = Cell::new(None);
        let result = load_authenticated_history(
            Some(PostId::from(7)),
            history_reconcile(&reconciled, Ok(Some(()))),
            history_fetch(&fetched, Ok(PostId::from(7))),
        )
        .await;

        assert_eq!(result, Ok(Some(PostId::from(7))));
        assert!(reconciled.get());
        assert_eq!(fetched.get(), Some(PostId::from(7)));
    }

    #[tokio::test]
    async fn invalid_history_route_short_circuits_session_and_fetch() {
        let reconciled = Cell::new(false);
        let fetched = Cell::new(None);
        let result = load_authenticated_history(
            None::<PostId>,
            history_reconcile(&reconciled, Ok(Some(()))),
            history_fetch(&fetched, Ok(PostId::from(7))),
        )
        .await;

        assert_eq!(result, Ok(None));
        assert!(!reconciled.get());
        assert_eq!(fetched.get(), None);
    }

    #[tokio::test]
    async fn anonymous_history_route_requires_auth_without_fetching() {
        let reconciled = Cell::new(false);
        let fetched = Cell::new(None);
        let result = load_authenticated_history(
            Some(PostId::from(7)),
            history_reconcile(&reconciled, Ok(None)),
            history_fetch(&fetched, Ok(PostId::from(7))),
        )
        .await;

        assert_eq!(result, Ok(None));
        assert!(reconciled.get());
        assert_eq!(fetched.get(), None);
    }

    #[tokio::test]
    async fn history_resolution_propagates_reconcile_and_fetch_errors() {
        let reconciled = Cell::new(false);
        let fetched = Cell::new(None);
        let reconcile_error = load_authenticated_history(
            Some(PostId::from(7)),
            history_reconcile(&reconciled, Err(WebError::validation("session"))),
            history_fetch(&fetched, Ok(PostId::from(7))),
        )
        .await;
        assert_eq!(reconcile_error, Err(WebError::validation("session")));
        assert!(reconciled.get());
        assert_eq!(fetched.get(), None);

        let fetch_error = load_authenticated_history(
            Some(PostId::from(7)),
            history_reconcile(&reconciled, Ok(Some(()))),
            history_fetch(&fetched, Err(WebError::validation("history"))),
        )
        .await;
        assert_eq!(fetch_error, Err(WebError::validation("history")));
        assert_eq!(fetched.get(), Some(PostId::from(7)));
    }

    #[test]
    fn serializable_history_result_projects_to_each_paint_state() {
        assert_eq!(
            authenticated_history_state(false, Ok::<Option<i32>, WebError>(None)),
            AuthenticatedHistoryState::NotFound
        );
        assert_eq!(
            authenticated_history_state(true, Ok::<Option<i32>, WebError>(None)),
            AuthenticatedHistoryState::AuthRequired
        );
        assert_eq!(
            authenticated_history_state(true, Ok::<_, WebError>(Some(7))),
            AuthenticatedHistoryState::Ready(7)
        );
        assert_eq!(
            authenticated_history_state::<i32>(true, Err(WebError::validation("history"))),
            AuthenticatedHistoryState::Failed(WebError::validation("history"))
        );
    }
    #[test]
    fn history_ready_projection_preserves_all_non_ready_states() {
        assert_eq!(
            AuthenticatedHistoryState::<i32>::NotFound.map_ready(|value| value.to_string()),
            AuthenticatedHistoryState::NotFound
        );
        assert_eq!(
            AuthenticatedHistoryState::<i32>::AuthRequired.map_ready(|value| value.to_string()),
            AuthenticatedHistoryState::AuthRequired
        );
        assert_eq!(
            AuthenticatedHistoryState::Ready(7).map_ready(|value| value.to_string()),
            AuthenticatedHistoryState::Ready("7".to_owned())
        );
        assert_eq!(
            AuthenticatedHistoryState::<i32>::Failed(WebError::validation("history"))
                .map_ready(|value| value.to_string()),
            AuthenticatedHistoryState::Failed(WebError::validation("history"))
        );
    }

    #[test]
    fn revision_collections_project_populated_and_empty_snapshots_to_source_text() {
        let media = vec!["sha256:first".to_owned(), "https://media/second".to_owned()];
        let populated = revision_collection_displays(
            vec![RevisionHistoryTag {
                tag: parse_tag("rust"),
                display: parse_tag_label("Rust"),
            }],
            vec![
                RevisionHistoryAudience {
                    kind: "public".to_owned(),
                    audience_id: None,
                },
                RevisionHistoryAudience {
                    kind: "named".to_owned(),
                    audience_id: Some(common::ids::AudienceId::from(11)),
                },
            ],
            &media,
        );
        assert_eq!(
            populated,
            vec![
                HistoryCollectionDisplay {
                    heading: "Tags",
                    heading_id: "revision-tags-heading",
                    data_test: "history-tags",
                    value: "Rust (rust)".to_owned(),
                },
                HistoryCollectionDisplay {
                    heading: "Audiences",
                    heading_id: "revision-audiences-heading",
                    data_test: "history-audiences",
                    value: "public\nnamed (11)".to_owned(),
                },
                HistoryCollectionDisplay {
                    heading: "Media references",
                    heading_id: "revision-media-heading",
                    data_test: "history-media",
                    value: "sha256:first\nhttps://media/second".to_owned(),
                },
            ]
        );

        let empty = revision_collection_displays(Vec::new(), Vec::new(), &[]);
        assert_eq!(
            empty,
            vec![
                HistoryCollectionDisplay {
                    heading: "Tags",
                    heading_id: "revision-tags-heading",
                    data_test: "history-tags",
                    value: "No tags in this snapshot.".to_owned(),
                },
                HistoryCollectionDisplay {
                    heading: "Audiences",
                    heading_id: "revision-audiences-heading",
                    data_test: "history-audiences",
                    value: "No audiences in this snapshot.".to_owned(),
                },
                HistoryCollectionDisplay {
                    heading: "Media references",
                    heading_id: "revision-media-heading",
                    data_test: "history-media",
                    value: "No media references in this snapshot.".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn current_history_rows_project_labels_values_and_lifecycle_selector() {
        let at = parse_utc_instant("2026-08-27T12:00:00Z");
        for (lifecycle, label) in [
            (RevisionLifecycle::Draft, "Draft"),
            (RevisionLifecycle::Scheduled, "Scheduled"),
            (RevisionLifecycle::Published, "Published"),
            (RevisionLifecycle::Deleted, "Deleted"),
        ] {
            let rows = current_history_rows(CurrentPostHistory {
                post_id: PostId::from(7),
                title: None,
                slug: parse_slug("history-post"),
                format: common::render::PostFormat::Markdown,
                created_at: at,
                updated_at: at,
                published_at: None,
                deleted_at: None,
                lifecycle,
            });
            assert_eq!(
                rows,
                vec![
                    HistoryDisplayRow {
                        label: "Post ID",
                        value: "7".to_owned(),
                        data_test: None
                    },
                    HistoryDisplayRow {
                        label: "Title",
                        value: "No title".to_owned(),
                        data_test: None
                    },
                    HistoryDisplayRow {
                        label: "Slug",
                        value: "history-post".to_owned(),
                        data_test: None
                    },
                    HistoryDisplayRow {
                        label: "Format",
                        value: "markdown".to_owned(),
                        data_test: None
                    },
                    HistoryDisplayRow {
                        label: "Lifecycle",
                        value: label.to_owned(),
                        data_test: Some("history-current-lifecycle")
                    },
                    HistoryDisplayRow {
                        label: "Created",
                        value: at.to_string(),
                        data_test: None
                    },
                    HistoryDisplayRow {
                        label: "Updated",
                        value: at.to_string(),
                        data_test: None
                    },
                    HistoryDisplayRow {
                        label: "Published",
                        value: "Not set".to_owned(),
                        data_test: None
                    },
                    HistoryDisplayRow {
                        label: "Deleted",
                        value: "Not set".to_owned(),
                        data_test: None
                    },
                ],
                "the {label} projection stays a literal current-history contract"
            );
        }
    }

    #[test]
    fn current_history_rows_preserve_present_title_and_timestamps() {
        let at = parse_utc_instant("2026-08-27T12:00:00Z");
        let rows = current_history_rows(CurrentPostHistory {
            post_id: PostId::from(7),
            title: Some(common::test_support::parse_post_title("Current title")),
            slug: parse_slug("history-post"),
            format: common::render::PostFormat::Org,
            created_at: at,
            updated_at: at,
            published_at: Some(at),
            deleted_at: Some(at),
            lifecycle: RevisionLifecycle::Published,
        });
        assert_eq!(rows[1].value, "Current title");
        assert_eq!(rows[3].value, "org");
        assert_eq!(rows[7].value, at.to_string());
        assert_eq!(rows[8].value, at.to_string());
    }

    #[test]
    fn revision_history_rows_project_every_scalar_and_optional_value() {
        let at = parse_utc_instant("2026-08-27T12:00:00Z");
        let detail = RevisionHistoryDetail {
            revision_id: common::ids::RevisionId::from(8),
            post_id: PostId::from(7),
            title: Some(common::test_support::parse_post_title("Snapshot title")),
            slug: parse_slug("history-post"),
            body: parse_post_body("body"),
            format: common::render::PostFormat::Org,
            rendered_html: common::test_support::rendered_html("<p>body</p>"),
            summary: Some(common::test_support::parse_post_summary("Snapshot summary")),
            created_at: at,
            updated_at: at,
            published_at: Some(at),
            deleted_at: Some(at),
            captured_at: at,
            tags: Vec::new(),
            audiences: Vec::new(),
            media: Vec::new(),
        };
        assert_eq!(
            revision_history_rows(&detail),
            vec![
                HistoryDisplayRow {
                    label: "Revision ID",
                    value: "8".to_owned(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Post ID",
                    value: "7".to_owned(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Title",
                    value: "Snapshot title".to_owned(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Slug",
                    value: "history-post".to_owned(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Format",
                    value: "org".to_owned(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Summary",
                    value: "Snapshot summary".to_owned(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Created",
                    value: at.to_string(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Updated",
                    value: at.to_string(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Published",
                    value: at.to_string(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Deleted",
                    value: at.to_string(),
                    data_test: None
                },
                HistoryDisplayRow {
                    label: "Captured",
                    value: at.to_string(),
                    data_test: None
                },
            ]
        );

        let absent = RevisionHistoryDetail {
            title: None,
            summary: None,
            published_at: None,
            deleted_at: None,
            ..detail
        };
        let rows = revision_history_rows(&absent);
        assert_eq!(rows[2].value, "No title");
        assert_eq!(rows[5].value, "Not set");
        assert_eq!(rows[8].value, "Not set");
        assert_eq!(rows[9].value, "Not set");
    }

    // --- parent-callback plumbing ---

    /// Run `body` under a fresh reactive `Owner` (the `media::upload_state` /
    /// `forms::Field` convention), so `RwSignal`s and `Callback`s work host-side
    /// without a browser.
    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    fn history_page(
        revisions: Vec<RevisionHistoryMetadata>,
        next_cursor: Option<RevisionHistoryCursor>,
        has_more: bool,
    ) -> RevisionHistoryPage {
        RevisionHistoryPage {
            revisions,
            next_cursor,
            has_more,
        }
    }

    fn history_row(revision_id: i64) -> RevisionHistoryMetadata {
        RevisionHistoryMetadata {
            revision_id: common::ids::RevisionId::from(revision_id),
            post_id: PostId::from(7),
            title: None,
            slug: parse_slug("history-post"),
            captured_at: parse_utc_instant("2026-08-27T12:00:00Z"),
            snapshot_lifecycle: crate::posts::RevisionLifecycle::Draft,
            current_deleted: false,
        }
    }

    #[test]
    fn history_list_load_transition_appends_rows_and_adopts_paging_state() {
        with_owner(|| {
            let cursor = RevisionHistoryCursor {
                revision_id: common::ids::RevisionId::from(9),
            };
            let state = HistoryListState::new(history_page(Vec::new(), Some(cursor.clone()), true));
            state.load_error.set(Some("old failure".to_owned()));

            assert_eq!(state.begin_load_more(), Some(cursor));
            assert!(state.loading_more.get());
            assert_eq!(state.load_error.get(), None);
            assert_eq!(
                state.begin_load_more(),
                None,
                "a request is already in flight"
            );

            state.finish_load_more(Ok(history_page(vec![history_row(10)], None, false)));
            assert_eq!(state.rows.get().len(), 1);
            assert_eq!(state.cursor.get(), None);
            assert!(!state.has_more.get());
            assert!(!state.loading_more.get());
            assert_eq!(state.begin_load_more(), None, "there is no next cursor");
        });
    }

    #[test]
    fn history_list_failure_is_visible_and_reenables_retry() {
        with_owner(|| {
            let cursor = RevisionHistoryCursor {
                revision_id: common::ids::RevisionId::from(9),
            };
            let state = HistoryListState::new(history_page(Vec::new(), Some(cursor), true));
            assert!(state.begin_load_more().is_some());

            let error = WebError::validation("history unavailable");
            let message = error.to_string();
            state.finish_load_more(Err(error));

            assert_eq!(state.load_error.get(), Some(message));
            assert!(!state.loading_more.get());
            assert!(
                state.begin_load_more().is_some(),
                "the same cursor is retryable"
            );
        });
    }

    /// A real `Callback` that records having run into `fired`.
    ///
    /// **One helper, not a closure per test.** The cases that assert a callback did
    /// *not* fire build their sink through this same constructor, so "the signal is
    /// still false" means "this very callback — demonstrably capable of writing it,
    /// two tests up — was never run", not "nothing here could ever have written".
    fn recorder(fired: RwSignal<bool>) -> Callback<()> {
        Callback::new(move |()| fired.set(true))
    }

    #[test]
    fn notify_runs_a_supplied_callback() {
        with_owner(|| {
            let fired = RwSignal::new(false);
            notify(Some(recorder(fired)));
            assert!(fired.get(), "the callback must actually run");
        });
    }

    #[test]
    fn notify_without_a_callback_is_a_no_op() {
        with_owner(|| {
            let fired = RwSignal::new(false);
            // Build the callback a caller WOULD have passed, then pass none: the sink
            // is writable, so an unwritten sink is an observation, not a vacuum.
            let unsupplied = Some(recorder(fired));
            notify(None);
            assert!(!fired.get(), "no callback, nothing fired");
            notify(unsupplied);
            assert!(fired.get(), "and the sink was writable all along");
        });
    }

    #[test]
    fn the_preferred_callback_wins_when_both_are_supplied() {
        // `PostCard`'s unpublish arm: a caller that supplied `on_unpublish` must not
        // also get `on_mutate` — the two are distinct notifications (#592).
        with_owner(|| {
            let preferred = RwSignal::new(false);
            let shared = RwSignal::new(false);
            notify_with_fallback(Some(recorder(preferred)), Some(recorder(shared)));
            assert!(preferred.get(), "the preferred callback runs");
            assert!(!shared.get(), "and the fallback must not also run");
        });
    }

    #[test]
    fn the_shared_callback_runs_when_the_preferred_one_is_absent() {
        with_owner(|| {
            let shared = RwSignal::new(false);
            notify_with_fallback(None, Some(recorder(shared)));
            assert!(
                shared.get(),
                "a caller that supplied only the shared one is told"
            );
        });
    }

    #[test]
    fn a_caller_that_supplied_neither_callback_is_fine() {
        with_owner(|| {
            let never = RwSignal::new(false);
            let unsupplied = Some(recorder(never));
            notify_with_fallback(None, None);
            assert!(!never.get());
            notify_with_fallback(unsupplied, None);
            assert!(never.get(), "the sink was writable all along");
        });
    }

    #[test]
    fn audience_picker_loading_and_failed_states_cannot_submit() {
        let selection = AudienceSelection {
            base: common::visibility::AudienceBase::Subscribers,
            named: vec![common::ids::AudienceId::from(7)],
        };
        let loading = NamedAudienceState::resolve(None);
        let failed = NamedAudienceState::resolve(Some(Err(WebError::server_message("boom"))));

        assert_eq!(loading, NamedAudienceState::Loading);
        assert_eq!(failed, NamedAudienceState::Failed);
        assert_eq!(loading.selection_for_submit(&selection), None);
        assert_eq!(failed.selection_for_submit(&selection), None);
    }

    #[test]
    fn audience_picker_ready_empty_is_a_real_loaded_state() {
        let selection = AudienceSelection {
            base: common::visibility::AudienceBase::Subscribers,
            named: Vec::new(),
        };
        let state = NamedAudienceState::resolve(Some(Ok(Vec::new())));

        assert_eq!(state, NamedAudienceState::Ready(Vec::new()));
        assert_eq!(
            state.selection_for_submit(&selection),
            Some(&selection),
            "loaded-empty still authorizes the real base selection"
        );
    }

    #[test]
    fn audience_picker_ready_nonempty_preserves_named_selection() {
        let audience_id = common::ids::AudienceId::from(7);
        let audiences = vec![audiences::Summary {
            audience_id,
            name: "Confidants".parse().unwrap(),
        }];
        let selection = AudienceSelection {
            base: common::visibility::AudienceBase::Subscribers,
            named: vec![audience_id],
        };
        let state = NamedAudienceState::resolve(Some(Ok(audiences.clone())));

        assert_eq!(state, NamedAudienceState::Ready(audiences));
        assert_eq!(
            state.selection_for_submit(&selection),
            Some(&selection),
            "a successful load must not discard the named choice"
        );
    }
}
