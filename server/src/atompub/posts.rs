//! `AtomPub` posts collection read/delete/create/update handlers.

use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::Deserialize;

use common::atompub::{entry_from_xml, entry_to_xml, render_feed, FeedMeta};
use storage::{CollectionCursor, PostRecord, PostStorage, SiteConfigStorage, UserConfigStorage};
use web::auth::AuthUser;

use super::mapping::{entry_to_post_fields, post_to_entry};
use super::{base_url, HandlerError};

const FEED_CONTENT_TYPE: &str = "application/atom+xml;type=feed;charset=utf-8";
const ENTRY_CONTENT_TYPE: &str = "application/atom+xml;type=entry;charset=utf-8";
const DEFAULT_PAGE_SIZE: u32 = 25;
const MAX_PAGE_SIZE: u32 = 50;

/// A strong `ETag` for a post, derived from its last-update time.
/// Shared with the member update handler (later task).
pub(crate) fn etag_for(post: &PostRecord) -> String {
    format!("\"{}\"", post.updated_at.timestamp_millis())
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
    Path(username): Path<String>,
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

/// Loads a post that the authenticated user owns and that is not soft-deleted.
/// Returns `404` for missing, foreign, or deleted posts.
async fn owned_post(
    posts: &dyn PostStorage,
    auth_user: &AuthUser,
    username: &str,
    post_id: i64,
) -> Result<PostRecord, HandlerError> {
    super::require_user_match(auth_user, username)?;
    let post = posts
        .get_post_by_id(post_id)
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
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(String, i64)>,
) -> Result<Response, HandlerError> {
    let post = owned_post(posts.as_ref(), &auth_user, &username, post_id).await?;
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
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.member_delete", skip_all)]
pub async fn member_delete(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(String, i64)>,
) -> Result<Response, HandlerError> {
    let post = owned_post(posts.as_ref(), &auth_user, &username, post_id).await?;
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
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(user_config): Extension<Arc<dyn UserConfigStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: AuthUser,
    Path(username): Path<String>,
    body: String,
) -> Result<Response, HandlerError> {
    super::require_user_match(&auth_user, &username)?;
    let entry = entry_from_xml(&body)?;
    let default_format =
        storage::get_default_post_format(user_config.as_ref(), auth_user.user_id).await?;
    let fields = entry_to_post_fields(&entry, default_format);
    let published_at = if fields.is_draft {
        None
    } else {
        Some(chrono::Utc::now())
    };

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
        },
    )
    .await?;

    apply_categories(posts.as_ref(), created.post_id, &fields.categories).await?;

    // Re-fetch so the response entry includes the freshly applied tags.
    let post = posts
        .get_post_by_id(created.post_id)
        .await?
        .ok_or(HandlerError::Internal)?;

    let base = base_url(site_config.as_ref()).await;
    let location = format!("{base}/atompub/{username}/posts/{}", post.post_id);
    let entry_out = post_to_entry(&post, &base);
    let xml = entry_to_xml(&entry_out);

    Ok((
        StatusCode::CREATED,
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::LOCATION, location),
            (header::ETAG, etag_for(&post)),
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
/// Returns `400` if the entry is malformed.
/// Returns `403` if the authenticated user does not match the target username.
/// Returns `404` if the post is not found, is deleted, or belongs to another user.
/// Returns `412` if `If-Match` is present and stale.
/// Returns `500` if storage fails.
#[tracing::instrument(name = "atompub.posts.member_put", skip_all)]
pub async fn member_put(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(user_config): Extension<Arc<dyn UserConfigStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(String, i64)>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, HandlerError> {
    let current = owned_post(posts.as_ref(), &auth_user, &username, post_id).await?;

    if let Some(if_match) = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) {
        if if_match != "*" && if_match != etag_for(&current) {
            return Err(HandlerError::PreconditionFailed);
        }
    }

    let entry = entry_from_xml(&body)?;
    let default_format =
        storage::get_default_post_format(user_config.as_ref(), auth_user.user_id).await?;
    let fields = entry_to_post_fields(&entry, default_format);

    storage::perform_post_update(
        posts.as_ref(),
        storage::PostUpdate {
            post_id,
            editor_user_id: auth_user.user_id,
            body: fields.body,
            title: fields.title.as_deref(),
            format: fields.format,
            slug_override: None,
            publish: !fields.is_draft,
            summary: fields.summary,
        },
    )
    .await?;

    apply_categories(posts.as_ref(), post_id, &fields.categories).await?;

    let post = posts
        .get_post_by_id(post_id)
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
