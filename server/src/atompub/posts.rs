//! `AtomPub` posts collection read/delete handlers.

use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::Deserialize;

use common::atompub::{entry_to_xml, render_feed, FeedMeta};
use storage::{AppState, CollectionCursor, PostRecord};
use web::auth::AuthUser;

use super::base_url;
use super::mapping::post_to_entry;

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
