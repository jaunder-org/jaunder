use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use chrono::Utc;
#[cfg(feature = "ssr")]
use common::{
    render::create_rendered_post,
    slug::Slug,
    storage::{AppState, CreatePostError, PostFormat},
};
#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Result returned by [`create_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatePostResult {
    pub post_id: i64,
    pub slug: String,
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
        title,
        body,
        format,
        slug_seed,
        published_at,
    )
    .await?;

    Ok(created)
}

#[cfg(feature = "ssr")]
async fn create_post_with_unique_slug(
    state: &AppState,
    user_id: i64,
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
                return Ok(CreatePostResult {
                    post_id,
                    slug: slug_string,
                    published_at: published_at.map(|timestamp| timestamp.to_rfc3339()),
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

fn slugify_title(title: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_dash = false;
        } else if !slug.is_empty() && !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

fn candidate_slug(slug_seed: &str, attempt: usize) -> String {
    if attempt == 0 {
        slug_seed.to_owned()
    } else {
        format!("{slug_seed}-{}", attempt + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::{candidate_slug, slugify_title};

    #[test]
    fn slugify_title_lowercases_and_separates_words() {
        assert_eq!(
            slugify_title("Hello, World from Rust"),
            Some("hello-world-from-rust".to_string())
        );
    }

    #[test]
    fn slugify_title_trims_non_alphanumeric_boundaries() {
        assert_eq!(slugify_title("  ---Hello!!!  "), Some("hello".to_string()));
    }

    #[test]
    fn slugify_title_rejects_titles_without_ascii_alphanumerics() {
        assert_eq!(slugify_title("!!!"), None);
    }

    #[test]
    fn candidate_slug_returns_seed_for_first_attempt() {
        assert_eq!(candidate_slug("hello-world", 0), "hello-world");
    }

    #[test]
    fn candidate_slug_appends_numeric_suffix_after_conflict() {
        assert_eq!(candidate_slug("hello-world", 1), "hello-world-2");
        assert_eq!(candidate_slug("hello-world", 2), "hello-world-3");
    }
}
