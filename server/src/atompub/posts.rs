//! `AtomPub` posts collection read/delete/create/update handlers.

use std::{collections::BTreeSet, sync::Arc};

use axum::Extension;
use axum::extract::rejection::ExtensionRejection;
use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use common::etag::ETag;
use common::idempotency_key::IdempotencyKey;
use common::ids::PostId;
use common::org::{self, OrgOperation, OrgStructuredMetadata, Presence, PublicationState};
use common::pagination::PageSize;
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::tag::{Tag, TagLabel};
use common::tagged_url::{self, BaseUrl, EditUriUrl, FeedUrl, PaginationUrl};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::{AudienceTarget, ViewerIdentity};
use host::atompub::{self, CollectionFeedTitle, Entry, FeedMeta};
use host::{etag, feed};
use storage::{
    AudienceStorage, CollectionCursor, FeedEventError, FeedEventStorage, InvalidAudienceTargets,
    MediaContentLocks, PostRecord, PostStorage, SiteConfigStorage, UserConfigStorage, WriteScope,
    WriteScopeError,
};
use web::auth;

use super::HandlerError;
use super::mapping::{self, PostFields};

const FEED_CONTENT_TYPE: &str = "application/atom+xml;type=feed;charset=utf-8";
const ENTRY_CONTENT_TYPE: &str = "application/atom+xml;type=entry;charset=utf-8";
/// `AtomPub`'s own default page size (its policy, distinct from the web default of 50);
/// `PageSize::clamped` makes it a compile-time-checked in-range constant. The `1..=50` bound
/// itself lives in `PageSize`.
const DEFAULT_PAGE_SIZE: PageSize = PageSize::clamped(25);

/// The storage dependencies the post handlers share, bundled into one extractor
/// so a handler stays under the argument limit without suppressing the lint.
/// Each field is pulled from the request `Extension`s the app router layers.
pub struct PostServices {
    posts: Arc<dyn PostStorage>,
    audiences: Arc<dyn AudienceStorage>,
    user_config: Arc<dyn UserConfigStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    content_locks: Arc<MediaContentLocks>,
}

impl<S: Send + Sync> FromRequestParts<S> for PostServices {
    type Rejection = ExtensionRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self {
            posts: Extension::<Arc<dyn PostStorage>>::from_request_parts(parts, state)
                .await?
                .0,
            audiences: Extension::<Arc<dyn AudienceStorage>>::from_request_parts(parts, state)
                .await?
                .0,
            user_config: Extension::<Arc<dyn UserConfigStorage>>::from_request_parts(parts, state)
                .await?
                .0,
            site_config: Extension::<Arc<dyn SiteConfigStorage>>::from_request_parts(parts, state)
                .await?
                .0,
            content_locks: Extension::<Arc<MediaContentLocks>>::from_request_parts(parts, state)
                .await?
                .0,
        })
    }
}

impl PostServices {
    /// Clones the post-storage capability for an independent handler operation.
    #[must_use]
    pub fn posts(&self) -> Arc<dyn PostStorage> {
        Arc::clone(&self.posts)
    }

    /// Borrows the named-audience store for author-scoped target authorization.
    #[must_use]
    pub fn audiences(&self) -> &dyn AudienceStorage {
        self.audiences.as_ref()
    }

    /// Borrows the per-user configuration store for one handler operation.
    #[must_use]
    pub fn user_config(&self) -> &dyn UserConfigStorage {
        self.user_config.as_ref()
    }

    /// Borrows the site configuration store for one handler operation.
    #[must_use]
    pub fn site_config(&self) -> &dyn SiteConfigStorage {
        self.site_config.as_ref()
    }

    /// Borrows the media filesystem coordinator for Post writes.
    #[must_use]
    pub fn content_locks(&self) -> &MediaContentLocks {
        self.content_locks.as_ref()
    }
}

/// A strong, content-hash `ETag` for a post's mutable representation. The
/// transport-neutral projection is owned by `common`; this storage adapter only
/// projects ordered post-tag labels into it.
pub(crate) fn etag_for(post: &PostRecord) -> ETag {
    etag::post_content_etag(
        post.title.as_ref(),
        &post.body,
        &post.format,
        post.summary.as_ref(),
        post.tags.iter().map(|tag| &tag.tag_display),
        post.published_at.is_none(),
    )
}

/// Whether a request's `If-Match` precondition is satisfied for a post with ETAG.
/// An absent (or non-UTF-8) header is unconditional; `*` matches any current
/// representation; otherwise the value must equal ETAG. Shared by PUT and DELETE.
fn if_match_satisfied(headers: &HeaderMap, etag: &ETag) -> bool {
    match headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) {
        // `ETag: PartialEq<&str>` (the reverse `str: PartialEq<ETag>` isn't derived).
        Some(if_match) => if_match == "*" || *etag == if_match,
        None => true,
    }
}

/// Parses the optional retry key while preserving `AtomPub`'s compatibility policy:
/// unreadable or blank headers do not opt the request into deduplication.
fn idempotency_key_from_headers(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok()?.parse().ok())
}

fn scalar_presence<T: Clone>(value: Option<&T>) -> Presence<T> {
    value.cloned().map_or(Presence::Absent, Presence::Present)
}

fn validated_categories(
    categories: Presence<Vec<TagLabel>>,
) -> Result<Presence<Vec<TagLabel>>, HandlerError> {
    match categories {
        Presence::Absent => Ok(Presence::Absent),
        Presence::Present(categories) => Ok(Presence::Present(
            common::tag::parse_and_validate_tags(categories)?,
        )),
    }
}

async fn authorize_audiences(
    audiences: &dyn AudienceStorage,
    author_user_id: common::ids::UserId,
    targets: Presence<Vec<AudienceTarget>>,
) -> Result<Presence<Vec<AudienceTarget>>, HandlerError> {
    let Presence::Present(targets) = targets else {
        return Ok(Presence::Absent);
    };
    storage::validate_named_audience_targets(audiences, author_user_id, &targets)
        .await
        .map_err(|error| match error {
            InvalidAudienceTargets::Invalid => HandlerError::BadRequest,
            InvalidAudienceTargets::Storage(error) => HandlerError::from(error),
        })?;
    Ok(Presence::Present(targets))
}

/// Atom entry fields after format-specific normalization and shared validation.
struct NormalizedAtomInput {
    body: PostBody,
    title: Option<PostTitle>,
    summary: Option<PostSummary>,
    categories: Vec<TagLabel>,
    lifecycle: Presence<PublicationState>,
    audiences: Presence<Vec<AudienceTarget>>,
    expectations: storage::PostBookkeepingExpectation,
}

/// Normalizes an incoming entry's format-dependent metadata and validates its
/// common storage fields before the create/update handler applies its fallback policy.
async fn normalize_atom_input(
    fields: PostFields,
    operation: OrgOperation,
    request_clock: UtcInstant,
    audiences: &dyn AudienceStorage,
    author_user_id: common::ids::UserId,
) -> Result<NormalizedAtomInput, HandlerError> {
    if fields.format != storage::PostFormat::Org {
        let categories = match validated_categories(fields.categories)? {
            Presence::Present(tags) => tags,
            Presence::Absent => Vec::new(),
        };
        return Ok(NormalizedAtomInput {
            body: fields.body,
            title: fields.title,
            summary: fields.summary,
            categories,
            lifecycle: fields.lifecycle,
            audiences: Presence::Absent,
            expectations: storage::PostBookkeepingExpectation::default(),
        });
    }

    let normalized = org::normalize_org(
        fields.body.as_ref(),
        OrgStructuredMetadata {
            title: scalar_presence(fields.title.as_ref()),
            summary: scalar_presence(fields.summary.as_ref()),
            tags: validated_categories(fields.categories)?,
            audiences: Presence::Absent,
            lifecycle: fields.lifecycle,
        },
        operation,
        request_clock,
    )?;
    let audiences =
        authorize_audiences(audiences, author_user_id, normalized.metadata.audiences).await?;

    Ok(NormalizedAtomInput {
        body: normalized.body,
        title: match normalized.metadata.title {
            Presence::Present(title) => Some(title),
            Presence::Absent => None,
        },
        summary: match normalized.metadata.summary {
            Presence::Present(summary) => Some(summary),
            Presence::Absent => None,
        },
        categories: match normalized.metadata.tags {
            Presence::Present(tags) => tags,
            Presence::Absent => Vec::new(),
        },
        lifecycle: normalized.metadata.lifecycle,
        audiences,
        expectations: normalized.bookkeeping.into(),
    })
}

fn create_published_at(
    lifecycle: &Presence<PublicationState>,
    is_draft: bool,
    request_clock: UtcInstant,
) -> Option<UtcInstant> {
    match lifecycle {
        Presence::Present(state) => state.published_at(),
        Presence::Absent if is_draft => None,
        Presence::Absent => Some(request_clock),
    }
}

fn update_publish(
    lifecycle: &Presence<PublicationState>,
    is_draft: bool,
) -> storage::PublishUpdate {
    match lifecycle {
        Presence::Present(state) => (*state).into(),
        Presence::Absent if is_draft => storage::PublishUpdate::Unpublish,
        Presence::Absent => storage::PublishUpdate::Publish { at: None },
    }
}

/// Keyset-paging query parameters for the collection.
#[derive(Debug, Deserialize)]
pub struct CollectionPaging {
    /// `updated_at` of the last item on the previous page (RFC 3339).
    updated_before: Option<UtcInstant>,
    /// `post_id` of the last item on the previous page.
    id_before: Option<PostId>,
    /// Requested page size (clamped into `PageSize`'s `1..=50` range).
    limit: Option<u32>,
}

/// `GET /atompub/{username}/posts` — the user's collection as an Atom feed.
///
/// # Errors
///
/// Returns `400` if the pagination query contains malformed values.
/// Returns `403` if the authenticated user attempts to access another user's collection.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.collection_get", skip_all)]
pub async fn collection_get(
    services: PostServices,
    auth_user: auth::User,
    Path(username): Path<Username>,
    Query(paging): Query<CollectionPaging>,
) -> Result<Response, HandlerError> {
    let posts = services.posts();
    let site_config = services.site_config();
    super::require_user_match(&auth_user, &username)?;

    let limit = paging.limit.map_or(DEFAULT_PAGE_SIZE, PageSize::clamped);

    let cursor = match (paging.updated_before, paging.id_before) {
        (Some(updated_before), Some(post_id)) => Some(CollectionCursor {
            updated_at: updated_before,
            post_id,
        }),
        _ => None,
    };

    // `fetch_limit` over-fetches one row and `has_more` reads that row back — the two
    // halves of the rule, both from `PageSize` so neither is spelled here (#696).
    let mut records = posts
        .list_collection_by_user(auth_user.user_id, cursor.as_ref(), limit.fetch_limit())
        .await?;

    let has_more = limit.has_more(records.len());
    if has_more {
        records.truncate(limit.page_len());
    }

    let base = super::required_base_url(site_config).await?;
    let collection_path = format!("/atompub/{username}/posts");
    let collection_url: FeedUrl = tagged_url::compose(&base, &collection_path);

    let next: Option<PaginationUrl> = if has_more {
        records.last().map(|last| {
            // Build the cursor query via `url`'s encoder (#560, D5), not `format!`.
            let updated_before = last.updated_at.to_string();
            let id_before = last.post_id.to_string();
            collection_url.with_query_pairs(&[
                ("updated_before", updated_before.as_str()),
                ("id_before", id_before.as_str()),
            ])
        })
    } else {
        None
    };

    let entries: Vec<_> = records
        .iter()
        .map(|p| mapping::post_to_entry(p, &base))
        .collect();

    let updated = records
        .first()
        .map_or_else(UtcInstant::now, |p| p.updated_at);

    let meta = FeedMeta {
        // The collection URL *is* the feed's atom:id.
        id: collection_url.clone().retag(),
        title: CollectionFeedTitle::posts(&username),
        updated,
        self_url: collection_url.clone(),
        // The collection URL *is* its own first page.
        first: Some(collection_url.retag()),
        next,
        previous: None,
    };

    let xml = atompub::render_feed(&meta, &entries)?;
    Ok(([(header::CONTENT_TYPE, FEED_CONTENT_TYPE)], xml).into_response())
}

/// Builds the `ViewerIdentity` for the authenticated `AtomPub` user.
///
/// `AtomPub` requests are authenticated (the author), so the owner-post-load paths
/// resolve the post as the local viewer for that user — otherwise the resolution
/// filter would hide the user's own non-Public posts (a `404` before the owner
/// check ever runs).
fn owner_viewer(auth_user: &auth::User) -> ViewerIdentity {
    ViewerIdentity::local(auth_user.user_id)
}

/// Loads a post that the authenticated user owns and that is not soft-deleted.
/// Returns `404` for missing, foreign, or deleted posts.
///
/// The post is loaded as the authenticated owner (not `Anonymous`) so the
/// resolution filter does not hide the owner's own non-Public posts.
async fn owned_post(
    posts: &dyn PostStorage,
    auth_user: &auth::User,
    username: &Username,
    post_id: PostId,
) -> Result<PostRecord, HandlerError> {
    super::require_user_match(auth_user, username)?;
    let viewer = owner_viewer(auth_user);
    let post = posts
        .get_post_by_id(post_id, &viewer)
        .await?
        .ok_or(HandlerError::NotFound)?;
    if post.user_id != auth_user.user_id || post.deleted_at.is_some() {
        return Err(HandlerError::NotFound);
    }
    Ok(post)
}

/// `GET /atompub/{username}/posts/{post_id}` — a single member entry.
///
/// # Errors
///
/// Returns `403` if the authenticated user attempts to access another user's post.
/// Returns `404` if the post is not found, is soft-deleted, or belongs to another user.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.member_get", skip_all)]
pub async fn member_get(
    services: PostServices,
    auth_user: auth::User,
    Path((username, post_id)): Path<(Username, PostId)>,
) -> Result<Response, HandlerError> {
    let posts = services.posts();
    let site_config = services.site_config();
    let post = owned_post(posts.as_ref(), &auth_user, &username, post_id).await?;
    let base = super::required_base_url(site_config).await?;
    let entry = mapping::post_to_entry(&post, &base);
    let xml = atompub::entry_to_xml(&entry)?;
    Ok((
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::ETAG, etag_for(&post).to_string()),
        ],
        xml,
    )
        .into_response())
}

fn member_delete_update_error(error: storage::UpdatePostError) -> HandlerError {
    HandlerError::from(storage::PerformUpdateError::from(error))
}

fn member_delete_feed_event_error(error: FeedEventError) -> HandlerError {
    match error {
        FeedEventError::Db(error) => HandlerError::from(error),
    }
}

fn member_delete_write_scope_error(error: WriteScopeError<HandlerError>) -> HandlerError {
    match error {
        WriteScopeError::Operation(error) => error,
        WriteScopeError::Begin(error) => HandlerError::from(error),
    }
}

/// `DELETE /atompub/{username}/posts/{post_id}` — soft-deletes a post.
///
/// # Errors
///
/// Returns `403` if the authenticated user attempts to delete another user's post.
/// Returns `404` if the post is not found, is already soft-deleted, or belongs to another user.
/// Returns `412` if an `If-Match` header is present and does not match the post's `ETag`.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.member_delete", skip_all)]
pub async fn member_delete(
    services: PostServices,
    Extension(write_scope): Extension<WriteScope>,
    Extension(feed_events): Extension<Arc<dyn FeedEventStorage>>,
    auth_user: auth::User,
    Path((username, post_id)): Path<(Username, PostId)>,
    headers: HeaderMap,
) -> Result<Response, HandlerError> {
    let posts = services.posts();
    let feed_events = Arc::clone(&feed_events);
    let post = owned_post(posts.as_ref(), &auth_user, &username, post_id).await?;

    // Conditional delete: honour `If-Match` against the content ETag, as `member_put` does.
    if !if_match_satisfied(&headers, &etag_for(&post)) {
        return Err(HandlerError::PreconditionFailed);
    }
    let tag_slugs: BTreeSet<Tag> = post.tags.iter().map(|tag| tag.tag_slug.clone()).collect();
    let feed_paths = feed::affected_feed_urls(&post.author_username, &tag_slugs);
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .soft_delete_post(transaction, post.post_id, auth_user.user_id)
                    .await
                    .map_err(member_delete_update_error)?;
                feed_events
                    .enqueue_many(transaction, &feed_paths)
                    .await
                    .map_err(member_delete_feed_event_error)
            })
        })
        .await
        .map_err(member_delete_write_scope_error)?;
    if let Err(status) = super::mutation::confirmed_or_accepted(outcome) {
        return Ok(status.into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /atompub/{username}/posts` — create a post from an `AtomPub` entry.
///
/// # Errors
///
/// Returns `400` if the entry is malformed or invalid for post creation.
/// Returns `403` if the authenticated user does not match the target username.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.collection_post", skip_all)]
pub async fn collection_post(
    services: PostServices,
    Extension(write_scope): Extension<WriteScope>,
    Extension(feed_events): Extension<Arc<dyn FeedEventStorage>>,
    auth_user: auth::User,
    Path(username): Path<Username>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, HandlerError> {
    let posts = services.posts();
    let feed_events = Arc::clone(&feed_events);
    let audiences = services.audiences();
    let user_config = services.user_config();
    let site_config = services.site_config();
    super::require_user_match(&auth_user, &username)?;
    let entry: Entry = body.parse()?;
    let request_clock = UtcInstant::now();
    let default_format = storage::get_default_post_format(user_config, auth_user.user_id).await?;
    let fields = mapping::entry_to_post_fields(&entry, default_format, request_clock)?;
    let format = fields.format;
    let is_draft = fields.is_draft;
    let NormalizedAtomInput {
        body,
        title,
        summary,
        categories,
        lifecycle,
        audiences: audience_input,
        expectations,
    } = normalize_atom_input(
        fields,
        OrgOperation::Create,
        request_clock,
        audiences,
        auth_user.user_id,
    )
    .await?;
    let published_at = create_published_at(&lifecycle, is_draft, request_clock);
    let audiences = match audience_input {
        Presence::Present(audiences) => audiences,
        Presence::Absent => vec![site_config.get_default_audience().await?.into()],
    };
    let idempotency_key = idempotency_key_from_headers(&headers);

    let created = storage::perform_post_creation_at(
        &write_scope,
        services.content_locks(),
        Arc::clone(&posts),
        Arc::clone(&feed_events),
        request_clock,
        storage::PostCreation {
            user_id: auth_user.user_id,
            body,
            title: title.as_ref(),
            format,
            slug_override: None,
            published_at,
            max_attempts: 100,
            summary,
            audiences,
            tags: categories.clone(),
            idempotency_key: idempotency_key.as_ref(),
            expectations,
        },
    )
    .await;

    // Re-fetch as the authenticated owner so a non-Public default audience is not
    // hidden, and so the response entry carries the post's tags.
    let viewer = owner_viewer(&auth_user);

    // A reused idempotency key returns the original post as `200` — skipping category
    // re-application (the original already carries its tags).
    if let Err(storage::PerformCreationError::IdempotencyConflict) = &created {
        let key = idempotency_key.as_ref().ok_or(HandlerError::Invariant)?;
        let post_id = posts
            .post_id_for_idempotency_key(auth_user.user_id, key, request_clock)
            .await?
            .ok_or(HandlerError::Invariant)?;
        // If the original was soft-deleted between the create and this replay, a
        // stale-key retry deserves a 404 rather than a 500.
        let post = posts
            .get_post_by_id(post_id, &viewer)
            .await?
            .ok_or(HandlerError::NotFound)?;
        let base = super::required_base_url(site_config).await?;
        host::metrics::idempotency(host::metrics::IdempotencyEvent::Replayed);
        return post_entry_response(StatusCode::OK, &post, &base, &username);
    }

    // Fresh create: a non-conflict error propagates via `?`; an unavailable commit
    // acknowledgement tells the client to revalidate with `202 Accepted`.
    let created = match super::mutation::confirmed_or_accepted(created?) {
        Ok(created) => created,
        Err(status) => return Ok(status.into_response()),
    };
    if idempotency_key.is_some() {
        host::metrics::idempotency(host::metrics::IdempotencyEvent::Created);
    }
    let base = super::required_base_url(site_config).await?;
    let post = posts
        .get_post_by_id(created.post_id, &viewer)
        .await?
        .ok_or(HandlerError::Invariant)?;
    post_entry_response(StatusCode::CREATED, &post, &base, &username)
}

/// Builds a member-entry response (used by create `201` and the idempotent-replay
/// `200`): the atom entry body plus `Location` and content-hash `ETag` headers.
fn post_entry_response(
    status: StatusCode,
    post: &PostRecord,
    base: &BaseUrl,
    username: &Username,
) -> Result<Response, HandlerError> {
    let location_path = format!("/atompub/{username}/posts/{}", post.post_id);
    let location: EditUriUrl = tagged_url::compose(base, &location_path);
    let xml = atompub::entry_to_xml(&mapping::post_to_entry(post, base))?;
    Ok((
        status,
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::LOCATION, location.to_string()),
            (header::ETAG, etag_for(post).to_string()),
        ],
        xml,
    )
        .into_response())
}

/// `PUT /atompub/{username}/posts/{post_id}` — replace a post from an `AtomPub` entry.
///
/// Honors `If-Match` (a stale `ETag` yields `412`). `app:draft` toggles publication.
///
/// # Errors
///
/// Returns `400` if the entry is malformed, invalid for replacement, or names
/// an audience the authenticated author does not own.
/// Returns `403` if the authenticated user does not match the target username.
/// Returns `404` if the post is not found, soft-deleted, or belongs to another user.
/// Returns `412` if an `If-Match` header is present and does not match the post's `ETag`.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.member_put", skip_all)]
pub async fn member_put(
    services: PostServices,
    Extension(write_scope): Extension<WriteScope>,
    Extension(feed_events): Extension<Arc<dyn FeedEventStorage>>,
    auth_user: auth::User,
    Path((username, post_id)): Path<(Username, PostId)>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, HandlerError> {
    let posts = services.posts();
    let feed_events = Arc::clone(&feed_events);
    let audiences = services.audiences();
    let user_config = services.user_config();
    let site_config = services.site_config();
    let current = owned_post(posts.as_ref(), &auth_user, &username, post_id).await?;
    let previous_tag_slugs = current
        .tags
        .iter()
        .map(|tag| tag.tag_slug.clone())
        .collect();

    if !if_match_satisfied(&headers, &etag_for(&current)) {
        return Err(HandlerError::PreconditionFailed);
    }

    let entry: Entry = body.parse()?;
    let request_clock = UtcInstant::now();
    let default_format = storage::get_default_post_format(user_config, auth_user.user_id).await?;
    let fields = mapping::entry_to_post_fields(&entry, default_format, request_clock)?;
    let format = fields.format;
    let is_draft = fields.is_draft;
    let NormalizedAtomInput {
        body,
        title,
        summary,
        categories,
        lifecycle,
        audiences: audience_input,
        expectations,
    } = normalize_atom_input(
        fields,
        OrgOperation::Update { post_id },
        request_clock,
        audiences,
        auth_user.user_id,
    )
    .await?;
    let audiences = match audience_input {
        Presence::Present(audiences) => audiences,
        Presence::Absent => posts.get_post_audiences(post_id).await?,
    };
    let update_outcome = storage::perform_post_update(
        &write_scope,
        services.content_locks(),
        Arc::clone(&posts),
        Arc::clone(&feed_events),
        storage::PostUpdate {
            post_id,
            editor_user_id: auth_user.user_id,
            body,
            title: title.as_ref(),
            format,
            slug_override: None,
            publish: update_publish(&lifecycle, is_draft),
            request_clock,
            expectations,
            summary,
            audiences,
            tags: categories,
            previous_tag_slugs,
        },
    )
    .await?;
    if let Err(status) = super::mutation::confirmed_or_accepted(update_outcome) {
        return Ok(status.into_response());
    }

    let viewer = owner_viewer(&auth_user);
    let post = posts
        .get_post_by_id(post_id, &viewer)
        .await?
        .ok_or(HandlerError::Invariant)?;
    let base = super::required_base_url(site_config).await?;
    let xml = atompub::entry_to_xml(&mapping::post_to_entry(&post, &base))?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::ETAG, etag_for(&post).to_string()),
        ],
        xml,
    )
        .into_response())
}

#[cfg(test)]
mod etag_tests {
    use super::*;
    use axum::response::IntoResponse;
    use chrono::{TimeZone, Utc};
    use common::ids::{TagId, UserId};
    use common::tag::{Tag, TagLabel};
    use common::test_support::{
        parse_post_body, parse_post_summary, parse_post_title, parse_utc_instant,
    };
    use std::error::Error;
    use storage::{MockAudienceStorage, PostFormat, PostTag, PublishUpdate};

    #[test]
    fn member_delete_update_storage_error_is_internal_with_sqlx_source() {
        let error =
            member_delete_update_error(storage::UpdatePostError::Internal(sqlx::Error::PoolClosed));

        let HandlerError::Internal(source) = &error else {
            unreachable!("storage update errors must map to HandlerError::Internal");
        };
        let update = source
            .downcast_ref::<storage::PerformUpdateError>()
            .expect("internal source should retain the update error");
        assert!(matches!(
            update
                .source()
                .and_then(|source| source.downcast_ref::<sqlx::Error>()),
            Some(sqlx::Error::PoolClosed)
        ));
        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn member_delete_feed_event_db_error_is_internal_with_sqlx_source() {
        let error = member_delete_feed_event_error(FeedEventError::Db(sqlx::Error::RowNotFound));

        let HandlerError::Internal(source) = &error else {
            unreachable!("feed event database errors must map to HandlerError::Internal");
        };
        assert!(matches!(
            source.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::RowNotFound)
        ));
        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn member_delete_write_scope_operation_preserves_handler_error() {
        let error =
            member_delete_write_scope_error(WriteScopeError::Operation(HandlerError::NotFound));

        assert!(matches!(&error, HandlerError::NotFound));
        assert_eq!(error.into_response().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn member_delete_write_scope_begin_is_internal_with_sqlx_source() {
        let error =
            member_delete_write_scope_error(WriteScopeError::Begin(sqlx::Error::PoolTimedOut));

        let HandlerError::Internal(source) = &error else {
            unreachable!("write scope begin errors must map to HandlerError::Internal");
        };
        assert!(matches!(
            source.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        ));
        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    fn mk_tag(post_id: PostId, tag_id: TagId, slug: Tag, display: TagLabel) -> PostTag {
        PostTag {
            post_id,
            tag_id,
            tag_slug: slug,
            tag_display: display,
        }
    }

    fn base_post() -> PostRecord {
        let t = Utc
            .timestamp_opt(1_000_000, 0)
            .single()
            .expect("valid time");
        PostRecord {
            post_id: PostId::from(1),
            user_id: UserId::from(1),
            author_username: "alice".parse().expect("parse username"),
            title: Some(parse_post_title("Title")),
            slug: "my-post".parse().expect("parse slug"),
            body: parse_post_body("Body text."),
            format: PostFormat::Org,
            rendered_html: common::test_support::rendered_html("<p>Body text.</p>"),
            created_at: UtcInstant::from(t),
            updated_at: UtcInstant::from(t),
            published_at: Some(UtcInstant::from(t)),
            deleted_at: None,
            summary: Some(parse_post_summary("Summary")),
            tags: vec![
                mk_tag(
                    PostId::from(1),
                    TagId::from(1),
                    "rust".parse().unwrap(),
                    "Rust".parse().unwrap(),
                ),
                mk_tag(
                    PostId::from(1),
                    TagId::from(2),
                    "emacs".parse().unwrap(),
                    "Emacs".parse().unwrap(),
                ),
            ],
        }
    }

    #[test]
    fn etag_for_is_quoted_sha256() {
        let e = etag_for(&base_post());
        let hex = e
            .strip_prefix("\"sha256-")
            .and_then(|s| s.strip_suffix('"'))
            .expect("etag is a quoted sha256- token");
        assert_eq!(hex.len(), 64);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }

    #[test]
    fn etag_for_is_deterministic() {
        assert_eq!(etag_for(&base_post()), etag_for(&base_post()));
    }

    #[test]
    fn etag_for_ignores_identity_and_timestamps() {
        // AC2/AC5: nothing outside the content fields moves the ETag — including a
        // published_at whose *value* advances while staying Some (non-draft).
        let e = etag_for(&base_post());
        let later = Utc
            .timestamp_opt(9_000_000, 0)
            .single()
            .expect("valid time");
        let mut p = base_post();
        p.post_id = PostId::from(999);
        p.user_id = UserId::from(42);
        p.slug = "other-slug".parse().expect("parse slug");
        p.created_at = UtcInstant::from(later);
        p.updated_at = UtcInstant::from(later);
        p.published_at = Some(UtcInstant::from(later));
        p.rendered_html = common::test_support::rendered_html("<p>totally different</p>");
        p.tags = vec![
            mk_tag(
                PostId::from(999),
                TagId::from(55),
                "rust".parse().unwrap(),
                "Rust".parse().unwrap(),
            ),
            mk_tag(
                PostId::from(999),
                TagId::from(56),
                "emacs".parse().unwrap(),
                "Emacs".parse().unwrap(),
            ),
        ];
        assert_eq!(etag_for(&p), e);
    }

    #[test]
    fn etag_for_changes_on_each_content_field() {
        let e = etag_for(&base_post());
        let flip = |f: &dyn Fn(&mut PostRecord)| {
            let mut p = base_post();
            f(&mut p);
            etag_for(&p)
        };
        assert_ne!(flip(&|p| p.title = Some(parse_post_title("Other"))), e); // title value
        assert_ne!(flip(&|p| p.title = None), e); // title present->absent
        assert_ne!(flip(&|p| p.body = parse_post_body("Different body.")), e); // body
        assert_ne!(flip(&|p| p.summary = Some(parse_post_summary("Other"))), e); // summary value
        assert_ne!(flip(&|p| p.summary = None), e); // summary present->absent
        assert_ne!(flip(&|p| p.format = PostFormat::Markdown), e); // format
        assert_ne!(
            flip(&|p| p.tags = vec![
                mk_tag(
                    PostId::from(1),
                    TagId::from(1),
                    "rust".parse().unwrap(),
                    "Rust".parse().unwrap()
                ),
                mk_tag(
                    PostId::from(1),
                    TagId::from(2),
                    "lisp".parse().unwrap(),
                    "Lisp".parse().unwrap()
                ),
            ]),
            e
        ); // tag display set
        assert_ne!(flip(&|p| p.published_at = None), e); // draft flip
    }
    fn org_fields(body: &str) -> PostFields {
        PostFields {
            title: None,
            body: parse_post_body(body),
            format: PostFormat::Org,
            summary: None,
            categories: Presence::Absent,
            lifecycle: Presence::Absent,
            is_draft: false,
        }
    }

    #[tokio::test]
    async fn org_normalization_keeps_an_absent_title_absent() {
        let normalized = normalize_atom_input(
            org_fields("Body"),
            OrgOperation::Create,
            parse_utc_instant("2026-08-26T12:00:00Z"),
            &MockAudienceStorage::new(),
            UserId::from(1),
        )
        .await
        .expect("normalization succeeds");

        assert_eq!(normalized.title, None);
    }

    #[tokio::test]
    async fn org_audience_storage_failure_is_an_internal_handler_error() {
        let mut audiences = MockAudienceStorage::new();
        audiences
            .expect_list_audiences()
            .returning(|_| Err(sqlx::Error::PoolClosed));

        let result = normalize_atom_input(
            org_fields("#+PROPERTY: JAUNDER_AUDIENCE named:42\nBody"),
            OrgOperation::Create,
            parse_utc_instant("2026-08-26T12:00:00Z"),
            &audiences,
            UserId::from(1),
        )
        .await;

        assert!(matches!(result, Err(HandlerError::Internal(_))));
    }

    #[test]
    fn legacy_draft_lifecycle_fallbacks_remain_unpublished() {
        let clock = parse_utc_instant("2026-08-26T12:00:00Z");
        assert_eq!(
            create_published_at(&Presence::Absent, true, clock),
            None,
            "a legacy Atom draft stays unpublished at create"
        );
        assert_eq!(
            update_publish(&Presence::Absent, true),
            PublishUpdate::Unpublish,
            "a legacy Atom draft stays unpublished at update"
        );
    }
}
