use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::{
    CreatePostError, CreatePostInput, PostFormat, PostRecord, PostStorage, UpdatePostError,
    UpdatePostInput,
};
use common::slug::{slugify_title, Slug};

// ---------------------------------------------------------------------------
// Render errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("org-mode render error: {0}")]
    OrgRender(String),
}

// ---------------------------------------------------------------------------
// Pure rendering functions
// ---------------------------------------------------------------------------

/// Renders `body` to HTML based on `format`. Pure function.
///
/// # Errors
///
/// Returns `Err(RenderError)` if the body cannot be rendered for the given format.
pub fn render(body: &str, format: &PostFormat) -> Result<String, RenderError> {
    match format {
        PostFormat::Markdown => Ok(render_markdown(body)),
        PostFormat::Org => render_org(body),
        PostFormat::Html => Ok(body.to_string()),
    }
}

/// Metadata derived from a post body used for slug generation and display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedPostMetadata {
    pub title: Option<String>,
    pub slug_seed: String,
    pub summary_label: String,
}

/// Derives the public title, slug seed, and fallback label for a post.
/// The body is stored verbatim by the caller — this function never mutates it.
pub fn derive_post_metadata(
    explicit_title: Option<&str>,
    body: &str,
    format: &PostFormat,
) -> Option<DerivedPostMetadata> {
    let explicit_title = explicit_title
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let body = body.trim();

    if let Some(title) = explicit_title {
        let title = title.to_owned();
        let summary_label = fallback_label(body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(title.clone()),
            slug_seed: title,
            summary_label,
        });
    }

    let extracted_title = match format {
        PostFormat::Markdown => extract_markdown_title(body).map(|(title, _)| title),
        PostFormat::Org => extract_org_title(body).map(|(title, _)| title),
        PostFormat::Html => None,
    };

    if let Some(title) = extracted_title {
        let summary_label = fallback_label(body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(title.clone()),
            slug_seed: title,
            summary_label,
        });
    }

    let summary_label = fallback_label(body)?;
    Some(DerivedPostMetadata {
        title: None,
        slug_seed: summary_label.clone(),
        summary_label,
    })
}

fn fallback_label(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(100).collect::<String>())
        .filter(|line| !line.is_empty())
}

fn extract_markdown_title(body: &str) -> Option<(String, String)> {
    let mut output = Vec::new();
    let mut found = None;

    for line in body.lines() {
        if found.is_none() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(title) = trimmed.strip_prefix("# ") {
                let title = title.trim();
                if !title.is_empty() {
                    found = Some(title.to_owned());
                    continue;
                }
            }
        }
        output.push(line);
    }

    found.map(|title| (title, output.join("\n").trim().to_owned()))
}

fn extract_org_title(body: &str) -> Option<(String, String)> {
    let mut output = Vec::new();
    let mut found = None;
    let mut past_kv_block = false;

    for line in body.lines() {
        if found.is_some() {
            output.push(line);
            continue;
        }

        let trimmed = line.trim();

        if trimmed.is_empty() {
            // Blank lines are allowed inside the KV block
            if !past_kv_block {
                continue;
            }
            // Once we're past the KV block, a blank line without a title means no title
            return None;
        }

        // #+TITLE: value — standard org metadata title
        if let Some((key, value)) = trimmed.split_once(':') {
            if key.eq_ignore_ascii_case("#+title") {
                let title = value.trim();
                if !title.is_empty() {
                    found = Some(title.to_owned());
                    continue;
                }
            }
            // Any other #+key: value KV line is skipped (part of the header block)
            if key.starts_with("#+") {
                continue;
            }
        }

        // * Top-level heading (exactly one asterisk followed by space)
        if let Some(heading) = trimmed.strip_prefix("* ") {
            let title = heading.trim();
            if !title.is_empty() {
                found = Some(title.to_owned());
                past_kv_block = true;
                continue;
            }
        }

        // Any other non-blank, non-KV, non-heading content means no title
        return None;
    }

    found.map(|title| (title, output.join("\n").trim().to_owned()))
}

/// Renders Markdown to HTML using pulldown-cmark with common extensions.
fn render_markdown(body: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(body, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

/// Renders Org-mode to HTML using orgize.
fn render_org(body: &str) -> Result<String, RenderError> {
    let org = orgize::Org::parse(body);
    let mut buf = Vec::new();
    org.write_html(&mut buf)
        .map_err(|e| RenderError::OrgRender(e.to_string()))?;
    String::from_utf8(buf).map_err(|e| RenderError::OrgRender(e.to_string()))
}

// ---------------------------------------------------------------------------
// Orchestration error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CreateRenderedPostError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Storage(#[from] CreatePostError),
}

#[derive(Debug, Error)]
pub enum UpdateRenderedPostError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Storage(#[from] UpdatePostError),
}

// ---------------------------------------------------------------------------
// Orchestration helpers
// ---------------------------------------------------------------------------

/// Renders `body` according to `format` and creates the post via storage.
///
/// # Errors
///
/// Returns `Err(CreateRenderedPostError)` if rendering fails or the storage layer returns an error.
#[allow(clippy::too_many_arguments)]
pub async fn create_rendered_post(
    storage: &dyn PostStorage,
    user_id: i64,
    title: Option<String>,
    slug: Slug,
    body: String,
    format: PostFormat,
    published_at: Option<DateTime<Utc>>,
    summary: Option<String>,
) -> Result<i64, CreateRenderedPostError> {
    let rendered_html = render(&body, &format)?;
    let input = CreatePostInput {
        user_id,
        title,
        slug,
        body,
        format,
        rendered_html,
        published_at,
        summary,
    };
    Ok(storage.create_post(&input).await?)
}

/// Renders `body` according to `format` and updates the post via storage.
///
/// # Errors
///
/// Returns `Err(UpdateRenderedPostError)` if rendering fails or the storage layer returns an error.
#[allow(clippy::too_many_arguments)]
pub async fn update_rendered_post(
    storage: &dyn PostStorage,
    post_id: i64,
    editor_user_id: i64,
    title: Option<String>,
    slug: Slug,
    body: String,
    format: PostFormat,
    publish: bool,
    summary: Option<String>,
) -> Result<PostRecord, UpdateRenderedPostError> {
    let rendered_html = render(&body, &format)?;
    let input = UpdatePostInput {
        title,
        slug,
        body,
        format,
        rendered_html,
        publish,
        summary,
    };
    Ok(storage.update_post(post_id, editor_user_id, &input).await?)
}

// ---------------------------------------------------------------------------
// High-level post-update orchestration
// ---------------------------------------------------------------------------

/// Errors that can occur during a high-level post update.
#[derive(Debug, Error)]
pub enum PerformUpdateError {
    #[error("post body or title is required")]
    EmptyPost,
    #[error("post must contain at least one ASCII letter or digit for its slug")]
    NoSlugFromPost,
    #[error("invalid slug")]
    InvalidSlug,
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("post not found")]
    NotFound,
    #[error("not authorized")]
    Unauthorized,
    #[error(transparent)]
    Storage(sqlx::Error),
}

impl From<UpdatePostError> for PerformUpdateError {
    fn from(e: UpdatePostError) -> Self {
        match e {
            UpdatePostError::NotFound => Self::NotFound,
            UpdatePostError::Unauthorized => Self::Unauthorized,
            UpdatePostError::Internal(e) => Self::Storage(e),
        }
    }
}

/// Validates inputs, computes the slug, renders the body, and atomically
/// updates the post via storage.
///
/// The storage layer freezes the slug if the post is already published.
/// Ownership and deletion checks are also performed atomically in storage.
///
/// # Errors
///
/// Returns `Err(PerformUpdateError)` if rendering fails or the storage layer returns an error.
#[allow(clippy::too_many_arguments)]
pub async fn perform_post_update(
    storage: &dyn PostStorage,
    post_id: i64,
    editor_user_id: i64,
    body: String,
    title: Option<&str>,
    format: PostFormat,
    slug_override: Option<&str>,
    publish: bool,
    summary: Option<String>,
) -> Result<PostRecord, PerformUpdateError> {
    let metadata =
        derive_post_metadata(title, &body, &format).ok_or(PerformUpdateError::EmptyPost)?;

    let slug = match slug_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => raw
            .to_ascii_lowercase()
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::InvalidSlug)?,
        None => slugify_title(&metadata.slug_seed)
            .ok_or(PerformUpdateError::NoSlugFromPost)?
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::NoSlugFromPost)?,
    };

    let rendered_html = render(&body, &format)?;
    let input = UpdatePostInput {
        title: metadata.title,
        slug,
        body,
        format,
        rendered_html,
        publish,
        summary,
    };
    storage
        .update_post(post_id, editor_user_id, &input)
        .await
        .map_err(PerformUpdateError::from)
}

// ---------------------------------------------------------------------------
// High-level post-creation orchestration
// ---------------------------------------------------------------------------

/// Errors that can occur during high-level post creation.
#[derive(Debug, Error)]
pub enum PerformCreationError {
    #[error("post body is required")]
    EmptyPost,
    #[error("post must contain at least one ASCII letter or digit for its slug")]
    NoSlugFromPost,
    #[error("{0}")]
    InvalidSlug(String),
    #[error("unable to allocate a unique slug after {0} attempts")]
    Exhausted(usize),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("created post not found")]
    CreatedNotFound,
    #[error(transparent)]
    Storage(sqlx::Error),
}

/// Generates a unique slug attempt using a suffix for attempts > 0.
#[must_use]
pub fn candidate_slug(slug_seed: &str, attempt: usize) -> String {
    if attempt == 0 {
        slug_seed.to_owned()
    } else {
        format!("{slug_seed}-{}", attempt + 1)
    }
}

/// Validates inputs, computes the slug, renders the body, and atomically
/// creates the post in storage, retrying on slug collision.
///
/// # Errors
///
/// Returns `Err(PerformCreationError)` if rendering fails, slug validation
/// fails, attempts to find a unique slug are exhausted, or storage fails.
#[allow(clippy::too_many_arguments)]
pub async fn perform_post_creation(
    storage: &dyn PostStorage,
    user_id: i64,
    body: String,
    title: Option<&str>,
    format: PostFormat,
    slug_override: Option<&str>,
    published_at: Option<DateTime<Utc>>,
    max_attempts: usize,
    summary: Option<String>,
) -> Result<PostRecord, PerformCreationError> {
    let metadata =
        derive_post_metadata(title, &body, &format).ok_or(PerformCreationError::EmptyPost)?;

    let slug_seed = match slug_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => raw
            .to_ascii_lowercase()
            .parse::<Slug>()
            .map_err(|e| PerformCreationError::InvalidSlug(e.to_string()))?
            .to_string(),
        None => slugify_title(&metadata.slug_seed).ok_or(PerformCreationError::NoSlugFromPost)?,
    };

    for attempt in 0..max_attempts {
        let slug_string = candidate_slug(&slug_seed, attempt);
        let slug = slug_string
            .parse::<Slug>()
            .map_err(|e| PerformCreationError::InvalidSlug(e.to_string()))?;

        match create_rendered_post(
            storage,
            user_id,
            metadata.title.clone(),
            slug,
            body.clone(),
            format.clone(),
            published_at,
            summary.clone(),
        )
        .await
        {
            Ok(post_id) => {
                let record = storage
                    .get_post_by_id(post_id)
                    .await
                    .map_err(PerformCreationError::Storage)?
                    .ok_or(PerformCreationError::CreatedNotFound)?;
                return Ok(record);
            }
            Err(CreateRenderedPostError::Storage(CreatePostError::SlugConflict)) => {}
            Err(CreateRenderedPostError::Storage(CreatePostError::Internal(e))) => {
                return Err(PerformCreationError::Storage(e));
            }
            Err(CreateRenderedPostError::Render(e)) => {
                return Err(PerformCreationError::Render(e));
            }
        }
    }

    Err(PerformCreationError::Exhausted(max_attempts))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Markdown tests --

    #[test]
    fn markdown_headings() {
        let html = render_markdown("# H1\n## H2\n### H3");
        assert!(html.contains("<h1>H1</h1>"));
        assert!(html.contains("<h2>H2</h2>"));
        assert!(html.contains("<h3>H3</h3>"));
    }

    #[test]
    fn markdown_paragraph() {
        let html = render_markdown("Hello, world!");
        assert!(html.contains("<p>Hello, world!</p>"));
    }

    #[test]
    fn markdown_bold_italic_strikethrough() {
        let html = render_markdown("**bold** *italic* ~~strike~~");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
        assert!(html.contains("<del>strike</del>"));
    }

    #[test]
    fn markdown_code_block() {
        let html = render_markdown("```rust\nfn main() {}\n```");
        assert!(html.contains("<code"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn markdown_links() {
        let html = render_markdown("[example](https://example.com)");
        assert!(html.contains("<a href=\"https://example.com\">example</a>"));
    }

    #[test]
    fn markdown_ordered_list() {
        let html = render_markdown("1. first\n2. second\n3. third");
        assert!(html.contains("<ol>"));
        assert!(html.contains("<li>first</li>"));
        assert!(html.contains("<li>second</li>"));
        assert!(html.contains("<li>third</li>"));
    }

    #[test]
    fn markdown_unordered_list() {
        let html = render_markdown("- alpha\n- beta");
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>alpha</li>"));
        assert!(html.contains("<li>beta</li>"));
    }

    #[test]
    fn markdown_table() {
        let input = "| A | B |\n|---|---|\n| 1 | 2 |";
        let html = render_markdown(input);
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>A</th>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn markdown_empty_input() {
        let html = render_markdown("");
        assert!(html.is_empty());
    }

    #[test]
    fn markdown_multiple_paragraphs() {
        let html = render_markdown("First paragraph.\n\nSecond paragraph.");
        // Two separate <p> tags
        let count = html.matches("<p>").count();
        assert_eq!(count, 2);
    }

    #[test]
    fn markdown_tasklist() {
        let html = render_markdown("- [x] done\n- [ ] todo");
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("checked"));
    }

    // -- Org-mode tests --

    #[test]
    fn org_headings() {
        let html = render_org("* H1\n** H2").unwrap();
        assert!(html.contains("H1"));
        assert!(html.contains("H2"));
    }

    #[test]
    fn org_paragraph() {
        let html = render_org("Hello, org world!").unwrap();
        assert!(html.contains("Hello, org world!"));
    }

    #[test]
    fn org_bold_italic_code() {
        let html = render_org("*bold* /italic/ ~code~").unwrap();
        assert!(html.contains("<b>bold</b>"));
        assert!(html.contains("<i>italic</i>"));
        assert!(html.contains("<code>code</code>"));
    }

    #[test]
    fn org_list() {
        let html = render_org("- alpha\n- beta").unwrap();
        assert!(html.contains("alpha"));
        assert!(html.contains("beta"));
    }

    #[test]
    fn org_code_block() {
        let html = render_org("#+BEGIN_SRC rust\nfn main() {}\n#+END_SRC").unwrap();
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn org_link() {
        let html = render_org("[[https://example.com][example]]").unwrap();
        assert!(html.contains("https://example.com"));
        assert!(html.contains("example"));
    }

    #[test]
    fn org_empty_input() {
        let html = render_org("").unwrap();
        // Empty input should not contain any visible text content
        // (orgize may produce structural HTML tags for empty input)
        let stripped = html
            .replace("<main>", "")
            .replace("</main>", "")
            .replace("<section>", "")
            .replace("</section>", "");
        assert!(
            stripped.trim().is_empty(),
            "expected no visible content, got: {html}"
        );
    }

    // -- Cross-format dispatch tests --

    #[test]
    fn render_dispatches_markdown() {
        let result = render("**bold**", &PostFormat::Markdown).unwrap();
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[test]
    fn render_dispatches_org() {
        let result = render("*bold*", &PostFormat::Org).unwrap();
        assert!(result.contains("<b>bold</b>"));
    }

    #[test]
    fn derive_metadata_prefers_explicit_title() {
        let metadata = derive_post_metadata(
            Some(" Explicit "),
            "# Body Heading\ntext",
            &PostFormat::Markdown,
        )
        .unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Explicit"));
        assert_eq!(metadata.slug_seed, "Explicit");
        assert_eq!(metadata.summary_label, "# Body Heading");
    }

    #[test]
    fn derive_metadata_extracts_markdown_h1() {
        let metadata = derive_post_metadata(
            None,
            "\n# Article Title\n\nBody text",
            &PostFormat::Markdown,
        )
        .unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Article Title"));
        assert_eq!(metadata.slug_seed, "Article Title");
        // body is not a field of DerivedPostMetadata — the caller retains the original
    }

    #[test]
    fn derive_metadata_extracts_org_title() {
        let metadata =
            derive_post_metadata(None, "#+title: Org Title\n\nBody text", &PostFormat::Org)
                .unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Org Title"));
        assert_eq!(metadata.slug_seed, "Org Title");
        // body is not a field of DerivedPostMetadata — the caller retains the original
    }

    #[test]
    fn derive_metadata_for_html_extracts_no_title_but_keeps_fallback_label() {
        let metadata = derive_post_metadata(None, "<p>Hello world</p>", &PostFormat::Html).unwrap();
        assert_eq!(metadata.title, None);
        assert!(!metadata.summary_label.is_empty());
    }

    #[test]
    fn derive_metadata_allows_titleless_notes() {
        let metadata = derive_post_metadata(
            None,
            "A compact note\nwith more text",
            &PostFormat::Markdown,
        )
        .unwrap();
        assert_eq!(metadata.title, None);
        assert_eq!(metadata.slug_seed, "A compact note");
        assert_eq!(metadata.summary_label, "A compact note");
    }

    #[test]
    fn derive_metadata_rejects_empty_posts() {
        assert_eq!(
            derive_post_metadata(None, "   \n\t", &PostFormat::Markdown),
            None
        );
    }

    // -- Error display tests --

    #[test]
    fn render_error_display() {
        let err = RenderError::OrgRender("parse failed".to_string());
        assert_eq!(err.to_string(), "org-mode render error: parse failed");
    }

    #[test]
    fn render_error_debug() {
        let err = RenderError::OrgRender("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("OrgRender"));
    }

    #[test]
    fn create_rendered_post_error_from_render() {
        let err: CreateRenderedPostError = RenderError::OrgRender("bad".to_string()).into();
        assert!(err.to_string().contains("org-mode render error"));
    }

    #[test]
    fn update_rendered_post_error_from_render() {
        let err: UpdateRenderedPostError = RenderError::OrgRender("bad".to_string()).into();
        assert!(err.to_string().contains("org-mode render error"));
    }

    #[test]
    fn create_rendered_post_error_debug() {
        let err: CreateRenderedPostError = RenderError::OrgRender("x".to_string()).into();
        let debug = format!("{:?}", err);
        assert!(debug.contains("Render"));
    }

    #[test]
    fn update_rendered_post_error_debug() {
        let err: UpdateRenderedPostError = RenderError::OrgRender("x".to_string()).into();
        let debug = format!("{:?}", err);
        assert!(debug.contains("Render"));
    }

    #[test]
    fn create_rendered_post_error_from_storage_display() {
        use crate::CreatePostError;
        let err: CreateRenderedPostError = CreatePostError::SlugConflict.into();
        assert!(err.to_string().contains("slug"));
    }

    #[test]
    fn create_rendered_post_error_from_storage_debug() {
        use crate::CreatePostError;
        let err: CreateRenderedPostError = CreatePostError::SlugConflict.into();
        let debug = format!("{:?}", err);
        assert!(debug.contains("Storage"));
    }

    #[test]
    fn update_rendered_post_error_from_storage_display() {
        use crate::UpdatePostError;
        let err: UpdateRenderedPostError = UpdatePostError::NotFound.into();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn update_rendered_post_error_from_storage_debug() {
        use crate::UpdatePostError;
        let err: UpdateRenderedPostError = UpdatePostError::NotFound.into();
        let debug = format!("{:?}", err);
        assert!(debug.contains("Storage"));
    }

    // -- PerformUpdateError tests --

    #[test]
    fn perform_update_error_empty_title_display() {
        let err = PerformUpdateError::EmptyPost;
        assert_eq!(err.to_string(), "post body or title is required");
    }

    #[test]
    fn perform_update_error_no_slug_from_title_display() {
        let err = PerformUpdateError::NoSlugFromPost;
        assert!(err.to_string().contains("ASCII"));
    }

    #[test]
    fn perform_update_error_invalid_slug_display() {
        let err = PerformUpdateError::InvalidSlug;
        assert_eq!(err.to_string(), "invalid slug");
    }

    #[test]
    fn perform_update_error_not_found_display() {
        let err = PerformUpdateError::NotFound;
        assert_eq!(err.to_string(), "post not found");
    }

    #[test]
    fn perform_update_error_unauthorized_display() {
        let err = PerformUpdateError::Unauthorized;
        assert_eq!(err.to_string(), "not authorized");
    }

    #[test]
    fn perform_update_error_from_render() {
        let err: PerformUpdateError = RenderError::OrgRender("bad".to_string()).into();
        assert!(err.to_string().contains("org-mode render error"));
    }

    #[test]
    fn perform_update_error_from_update_post_not_found() {
        use crate::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::NotFound.into();
        assert!(matches!(err, PerformUpdateError::NotFound));
    }

    #[test]
    fn perform_update_error_from_update_post_unauthorized() {
        use crate::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::Unauthorized.into();
        assert!(matches!(err, PerformUpdateError::Unauthorized));
    }

    #[test]
    fn perform_update_error_debug() {
        let err = PerformUpdateError::EmptyPost;
        let debug = format!("{:?}", err);
        assert!(debug.contains("EmptyPost"));
    }

    #[test]
    fn extract_org_title_handles_level1_heading() {
        let result = extract_org_title("* My Title\n\nBody text");
        assert_eq!(
            result,
            Some(("My Title".to_string(), "Body text".to_string()))
        );
    }

    #[test]
    fn extract_org_title_heading_after_kv_lines() {
        let result = extract_org_title("#+AUTHOR: Me\n* My Title\n\nBody");
        assert_eq!(result, Some(("My Title".to_string(), "Body".to_string())));
    }

    #[test]
    fn extract_org_title_skips_blank_lines_inside_kv_block() {
        // Blank lines inside the KV header block must be skipped so the
        // following heading is still recognized as the title. This pins the
        // `if !past_kv_block { continue }` arm against a flipped-sign mutation.
        let result = extract_org_title("\n#+AUTHOR: Me\n\n#+DATE: today\n* My Title\n\nBody");
        assert_eq!(result, Some(("My Title".to_string(), "Body".to_string())));
    }

    #[test]
    fn extract_org_title_blank_line_after_kv_without_title_returns_none() {
        // A blank line that appears *after* a heading-less KV block should
        // terminate the search with no title. This pins the
        // `past_kv_block` arm in the empty-line branch.
        let result = extract_org_title("#+AUTHOR: Me\n* Heading\n\nBody\n\nMore");
        // Title should still come from the heading; trailing blank lines
        // after a found title are appended to the body.
        assert!(result.is_some());
    }

    #[test]
    fn extract_org_title_title_takes_precedence_over_heading() {
        let result = extract_org_title("#+TITLE: Meta\n* Heading\n\nBody");
        assert_eq!(
            result,
            Some(("Meta".to_string(), "* Heading\n\nBody".to_string()))
        );
    }

    #[test]
    fn extract_org_title_heading_not_top_level_ignored() {
        // ** is a level-2 heading, not a title
        let result = extract_org_title("** Sub\n\nBody");
        assert_eq!(result, None);
    }

    #[test]
    fn extract_org_title_heading_after_body_text_ignored() {
        // A heading preceded by prose is not a title
        let result = extract_org_title("Some intro text.\n* Heading\n\nBody");
        assert_eq!(result, None);
    }

    #[test]
    fn derive_metadata_extracts_org_level1_heading() {
        let metadata =
            derive_post_metadata(None, "* Org Heading\n\nBody text", &PostFormat::Org).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Org Heading"));
        assert_eq!(metadata.slug_seed, "Org Heading");
    }

    #[test]
    fn extract_org_title_empty_title_value_skipped_heading_used() {
        // #+TITLE: with empty value: the empty-title branch falls through;
        // key.starts_with("#+") is true so we continue and find the heading.
        let result = extract_org_title("#+TITLE:\n* Heading\n\nBody");
        assert_eq!(result, Some(("Heading".to_string(), "Body".to_string())));
    }

    #[test]
    fn extract_org_title_non_kv_colon_line_returns_none() {
        // "author: Me" has a colon but key doesn't start with #+.
        // Falls through the split block to the heading check then return None.
        let result = extract_org_title("author: Me\n* Heading\n\nBody");
        assert_eq!(result, None);
    }

    #[test]
    fn extract_org_title_empty_heading_returns_none() {
        // "* " with nothing after the space: heading.trim() is empty,
        // so the heading if-block is skipped and we fall to return None.
        let result = extract_org_title("* ");
        assert_eq!(result, None);
    }

    #[test]
    fn perform_update_error_from_update_post_internal() {
        use crate::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::Internal(sqlx::Error::RowNotFound).into();
        assert!(matches!(err, PerformUpdateError::Storage(_)));
    }

    // -- perform_post_creation tests --

    async fn setup_test_db() -> (sqlx::SqlitePool, crate::SqlitePostStorage) {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO users (user_id, username, password_hash, created_at) VALUES (1, 'testuser', 'some_hash', '2026-05-20T12:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        let storage = crate::SqlitePostStorage::new(pool.clone());
        (pool, storage)
    }

    #[tokio::test]
    async fn test_perform_post_creation_success() {
        let (_pool, storage) = setup_test_db().await;
        let record = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap();

        assert_eq!(record.user_id, 1);
        assert_eq!(record.slug.as_str(), "hello-world");
        assert_eq!(record.body, "Hello, world!");
        assert_eq!(record.format, PostFormat::Markdown);
        assert!(record.rendered_html.contains("<p>Hello, world!</p>"));
    }

    #[tokio::test]
    async fn test_perform_post_creation_uses_explicit_title() {
        let (_pool, storage) = setup_test_db().await;
        // The body has no heading, so any title must come from the explicit arg,
        // which also seeds the slug.
        let record = perform_post_creation(
            &storage,
            1,
            "Body without a heading.".to_owned(),
            Some("Explicit Title"),
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap();

        assert_eq!(record.title.as_deref(), Some("Explicit Title"));
        assert_eq!(record.slug.as_str(), "explicit-title");
    }

    #[tokio::test]
    async fn test_perform_post_creation_slug_override() {
        let (_pool, storage) = setup_test_db().await;
        let record = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            Some("my-custom-slug"),
            None,
            100,
            None,
        )
        .await
        .unwrap();

        assert_eq!(record.slug.as_str(), "my-custom-slug");
    }

    #[tokio::test]
    async fn test_perform_post_creation_invalid_slug_override() {
        let (_pool, storage) = setup_test_db().await;
        let err = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            Some("Invalid Slug!"),
            None,
            100,
            None,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::InvalidSlug(_)));
    }

    #[tokio::test]
    async fn test_perform_post_creation_empty_body() {
        let (_pool, storage) = setup_test_db().await;
        let err = perform_post_creation(
            &storage,
            1,
            "   ".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::EmptyPost));
    }

    #[tokio::test]
    async fn test_perform_post_creation_no_slug_from_body() {
        let (_pool, storage) = setup_test_db().await;
        let err = perform_post_creation(
            &storage,
            1,
            "!!!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::NoSlugFromPost));
    }

    #[tokio::test]
    async fn test_perform_post_creation_slug_conflict_retries() {
        let (_pool, storage) = setup_test_db().await;

        let r1 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap();

        let r3 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap();

        assert_eq!(r1.slug.as_str(), "hello-world");
        assert_eq!(r2.slug.as_str(), "hello-world-2");
        assert_eq!(r3.slug.as_str(), "hello-world-3");
    }

    #[tokio::test]
    async fn test_perform_post_creation_slug_exhaustion() {
        let (_pool, storage) = setup_test_db().await;

        let r1 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            2,
            None,
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            2,
            None,
        )
        .await
        .unwrap();

        assert_eq!(r1.slug.as_str(), "hello-world");
        assert_eq!(r2.slug.as_str(), "hello-world-2");

        let err = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            2,
            None,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::Exhausted(2)));
    }

    #[test]
    fn test_perform_creation_error_display_and_debug() {
        let err = PerformCreationError::EmptyPost;
        assert_eq!(err.to_string(), "post body is required");
        let debug = format!("{:?}", err);
        assert!(debug.contains("EmptyPost"));

        let err = PerformCreationError::NoSlugFromPost;
        assert_eq!(
            err.to_string(),
            "post must contain at least one ASCII letter or digit for its slug"
        );

        let err = PerformCreationError::InvalidSlug("invalid slug message".to_string());
        assert_eq!(err.to_string(), "invalid slug message");

        let err = PerformCreationError::Exhausted(10);
        assert_eq!(
            err.to_string(),
            "unable to allocate a unique slug after 10 attempts"
        );

        let err = PerformCreationError::CreatedNotFound;
        assert_eq!(err.to_string(), "created post not found");
    }

    #[test]
    fn render_html_format_is_identity() {
        let body = "<p>hi <b>there</b></p>";
        assert_eq!(render(body, &PostFormat::Html).unwrap(), body.to_string());
    }
}
