//! Host-compiled, host-tested decision logic for the posts vertical's pages
//! (#306/#58, ADR-0083).
//!
//! [`ListingRoute`] is the single public Post-listing decision surface. It owns
//! typed validation, exact projector-seed adoption, endpoint selection, and
//! exhaustive route presentation data. Those are folds over resolved values, not
//! browser wiring, so they live here where host tests can assert them; only
//! `Effect`/`Resource` construction and spawning remain in the wasm component.
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

use common::feed::FeedSurface;
use common::pagination::PageSize;
use common::revision_history::{
    RevisionHistoryAudience, RevisionHistoryDetail, RevisionHistoryTag,
};
use common::root_relative_url::RootRelativeUrl;
use common::seed::{AuthoredPost, Page, PageCursor, PageSeed, PublicPresentation, RenderedPost};
use common::tag::Tag;
use common::theme::PublishedThemePresentation;
use common::username::Username;
use common::visibility::AudienceSelection;
use common::{MutationOutcome, ids::PostId, permalink_route::PermalinkRoute};

use crate::audiences;
use crate::error::{WebError, WebResult};
use crate::taglist::TagCtx;
use crate::timeline;

use crate::posts::{
    CreatedPost, CurrentPostHistory, RevisionHistoryCursor, RevisionHistoryMetadata,
    RevisionHistoryPage, RevisionLifecycle, SavedPost, UnpublishedPost,
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

/// The exhaustive public Post listing route. Route parsing stops at the `Username`
/// and `Tag` boundaries; this value preserves malformed segments as `None` so the
/// lifecycle can reject them before it constructs an endpoint request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListingRoute {
    /// `/~:username` — the user timeline.
    Profile(Option<Username>),
    /// `/tags/:tag` — the site-wide tag listing.
    SiteTag(Option<Tag>),
    /// `/~:username/tags/:tag` — the per-user tag listing.
    UserTag(Option<Username>, Option<Tag>),
}

impl ListingRoute {
    /// Validate the route before any asynchronous listing work begins.
    ///
    /// User-tag routes deliberately validate Username first, preserving the prior
    /// malformed-route error precedence.
    ///
    /// # Errors
    ///
    /// Returns a validation error for a malformed route segment, with Username
    /// taking precedence over Tag for a User-tag route.
    pub fn validate(&self) -> WebResult<ValidatedListingRoute> {
        match self {
            Self::Profile(username) => Ok(ValidatedListingRoute::Profile(
                username
                    .clone()
                    .ok_or_else(|| WebError::validation("Invalid username"))?,
            )),
            Self::SiteTag(tag) => Ok(ValidatedListingRoute::SiteTag(
                tag.clone()
                    .ok_or_else(|| WebError::validation("Invalid tag"))?,
            )),
            Self::UserTag(username, tag) => Ok(ValidatedListingRoute::UserTag(
                username
                    .clone()
                    .ok_or_else(|| WebError::validation("Invalid username"))?,
                tag.clone()
                    .ok_or_else(|| WebError::validation("Invalid tag"))?,
            )),
        }
    }

    /// The route-specific title.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::Profile(username) => format!(
                "Posts by {}",
                username
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string)
            ),
            Self::SiteTag(tag) | Self::UserTag(_, tag) => {
                format!(
                    "#{}",
                    tag.as_ref().map_or_else(String::new, ToString::to_string)
                )
            }
        }
    }

    /// The route-specific topbar subtitle.
    #[must_use]
    pub fn subtitle(&self) -> String {
        match self {
            Self::Profile(_) => "User timeline".to_owned(),
            Self::SiteTag(_) => "Posts on this instance".to_owned(),
            Self::UserTag(username, _) => format!(
                "Posts by ~{}",
                username
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string)
            ),
        }
    }

    /// The public discovery surface, absent for malformed route data.
    #[must_use]
    pub fn feed_surface(&self) -> Option<FeedSurface> {
        match self {
            Self::Profile(Some(username)) => Some(FeedSurface::User {
                username: username.clone(),
            }),
            Self::SiteTag(Some(tag)) => Some(FeedSurface::SiteTag { tag: tag.clone() }),
            Self::UserTag(Some(username), Some(tag)) => Some(FeedSurface::UserTag {
                username: username.clone(),
                tag: tag.clone(),
            }),
            _ => None,
        }
    }

    /// The profile that receives `AtomPub` RSD and subscription controls.
    #[must_use]
    pub fn user_chrome(&self) -> Option<Username> {
        match self {
            Self::Profile(Some(username)) => Some(username.clone()),
            _ => None,
        }
    }

    /// The tag context each rendered Post receives.
    #[must_use]
    pub fn tag_context(&self) -> Option<TagCtx> {
        match self {
            Self::Profile(Some(username)) | Self::UserTag(Some(username), _) => {
                Some(TagCtx::ForUser(username.clone()))
            }
            Self::Profile(None) | Self::UserTag(None, _) => None,
            Self::SiteTag(_) => Some(TagCtx::SiteWide),
        }
    }

    /// The listing's existing route-specific empty state.
    #[must_use]
    pub const fn empty_text(&self) -> &'static str {
        match self {
            Self::Profile(_) => "No posts yet.",
            Self::SiteTag(_) | Self::UserTag(_, _) => "No posts with this tag yet.",
        }
    }

    /// Adopt only a projector page whose kind and every typed route value match.
    #[must_use]
    pub fn seeded_page(&self, seed: Option<PageSeed>) -> Option<Page<RenderedPost>> {
        match (seed?, self) {
            (PageSeed::Profile { username, page }, Self::Profile(wanted))
                if wanted.as_ref() == Some(&username) =>
            {
                Some(page)
            }
            (PageSeed::SiteTag { tag, page }, Self::SiteTag(wanted))
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
                Self::UserTag(wanted_username, wanted_tag),
            ) if wanted_username.as_ref() == Some(&username)
                && wanted_tag.as_ref() == Some(&tag) =>
            {
                Some(page)
            }
            _ => None,
        }
    }

    /// Fetch the replacement first page after route validation.
    ///
    /// The lifecycle calls this only after synchronously advancing its generation;
    /// invalid values return before an endpoint future is constructed.
    ///
    /// # Errors
    ///
    /// Returns a validation error for malformed route data or propagates the
    /// selected public listing endpoint's failure.
    pub async fn destination(self) -> WebResult<(PublishedThemePresentation, Page<RenderedPost>)> {
        self.fetch_page(None, Some(PageSize::default()))
            .await
            .map(public_destination)
    }

    /// Fetch one route-specific page using the existing typed endpoint.
    ///
    /// # Errors
    ///
    /// Returns a validation error for malformed route data or propagates the
    /// selected public listing endpoint's failure.
    pub async fn fetch_page(
        self,
        cursor: Option<PageCursor>,
        limit: Option<PageSize>,
    ) -> WebResult<PublicPresentation<Page<RenderedPost>>> {
        match self.validate()? {
            // cov:ignore-start: constructing and awaiting the generated server-function client requires the hydrated browser transport unavailable to authoritative host coverage.
            ValidatedListingRoute::Profile(username) => {
                let request = timeline::list_by_user(username, cursor, limit);
                request.await
                // cov:ignore-stop
            }
            // cov:ignore-start: constructing and awaiting the generated server-function client requires the hydrated browser transport unavailable to authoritative host coverage.
            ValidatedListingRoute::SiteTag(tag) => {
                let request = timeline::list_by_tag(tag, cursor, limit);
                request.await
                // cov:ignore-stop
            }
            // cov:ignore-start: constructing and awaiting the generated server-function client requires the hydrated browser transport unavailable to authoritative host coverage.
            ValidatedListingRoute::UserTag(username, tag) => {
                let request = timeline::list_by_user_and_tag(username, tag, cursor, limit);
                request.await
                // cov:ignore-stop
            }
        }
    }
}

/// Validated endpoint selection for the public listing route matrix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidatedListingRoute {
    /// Typed profile endpoint arguments.
    Profile(Username),
    /// Typed site-tag endpoint arguments.
    SiteTag(Tag),
    /// Typed user-tag endpoint arguments.
    UserTag(Username, Tag),
}

/// Deconstructs one server-resolved public destination so wasm route wiring can
/// commit its theme and content together without recreating resolution rules.
#[must_use]
pub fn public_destination<Page>(
    presentation: PublicPresentation<Page>,
) -> (PublishedThemePresentation, Page) {
    (presentation.theme, presentation.page)
}

/// Validates a permalink route before awaiting its presentation fetch.
///
/// # Errors
///
/// Returns validation failures for malformed route values and propagates fetch
/// failures.
pub async fn permalink_destination<Fetch, FetchFuture>(
    route: Option<PermalinkRoute>,
    fetch: Fetch,
) -> WebResult<(PublishedThemePresentation, AuthoredPost)>
where
    Fetch: FnOnce(PermalinkRoute) -> FetchFuture,
    FetchFuture: Future<Output = WebResult<PublicPresentation<AuthoredPost>>>,
{
    let route = route.ok_or_else(|| WebError::validation("Invalid permalink"))?;
    fetch(route).await.map(public_destination)
}

/// Notifies post-create parents and reports whether success UI may reset.
///
/// A possibly committed published post must revalidate its parent timeline, but
/// only a confirmed post may drive success UI or reset the composer.
#[must_use]
pub fn notify_create_settlement(
    outcome: MutationOutcome<CreatedPost>,
    on_mutation: Option<Callback<bool>>,
    on_success: Callback<CreatedPost>,
) -> bool {
    let published = outcome.value().post.published_at.is_some();
    if let Some(on_mutation) = on_mutation {
        on_mutation.run(published);
    }
    let MutationOutcome::Confirmed(created) = outcome else {
        return false;
    };
    on_success.run(created);
    true
}
/// Where an update settles the browser, shaped for `on_settled_ok`'s read closure:
/// `Some(Ok(permalink))` only when the update **confirmed** publication.
///
/// Editor → permalink is always a route change, so a fresh `PostPage` mount refetches
/// — no explicit invalidation needed (#592). A confirmed-but-still-draft update, an
/// indeterminate or failed update, and "not settled yet" all mean *nothing to navigate
/// to*. In particular, an indeterminate commit must not masquerade as confirmed by
/// redirecting to a post whose publication status the server could not establish.
///
/// The permalink stays a [`RootRelativeUrl`] all the way to `use_navigate`, which takes
/// `&str` by deref — unwrapping it here would trade the type for an allocation.
#[must_use]
pub fn publish_redirect<E>(
    settled: Option<Result<MutationOutcome<SavedPost>, E>>,
) -> Option<Result<RootRelativeUrl, E>> {
    let MutationOutcome::Confirmed(updated) = settled?.ok()? else {
        return None;
    };
    updated
        .published_at
        .is_some()
        .then_some(updated.permalink)
        .map(Ok)
}

/// Refetch an unpublished Post when its canonical draft permalink is the current route.
///
/// A route change mounts a fresh `PostPage`, but a
/// same-date unpublish leaves the URL unchanged and therefore needs explicit resource
/// invalidation (#783). Keeping the comparison here makes that browser policy
/// host-testable without allocating or erasing the [`RootRelativeUrl`].
pub fn refetch_unpublished_post_if_needed(
    current_path: &str,
    destination: &RootRelativeUrl,
    refetch: impl FnOnce(),
) {
    if current_path == destination.as_ref() {
        refetch();
    }
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
/// Settle `PostCard`'s unpublish mutation using its callback precedence.
///
/// A confirmed result goes to the specific unpublish callback when present and
/// otherwise invalidates the shared listing. A commit-indeterminate result
/// always invalidates because no confirmed value exists for the specific
/// callback. A rollback-confirmed operation failure notifies neither callback.
pub fn settle_unpublish_mutation(
    settled: WebResult<MutationOutcome<SavedPost>>,
    on_unpublish: Option<Callback<SavedPost>>,
    on_mutate: Option<Callback<()>>,
) {
    match settled {
        Ok(MutationOutcome::Confirmed(unpublished)) => match on_unpublish {
            Some(on_unpublish) => on_unpublish.run(unpublished),
            None => notify(on_mutate),
        },
        Ok(MutationOutcome::CommitIndeterminate(_)) => notify(on_mutate),
        Err(_) => {}
    }
}

/// Settle a public listing mutation and return its confirmed value, if any.
///
/// Confirmed and commit-indeterminate outcomes notify because either may have
/// changed the authoritative first page. Only a confirmed outcome yields a value
/// for the local delete or navigation effect; operation failures yield neither.
#[must_use]
pub fn settle_listing_mutation<T>(
    settled: WebResult<MutationOutcome<T>>,
    callback: Option<Callback<()>>,
) -> Option<T> {
    match settled {
        Ok(MutationOutcome::Confirmed(value)) => {
            notify(callback);
            Some(value)
        }
        Ok(MutationOutcome::CommitIndeterminate(_)) => {
            notify(callback);
            None
        }
        Err(_) => None,
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

/// A claimed next-page request for the drafts list.
///
/// The generation binds a completion to the first page it extended. A first-page
/// revalidation advances that generation, so an older request cannot append rows
/// whose membership was computed before a Publish or Delete changed the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftLoadMoreClaim {
    pub cursor: PageCursor,
    generation: u64,
}

/// The complete visible state of the incremental drafts region.
///
/// The first-page failure stays on [`WebResult`]'s error axis. A failed next-page
/// request remains data because the already-painted rows stay usable and retryable.
/// Keeping the error and control in one closed enum prevents the renderer from
/// receiving contradictory combinations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftLoadMorePaint {
    /// The server reported the terminal page.
    Hidden,
    /// Another page is available and may be requested.
    Ready,
    /// The sole next-page request is in flight.
    Loading,
    /// The request failed; retained rows and cursor remain available for retry.
    Failed(WebError),
}

/// The drafts-list paint decision, excluding initial-page failure.
///
/// Initial failure travels on [`WebResult`]'s error axis so this enum names only
/// successful paints (ADR-0083).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftListPaint {
    Empty,
    Rows {
        drafts: Vec<UnpublishedPost>,
        load_more: DraftLoadMorePaint,
    },
}

/// Reactive state and transition logic for the author's cursor-paginated drafts.
///
/// The wasm component owns dispatch only; this host-tested bundle owns the
/// guarded claim, append, retry, and revalidation transitions.
#[derive(Clone, Copy, Default)]
pub struct DraftListState {
    /// All unpublished-post rows loaded from the current first page onward.
    pub rows: RwSignal<Vec<UnpublishedPost>>,
    /// Opaque server cursor for the next request.
    pub cursor: RwSignal<Option<PageCursor>>,
    /// Whether the current terminal page exposes another cursor page.
    pub has_more: RwSignal<bool>,
    /// Whether a claimed next-page request has yet to settle.
    pub loading_more: RwSignal<bool>,
    /// The typed next-page failure, retained until its retry begins.
    pub load_error: RwSignal<Option<WebError>>,
    /// The typed failure from the current first-page resource.
    initial_error: RwSignal<Option<WebError>>,
    generation: RwSignal<u64>,
}

impl DraftListState {
    /// Adopt a first-page resource result.
    ///
    /// A resource failure belongs on [`WebResult`]'s error axis when [`Self::paint`]
    /// runs. A later successful first page both clears that failure and establishes a
    /// new generation, invalidating any next-page completion from the old list.
    pub fn adopt(self, result: WebResult<Page<UnpublishedPost>>) {
        let Ok(page) = result else {
            self.initial_error.set(result.err());
            return;
        };

        self.initial_error.set(None);
        self.generation.update(|generation| *generation += 1);
        self.rows.set(page.posts);
        self.cursor.set(page.next_cursor);
        self.has_more.set(page.has_more);
        self.loading_more.set(false);
        self.load_error.set(None);
    }

    /// Claim the sole next-page slot, returning its opaque cursor for dispatch.
    #[must_use]
    pub fn begin_load_more(self) -> Option<DraftLoadMoreClaim> {
        if self.loading_more.get_untracked() || !self.has_more.get_untracked() {
            return None;
        }
        let cursor = self.cursor.get_untracked()?;
        self.loading_more.set(true);
        self.load_error.set(None);
        Some(DraftLoadMoreClaim {
            cursor,
            generation: self.generation.get_untracked(),
        })
    }

    /// Settle a claimed next page, ignoring a completion invalidated by revalidation.
    pub fn finish_load_more(
        self,
        claim: DraftLoadMoreClaim,
        result: WebResult<Page<UnpublishedPost>>,
    ) {
        if claim.generation != self.generation.get_untracked() {
            return;
        }

        match result {
            Ok(page) => {
                self.rows.update(|rows| rows.extend(page.posts));
                self.cursor.set(page.next_cursor);
                self.has_more.set(page.has_more);
            }
            Err(error) => self.load_error.set(Some(error)),
        }
        self.loading_more.set(false);
    }

    /// Fold the resolved resource and pagination signals into the current paint.
    ///
    /// An initial fetch failure outranks all successful shapes and travels on
    /// [`WebResult`]'s error axis. A load-more failure instead stays in the typed
    /// success data because the existing rows deliberately remain visible and retryable.
    ///
    /// # Errors
    ///
    /// Returns the typed error from the current first-page resource.
    pub fn paint(self) -> WebResult<DraftListPaint> {
        if let Some(error) = self.initial_error.get() {
            return Err(error);
        }

        let drafts = self.rows.get();
        if drafts.is_empty() {
            return Ok(DraftListPaint::Empty);
        }

        let load_more = if !self.has_more.get() {
            DraftLoadMorePaint::Hidden
        } else if self.loading_more.get() {
            DraftLoadMorePaint::Loading
        } else {
            match self.load_error.get() {
                Some(error) => DraftLoadMorePaint::Failed(error),
                None => DraftLoadMorePaint::Ready,
            }
        };
        Ok(DraftListPaint::Rows { drafts, load_more })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::{Ready, ready};

    use super::*;
    use common::ids::PostId;
    use common::test_support::{
        parse_post_body, parse_root_relative_url, parse_slug, parse_tag, parse_tag_label,
        parse_username, parse_utc_instant,
    };
    use common::theme::Theme;
    use common::time::UtcInstant;

    fn theme(theme: Theme) -> PublishedThemePresentation {
        PublishedThemePresentation::built_in(theme)
    }

    fn page(has_more: bool) -> Page<RenderedPost> {
        Page {
            posts: Vec::new(),
            next_cursor: None,
            has_more,
        }
    }

    fn draft_cursor(post_id: i64) -> PageCursor {
        PageCursor {
            created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            post_id: PostId::from(post_id),
        }
    }

    fn draft(post_id: i64) -> UnpublishedPost {
        UnpublishedPost {
            post: SavedPost {
                post_id: PostId::from(post_id),
                slug: format!("draft-{post_id}").parse().unwrap(),
                published_at: None,
                permalink: parse_root_relative_url(&format!("/~alice/draft-{post_id}")),
            },
            title: None,
            summary_label: format!("Draft {post_id}").parse().unwrap(),
            edit_url: parse_root_relative_url(&format!("/posts/{post_id}/edit")),
        }
    }

    fn draft_page(
        posts: Vec<UnpublishedPost>,
        next_cursor: Option<PageCursor>,
        has_more: bool,
    ) -> Page<UnpublishedPost> {
        Page {
            posts,
            next_cursor,
            has_more,
        }
    }

    fn alice() -> Username {
        parse_username("alice")
    }

    fn rust() -> Tag {
        parse_tag("rust")
    }

    #[test]
    fn public_destination_preserves_route_theme_and_page() {
        let (presentation, page) = public_destination(PublicPresentation {
            theme: theme(Theme::Reader),
            page: page(true),
        });

        assert_eq!(presentation, theme(Theme::Reader));
        assert!(page.has_more);
    }

    #[test]
    fn listing_route_validation_selects_the_typed_route_matrix() {
        assert_eq!(
            ListingRoute::Profile(Some(alice())).validate(),
            Ok(ValidatedListingRoute::Profile(alice()))
        );
        assert_eq!(
            ListingRoute::SiteTag(Some(rust())).validate(),
            Ok(ValidatedListingRoute::SiteTag(rust()))
        );
        assert_eq!(
            ListingRoute::UserTag(Some(alice()), Some(rust())).validate(),
            Ok(ValidatedListingRoute::UserTag(alice(), rust()))
        );
    }

    #[test]
    fn user_tag_validation_reports_username_before_tag() {
        let error = ListingRoute::UserTag(None, None)
            .validate()
            .expect_err("both malformed values reject the route");

        assert_eq!(error, WebError::validation("Invalid username"));
    }

    #[test]
    fn listing_route_presentation_preserves_each_public_surface() {
        let profile = ListingRoute::Profile(Some(alice()));
        assert_eq!(profile.title(), "Posts by alice");
        assert_eq!(profile.subtitle(), "User timeline");
        assert_eq!(
            profile.feed_surface(),
            Some(FeedSurface::User { username: alice() })
        );
        assert_eq!(profile.user_chrome(), Some(alice()));
        assert_eq!(profile.tag_context(), Some(TagCtx::ForUser(alice())));
        assert_eq!(profile.empty_text(), "No posts yet.");

        let site_tag = ListingRoute::SiteTag(Some(rust()));
        assert_eq!(site_tag.title(), "#rust");
        assert_eq!(site_tag.subtitle(), "Posts on this instance");
        assert_eq!(
            site_tag.feed_surface(),
            Some(FeedSurface::SiteTag { tag: rust() })
        );
        assert_eq!(site_tag.user_chrome(), None);
        assert_eq!(site_tag.tag_context(), Some(TagCtx::SiteWide));
        assert_eq!(site_tag.empty_text(), "No posts with this tag yet.");

        let user_tag = ListingRoute::UserTag(Some(alice()), Some(rust()));
        assert_eq!(user_tag.title(), "#rust");
        assert_eq!(user_tag.subtitle(), "Posts by ~alice");
        assert_eq!(
            user_tag.feed_surface(),
            Some(FeedSurface::UserTag {
                username: alice(),
                tag: rust(),
            })
        );
        assert_eq!(user_tag.user_chrome(), None);
        assert_eq!(user_tag.tag_context(), Some(TagCtx::ForUser(alice())));
        assert_eq!(user_tag.empty_text(), "No posts with this tag yet.");
    }

    #[test]
    fn malformed_listing_presentation_never_invents_discovery_context() {
        let profile = ListingRoute::Profile(None);
        assert_eq!(profile.title(), "Posts by ");
        assert_eq!(profile.feed_surface(), None);
        assert_eq!(profile.user_chrome(), None);
        assert_eq!(profile.tag_context(), None);

        let site_tag = ListingRoute::SiteTag(None);
        assert_eq!(site_tag.title(), "#");
        assert_eq!(site_tag.feed_surface(), None);

        let user_tag = ListingRoute::UserTag(None, None);
        assert_eq!(user_tag.subtitle(), "Posts by ~");
        assert_eq!(user_tag.feed_surface(), None);
        assert_eq!(user_tag.tag_context(), None);
    }
    #[tokio::test]
    async fn malformed_listing_routes_fail_before_constructing_a_client_request() {
        assert_eq!(
            ListingRoute::Profile(None).fetch_page(None, None).await,
            Err(WebError::validation("Invalid username"))
        );
        assert_eq!(
            ListingRoute::SiteTag(None).destination().await,
            Err(WebError::validation("Invalid tag"))
        );
    }
    #[tokio::test]
    async fn permalink_destination_fetches_a_validated_route() {
        let route =
            PermalinkRoute::parse("alice", "2026", "01", "02", "hello").expect("valid permalink");
        let expected_post = crate::posts::render::test_fixtures::sample_post();
        assert_eq!(
            permalink_destination(Some(route.clone()), |actual| async move {
                assert_eq!(actual, route);
                Ok(PublicPresentation {
                    theme: theme(Theme::Reader),
                    page: expected_post.clone(),
                })
            })
            .await,
            Ok((
                theme(Theme::Reader),
                crate::posts::render::test_fixtures::sample_post()
            ))
        );
    }

    // --- seed adoption ---

    #[test]
    fn listing_seed_adoption_requires_exact_kind_and_route_values() {
        let profile = ListingRoute::Profile(Some(alice()));
        assert!(
            profile
                .seeded_page(Some(PageSeed::Profile {
                    username: alice(),
                    page: page(true),
                }))
                .is_some()
        );
        assert!(
            profile
                .seeded_page(Some(PageSeed::Profile {
                    username: parse_username("bob"),
                    page: page(true),
                }))
                .is_none()
        );
        assert!(
            profile
                .seeded_page(Some(PageSeed::SiteTimeline(page(true))))
                .is_none()
        );
        assert!(profile.seeded_page(None).is_none());

        let user_tag = ListingRoute::UserTag(Some(alice()), Some(rust()));
        assert!(
            user_tag
                .seeded_page(Some(PageSeed::UserTag {
                    username: alice(),
                    tag: rust(),
                    page: page(true),
                }))
                .is_some()
        );
        assert!(
            user_tag
                .seeded_page(Some(PageSeed::UserTag {
                    username: alice(),
                    tag: parse_tag("leptos"),
                    page: page(true),
                }))
                .is_none()
        );
    }

    #[test]
    fn site_tag_seed_adoption_requires_the_exact_tag() {
        let route = ListingRoute::SiteTag(Some(rust()));
        assert!(
            route
                .seeded_page(Some(PageSeed::SiteTag {
                    tag: rust(),
                    page: page(true),
                }))
                .is_some()
        );
        assert!(
            route
                .seeded_page(Some(PageSeed::SiteTag {
                    tag: parse_tag("leptos"),
                    page: page(true),
                }))
                .is_none()
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
    fn created_post(published_at: Option<UtcInstant>) -> CreatedPost {
        CreatedPost {
            post: saved_post(published_at),
            publication: if published_at.is_some() {
                crate::posts::CreatePublication::Published
            } else {
                crate::posts::CreatePublication::Draft
            },
        }
    }

    #[test]
    fn create_settlement_classifies_published_and_draft_outcomes() {
        Owner::new().with(|| {
            let publications = RwSignal::new(Vec::new());
            let success_count = RwSignal::new(0_u8);
            let on_mutation = Callback::new(move |published| {
                publications.update(|values| values.push(published));
            });
            let on_success =
                Callback::new(move |_created| success_count.update(|count| *count += 1));
            let published_at = "2026-01-02T00:00:00Z".parse().expect("a real instant");

            assert!(notify_create_settlement(
                MutationOutcome::Confirmed(created_post(Some(published_at))),
                Some(on_mutation),
                on_success,
            ));
            assert!(!notify_create_settlement(
                MutationOutcome::CommitIndeterminate(created_post(Some(published_at))),
                Some(on_mutation),
                on_success,
            ));
            assert!(notify_create_settlement(
                MutationOutcome::Confirmed(created_post(None)),
                Some(on_mutation),
                on_success,
            ));
            assert!(!notify_create_settlement(
                MutationOutcome::CommitIndeterminate(created_post(None)),
                Some(on_mutation),
                on_success,
            ));
            assert_eq!(publications.get_untracked(), [true, true, false, false]);
            assert_eq!(success_count.get_untracked(), 2);
        });
    }

    #[test]
    fn a_published_update_redirects_to_its_typed_permalink() {
        let outcome = MutationOutcome::Confirmed(saved_post(Some(
            "2026-01-02T00:00:00Z".parse().expect("a real instant"),
        )));
        assert_eq!(
            publish_redirect::<WebError>(Some(Ok(outcome))).expect("a published update navigates"),
            Ok(parse_root_relative_url("/~alice/2026/01/02/hello")),
        );
    }

    #[test]
    fn a_still_unpublished_update_stays_put() {
        // The editor must not navigate away when the author saved a draft — the
        // invariant the `published_at.is_some()` gate exists to keep.
        let outcome = MutationOutcome::Confirmed(saved_post(None));
        assert_eq!(publish_redirect::<WebError>(Some(Ok(outcome))), None);
    }

    #[test]
    fn an_unsettled_failed_or_indeterminate_update_navigates_nowhere() {
        assert_eq!(publish_redirect::<WebError>(None), None);
        assert_eq!(
            publish_redirect(Some(Err(WebError::validation("boom")))),
            None
        );
        assert_eq!(
            publish_redirect::<WebError>(Some(Ok(MutationOutcome::CommitIndeterminate(
                saved_post(Some(
                    "2026-01-02T00:00:00Z".parse().expect("a real instant"),
                )),
            )))),
            None
        );
    }

    #[test]
    fn unpublish_refetches_only_when_the_permalink_stays_the_same() {
        let refetches = Cell::new(0);
        let refetch = || {
            refetches.set(refetches.get() + 1);
        };
        let same_permalink = parse_root_relative_url("/~alice/2026/01/02/hello");
        refetch_unpublished_post_if_needed("/~alice/2026/01/02/hello", &same_permalink, refetch);
        let moved_permalink = parse_root_relative_url("/~alice/2025/12/31/hello");
        refetch_unpublished_post_if_needed("/~alice/2026/01/02/hello", &moved_permalink, refetch);
        assert_eq!(refetches.get(), 1);
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
        Owner::new().with(|| {
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
        Owner::new().with(|| {
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
        Owner::new().with(|| {
            let fired = RwSignal::new(false);
            notify(Some(recorder(fired)));
            assert!(fired.get(), "the callback must actually run");
        });
    }
    #[test]
    fn listing_mutation_settlement_matches_the_post_outcome_matrix() {
        Owner::new().with(|| {
            let confirmed = RwSignal::new(false);
            let indeterminate = RwSignal::new(false);
            let rolled_back = RwSignal::new(false);

            assert_eq!(
                settle_listing_mutation(
                    Ok(MutationOutcome::Confirmed(7_u8)),
                    Some(recorder(confirmed)),
                ),
                Some(7)
            );
            assert_eq!(
                settle_listing_mutation(
                    Ok(MutationOutcome::CommitIndeterminate(8_u8)),
                    Some(recorder(indeterminate)),
                ),
                None
            );
            assert_eq!(
                settle_listing_mutation(
                    Err::<MutationOutcome<u8>, _>(WebError::validation("operation failed")),
                    Some(recorder(rolled_back)),
                ),
                None
            );

            assert!(confirmed.get(), "a confirmed Post mutation revalidates");
            assert!(
                indeterminate.get(),
                "a commit-indeterminate Post mutation may have committed and revalidates"
            );
            assert!(
                !rolled_back.get(),
                "a rollback-confirmed operation failure does not revalidate"
            );
        });
    }

    #[test]
    fn unpublish_settlement_preserves_specific_callback_precedence() {
        Owner::new().with(|| {
            let preferred = RwSignal::new(false);
            let shared_for_preferred = RwSignal::new(false);
            settle_unpublish_mutation(
                Ok(MutationOutcome::Confirmed(saved_post(None))),
                Some(Callback::new(move |_post| preferred.set(true))),
                Some(recorder(shared_for_preferred)),
            );
            assert!(preferred.get());
            assert!(!shared_for_preferred.get());

            let shared_confirmed = RwSignal::new(false);
            settle_unpublish_mutation(
                Ok(MutationOutcome::Confirmed(saved_post(None))),
                None,
                Some(recorder(shared_confirmed)),
            );
            assert!(shared_confirmed.get());

            let shared_indeterminate = RwSignal::new(false);
            settle_unpublish_mutation(
                Ok(MutationOutcome::CommitIndeterminate(saved_post(None))),
                Some(Callback::new(|_post| panic!("no confirmed value exists"))),
                Some(recorder(shared_indeterminate)),
            );
            assert!(shared_indeterminate.get());

            let preferred_rolled_back = RwSignal::new(false);
            let shared_rolled_back = RwSignal::new(false);
            settle_unpublish_mutation(
                Err(WebError::validation("operation failed")),
                Some(Callback::new(move |_post| preferred_rolled_back.set(true))),
                Some(recorder(shared_rolled_back)),
            );
            assert!(!preferred_rolled_back.get());
            assert!(!shared_rolled_back.get());
        });
    }

    #[test]
    fn notify_without_a_callback_is_a_no_op() {
        Owner::new().with(|| {
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
        Owner::new().with(|| {
            let preferred = RwSignal::new(false);
            let shared = RwSignal::new(false);
            notify_with_fallback(Some(recorder(preferred)), Some(recorder(shared)));
            assert!(preferred.get(), "the preferred callback runs");
            assert!(!shared.get(), "and the fallback must not also run");
        });
    }

    #[test]
    fn the_shared_callback_runs_when_the_preferred_one_is_absent() {
        Owner::new().with(|| {
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
        Owner::new().with(|| {
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
    #[test]
    fn draft_list_adopts_the_first_page() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));

            assert_eq!(state.rows.get(), vec![draft(1)]);
            assert_eq!(state.cursor.get(), Some(draft_cursor(1)));
            assert!(state.has_more.get());
            assert!(!state.loading_more.get());
            assert_eq!(state.load_error.get(), None);
        });
    }

    #[test]
    fn draft_list_claims_only_one_available_next_page() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            assert_eq!(state.begin_load_more(), None, "no first-page cursor");

            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));
            let claim = state.begin_load_more().expect("the first next page");
            assert_eq!(claim.cursor, draft_cursor(1));
            assert!(state.loading_more.get());
            assert_eq!(state.begin_load_more(), None, "one in-flight request");
        });
    }

    #[test]
    fn draft_list_appends_in_order_and_removes_the_terminal_control() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));
            let claim = state.begin_load_more().expect("next page");

            state.finish_load_more(claim, Ok(draft_page(vec![draft(2)], None, false)));

            assert_eq!(state.rows.get(), vec![draft(1), draft(2)]);
            assert_eq!(state.cursor.get(), None);
            assert!(!state.has_more.get());
            assert!(!state.loading_more.get());
            assert_eq!(state.begin_load_more(), None, "terminal page cannot fetch");
        });
    }

    #[test]
    fn draft_list_preserves_rows_and_cursor_after_a_typed_error_for_retry() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));
            let first_claim = state.begin_load_more().expect("next page");
            let error = WebError::validation("next page failed");

            state.finish_load_more(first_claim, Err(error.clone()));

            assert_eq!(state.rows.get(), vec![draft(1)]);
            assert_eq!(state.cursor.get(), Some(draft_cursor(1)));
            assert_eq!(state.load_error.get(), Some(error));
            let retry = state.begin_load_more().expect("the cursor stays retryable");
            assert_eq!(retry.cursor, draft_cursor(1));
            assert_eq!(state.load_error.get(), None, "retry clears the old error");
        });
    }

    #[test]
    fn draft_list_rejects_a_stale_next_page_after_first_page_revalidation() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));
            let stale_claim = state.begin_load_more().expect("next page");

            state.adopt(Ok(draft_page(vec![draft(3)], None, false)));
            state.finish_load_more(stale_claim, Ok(draft_page(vec![draft(2)], None, false)));

            assert_eq!(state.rows.get(), vec![draft(3)]);
            assert_eq!(state.cursor.get(), None);
            assert!(!state.has_more.get());
            assert!(!state.loading_more.get());
        });
    }

    #[test]
    fn draft_list_paint_keeps_initial_page_failure_on_the_error_axis() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            let error = WebError::validation("first page failed");

            state.adopt(Err(error.clone()));

            assert_eq!(state.paint(), Err(error));
        });
    }

    #[test]
    fn draft_list_paint_distinguishes_an_empty_first_page() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(Vec::new(), None, false)));

            assert_eq!(state.paint(), Ok(DraftListPaint::Empty));
        });
    }

    #[test]
    fn draft_list_paint_exposes_rows_and_a_ready_load_more_control() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));

            assert_eq!(
                state.paint(),
                Ok(DraftListPaint::Rows {
                    drafts: vec![draft(1)],
                    load_more: DraftLoadMorePaint::Ready,
                })
            );
        });
    }

    #[test]
    fn draft_list_paint_exposes_the_loading_load_more_control() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));
            let _claim = state.begin_load_more().expect("next page");

            assert_eq!(
                state.paint(),
                Ok(DraftListPaint::Rows {
                    drafts: vec![draft(1)],
                    load_more: DraftLoadMorePaint::Loading,
                })
            );
        });
    }

    #[test]
    fn draft_list_paint_keeps_rows_with_a_typed_inline_load_more_error() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], Some(draft_cursor(1)), true)));
            let claim = state.begin_load_more().expect("next page");
            let error = WebError::validation("next page failed");
            state.finish_load_more(claim, Err(error.clone()));

            assert_eq!(
                state.paint(),
                Ok(DraftListPaint::Rows {
                    drafts: vec![draft(1)],
                    load_more: DraftLoadMorePaint::Failed(error),
                })
            );
        });
    }

    #[test]
    fn draft_list_paint_hides_load_more_after_the_terminal_page() {
        Owner::new().with(|| {
            let state = DraftListState::default();
            state.adopt(Ok(draft_page(vec![draft(1)], None, false)));

            assert_eq!(
                state.paint(),
                Ok(DraftListPaint::Rows {
                    drafts: vec![draft(1)],
                    load_more: DraftLoadMorePaint::Hidden,
                })
            );
        });
    }
}
