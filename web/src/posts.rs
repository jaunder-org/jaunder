use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::auth::{require_auth, AuthUser};
use crate::error::WebResult;
#[cfg(feature = "ssr")]
use crate::error::{InternalError, InternalResult, WebError};
use crate::tags::TagSummary;
#[cfg(feature = "ssr")]
use chrono::{Datelike, NaiveDate, Utc};
#[cfg(feature = "ssr")]
use common::{slug::Slug, username::Username};
#[cfg(feature = "ssr")]
use std::sync::Arc;
#[cfg(feature = "ssr")]
use storage::{
    perform_post_update, PerformUpdateError, PostCursor, PostFormat, PostRecord, PostStorage,
    UpdatePostInput, UserStorage,
};

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
    /// True when the viewing user is the post author.
    pub is_author: bool,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
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
    /// Permalink URL for published posts; `None` for drafts.
    pub permalink: Option<String>,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
}

/// Creates a post for the authenticated user.
#[server(endpoint = "/create_post", input = Json)]
pub async fn create_post(
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
    tags: Option<Vec<String>>,
) -> WebResult<CreatePostResult> {
    crate::web_server_fn!("create_post", body, format, slug_override, publish, tags => {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let validated_tags = crate::tags::parse_and_validate_tags(tags.unwrap_or_default())?;

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let published_at = publish.then(Utc::now);

        let record = storage::perform_post_creation(
            posts.as_ref(),
            auth.user_id,
            body,
            format,
            slug_override.as_deref(),
            published_at,
            100,
        )
        .await
        .map_err(perform_creation_error)?;

        let created_at = record.created_at.to_rfc3339();
        let published_at_str = record.published_at.map(|timestamp| timestamp.to_rfc3339());
        let permalink = record
            .published_at
            .map(|ts| build_permalink(&auth.username, ts, &record.slug));
        let preview_url = format!("/draft/{}/preview", record.post_id);

        let created = CreatePostResult {
            post_id: record.post_id,
            slug: record.slug.to_string(),
            created_at,
            published_at: published_at_str,
            preview_url,
            permalink,
        };

        for display in &validated_tags {
            posts
                .tag_post(created.post_id, display)
                .await
                .map_err(|e| InternalError::server_message(e.to_string()))?;
        }

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

        let posts = expect_context::<Arc<dyn PostStorage>>();

        let username_parsed = username
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let slug_parsed = slug.parse::<Slug>().map_err(|e| InternalError::validation(e.to_string()))?;

        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| InternalError::validation("Invalid permalink"))?;

        if let Some(post) = posts
            .get_post_by_permalink(&username_parsed, year, month, day, &slug_parsed)
            .await
            .map_err(InternalError::storage)?
        {
            let is_author = require_auth()
                .await
                .map(|auth| auth.user_id == post.user_id)
                .unwrap_or(false);
            return Ok(post_response(post, is_author));
        }

        let auth = require_auth()
            .await
            .map_err(private_post_not_found_error)?;
        if auth.username != username_parsed {
            return Err(not_found_error());
        }

        let draft = find_draft_by_permalink_for_user(
            posts.as_ref(),
            auth.user_id,
            year,
            month,
            day,
            &slug_parsed,
        )
        .await?
        .ok_or_else(not_found_error)?;

        Ok(post_response(draft, true))
    })
}

/// Retrieves a draft preview for the authenticated author.
#[server(endpoint = "/get_post_preview")]
pub async fn get_post_preview(post_id: i64) -> WebResult<PostResponse> {
    crate::web_server_fn!("get_post_preview", post_id => {
        let auth = require_auth()
            .await
            .map_err(private_post_not_found_error)?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let post = posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(not_found_error)?;

        if post.deleted_at.is_some() || post.user_id != auth.user_id {
            return Err(not_found_error());
        }

        Ok(post_response(post, true))
    })
}

/// Updates an existing post for the authenticated author.
#[server(endpoint = "/update_post", input = Json)]
pub async fn update_post(
    post_id: i64,
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
    tags: Option<Vec<String>>,
) -> WebResult<UpdatePostResult> {
    crate::web_server_fn!("update_post", post_id, body, format, slug_override, publish, tags => {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        // Validate tags up-front so a malformed input rejects before any
        // post mutation lands.
        let new_tags = tags
            .map(crate::tags::parse_and_validate_tags)
            .transpose()?;

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        let record = perform_post_update(
            posts.as_ref(),
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

        if let Some(new_tags) = new_tags {
            apply_post_tag_diff(posts.as_ref(), post_id, &new_tags).await?;
        }

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
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let parsed_cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let drafts = posts
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
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        let updated = posts
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
                storage::UpdatePostError::NotFound
                | storage::UpdatePostError::Unauthorized => InternalError::not_found("Post"),
                storage::UpdatePostError::Internal(error) => InternalError::storage(error),
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
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let username = username
            .trim()
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;

        let viewer_user_id = leptos_axum::extract::<AuthUser>()
            .await
            .ok()
            .map(|a| a.user_id);

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = posts
            .list_published_by_user(&username, cursor.as_ref(), fetch_limit)
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();

        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
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
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer_user_id = leptos_axum::extract::<AuthUser>()
            .await
            .ok()
            .map(|a| a.user_id);

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = posts
            .list_published(cursor.as_ref(), fetch_limit)
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
            .collect();

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
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = posts
            .list_published_by_user(&auth.username, cursor.as_ref(), fetch_limit)
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, Some(auth.user_id)))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// Lists published, non-deleted posts site-wide carrying `tag`.
#[server(endpoint = "/list_posts_by_tag")]
pub async fn list_posts_by_tag(
    tag: String,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    crate::web_server_fn!("list_posts_by_tag", tag, cursor_created_at, cursor_post_id, limit => {
        use common::tag::Tag;

        let posts = expect_context::<Arc<dyn PostStorage>>();
        let tag_slug = tag
            .trim()
            .parse::<Tag>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer_user_id = leptos_axum::extract::<AuthUser>()
            .await
            .ok()
            .map(|a| a.user_id);

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let rows = list_by_tag_rows(
            posts.list_posts_by_tag(&tag_slug, cursor.as_ref(), fetch_limit).await,
        )?;

        let has_more = rows.len() > page_size as usize;
        let mut rows = rows;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[server(endpoint = "/list_user_posts_by_tag")]
pub async fn list_user_posts_by_tag(
    username: String,
    tag: String,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    crate::web_server_fn!("list_user_posts_by_tag", username, tag, cursor_created_at, cursor_post_id, limit => {
        use common::tag::Tag;

        let posts = expect_context::<Arc<dyn PostStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let username = username
            .trim()
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let tag_slug = tag
            .trim()
            .parse::<Tag>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer_user_id = leptos_axum::extract::<AuthUser>()
            .await
            .ok()
            .map(|a| a.user_id);

        let author = users
            .get_user_by_username(&username)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("user"))?;

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let rows = list_by_tag_rows(
            posts.list_user_posts_by_tag(author.user_id, &tag_slug, cursor.as_ref(), fetch_limit).await,
        )?;

        let has_more = rows.len() > page_size as usize;
        let mut rows = rows;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
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
fn timeline_post_summary(
    post: PostRecord,
    viewer_user_id: Option<i64>,
) -> Option<TimelinePostSummary> {
    let PostRecord {
        post_id,
        user_id,
        author_username,
        title,
        slug,
        rendered_html,
        created_at,
        published_at,
        tags,
        ..
    } = post;
    let published_at = published_at?;
    let permalink = build_permalink(&author_username, published_at, &slug);
    Some(TimelinePostSummary {
        post_id,
        username: author_username.to_string(),
        title,
        slug: slug.to_string(),
        rendered_html,
        created_at: created_at.to_rfc3339(),
        published_at: published_at.to_rfc3339(),
        permalink,
        is_author: viewer_user_id == Some(user_id),
        tags: post_tags_to_summaries(tags),
    })
}

#[cfg(feature = "ssr")]
fn post_tags_to_summaries(tags: Vec<storage::PostTag>) -> Vec<TagSummary> {
    tags.into_iter()
        .map(|t| TagSummary {
            slug: t.tag_slug.to_string(),
            display: t.tag_display,
        })
        .collect()
}

#[cfg(feature = "ssr")]
fn list_by_tag_rows(
    result: Result<Vec<PostRecord>, storage::ListByTagError>,
) -> InternalResult<Vec<PostRecord>> {
    match result {
        Ok(rows) => Ok(rows),
        Err(storage::ListByTagError::TagNotFound) => Ok(Vec::new()),
        Err(storage::ListByTagError::Internal(e)) => Err(InternalError::storage(e)),
    }
}

/// Diff the existing tag set against `desired` (a Vec of validated display
/// tokens) and apply the difference: `tag_post` for new entries, `untag_post`
/// for removed entries. Re-applying an existing tag with new display casing
/// is a no-op at the slug level (the storage layer keys on slug); the
/// display casing of the existing row is preserved.
#[cfg(feature = "ssr")]
async fn apply_post_tag_diff(
    posts: &dyn PostStorage,
    post_id: i64,
    desired: &[String],
) -> InternalResult<()> {
    use common::tag::Tag;
    use std::collections::HashSet;
    use std::str::FromStr;

    let existing = posts
        .get_tags_for_post(post_id)
        .await
        .map_err(InternalError::storage)?;
    let existing_slugs: HashSet<String> = existing.iter().map(|t| t.tag_slug.to_string()).collect();
    let desired_slugs: HashSet<String> = desired
        .iter()
        .filter_map(|d| Tag::from_str(d).ok())
        .map(|t| t.to_string())
        .collect();

    // Add: every desired tag whose slug isn't already present.
    for display in desired {
        let Ok(slug) = Tag::from_str(display) else {
            continue;
        };
        if !existing_slugs.contains(&slug.to_string()) {
            posts
                .tag_post(post_id, display)
                .await
                .map_err(|e| InternalError::server_message(e.to_string()))?;
        }
    }
    // Remove: every existing tag whose slug isn't in desired.
    for tag in &existing {
        if !desired_slugs.contains(&tag.tag_slug.to_string()) {
            posts
                .untag_post(post_id, &tag.tag_slug)
                .await
                .map_err(|e| InternalError::server_message(e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(feature = "ssr")]
fn to_post_cursor(post: &PostRecord) -> PostCursor {
    PostCursor {
        created_at: post.created_at,
        post_id: post.post_id,
    }
}

#[cfg(feature = "ssr")]
fn post_response(post: PostRecord, is_author: bool) -> PostResponse {
    use chrono::Datelike;
    let PostRecord {
        post_id,
        author_username,
        title,
        slug,
        body,
        format,
        rendered_html,
        created_at,
        published_at,
        tags,
        ..
    } = post;
    let permalink = published_at.as_ref().map(|t| {
        format!(
            "/~{}/{:04}/{:02}/{:02}/{}",
            author_username.as_str(),
            t.year(),
            t.month(),
            t.day(),
            slug.as_str()
        )
    });
    PostResponse {
        post_id,
        username: author_username.to_string(),
        title,
        slug: slug.to_string(),
        body,
        format: format.to_string(),
        rendered_html,
        created_at: created_at.to_rfc3339(),
        is_draft: published_at.is_none(),
        published_at: published_at.map(|t| t.to_rfc3339()),
        is_author,
        tags: post_tags_to_summaries(tags),
        permalink,
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
    posts: &dyn PostStorage,
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
        let drafts = posts
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
#[allow(clippy::needless_pass_by_value)]
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
fn perform_creation_error(err: storage::PerformCreationError) -> InternalError {
    match err {
        storage::PerformCreationError::EmptyPost => {
            InternalError::validation("post body is required")
        }
        storage::PerformCreationError::NoSlugFromPost => InternalError::validation(
            "post must contain at least one ASCII letter or digit for its slug",
        ),
        storage::PerformCreationError::InvalidSlug(msg) => InternalError::validation(msg),
        storage::PerformCreationError::Exhausted(_) => {
            InternalError::server_message("unable to allocate a unique slug after 100 attempts")
        }
        storage::PerformCreationError::Render(e) => InternalError::validation(e.to_string()),
        storage::PerformCreationError::CreatedNotFound => {
            InternalError::server_message("created post not found")
        }
        storage::PerformCreationError::Storage(e) => InternalError::storage(e),
    }
}

/// Soft-deletes a post owned by the authenticated user.
#[server(endpoint = "/delete_post")]
pub async fn delete_post(post_id: i64) -> WebResult<()> {
    crate::web_server_fn!("delete_post", post_id => {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        posts
            .soft_delete_post(post_id)
            .await
            .map_err(InternalError::storage)
    })
}

/// Reverts a published post owned by the authenticated user back to draft status.
#[server(endpoint = "/unpublish_post")]
pub async fn unpublish_post(post_id: i64) -> WebResult<()> {
    crate::web_server_fn!("unpublish_post", post_id => {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        posts
            .unpublish_post(post_id)
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
    use storage::candidate_slug;

    #[cfg(feature = "ssr")]
    use super::{
        build_permalink, fallback_summary_label, parse_post_cursor, post_response,
        timeline_post_summary,
    };
    #[cfg(feature = "ssr")]
    use chrono::{TimeZone, Utc};
    #[cfg(feature = "ssr")]
    use common::{slug::Slug, username::Username};
    #[cfg(feature = "ssr")]
    use storage::{PostFormat, PostRecord};

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
        let author_username = "author".parse::<Username>().unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();

        let body_label = fallback_summary_label(&PostRecord {
            post_id: 1,
            user_id: 2,
            author_username: author_username.clone(),
            title: Some("Stored Title".to_string()),
            slug: slug.clone(),
            body: "\nBody label\nmore".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Body label</p>".to_string(),
            created_at: base_time,
            updated_at: base_time,
            published_at: None,
            deleted_at: None,
            tags: vec![],
        });
        assert_eq!(body_label, "Body label");

        let title_label = fallback_summary_label(&PostRecord {
            post_id: 1,
            user_id: 2,
            author_username: author_username.clone(),
            title: Some("Stored Title".to_string()),
            slug: slug.clone(),
            body: "".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "".to_string(),
            created_at: base_time,
            updated_at: base_time,
            published_at: None,
            deleted_at: None,
            tags: vec![],
        });
        assert_eq!(title_label, "Stored Title");

        let slug_label = fallback_summary_label(&PostRecord {
            post_id: 1,
            user_id: 2,
            author_username,
            title: None,
            slug,
            body: "".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "".to_string(),
            created_at: base_time,
            updated_at: base_time,
            published_at: None,
            deleted_at: None,
            tags: vec![],
        });
        assert_eq!(slug_label, "hello-world");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn timeline_post_summary_keeps_titleless_posts_titleless() {
        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let slug = "titleless-note".parse::<Slug>().unwrap();

        let summary = timeline_post_summary(
            PostRecord {
                post_id: 1,
                user_id: 2,
                author_username: "author".parse::<Username>().unwrap(),
                title: None,
                slug,
                body: "Titleless note".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Titleless note</p>".to_string(),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
                tags: vec![],
            },
            None,
        )
        .expect("published post should summarize");

        assert_eq!(summary.title, None);
        assert_eq!(summary.username, "author");
        assert_eq!(summary.permalink, "/~author/2026/04/16/titleless-note");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn post_response_marks_draft_state_from_published_at() {
        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = "author".parse::<Username>().unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();

        let draft = post_response(
            PostRecord {
                post_id: 1,
                user_id: 2,
                author_username: author_username.clone(),
                title: Some("Draft".to_string()),
                slug: slug.clone(),
                body: "body".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                created_at: base_time,
                updated_at: base_time,
                published_at: None,
                deleted_at: None,
                tags: vec![],
            },
            true,
        );
        assert!(draft.is_draft);
        assert!(draft.published_at.is_none());
        assert_eq!(draft.username, "author");

        let published = post_response(
            PostRecord {
                post_id: 2,
                user_id: 2,
                author_username,
                title: Some("Published".to_string()),
                slug,
                body: "body".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
                tags: vec![],
            },
            false,
        );
        assert!(!published.is_draft);
        assert!(published.published_at.is_some());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn perform_update_error_maps_each_arm() {
        use super::perform_update_error;
        use crate::error::WebError;
        use storage::PerformUpdateError;

        assert!(matches!(
            perform_update_error(PerformUpdateError::EmptyPost).public(),
            WebError::Validation { .. }
        ));
        assert!(matches!(
            perform_update_error(PerformUpdateError::NoSlugFromPost).public(),
            WebError::Validation { .. }
        ));
        assert!(matches!(
            perform_update_error(PerformUpdateError::InvalidSlug).public(),
            WebError::Validation { .. }
        ));
        assert!(matches!(
            perform_update_error(PerformUpdateError::NotFound).public(),
            WebError::NotFound { .. }
        ));
        assert!(matches!(
            perform_update_error(PerformUpdateError::Unauthorized).public(),
            WebError::NotFound { .. }
        ));
        assert!(matches!(
            perform_update_error(PerformUpdateError::Storage(sqlx::Error::PoolClosed)).public(),
            WebError::Storage { .. }
        ));
        assert!(matches!(
            perform_update_error(PerformUpdateError::Render(storage::RenderError::OrgRender(
                "bad".to_string()
            )))
            .public(),
            WebError::Server { .. }
        ));
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn perform_creation_error_maps_each_arm() {
        use super::perform_creation_error;
        use crate::error::WebError;
        use storage::PerformCreationError;

        assert!(matches!(
            perform_creation_error(PerformCreationError::EmptyPost).public(),
            WebError::Validation { .. }
        ));
        assert!(matches!(
            perform_creation_error(PerformCreationError::NoSlugFromPost).public(),
            WebError::Validation { .. }
        ));
        assert!(matches!(
            perform_creation_error(PerformCreationError::InvalidSlug("invalid".to_string()))
                .public(),
            WebError::Validation { .. }
        ));
        assert!(matches!(
            perform_creation_error(PerformCreationError::Exhausted(5)).public(),
            WebError::Server { .. }
        ));
        assert!(matches!(
            perform_creation_error(PerformCreationError::Render(
                storage::RenderError::OrgRender("bad".to_string())
            ))
            .public(),
            WebError::Validation { .. }
        ));
        assert!(matches!(
            perform_creation_error(PerformCreationError::CreatedNotFound).public(),
            WebError::Server { .. }
        ));
        assert!(matches!(
            perform_creation_error(PerformCreationError::Storage(sqlx::Error::PoolClosed)).public(),
            WebError::Storage { .. }
        ));
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn list_by_tag_rows_maps_each_arm() {
        use super::list_by_tag_rows;
        use storage::ListByTagError;

        let ok = list_by_tag_rows(Ok(vec![]));
        assert!(ok.is_ok());

        let tag_not_found = list_by_tag_rows(Err(ListByTagError::TagNotFound));
        assert!(matches!(tag_not_found, Ok(rows) if rows.is_empty()));

        let internal = list_by_tag_rows(Err(ListByTagError::Internal(sqlx::Error::PoolClosed)));
        assert!(internal.is_err());
    }

    #[cfg(feature = "ssr")]
    mod sqlite_storage_tests {
        use super::super::{apply_post_tag_diff, find_draft_by_permalink_for_user};
        use common::slug::Slug;

        async fn in_memory_post_storage() -> storage::SqlitePostStorage {
            let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
            sqlx::migrate!("../storage/migrations/sqlite")
                .run(&pool)
                .await
                .unwrap();
            storage::SqlitePostStorage::new(pool)
        }

        #[tokio::test]
        async fn apply_post_tag_diff_skips_invalid_tag_display() {
            let posts = in_memory_post_storage().await;
            // "hello_world" contains an underscore → Tag::from_str fails → continue (line 721)
            let result = apply_post_tag_diff(&posts, 1, &["hello_world".to_string()]).await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn find_draft_by_permalink_for_user_returns_none_when_no_drafts() {
            let posts = in_memory_post_storage().await;
            let slug: Slug = "my-draft".parse().unwrap();
            let result = find_draft_by_permalink_for_user(&posts, 1, 2026, 5, 22, &slug).await;
            assert!(matches!(result, Ok(None)));
        }
    }
}
