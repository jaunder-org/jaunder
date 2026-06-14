use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
mod server;
#[cfg(feature = "ssr")]
use server::{
    apply_post_tag_diff, find_draft_by_permalink_for_user, list_by_tag_rows, not_found_error,
    parse_post_cursor, perform_creation_error, perform_update_error, post_response,
    private_post_not_found_error, timeline_post_summary, to_post_cursor,
};

use crate::error::WebResult;
use crate::tags::TagSummary;

// SSR-only imports for #[server] bodies
#[cfg(feature = "ssr")]
use {
    crate::auth::{require_auth, AuthUser},
    crate::error::InternalError,
    crate::feed_events::enqueue_feed_events,
    chrono::{NaiveDate, Utc},
    common::{slug::Slug, tag::Tag, username::Username},
    leptos_axum,
    std::{collections::BTreeSet, sync::Arc},
    storage::{
        get_default_post_format as storage_get_default_post_format, perform_post_creation,
        perform_post_update, set_default_post_format as storage_set_default_post_format,
        FeedEventStorage, PerformUpdateError, PostFormat, PostStorage, UpdatePostError,
        UpdatePostInput, UserConfigStorage, UserStorage,
    },
};

/// Normalizes an optional summary string: empty or whitespace-only strings become `None`.
#[cfg(feature = "ssr")]
fn normalize_summary(s: Option<String>) -> Option<String> {
    s.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Result returned by [`create_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatePostResult {
    pub post_id: i64,
    pub slug: String,
    pub created_at: String,
    pub published_at: Option<String>,
    pub preview_url: String,
    pub permalink: Option<String>,
    pub summary: Option<String>,
}

/// Result returned by [`update_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdatePostResult {
    pub post_id: i64,
    pub slug: String,
    pub published_at: Option<String>,
    pub preview_url: String,
    pub permalink: Option<String>,
    pub summary: Option<String>,
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
    pub summary: Option<String>,
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
    /// Optional summary/excerpt of the post.
    pub summary: Option<String>,
}

/// Creates a post for the authenticated user.
#[server(endpoint = "/create_post", input = Json)]
pub async fn create_post(
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
    tags: Option<Vec<String>>,
    summary: Option<String>,
) -> WebResult<CreatePostResult> {
    boundary!("create_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let validated_tags = common::tag::parse_and_validate_tags(tags.unwrap_or_default())
            .map_err(InternalError::validation)?;

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let published_at = publish.then(Utc::now);
        let normalized_summary = normalize_summary(summary);

        let record = perform_post_creation(
            posts.as_ref(),
            auth.user_id,
            body,
            None,
            format,
            slug_override.as_deref(),
            published_at,
            100,
            normalized_summary,
        )
        .await
        .map_err(perform_creation_error)?;

        let created_at = record.created_at.to_rfc3339();
        let published_at_str = record.published_at.map(|timestamp| timestamp.to_rfc3339());
        // Only published posts have a public permalink. For drafts, the permalink is None.
        let permalink = record.published_at.is_some().then(|| record.permalink());
        let preview_url = format!("/draft/{}/preview", record.post_id);

        let created = CreatePostResult {
            post_id: record.post_id,
            slug: record.slug.to_string(),
            created_at,
            published_at: published_at_str,
            preview_url,
            permalink,
            summary: record.summary,
        };

        for display in &validated_tags {
            posts
                .tag_post(created.post_id, display)
                .await
                .map_err(|e| InternalError::server_message(e.to_string()))?;
        }

        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        let tag_post_tags = posts
            .get_tags_for_post(created.post_id)
            .await
            .map_err(InternalError::storage)?;
        let tag_slugs: BTreeSet<Tag> = tag_post_tags.iter().map(|t| t.tag_slug.clone()).collect();
        enqueue_feed_events(feed_events.as_ref(), &auth.username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

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
    boundary!("get_post", {
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let username_parsed = username
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let slug_parsed = slug
            .parse::<Slug>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| InternalError::validation("Invalid permalink"))?;

        if let Some(post) = posts
            .get_post_by_permalink(&username_parsed, year, month, day, &slug_parsed)
            .await
            .map_err(InternalError::storage)?
        {
            let is_author = require_auth()
                .await
                .is_ok_and(|auth| auth.user_id == post.user_id);
            return Ok(post_response(post, is_author));
        }

        let auth = require_auth().await.map_err(private_post_not_found_error)?;
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
    boundary!("get_post_preview", {
        let auth = require_auth().await.map_err(private_post_not_found_error)?;
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
    summary: Option<String>,
) -> WebResult<UpdatePostResult> {
    boundary!("update_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        // Load old tags before mutation to union with new tags
        let old = posts
            .get_post_by_id(post_id)
            .await
            .map_err(InternalError::storage)?;
        let old_tag_slugs: BTreeSet<Tag> = old
            .as_ref()
            .map(|p| p.tags.iter().map(|t| t.tag_slug.clone()).collect())
            .unwrap_or_default();

        // Validate tags up-front so a malformed input rejects before any
        // post mutation lands.
        let new_tags = tags
            .map(|t| common::tag::parse_and_validate_tags(t).map_err(InternalError::validation))
            .transpose()?;

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let normalized_summary = normalize_summary(summary);

        let record = perform_post_update(
            posts.as_ref(),
            post_id,
            auth.user_id,
            body,
            None,
            format,
            slug_override.as_deref(),
            publish,
            normalized_summary,
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

        // Fetch current tags after mutation and union with old tags
        let current_tags = posts
            .get_tags_for_post(post_id)
            .await
            .map_err(InternalError::storage)?;
        let mut all_tag_slugs: BTreeSet<Tag> = old_tag_slugs;
        for tag in current_tags {
            all_tag_slugs.insert(tag.tag_slug);
        }

        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &auth.username, &all_tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        let published_at_str = record.published_at.map(|t| t.to_rfc3339());
        // Only published posts have a public permalink. For drafts, the permalink is None.
        let permalink = record.published_at.is_some().then(|| record.permalink());

        Ok(UpdatePostResult {
            post_id,
            slug: record.slug.to_string(),
            published_at: published_at_str,
            preview_url: format!("/draft/{post_id}/preview"),
            permalink,
            summary: record.summary,
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
    boundary!("list_drafts", {
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
                let permalink = draft.permalink();
                DraftSummary {
                    post_id: draft.post_id,
                    title: draft.title.clone(),
                    summary_label: draft.fallback_summary_label(),
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
    boundary!("publish_post", {
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
                    summary: existing.summary,
                    publish: true,
                },
            )
            .await
            .map_err(|e| match e {
                UpdatePostError::NotFound | UpdatePostError::Unauthorized => {
                    InternalError::not_found("Post")
                }
                UpdatePostError::Internal(error) => InternalError::storage(error),
            })?;

        let published_at = updated
            .published_at
            .ok_or_else(|| InternalError::not_found("Post"))?;

        let tag_slugs: BTreeSet<Tag> = updated.tags.iter().map(|t| t.tag_slug.clone()).collect();
        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &updated.author_username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        Ok(PublishPostResult {
            post_id: updated.post_id,
            slug: updated.slug.to_string(),
            published_at: published_at.to_rfc3339(),
            permalink: updated.permalink(),
        })
    })
}

/// Retrieves the authenticated user's default post format preference.
#[server(endpoint = "/get_default_post_format")]
pub async fn get_default_post_format() -> WebResult<String> {
    boundary!("get_default_post_format", {
        let auth = require_auth().await?;
        let config = expect_context::<Arc<dyn UserConfigStorage>>();
        let format = storage_get_default_post_format(config.as_ref(), auth.user_id)
            .await
            .map_err(InternalError::storage)?;
        Ok(format.to_string())
    })
}

/// Sets the authenticated user's default post format preference.
#[server(endpoint = "/set_default_post_format")]
pub async fn set_default_post_format(format: String) -> WebResult<()> {
    boundary!("set_default_post_format", {
        let auth = require_auth().await?;
        let config = expect_context::<Arc<dyn UserConfigStorage>>();
        let post_format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        storage_set_default_post_format(config.as_ref(), auth.user_id, post_format)
            .await
            .map_err(InternalError::storage)?;
        Ok(())
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
    boundary!("list_user_posts", {
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
    boundary!("list_local_timeline", {
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
    boundary!("list_home_feed", {
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
    boundary!("list_posts_by_tag", {
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
            posts
                .list_posts_by_tag(&tag_slug, cursor.as_ref(), fetch_limit)
                .await,
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
    boundary!("list_user_posts_by_tag", {
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
            posts
                .list_user_posts_by_tag(author.user_id, &tag_slug, cursor.as_ref(), fetch_limit)
                .await,
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

/// Soft-deletes a post owned by the authenticated user.
#[server(endpoint = "/delete_post")]
pub async fn delete_post(post_id: i64) -> WebResult<()> {
    boundary!("delete_post", {
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
            .map_err(InternalError::storage)?;

        // Only enqueue feed events for published posts
        if existing.published_at.is_some() {
            let tag_slugs: BTreeSet<Tag> =
                existing.tags.iter().map(|t| t.tag_slug.clone()).collect();
            let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
            enqueue_feed_events(feed_events.as_ref(), &existing.author_username, &tag_slugs)
                .await
                .map_err(InternalError::storage)?;
        }

        Ok(())
    })
}

/// Reverts a published post owned by the authenticated user back to draft status.
#[server(endpoint = "/unpublish_post")]
pub async fn unpublish_post(post_id: i64) -> WebResult<()> {
    boundary!("unpublish_post", {
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
            .map_err(InternalError::storage)?;

        let tag_slugs: BTreeSet<Tag> = existing.tags.iter().map(|t| t.tag_slug.clone()).collect();
        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &existing.author_username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use storage::candidate_slug;

    #[cfg(feature = "ssr")]
    use super::normalize_summary;

    #[test]
    fn candidate_slug_returns_seed_for_first_attempt() {
        assert_eq!(candidate_slug("hello-world", 0), "hello-world");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn normalize_summary_empty_and_whitespace_become_none() {
        assert_eq!(normalize_summary(None), None);
        assert_eq!(normalize_summary(Some(String::new())), None);
        assert_eq!(normalize_summary(Some("   ".into())), None);
        assert_eq!(
            normalize_summary(Some("hello".into())),
            Some("hello".into())
        );
        assert_eq!(normalize_summary(Some("  hi  ".into())), Some("hi".into()));
    }

    #[test]
    fn candidate_slug_appends_numeric_suffix_after_conflict() {
        assert_eq!(candidate_slug("hello-world", 1), "hello-world-2");
        assert_eq!(candidate_slug("hello-world", 2), "hello-world-3");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn parse_post_cursor_accepts_empty_cursor() {
        use crate::posts::server::parse_post_cursor;

        let cursor = parse_post_cursor(None, None).unwrap();
        assert!(cursor.is_none());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn timeline_post_summary_keeps_titleless_posts_titleless() {
        use crate::posts::server::timeline_post_summary;
        use chrono::{TimeZone, Utc};
        use common::{slug::Slug, username::Username};
        use storage::{PostFormat, PostRecord};

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
                summary: None,
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
        use crate::posts::server::post_response;
        use chrono::{TimeZone, Utc};
        use common::{slug::Slug, username::Username};
        use storage::{PostFormat, PostRecord};

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
                summary: None,
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
                summary: None,
                tags: vec![],
            },
            false,
        );
        assert!(!published.is_draft);
        assert!(published.published_at.is_some());
    }
}
