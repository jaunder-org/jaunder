use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use chrono::{Datelike, NaiveDate, Utc};
#[cfg(feature = "ssr")]
use common::{
    render::{create_rendered_post, perform_post_update, PerformUpdateError},
    slug::{slugify_title, Slug},
    storage::{AppState, CreatePostError, PostFormat, PostRecord},
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

/// Details of a post returned by [`get_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostResponse {
    pub post_id: i64,
    pub username: String,
    pub title: String,
    pub slug: String,
    pub body: String,
    pub format: String,
    pub rendered_html: String,
    pub created_at: String,
    pub published_at: Option<String>,
}

/// Creates a post for the authenticated user.
#[server(endpoint = "/create_post")]
pub async fn create_post(
    title: String,
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
) -> Result<CreatePostResult, ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let title = title.trim().to_owned();
    if title.is_empty() {
        return Err(ServerFnError::new("title is required"));
    }

    let format = format
        .parse::<PostFormat>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let published_at = publish.then(Utc::now);
    let slug_seed = slug_override
        .as_deref()
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .map(|slug| slug.to_ascii_lowercase())
        .map(|slug| slug.parse::<Slug>())
        .transpose()
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .map(|slug| slug.to_string())
        .or_else(|| slugify_title(&title))
        .ok_or_else(|| {
            ServerFnError::new("title must contain at least one ASCII letter or digit")
        })?;

    let created = create_post_with_unique_slug(
        state.as_ref(),
        auth.user_id,
        &auth.username,
        title,
        body,
        format,
        slug_seed,
        published_at,
    )
    .await?;

    Ok(created)
}

/// Retrieves a post by its permalink.
#[server(endpoint = "/get_post")]
pub async fn get_post(
    username: String,
    year: i32,
    month: u32,
    day: u32,
    slug: String,
) -> Result<PostResponse, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use common::slug::Slug;
        use common::username::Username;

        let state = expect_context::<Arc<AppState>>();

        let username_parsed = username
            .parse::<Username>()
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        let slug_parsed = slug
            .parse::<Slug>()
            .map_err(|e| ServerFnError::new(e.to_string()))?;

        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| ServerFnError::new("Invalid permalink"))?;

        let post = state
            .posts
            .get_post_by_permalink(&username_parsed, year, month, day, &slug_parsed)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .ok_or_else(not_found_error)?;

        let PostRecord {
            post_id,
            title,
            slug,
            body,
            format,
            rendered_html,
            created_at,
            published_at,
            deleted_at: _,
            ..
        } = post;

        Ok(PostResponse {
            post_id,
            username: username_parsed.to_string(),
            title,
            slug: slug.to_string(),
            body,
            format: format.to_string(),
            rendered_html,
            created_at: created_at.to_rfc3339(),
            published_at: published_at.map(|t| t.to_rfc3339()),
        })
    }
    #[cfg(not(feature = "ssr"))]
    {
        let _ = (username, year, month, day, slug);
        Err(ServerFnError::new("Not implemented"))
    }
}

/// Retrieves a draft preview for the authenticated author.
#[server(endpoint = "/get_post_preview")]
pub async fn get_post_preview(post_id: i64) -> Result<PostResponse, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let auth = require_auth().await.map_err(|_| not_found_error())?;
        let state = expect_context::<Arc<AppState>>();

        let post = state
            .posts
            .get_post_by_id(post_id)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
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
            published_at: published_at.map(|t| t.to_rfc3339()),
        })
    }
    #[cfg(not(feature = "ssr"))]
    {
        let _ = post_id;
        Err(ServerFnError::new("Not implemented"))
    }
}

/// Updates an existing post for the authenticated author.
#[server(endpoint = "/update_post")]
pub async fn update_post(
    post_id: i64,
    title: String,
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
) -> Result<UpdatePostResult, ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let format = format
        .parse::<PostFormat>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let record = perform_post_update(
        state.posts.as_ref(),
        post_id,
        auth.user_id,
        title,
        body,
        format,
        slug_override.as_deref(),
        publish,
    )
    .await
    .map_err(|e| match e {
        PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
            ServerFnError::new("Post not found")
        }
        other => ServerFnError::new(other.to_string()),
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
}

#[cfg(feature = "ssr")]
#[allow(clippy::too_many_arguments)]
async fn create_post_with_unique_slug(
    state: &AppState,
    user_id: i64,
    username: &Username,
    title: String,
    body: String,
    format: PostFormat,
    slug_seed: String,
    published_at: Option<chrono::DateTime<Utc>>,
) -> Result<CreatePostResult, ServerFnError> {
    for attempt in 0..100 {
        let slug_string = candidate_slug(&slug_seed, attempt);
        let slug = slug_string
            .parse::<Slug>()
            .map_err(|e| ServerFnError::new(e.to_string()))?;

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
                    .map_err(|e| ServerFnError::new(e.to_string()))?
                    .ok_or_else(|| ServerFnError::new("created post not found"))?;

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
            Err(err) => return Err(ServerFnError::new(err.to_string())),
        }
    }

    Err(ServerFnError::new(
        "unable to allocate a unique slug after 100 attempts",
    ))
}

fn candidate_slug(slug_seed: &str, attempt: usize) -> String {
    if attempt == 0 {
        slug_seed.to_owned()
    } else {
        format!("{slug_seed}-{}", attempt + 1)
    }
}

#[cfg(feature = "ssr")]
fn not_found_error() -> ServerFnError {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    if let Some(opts) = use_context::<ResponseOptions>() {
        opts.set_status(axum::http::StatusCode::NOT_FOUND);
    }

    ServerFnError::new("Post not found")
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
    use super::build_permalink;
    #[cfg(feature = "ssr")]
    use chrono::{TimeZone, Utc};
    #[cfg(feature = "ssr")]
    use common::{slug::Slug, username::Username};

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
}
