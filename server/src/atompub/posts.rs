//! `AtomPub` posts collection read/delete/create/update handlers.

use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::Deserialize;

use common::atompub::{entry_from_xml, entry_to_xml, render_feed, FeedMeta};
use storage::{AppState, CollectionCursor, PostRecord};
use web::auth::AuthUser;

use super::base_url;
use super::mapping::{entry_to_post_fields, post_to_entry};

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
/// Returns `500` if storage or serialization fails.
pub async fn collection_get(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
    Path(username): Path<String>,
    Query(paging): Query<CollectionPaging>,
) -> Result<Response, StatusCode> {
    if auth_user.username.as_str() != username {
        return Err(StatusCode::FORBIDDEN);
    }

    let limit = paging
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);

    let cursor = match (&paging.updated_before, paging.id_before) {
        (Some(ts), Some(post_id)) => {
            let updated_at = chrono::DateTime::parse_from_rfc3339(ts)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .with_timezone(&chrono::Utc);
            Some(CollectionCursor {
                updated_at,
                post_id,
            })
        }
        _ => None,
    };

    // Fetch one extra row to detect whether a next page exists.
    let mut posts = state
        .posts
        .list_collection_by_user(auth_user.user_id, cursor.as_ref(), limit + 1)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let has_more = usize::try_from(limit).unwrap_or(usize::MAX) < posts.len();
    if has_more {
        posts.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }

    let base = base_url(&state).await;
    let collection_url = format!("{base}/atompub/{username}/posts");

    let next = if has_more {
        posts.last().map(|last| {
            let ts = utf8_encode(&last.updated_at.to_rfc3339());
            format!(
                "{collection_url}?updated_before={ts}&id_before={}",
                last.post_id
            )
        })
    } else {
        None
    };

    let entries: Vec<_> = posts.iter().map(|p| post_to_entry(p, &base)).collect();

    let updated_rfc3339 = posts.first().map_or_else(
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

    let xml = render_feed(&meta, &entries).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(([(header::CONTENT_TYPE, FEED_CONTENT_TYPE)], xml).into_response())
}

fn utf8_encode(s: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Loads a post that the authenticated user owns and that is not soft-deleted.
/// Returns `404` for missing, foreign, or deleted posts.
async fn owned_post(
    state: &AppState,
    auth_user: &AuthUser,
    username: &str,
    post_id: i64,
) -> Result<PostRecord, StatusCode> {
    if auth_user.username.as_str() != username {
        return Err(StatusCode::FORBIDDEN);
    }
    let post = state
        .posts
        .get_post_by_id(post_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if post.user_id != auth_user.user_id || post.deleted_at.is_some() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(post)
}

/// `GET /atompub/{username}/posts/{post_id}` — a single member entry.
///
/// # Errors
///
/// Returns `403` if the authenticated user attempts to access another user's post.
/// Returns `404` if the post is not found, is soft-deleted, or belongs to another user.
/// Returns `500` if storage or serialization fails.
pub async fn member_get(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(String, i64)>,
) -> Result<Response, StatusCode> {
    let post = owned_post(&state, &auth_user, &username, post_id).await?;
    let base = base_url(&state).await;
    let entry = post_to_entry(&post, &base);
    let xml = entry_to_xml(&entry).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::ETAG, etag_for(&post)),
        ],
        xml,
    )
        .into_response())
}

/// Maps a `PerformCreationError` to the appropriate HTTP status code.
fn creation_status(err: &storage::PerformCreationError) -> StatusCode {
    match err {
        storage::PerformCreationError::EmptyPost
        | storage::PerformCreationError::NoSlugFromPost
        | storage::PerformCreationError::InvalidSlug(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Maps a `PerformUpdateError` to the appropriate HTTP status code.
fn update_status(err: &storage::PerformUpdateError) -> StatusCode {
    match err {
        storage::PerformUpdateError::EmptyPost
        | storage::PerformUpdateError::NoSlugFromPost
        | storage::PerformUpdateError::InvalidSlug => StatusCode::BAD_REQUEST,
        storage::PerformUpdateError::NotFound | storage::PerformUpdateError::Unauthorized => {
            StatusCode::NOT_FOUND
        }
        storage::PerformUpdateError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
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
) -> Result<(), StatusCode> {
    use common::tag::Tag;
    use std::collections::HashSet;
    use std::str::FromStr;

    let existing = posts
        .get_tags_for_post(post_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let existing_slugs: HashSet<String> = existing.iter().map(|t| t.tag_slug.to_string()).collect();
    let desired_slugs: HashSet<String> = desired
        .iter()
        .filter_map(|d| Tag::from_str(d).ok())
        .map(|t| t.to_string())
        .collect();

    for display in desired {
        let Ok(slug) = Tag::from_str(display) else {
            continue;
        };
        if !existing_slugs.contains(&slug.to_string()) {
            posts
                .tag_post(post_id, display)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
    }
    for tag in &existing {
        if !desired_slugs.contains(&tag.tag_slug.to_string()) {
            posts
                .untag_post(post_id, &tag.tag_slug)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
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
pub async fn member_delete(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(String, i64)>,
) -> Result<Response, StatusCode> {
    let post = owned_post(&state, &auth_user, &username, post_id).await?;
    state
        .posts
        .soft_delete_post(post.post_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /atompub/{username}/posts` — create a post from an `AtomPub` entry.
///
/// # Errors
///
/// Returns `400` if the entry is malformed or invalid for post creation.
/// Returns `403` if the authenticated user does not match the target username.
/// Returns `500` if storage or serialization fails.
pub async fn collection_post(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
    Path(username): Path<String>,
    body: String,
) -> Result<Response, StatusCode> {
    if auth_user.username.as_str() != username {
        return Err(StatusCode::FORBIDDEN);
    }
    let entry = entry_from_xml(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let default_format =
        storage::get_default_post_format(state.user_config.as_ref(), auth_user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let fields = entry_to_post_fields(&entry, default_format);
    let published_at = if fields.is_draft {
        None
    } else {
        Some(chrono::Utc::now())
    };

    let created = storage::perform_post_creation(
        state.posts.as_ref(),
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
    .await
    .map_err(|e| creation_status(&e))?;

    apply_categories(state.posts.as_ref(), created.post_id, &fields.categories).await?;

    // Re-fetch so the response entry includes the freshly applied tags.
    let post = state
        .posts
        .get_post_by_id(created.post_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let base = base_url(&state).await;
    let location = format!("{base}/atompub/{username}/posts/{}", post.post_id);
    let entry_out = post_to_entry(&post, &base);
    let xml = entry_to_xml(&entry_out).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
/// Returns `500` if storage or serialization fails.
pub async fn member_put(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
    Path((username, post_id)): Path<(String, i64)>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, StatusCode> {
    let current = owned_post(&state, &auth_user, &username, post_id).await?;

    if let Some(if_match) = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()) {
        if if_match != "*" && if_match != etag_for(&current) {
            return Err(StatusCode::PRECONDITION_FAILED);
        }
    }

    let entry = entry_from_xml(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let default_format =
        storage::get_default_post_format(state.user_config.as_ref(), auth_user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let fields = entry_to_post_fields(&entry, default_format);

    storage::perform_post_update(
        state.posts.as_ref(),
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
    .await
    .map_err(|e| update_status(&e))?;

    apply_categories(state.posts.as_ref(), post_id, &fields.categories).await?;

    let post = state
        .posts
        .get_post_by_id(post_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let base = base_url(&state).await;
    let entry_out = post_to_entry(&post, &base);
    let xml = entry_to_xml(&entry_out).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
mod tests {
    use super::{creation_status, update_status};
    use axum::http::StatusCode;
    use storage::{PerformCreationError, PerformUpdateError};

    #[test]
    fn creation_status_maps_validation_to_400_else_500() {
        assert_eq!(
            creation_status(&PerformCreationError::EmptyPost),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            creation_status(&PerformCreationError::NoSlugFromPost),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            creation_status(&PerformCreationError::InvalidSlug(
                common::slug::InvalidSlug
            )),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            creation_status(&PerformCreationError::CreatedNotFound),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn update_status_maps_each_error_class() {
        assert_eq!(
            update_status(&PerformUpdateError::EmptyPost),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            update_status(&PerformUpdateError::NoSlugFromPost),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            update_status(&PerformUpdateError::InvalidSlug),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            update_status(&PerformUpdateError::NotFound),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            update_status(&PerformUpdateError::Unauthorized),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            update_status(&PerformUpdateError::Storage(sqlx::Error::PoolClosed)),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
