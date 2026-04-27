use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
use crate::error::WebResult;
#[cfg(feature = "ssr")]
use crate::error::{InternalError, InternalResult, WebError};
#[cfg(feature = "ssr")]
use chrono::{Datelike, NaiveDate, Utc};
#[cfg(feature = "ssr")]
use common::{
    render::{
        create_rendered_post, derive_post_metadata, perform_post_update, CreateRenderedPostError,
        PerformUpdateError,
    },
    slug::{slugify_title, Slug},
    storage::{AppState, CreatePostError, PostCursor, PostFormat, PostRecord, UpdatePostInput},
    username::Username,
};
#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Result returned by [`create_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatePostResult {
    pub post_id: i64,
    pub slug: String,
    pub created_at: String,
    pub published_at: Option<String>,
    pub preview_url: String,
    pub permalink: Option<String>,
}

/// Result returned by [`update_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdatePostResult {
    pub post_id: i64,
    pub slug: String,
    pub published_at: Option<String>,
    pub preview_url: String,
    pub permalink: Option<String>,
}

/// A draft row returned by [`list_drafts`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftSummary {
    pub post_id: i64,
    pub title: Option<String>,
    pub summary_label: String,
    pub slug: String,
    pub created_at: String,
    pub updated_at: String,
    pub preview_url: String,
    pub edit_url: String,
    pub permalink: String,
}

/// Result returned by [`publish_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishPostResult {
    pub post_id: i64,
    pub slug: String,
    pub published_at: String,
    pub permalink: String,
}

/// A published post row returned by timeline listing endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePostSummary {
    pub post_id: i64,
    pub username: String,
    pub title: Option<String>,
    pub slug: String,
    pub rendered_html: String,
    pub created_at: String,
    pub published_at: String,
    pub permalink: String,
}

/// A cursor-paginated page of timeline posts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePage {
    pub posts: Vec<TimelinePostSummary>,
    pub next_cursor_created_at: Option<String>,
    pub next_cursor_post_id: Option<i64>,
    pub has_more: bool,
}

/// Details of a post returned by [`get_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostResponse {
    pub post_id: i64,
    pub username: String,
    pub title: Option<String>,
    pub slug: String,
    pub body: String,
    pub format: String,
    pub rendered_html: String,
    pub created_at: String,
    pub published_at: Option<String>,
    pub is_draft: bool,
    pub is_author: bool,
}

/// Creates a post for the authenticated user.
#[server(endpoint = "/create_post")]
pub async fn create_post(
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
) -> WebResult<CreatePostResult> {
    crate::web_server_fn!("create_post", body, format, slug_override, publish => {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let metadata = derive_post_metadata(None, &body, &format)
            .ok_or_else(|| InternalError::validation("post body is required"))?;
        let published_at = publish.then(Utc::now);
        let slug_seed = slug_override
            .as_deref()
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
            .map(|slug| slug.to_ascii_lowercase())
            .map(|slug| slug.parse::<Slug>())
            .transpose()
            .map_err(|e| InternalError::validation(e.to_string()))?
            .map(|slug| slug.to_string())
            .or_else(|| slugify_title(&metadata.slug_seed))
            .ok_or_else(|| {
                InternalError::validation(
                    "post must contain at least one ASCII letter or digit for its slug",
                )
            })?;

        let created = create_post_with_unique_slug(
            state.as_ref(),
            auth.user_id,
            &auth.username,
            metadata.title,
            body, // verbatim — no stripping
            format,
            slug_seed,
            published_at,
        )
        .await?;

        Ok(created)
    })
}

/// Retrieves a post by its permalink.
#[server(endpoint = "/get_post")]
pub async fn get_post(
    username: String,
    year: i32,
    month: u32,
    day: u32,
    slug: String,
) -> WebResult<PostResponse> {
    crate::web_server_fn!("get_post", username, year, month, day, slug => {
        use common::slug::Slug;
        use common::username::Username;

        let state = expect_context::<Arc<AppState>>();

        let username_parsed = username
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let slug_parsed = slug.parse::<Slug>().map_err(|e| InternalError::validation(e.to_string()))?;

        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| InternalError::validation("Invalid permalink"))?;

        if let Some(post) = state
            .posts
            .get_post_by_permalink(&username_parsed, year, month, day, &slug_parsed)
            .await
            .map_err(InternalError::storage)?
        {
            let is_author = require_auth()
                .await
                .map(|auth| auth.user_id == post.user_id)
                .unwrap_or(false);
            return Ok(post_response(post, username_parsed.to_string(), is_author));
        }

        let auth = require_auth()
            .await
            .map_err(private_post_not_found_error)?;
        if auth.username != username_parsed {
            return Err(not_found_error());
        }

        let draft = find_draft_by_permalink_for_user(
            state.as_ref(),
            auth.user_id,
            year,
            month,
            day,
            &slug_parsed,
        )
        .await?
        .ok_or_else(not_found_error)?;

        Ok(post_response(draft, auth.username.to_string(), true))
    })
}

/// Retrieves a draft preview for the authenticated author.
#[server(endpoint = "/get_post_preview")]
pub async fn get_post_preview(post_id: i64) -> WebResult<PostResponse> {
    crate::web_server_fn!("get_post_preview", post_id => {
        let auth = require_auth()
            .await
            .map_err(private_post_not_found_error)?;
        let state = expect_context::<Arc<AppState>>();

        let post = state
            .posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(not_found_error)?;

        let PostRecord {
            post_id,
            user_id,
            title,
            slug,
            body,
            format,
            rendered_html,
            created_at,
            published_at,
            deleted_at,
            ..
        } = post;

        if deleted_at.is_some() || user_id != auth.user_id {
            return Err(not_found_error());
        }

        Ok(PostResponse {
            post_id,
            username: auth.username.to_string(),
            title,
            slug: slug.to_string(),
            body,
            format: format.to_string(),
            rendered_html,
            created_at: created_at.to_rfc3339(),
            is_draft: published_at.is_none(),
            published_at: published_at.map(|t| t.to_rfc3339()),
            is_author: true,
        })
    })
}

/// Updates an existing post for the authenticated author.
#[server(endpoint = "/update_post")]
pub async fn update_post(
    post_id: i64,
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
) -> WebResult<UpdatePostResult> {
    crate::web_server_fn!("update_post", post_id, body, format, slug_override, publish => {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        let record = perform_post_update(
            state.posts.as_ref(),
            post_id,
            auth.user_id,
            body,
            format,
            slug_override.as_deref(),
            publish,
        )
        .await
        .map_err(|e| match e {
            PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
                InternalError::not_found("Post")
            }
            other => perform_update_error(other),
        })?;

        let published_at_str = record.published_at.map(|t| t.to_rfc3339());
        let permalink = record
            .published_at
            .map(|ts| build_permalink(&auth.username, ts, &record.slug));

        Ok(UpdatePostResult {
            post_id,
            slug: record.slug.to_string(),
            published_at: published_at_str,
            preview_url: format!("/draft/{post_id}/preview"),
            permalink,
        })
    })
}

/// Lists drafts for the authenticated user.
#[server(endpoint = "/list_drafts")]
pub async fn list_drafts(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<Vec<DraftSummary>> {
    crate::web_server_fn!("list_drafts", cursor_created_at, cursor_post_id, limit => {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();

        let parsed_cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let drafts = state
            .posts
            .list_drafts_by_user(auth.user_id, parsed_cursor.as_ref(), page_size)
            .await
            .map_err(InternalError::storage)?;

        Ok(drafts
            .into_iter()
            .map(|draft| {
                let permalink = build_permalink(&auth.username, draft.created_at, &draft.slug);
                DraftSummary {
                    post_id: draft.post_id,
                    title: draft.title.clone(),
                    summary_label: fallback_summary_label(&draft),
                    slug: draft.slug.to_string(),
                    created_at: draft.created_at.to_rfc3339(),
                    updated_at: draft.updated_at.to_rfc3339(),
                    preview_url: format!("/draft/{}/preview", draft.post_id),
                    edit_url: format!("/posts/{}/edit", draft.post_id),
                    permalink,
                }
            })
            .collect())
    })
}

/// Publishes an existing draft owned by the authenticated user.
#[server(endpoint = "/publish_post")]
pub async fn publish_post(post_id: i64) -> WebResult<PublishPostResult> {
    crate::web_server_fn!("publish_post", post_id => {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();

        let existing = state
            .posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        let updated = state
            .posts
            .update_post(
                post_id,
                auth.user_id,
                &UpdatePostInput {
                    title: existing.title,
                    slug: existing.slug,
                    body: existing.body,
                    format: existing.format,
                    rendered_html: existing.rendered_html,
                    publish: true,
                },
            )
            .await
            .map_err(|e| match e {
                common::storage::UpdatePostError::NotFound
                | common::storage::UpdatePostError::Unauthorized => InternalError::not_found("Post"),
                common::storage::UpdatePostError::Internal(error) => InternalError::storage(error),
            })?;

        let published_at = updated
            .published_at
            .ok_or_else(|| InternalError::not_found("Post"))?;

        Ok(PublishPostResult {
            post_id: updated.post_id,
            slug: updated.slug.to_string(),
            published_at: published_at.to_rfc3339(),
            permalink: build_permalink(&auth.username, published_at, &updated.slug),
        })
    })
}

/// Lists published, non-deleted posts for a user using cursor pagination.
#[server(endpoint = "/list_user_posts")]
pub async fn list_user_posts(
    username: String,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    crate::web_server_fn!("list_user_posts", username, cursor_created_at, cursor_post_id, limit => {
        let state = expect_context::<Arc<AppState>>();

        let username = username
            .trim()
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = state
            .posts
            .list_published_by_user(&username, cursor.as_ref(), fetch_limit)
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();

        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(&username, post))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// Lists published, non-deleted posts across all users using cursor pagination.
#[server(endpoint = "/list_local_timeline")]
pub async fn list_local_timeline(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    crate::web_server_fn!("list_local_timeline", cursor_created_at, cursor_post_id, limit => {
        let state = expect_context::<Arc<AppState>>();

        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = state
            .posts
            .list_published(cursor.as_ref(), fetch_limit)
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let mut posts = Vec::with_capacity(rows.len());

        for post in rows {
            let author = state
                .users
                .get_user(post.user_id)
                .await
                .map_err(InternalError::storage)?
                .ok_or_else(|| InternalError::not_found("post author"))?;
            if let Some(summary) = timeline_post_summary(&author.username, post) {
                posts.push(summary);
            }
        }

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[server(endpoint = "/list_home_feed")]
pub async fn list_home_feed(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    crate::web_server_fn!("list_home_feed", cursor_created_at, cursor_post_id, limit => {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();

        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = state
            .posts
            .list_published_by_user(&auth.username, cursor.as_ref(), fetch_limit)
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(&auth.username, post))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
async fn create_post_with_unique_slug(
    state: &AppState,
    user_id: i64,
    username: &Username,
    title: Option<String>,
    body: String,
    format: PostFormat,
    slug_seed: String,
    published_at: Option<chrono::DateTime<Utc>>,
) -> InternalResult<CreatePostResult> {
    for attempt in 0..100 {
        let slug_string = candidate_slug(&slug_seed, attempt);
        let slug = slug_string
            .parse::<Slug>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        match create_rendered_post(
            state.posts.as_ref(),
            user_id,
            title.clone(),
            slug,
            body.clone(),
            format.clone(),
            published_at,
        )
        .await
        {
            Ok(post_id) => {
                let record = state
                    .posts
                    .get_post_by_id(post_id)
                    .await
                    .map_err(InternalError::storage)?
                    .ok_or_else(|| InternalError::server_message("created post not found"))?;

                let created_at = record.created_at.to_rfc3339();
                let published_at = record.published_at.map(|timestamp| timestamp.to_rfc3339());
                let permalink = record
                    .published_at
                    .map(|ts| build_permalink(username, ts, &record.slug));

                let preview_url = format!("/draft/{post_id}/preview");

                return Ok(CreatePostResult {
                    post_id,
                    slug: record.slug.to_string(),
                    created_at,
                    published_at,
                    preview_url,
                    permalink,
                });
            }
            Err(common::render::CreateRenderedPostError::Storage(
                CreatePostError::SlugConflict,
            )) => {}
            Err(err) => return Err(create_rendered_post_error(err)),
        }
    }

    Err(InternalError::server_message(
        "unable to allocate a unique slug after 100 attempts",
    ))
}

#[cfg(any(feature = "ssr", test))]
fn candidate_slug(slug_seed: &str, attempt: usize) -> String {
    if attempt == 0 {
        slug_seed.to_owned()
    } else {
        format!("{slug_seed}-{}", attempt + 1)
    }
}

#[cfg(feature = "ssr")]
fn timeline_post_summary(username: &Username, post: PostRecord) -> Option<TimelinePostSummary> {
    let PostRecord {
        post_id,
        title,
        slug,
        rendered_html,
        created_at,
        published_at,
        ..
    } = post;
    let published_at = published_at?;
    let permalink = build_permalink(username, published_at, &slug);
    Some(TimelinePostSummary {
        post_id,
        username: username.to_string(),
        title,
        slug: slug.to_string(),
        rendered_html,
        created_at: created_at.to_rfc3339(),
        published_at: published_at.to_rfc3339(),
        permalink,
    })
}

#[cfg(feature = "ssr")]
fn to_post_cursor(post: &PostRecord) -> PostCursor {
    PostCursor {
        created_at: post.created_at,
        post_id: post.post_id,
    }
}

#[cfg(feature = "ssr")]
fn post_response(post: PostRecord, username: String, is_author: bool) -> PostResponse {
    let PostRecord {
        post_id,
        title,
        slug,
        body,
        format,
        rendered_html,
        created_at,
        published_at,
        ..
    } = post;
    PostResponse {
        post_id,
        username,
        title,
        slug: slug.to_string(),
        body,
        format: format.to_string(),
        rendered_html,
        created_at: created_at.to_rfc3339(),
        is_draft: published_at.is_none(),
        published_at: published_at.map(|t| t.to_rfc3339()),
        is_author,
    }
}

#[cfg(feature = "ssr")]
fn fallback_summary_label(post: &PostRecord) -> String {
    post.body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(100).collect::<String>())
        .filter(|line| !line.is_empty())
        .or_else(|| post.title.clone())
        .unwrap_or_else(|| post.slug.to_string())
}

#[cfg(feature = "ssr")]
fn parse_post_cursor(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
) -> InternalResult<Option<PostCursor>> {
    match (cursor_created_at, cursor_post_id) {
        (None, None) => Ok(None),
        (Some(created_at), Some(post_id)) => {
            let created_at = chrono::DateTime::parse_from_rfc3339(created_at.trim())
                .map_err(|_| InternalError::validation("invalid cursor_created_at"))?
                .with_timezone(&Utc);
            Ok(Some(PostCursor {
                created_at,
                post_id,
            }))
        }
        _ => Err(InternalError::validation(
            "cursor_created_at and cursor_post_id must be provided together",
        )),
    }
}

#[cfg(feature = "ssr")]
async fn find_draft_by_permalink_for_user(
    state: &AppState,
    user_id: i64,
    year: i32,
    month: u32,
    day: u32,
    slug: &Slug,
) -> InternalResult<Option<PostRecord>> {
    let mut cursor = None;

    // Search through up to 10,000 drafts (200 pages of 50). This 200-iteration
    // limit is a safety bound to prevent infinite loops or excessive DB load
    // while still being large enough for almost any user's draft list.
    for _ in 0..200 {
        let drafts = state
            .posts
            .list_drafts_by_user(user_id, cursor.as_ref(), 50)
            .await
            .map_err(InternalError::storage)?;
        if drafts.is_empty() {
            return Ok(None);
        }

        let next_cursor = drafts.last().map(to_post_cursor);

        if let Some(found) = drafts.into_iter().find(|post| {
            post.slug == *slug
                && post.created_at.year() == year
                && post.created_at.month() == month
                && post.created_at.day() == day
        }) {
            return Ok(Some(found));
        }

        let Some(next_cursor) = next_cursor else {
            return Ok(None);
        };
        cursor = Some(next_cursor);
    }

    Ok(None)
}

#[cfg(feature = "ssr")]
fn not_found_error() -> InternalError {
    set_not_found_status();
    InternalError::not_found("Post")
}

#[cfg(feature = "ssr")]
fn set_not_found_status() {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    if let Some(opts) = use_context::<ResponseOptions>() {
        opts.set_status(axum::http::StatusCode::NOT_FOUND);
    }
}

#[cfg(feature = "ssr")]
fn private_post_not_found_error(error: InternalError) -> InternalError {
    set_not_found_status();
    InternalError::masked(
        WebError::not_found("Post"),
        format!(
            "private post hidden behind not-found response: {}",
            error.operator_message()
        ),
    )
}

#[cfg(feature = "ssr")]
fn perform_update_error(error: PerformUpdateError) -> InternalError {
    match error {
        PerformUpdateError::EmptyPost
        | PerformUpdateError::NoSlugFromPost
        | PerformUpdateError::InvalidSlug => InternalError::validation(error.to_string()),
        PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
            InternalError::not_found("Post")
        }
        PerformUpdateError::Render(_) => InternalError::server(error),
        PerformUpdateError::Storage(error) => InternalError::storage(error),
    }
}

#[cfg(feature = "ssr")]
fn create_rendered_post_error(error: CreateRenderedPostError) -> InternalError {
    match error {
        CreateRenderedPostError::Storage(CreatePostError::SlugConflict) => {
            InternalError::conflict("slug already taken for this user on this date")
        }
        CreateRenderedPostError::Storage(CreatePostError::Internal(error)) => {
            InternalError::storage(error)
        }
        CreateRenderedPostError::Render(error) => InternalError::server(error),
    }
}

/// Soft-deletes a post owned by the authenticated user.
#[server(endpoint = "/delete_post")]
pub async fn delete_post(post_id: i64) -> WebResult<()> {
    crate::web_server_fn!("delete_post", post_id => {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();

        let existing = state
            .posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        state
            .posts
            .soft_delete_post(post_id)
            .await
            .map_err(InternalError::storage)
    })
}

#[cfg(feature = "ssr")]
fn build_permalink(username: &Username, timestamp: chrono::DateTime<Utc>, slug: &Slug) -> String {
    format!(
        "/~{}/{:04}/{:02}/{:02}/{}",
        username.as_str(),
        timestamp.year(),
        timestamp.month(),
        timestamp.day(),
        slug.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::candidate_slug;

    #[cfg(feature = "ssr")]
    use super::{
        build_permalink, fallback_summary_label, parse_post_cursor, post_response,
        timeline_post_summary,
    };
    #[cfg(feature = "ssr")]
    use chrono::{TimeZone, Utc};
    #[cfg(feature = "ssr")]
    use common::{
        slug::Slug,
        storage::{PostFormat, PostRecord},
        username::Username,
    };

    #[test]
    fn candidate_slug_returns_seed_for_first_attempt() {
        assert_eq!(candidate_slug("hello-world", 0), "hello-world");
    }

    #[test]
    fn candidate_slug_appends_numeric_suffix_after_conflict() {
        assert_eq!(candidate_slug("hello-world", 1), "hello-world-2");
        assert_eq!(candidate_slug("hello-world", 2), "hello-world-3");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn build_permalink_formats_username_date_and_slug() {
        let username = "author".parse::<Username>().unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();
        let timestamp = Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap();

        let permalink = build_permalink(&username, timestamp, &slug);

        assert_eq!(permalink, "/~author/2026/04/12/hello-world");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn parse_post_cursor_accepts_empty_cursor() {
        let cursor = parse_post_cursor(None, None).unwrap();
        assert!(cursor.is_none());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn fallback_summary_label_prefers_body_then_title_then_slug() {
        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();

        let body_label = fallback_summary_label(&PostRecord {
            post_id: 1,
            user_id: 2,
            title: Some("Stored Title".to_string()),
            slug: slug.clone(),
            body: "\nBody label\nmore".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Body label</p>".to_string(),
            created_at: base_time,
            updated_at: base_time,
            published_at: None,
            deleted_at: None,
        });
        assert_eq!(body_label, "Body label");

        let title_label = fallback_summary_label(&PostRecord {
            post_id: 1,
            user_id: 2,
            title: Some("Stored Title".to_string()),
            slug: slug.clone(),
            body: "".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "".to_string(),
            created_at: base_time,
            updated_at: base_time,
            published_at: None,
            deleted_at: None,
        });
        assert_eq!(title_label, "Stored Title");

        let slug_label = fallback_summary_label(&PostRecord {
            post_id: 1,
            user_id: 2,
            title: None,
            slug,
            body: "".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "".to_string(),
            created_at: base_time,
            updated_at: base_time,
            published_at: None,
            deleted_at: None,
        });
        assert_eq!(slug_label, "hello-world");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn timeline_post_summary_keeps_titleless_posts_titleless() {
        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let username = "author".parse::<Username>().unwrap();
        let slug = "titleless-note".parse::<Slug>().unwrap();

        let summary = timeline_post_summary(
            &username,
            PostRecord {
                post_id: 1,
                user_id: 2,
                title: None,
                slug,
                body: "Titleless note".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Titleless note</p>".to_string(),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
            },
        )
        .expect("published post should summarize");

        assert_eq!(summary.title, None);
        assert_eq!(summary.permalink, "/~author/2026/04/16/titleless-note");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn post_response_marks_draft_state_from_published_at() {
        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();

        let draft = post_response(
            PostRecord {
                post_id: 1,
                user_id: 2,
                title: Some("Draft".to_string()),
                slug: slug.clone(),
                body: "body".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                created_at: base_time,
                updated_at: base_time,
                published_at: None,
                deleted_at: None,
            },
            "author".to_string(),
            true,
        );
        assert!(draft.is_draft);
        assert!(draft.published_at.is_none());

        let published = post_response(
            PostRecord {
                post_id: 2,
                user_id: 2,
                title: Some("Published".to_string()),
                slug,
                body: "body".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
            },
            "author".to_string(),
            false,
        );
        assert!(!published.is_draft);
        assert!(published.published_at.is_some());
    }
}
