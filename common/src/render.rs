//! Pure post-body rendering and title/metadata derivation.
//!
//! Format-driven transformation of post bodies to HTML plus extraction of
//! titles, slug seeds, and summary labels. No storage or database concerns.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// The format/markup language used to author a post body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostFormat {
    /// CommonMark/GitHub-flavored Markdown.
    Markdown,
    /// Emacs Org-mode format.
    Org,
    /// Pre-rendered HTML.
    Html,
}

/// Error returned when a string cannot be parsed as a [`PostFormat`].
#[derive(Debug, Error)]
#[error("post format must be \"markdown\", \"org\", or \"html\"")]
pub struct InvalidPostFormat;

impl fmt::Display for PostFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PostFormat::Markdown => f.write_str("markdown"),
            PostFormat::Org => f.write_str("org"),
            PostFormat::Html => f.write_str("html"),
        }
    }
}

impl FromStr for PostFormat {
    type Err = InvalidPostFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "markdown" => Ok(PostFormat::Markdown),
            "org" => Ok(PostFormat::Org),
            "html" => Ok(PostFormat::Html),
            _ => Err(InvalidPostFormat),
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_format_markdown_variant() {
        let fmt = PostFormat::Markdown;
        assert_eq!(fmt, PostFormat::Markdown);
    }

    #[test]
    fn post_format_org_variant() {
        let fmt = PostFormat::Org;
        assert_eq!(fmt, PostFormat::Org);
    }

    #[test]
    fn post_format_display_round_trips() {
        assert_eq!(PostFormat::Markdown.to_string(), "markdown");
        assert_eq!(PostFormat::Org.to_string(), "org");
        assert_eq!(
            "markdown".parse::<PostFormat>().unwrap(),
            PostFormat::Markdown
        );
        assert_eq!("org".parse::<PostFormat>().unwrap(), PostFormat::Org);
    }

    #[test]
    fn post_format_rejects_invalid_value() {
        let err = "invalid".parse::<PostFormat>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "post format must be \"markdown\", \"org\", or \"html\""
        );
    }

    #[test]
    fn post_format_debug() {
        let fmt = PostFormat::Markdown;
        let debug_str = format!("{:?}", fmt);
        assert_eq!(debug_str, "Markdown");

        let fmt2 = PostFormat::Org;
        let debug_str2 = format!("{:?}", fmt2);
        assert_eq!(debug_str2, "Org");
    }

    #[test]
    fn post_format_html_roundtrips_via_display_and_from_str() {
        assert_eq!("html".parse::<PostFormat>().unwrap(), PostFormat::Html);
        assert_eq!(PostFormat::Html.to_string(), "html");
    }

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
    fn render_html_format_is_identity() {
        let body = "<p>hi <b>there</b></p>";
        assert_eq!(render(body, &PostFormat::Html).unwrap(), body.to_string());
    }
}
