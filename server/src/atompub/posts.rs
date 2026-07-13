//! `AtomPub` posts collection read/delete/create/update handlers.

use std::sync::Arc;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use common::atompub::{entry_from_xml, entry_to_xml, render_feed, FeedMeta};
use common::username::Username;
use common::visibility::ViewerIdentity;
use storage::{
    CollectionCursor, PostRecord, PostStorage, SiteConfigStorage, SubscriptionStorage,
    UserConfigStorage,
};
use web::auth::AuthUser;

use super::mapping::{entry_to_post_fields, post_to_entry};
use super::{base_url, HandlerError};

const FEED_CONTENT_TYPE: &str = "application/atom+xml;type=feed;charset=utf-8";
const ENTRY_CONTENT_TYPE: &str = "application/atom+xml;type=entry;charset=utf-8";
const DEFAULT_PAGE_SIZE: u32 = 25;
const MAX_PAGE_SIZE: u32 = 50;

/// The storage dependencies the post handlers share, bundled into one extractor
/// so a handler stays under the argument limit without suppressing the lint.
/// Each field is pulled from the request `Extension`s the app router layers.
pub struct PostServices {
    posts: Arc<dyn PostStorage>,
    subscriptions: Arc<dyn SubscriptionStorage>,
    user_config: Arc<dyn UserConfigStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
}

impl<S: Send + Sync> FromRequestParts<S> for PostServices {
    type Rejection = ExtensionRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self {
            posts: Extension::<Arc<dyn PostStorage>>::from_request_parts(parts, state)
                .await?
                .0,
            subscriptions: Extension::<Arc<dyn SubscriptionStorage>>::from_request_parts(
                parts, state,
            )
            .await?
            .0,
            user_config: Extension::<Arc<dyn UserConfigStorage>>::from_request_parts(parts, state)
                .await?
                .0,
            site_config: Extension::<Arc<dyn SiteConfigStorage>>::from_request_parts(parts, state)
                .await?
                .0,
        })
    }
}

/// A strong, content-hash `ETag` for a post: `"sha256-<hex>"` over the post's
/// content fields (title, stored body, format, summary, tag display names, and
/// the draft flag) — never a timestamp. So identical content yields an identical
/// `ETag` and an idempotent re-publish does not change it, removing the time-based
/// divergence false-positive (#78).
pub(crate) fn etag_for(post: &PostRecord) -> String {
    /// The content projection that the `ETag` hashes.  Every field is reduced to a
    /// plain, `Serialize`-able primitive; `PostFormat`/`PostTag` are never hashed
    /// directly (the latter carries DB-assigned ids that would differ between two
    /// identical-content posts).  `draft` is time-independent (`published_at`
    /// presence, not its value).
    #[derive(Serialize)]
    struct EtagContent<'a> {
        title: Option<&'a str>,
        body: &'a str,
        format: String,
        summary: Option<&'a str>,
        tags: Vec<&'a str>,
        draft: bool,
    }
    let content = EtagContent {
        title: post.title.as_deref(),
        body: &post.body,
        format: post.format.to_string(),
        summary: post.summary.as_deref(),
        tags: post.tags.iter().map(|t| t.tag_display.as_str()).collect(),
        draft: post.published_at.is_none(),
    };
    let bytes = serde_json::to_vec(&content).unwrap_or_else(|_| Vec::new());
    format!("\"sha256-{:x}\"", Sha256::digest(&bytes))
}

/// Whether a request's `If-Match` precondition is satisfied for a post with ETAG.
/// An absent (or non-UTF-8) header is unconditional; `*` matches any current
/// representation; otherwise the value must equal ETAG. Shared by PUT and DELETE.
fn if_match_satisfied(headers: &HeaderMap, etag: &str) -> bool {
    match headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) {
        Some(if_match) => if_match == "*" || if_match == etag,
        None => true,
    }
}

/// Keyset-paging query parameters for the collection.
#[derive(Debug, Deserialize)]
pub struct CollectionPaging {
    /// `updated_at` of the last item on the previous page (RFC 3339).
    updated_before: Option<String>,
    /// `post_id` of the last item on the previous page.
    id_before: Option<i64>,
    /// Requested page size (clamped to `MAX_PAGE_SIZE`).
    limit: Option<u32>,
}

/// `GET /atompub/{username}/posts` — the user's collection as an Atom feed.
///
/// # Errors
///
/// Returns `400` if the pagination cursor contains an invalid RFC 3339 timestamp.
/// Returns `403` if the authenticated user attempts to access another user's collection.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.collection_get", skip_all)]
pub async fn collection_get(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: AuthUser,
    Path(username): Path<Username>,
    Query(paging): Query<CollectionPaging>,
) -> Result<Response, HandlerError> {
    super::require_user_match(&auth_user, &username)?;

    let limit = paging
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);

    let cursor = match (&paging.updated_before, paging.id_before) {
        (Some(ts), Some(post_id)) => {
            let updated_at = chrono::DateTime::parse_from_rfc3339(ts)
                .map_err(|_| HandlerError::BadRequest)?
                .with_timezone(&chrono::Utc);
            Some(CollectionCursor {
                updated_at,
                post_id,
            })
        }
        _ => None,
    };

    // Fetch one extra row to detect whether a next page exists.
    let mut records = posts
        .list_collection_by_user(auth_user.user_id, cursor.as_ref(), limit + 1)
        .await?;

    let has_more = usize::try_from(limit).unwrap_or(usize::MAX) < records.len();
    if has_more {
        records.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }

    let base = base_url(site_config.as_ref()).await;
    let collection_url = format!("{base}/atompub/{username}/posts");

    let next = if has_more {
        records.last().map(|last| {
            let ts = utf8_encode(&last.updated_at.to_rfc3339());
            format!(
                "{collection_url}?updated_before={ts}&id_before={}",
                last.post_id
            )
        })
    } else {
        None
    };

    let entries: Vec<_> = records.iter().map(|p| post_to_entry(p, &base)).collect();

    let updated_rfc3339 = records.first().map_or_else(
        || chrono::Utc::now().to_rfc3339(),
        |p| p.updated_at.to_rfc3339(),
    );

    let meta = FeedMeta {
        id: collection_url.clone(),
        title: format!("{username}'s posts"),
        updated_rfc3339,
        self_url: collection_url.clone(),
        first: Some(collection_url),
        next,
        previous: None,
    };

    let xml = render_feed(&meta, &entries);
    Ok(([(header::CONTENT_TYPE, FEED_CONTENT_TYPE)], xml).into_response())
}

fn utf8_encode(s: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Builds the `ViewerIdentity` for the authenticated `AtomPub` user.
///
/// `AtomPub` requests are authenticated (the author), so the owner-post-load paths
/// resolve the post as the local viewer for that user — otherwise the resolution
/// filter would hide the user's own non-Public posts (a `404` before the owner
/// check ever runs).
async fn owner_viewer(
    subscriptions: &dyn SubscriptionStorage,
    auth_user: &AuthUser,
) -> Result<ViewerIdentity, HandlerError> {
    let local_channel_id = subscriptions.local_channel_id().await?;
    Ok(ViewerIdentity::local(auth_user.user_id, local_channel_id))
}

/// Loads a post that the authenticated user owns and that is not soft-deleted.
/// Returns `404` for missing, foreign, or deleted posts.
///
/// The post is loaded as the authenticated owner (not `Anonymous`) so the
/// resolution filter does not hide the owner's own non-Public posts.
async fn owned_post(
    posts: &dyn PostStorage,
    subscriptions: &dyn SubscriptionStorage,
    auth_user: &AuthUser,
    username: &Username,
    post_id: i64,
) -> Result<PostRecord, HandlerError> {
    super::require_user_match(auth_user, username)?;
    let viewer = owner_viewer(subscriptions, auth_user).await?;
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
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(subscriptions): Extension<Arc<dyn SubscriptionStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(Username, i64)>,
) -> Result<Response, HandlerError> {
    let post = owned_post(
        posts.as_ref(),
        subscriptions.as_ref(),
        &auth_user,
        &username,
        post_id,
    )
    .await?;
    let base = base_url(site_config.as_ref()).await;
    let entry = post_to_entry(&post, &base);
    let xml = entry_to_xml(&entry);
    Ok((
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::ETAG, etag_for(&post)),
        ],
        xml,
    )
        .into_response())
}

/// Reconciles the post's tags with a desired set of category terms.
///
/// Tags missing from `desired` are removed; desired categories not yet tagged
/// are added. Invalid tag names (in the sense that they fail to parse as `Tag`)
/// are skipped.
async fn apply_categories(
    posts: &dyn storage::PostStorage,
    post_id: i64,
    desired: &[String],
) -> Result<(), HandlerError> {
    let existing = posts.get_tags_for_post(post_id).await?;
    let diff = storage::post_tag_diff(&existing, desired);

    for display in diff.to_add {
        posts.tag_post(post_id, display).await?;
    }
    for slug in diff.to_remove {
        posts.untag_post(post_id, slug).await?;
    }
    Ok(())
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
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(subscriptions): Extension<Arc<dyn SubscriptionStorage>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(Username, i64)>,
    headers: HeaderMap,
) -> Result<Response, HandlerError> {
    let post = owned_post(
        posts.as_ref(),
        subscriptions.as_ref(),
        &auth_user,
        &username,
        post_id,
    )
    .await?;

    // Conditional delete: honour `If-Match` against the content ETag, as `member_put` does.
    if !if_match_satisfied(&headers, &etag_for(&post)) {
        return Err(HandlerError::PreconditionFailed);
    }

    posts.soft_delete_post(post.post_id).await?;
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
    auth_user: AuthUser,
    Path(username): Path<Username>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, HandlerError> {
    let PostServices {
        posts,
        subscriptions,
        user_config,
        site_config,
    } = services;
    super::require_user_match(&auth_user, &username)?;
    let entry = entry_from_xml(&body)?;
    let default_format =
        storage::get_default_post_format(user_config.as_ref(), auth_user.user_id).await?;
    let fields = entry_to_post_fields(&entry, default_format);
    // Non-draft entries honor the wire `<published>`: a future time schedules
    // the post, a past time backdates it; absent falls back to "now".
    let published_at = if fields.is_draft {
        None
    } else {
        Some(fields.published.unwrap_or_else(chrono::Utc::now))
    };

    // AtomPub has no audience picker; new posts adopt the instance default.
    let default_audience = site_config.get_default_audience().await?;

    // A client-supplied idempotency key dedups a retried create (duplicate-on-retry).
    let idem = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let created = storage::perform_post_creation(
        posts.as_ref(),
        storage::PostCreation {
            user_id: auth_user.user_id,
            body: fields.body,
            title: fields.title.as_deref(),
            format: fields.format,
            slug_override: None,
            published_at,
            max_attempts: 100,
            summary: fields.summary,
            audiences: vec![default_audience],
            idempotency_key: idem,
        },
    )
    .await;

    let base = base_url(site_config.as_ref()).await;
    // Re-fetch as the authenticated owner so a non-Public default audience is not
    // hidden, and so the response entry carries the post's tags.
    let viewer = owner_viewer(subscriptions.as_ref(), &auth_user).await?;

    // A reused idempotency key returns the original post as `200` — skipping category
    // re-application (the original already carries its tags).
    if let Err(storage::PerformCreationError::IdempotencyConflict) = &created {
        let key = idem.ok_or(HandlerError::Internal)?;
        let post_id = posts
            .post_id_for_idempotency_key(auth_user.user_id, key)
            .await?
            .ok_or(HandlerError::Internal)?;
        // If the original was soft-deleted between the create and this replay, a
        // stale-key retry deserves a 404 rather than a 500.
        let post = posts
            .get_post_by_id(post_id, &viewer)
            .await?
            .ok_or(HandlerError::NotFound)?;
        return Ok(post_entry_response(StatusCode::OK, &post, &base, &username));
    }

    // Fresh create: a non-conflict error propagates via `?`.
    let created = created?;
    apply_categories(posts.as_ref(), created.post_id, &fields.categories).await?;
    let post = posts
        .get_post_by_id(created.post_id, &viewer)
        .await?
        .ok_or(HandlerError::Internal)?;
    Ok(post_entry_response(
        StatusCode::CREATED,
        &post,
        &base,
        &username,
    ))
}

/// Builds a member-entry response (used by create `201` and the idempotent-replay
/// `200`): the atom entry body plus `Location` and content-hash `ETag` headers.
fn post_entry_response(
    status: StatusCode,
    post: &PostRecord,
    base: &str,
    username: &str,
) -> Response {
    let location = format!("{base}/atompub/{username}/posts/{}", post.post_id);
    let xml = entry_to_xml(&post_to_entry(post, base));
    (
        status,
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::LOCATION, location),
            (header::ETAG, etag_for(post)),
        ],
        xml,
    )
        .into_response()
}

/// `PUT /atompub/{username}/posts/{post_id}` — replace a post from an `AtomPub` entry.
///
/// Honors `If-Match` (a stale `ETag` yields `412`). `app:draft` toggles publication.
///
/// # Errors
///
/// Returns `400` if the entry is malformed.
/// Returns `403` if the authenticated user does not match the target username.
/// Returns `404` if the post is not found, is deleted, or belongs to another user.
/// Returns `412` if `If-Match` is present and stale.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.member_put", skip_all)]
pub async fn member_put(
    services: PostServices,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(Username, i64)>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, HandlerError> {
    let PostServices {
        posts,
        subscriptions,
        user_config,
        site_config,
    } = services;
    let current = owned_post(
        posts.as_ref(),
        subscriptions.as_ref(),
        &auth_user,
        &username,
        post_id,
    )
    .await?;

    if !if_match_satisfied(&headers, &etag_for(&current)) {
        return Err(HandlerError::PreconditionFailed);
    }

    let entry = entry_from_xml(&body)?;
    let default_format =
        storage::get_default_post_format(user_config.as_ref(), auth_user.user_id).await?;
    let fields = entry_to_post_fields(&entry, default_format);

    // AtomPub has no audience picker; preserve the post's existing targeting
    // across the edit rather than resetting it.
    let audiences = posts.get_post_audiences(post_id).await?;
    storage::perform_post_update(
        posts.as_ref(),
        storage::PostUpdate {
            post_id,
            editor_user_id: auth_user.user_id,
            body: fields.body,
            title: fields.title.as_deref(),
            format: fields.format,
            slug_override: None,
            // A non-draft entry publishes at the wire `<published>` timestamp
            // (future = scheduled, past = backdated, absent = keep/now); a draft
            // clears publication.
            publish: if fields.is_draft {
                storage::PublishUpdate::Unpublish
            } else {
                storage::PublishUpdate::Publish {
                    at: fields.published,
                }
            },
            summary: fields.summary,
            audiences,
        },
    )
    .await?;

    apply_categories(posts.as_ref(), post_id, &fields.categories).await?;

    // Load as the authenticated owner so a non-Public post is not hidden.
    let viewer = owner_viewer(subscriptions.as_ref(), &auth_user).await?;
    let post = posts
        .get_post_by_id(post_id, &viewer)
        .await?
        .ok_or(HandlerError::Internal)?;

    let base = base_url(site_config.as_ref()).await;
    let entry_out = post_to_entry(&post, &base);
    let xml = entry_to_xml(&entry_out);

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::ETAG, etag_for(&post)),
        ],
        xml,
    )
        .into_response())
}

#[cfg(test)]
mod etag_tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use storage::{PostFormat, PostTag};

    fn mk_tag(post_id: i64, tag_id: i64, slug: &str, display: &str) -> PostTag {
        PostTag {
            post_id,
            tag_id,
            tag_slug: slug.parse().expect("parse tag slug"),
            tag_display: display.to_string(),
        }
    }

    fn base_post() -> PostRecord {
        let t = Utc
            .timestamp_opt(1_000_000, 0)
            .single()
            .expect("valid time");
        PostRecord {
            post_id: 1,
            user_id: 1,
            author_username: "alice".parse().expect("parse username"),
            title: Some("Title".to_string()),
            slug: "my-post".parse().expect("parse slug"),
            body: "Body text.".to_string(),
            format: PostFormat::Org,
            rendered_html: "<p>Body text.</p>".to_string(),
            created_at: t,
            updated_at: t,
            published_at: Some(t),
            deleted_at: None,
            summary: Some("Summary".to_string()),
            tags: vec![mk_tag(1, 1, "rust", "Rust"), mk_tag(1, 2, "emacs", "Emacs")],
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
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
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
        p.post_id = 999;
        p.user_id = 42;
        p.slug = "other-slug".parse().expect("parse slug");
        p.created_at = later;
        p.updated_at = later;
        p.published_at = Some(later);
        p.rendered_html = "<p>totally different</p>".to_string();
        p.tags = vec![
            mk_tag(999, 55, "rust", "Rust"),
            mk_tag(999, 56, "emacs", "Emacs"),
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
        assert_ne!(flip(&|p| p.title = Some("Other".to_string())), e); // title value
        assert_ne!(flip(&|p| p.title = None), e); // title present->absent
        assert_ne!(flip(&|p| p.body = "Different body.".to_string()), e); // body
        assert_ne!(flip(&|p| p.summary = Some("Other".to_string())), e); // summary value
        assert_ne!(flip(&|p| p.summary = None), e); // summary present->absent
        assert_ne!(flip(&|p| p.format = PostFormat::Markdown), e); // format
        assert_ne!(
            flip(&|p| p.tags = vec![mk_tag(1, 1, "rust", "Rust"), mk_tag(1, 2, "lisp", "Lisp")]),
            e
        ); // tag display set
        assert_ne!(flip(&|p| p.published_at = None), e); // draft flip
    }
}
