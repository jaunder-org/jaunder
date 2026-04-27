use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::slug::{slugify_title, Slug};
use crate::storage::{
    CreatePostError, CreatePostInput, PostFormat, PostRecord, PostStorage, UpdatePostError,
    UpdatePostInput,
};

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
pub fn render(body: &str, format: &PostFormat) -> Result<String, RenderError> {
    match format {
        PostFormat::Markdown => Ok(render_markdown(body)),
        PostFormat::Org => render_org(body),
    }
}

/// Public post metadata derived from the explicit title and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedPostMetadata {
    pub title: Option<String>,
    pub body: String,
    pub slug_seed: String,
    pub summary_label: String,
}

/// Derives the public title, stored body, slug seed, and fallback label for a post.
pub fn derive_post_metadata(
    explicit_title: Option<&str>,
    body: &str,
    format: &PostFormat,
) -> Option<DerivedPostMetadata> {
    let explicit_title = explicit_title
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let body = body.trim().to_owned();

    if let Some(title) = explicit_title {
        let title = title.to_owned();
        let summary_label = fallback_label(&body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(title.clone()),
            body,
            slug_seed: title,
            summary_label,
        });
    }

    let extracted = match format {
        PostFormat::Markdown => extract_markdown_title(&body),
        PostFormat::Org => extract_org_title(&body),
    };

    if let Some((title, body)) = extracted {
        let summary_label = fallback_label(&body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(title.clone()),
            body,
            slug_seed: title,
            summary_label,
        });
    }

    let summary_label = fallback_label(&body)?;
    Some(DerivedPostMetadata {
        title: None,
        body,
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

    for line in body.lines() {
        if found.is_none() {
            let trimmed = line.trim();
            if let Some((key, value)) = trimmed.split_once(':') {
                if key.eq_ignore_ascii_case("#+title") {
                    let title = value.trim();
                    if !title.is_empty() {
                        found = Some(title.to_owned());
                        continue;
                    }
                }
            }
        }
        output.push(line);
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
#[allow(clippy::too_many_arguments)]
pub async fn create_rendered_post(
    storage: &dyn PostStorage,
    user_id: i64,
    title: Option<String>,
    slug: Slug,
    body: String,
    format: PostFormat,
    published_at: Option<DateTime<Utc>>,
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
    };
    Ok(storage.create_post(&input).await?)
}

/// Renders `body` according to `format` and updates the post via storage.
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
) -> Result<PostRecord, UpdateRenderedPostError> {
    let rendered_html = render(&body, &format)?;
    let input = UpdatePostInput {
        title,
        slug,
        body,
        format,
        rendered_html,
        publish,
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
#[allow(clippy::too_many_arguments)]
pub async fn perform_post_update(
    storage: &dyn PostStorage,
    post_id: i64,
    editor_user_id: i64,
    title: Option<String>,
    body: String,
    format: PostFormat,
    slug_override: Option<&str>,
    publish: bool,
) -> Result<PostRecord, PerformUpdateError> {
    let metadata = derive_post_metadata(title.as_deref(), &body, &format)
        .ok_or(PerformUpdateError::EmptyPost)?;

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

    let rendered_html = render(&metadata.body, &format)?;
    let input = UpdatePostInput {
        title: metadata.title,
        slug,
        body: metadata.body,
        format,
        rendered_html,
        publish,
    };
    storage
        .update_post(post_id, editor_user_id, &input)
        .await
        .map_err(PerformUpdateError::from)
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
        assert_eq!(metadata.body, "# Body Heading\ntext");
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
        assert_eq!(metadata.body, "Body text");
        assert_eq!(metadata.summary_label, "Body text");
    }

    #[test]
    fn derive_metadata_extracts_org_title() {
        let metadata =
            derive_post_metadata(None, "#+title: Org Title\n\nBody text", &PostFormat::Org)
                .unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Org Title"));
        assert_eq!(metadata.slug_seed, "Org Title");
        assert_eq!(metadata.body, "Body text");
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
        assert_eq!(metadata.body, "A compact note\nwith more text");
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
        use crate::storage::CreatePostError;
        let err: CreateRenderedPostError = CreatePostError::SlugConflict.into();
        assert!(err.to_string().contains("slug"));
    }

    #[test]
    fn create_rendered_post_error_from_storage_debug() {
        use crate::storage::CreatePostError;
        let err: CreateRenderedPostError = CreatePostError::SlugConflict.into();
        let debug = format!("{:?}", err);
        assert!(debug.contains("Storage"));
    }

    #[test]
    fn update_rendered_post_error_from_storage_display() {
        use crate::storage::UpdatePostError;
        let err: UpdateRenderedPostError = UpdatePostError::NotFound.into();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn update_rendered_post_error_from_storage_debug() {
        use crate::storage::UpdatePostError;
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
        use crate::storage::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::NotFound.into();
        assert!(matches!(err, PerformUpdateError::NotFound));
    }

    #[test]
    fn perform_update_error_from_update_post_unauthorized() {
        use crate::storage::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::Unauthorized.into();
        assert!(matches!(err, PerformUpdateError::Unauthorized));
    }

    #[test]
    fn perform_update_error_debug() {
        let err = PerformUpdateError::EmptyPost;
        let debug = format!("{:?}", err);
        assert!(debug.contains("EmptyPost"));
    }
}
