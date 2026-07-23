//! Pure post-body rendering and title/metadata derivation.
//!
//! Format-driven transformation of post bodies to HTML plus extraction of
//! titles, slug seeds, and summary labels. No storage or database concerns.

use std::fmt;

use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;

/// The format/markup language used to author a post body.
///
/// A `strum` string enum (ADR: `docs/adr/drafts/adopt-strum-retire-str-enum.md`):
/// `serialize_all = "snake_case"` gives the wire/DB token, `VariantArray` the
/// enumeration, `EnumMessage` the editor label (absent = not user-authored), and
/// `parse_err_ty` the named `InvalidPostFormat`.
///
/// serde routes through an owned-`String` proxy (`into`/`try_from`), NOT the derived
/// enum (de)serializer: deserialize goes `String` → `FromStr`, so an invalid token
/// surfaces the domain `InvalidPostFormat` message (asserted at the web boundary,
/// `server/tests/web/web_posts.rs`) rather than serde's generic "unknown variant", and
/// so `serde_qs` form transport decodes a bare form value. It also single-sources the
/// wire token in `as_str`/`serialize_all` (no `rename_all`).
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
    strum::VariantArray,
    strum::AsRefStr,
    strum::Display,
    strum::EnumString,
    strum::EnumMessage,
)]
#[serde(into = "String", try_from = "String")]
#[strum(serialize_all = "snake_case")]
#[strum(parse_err_ty = InvalidPostFormat, parse_err_fn = post_format_parse_err)]
pub enum PostFormat {
    /// CommonMark/GitHub-flavored Markdown.
    #[default]
    #[strum(message = "Markdown")]
    Markdown,
    /// Emacs Org-mode format.
    #[strum(message = "Org")]
    Org,
    /// Pre-rendered HTML. Renderer-internal provenance (#445); never user-authored,
    /// so it carries no editor `message` and is filtered out of format toggles.
    Html,
}

/// Error returned when a string matches no `PostFormat` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("post format must be \"markdown\", \"org\", or \"html\"")]
pub struct InvalidPostFormat;

fn post_format_parse_err(_: &str) -> InvalidPostFormat {
    InvalidPostFormat
}

// serde `into`/`try_from` proxy: serialize the wire token, deserialize an owned
// `String` through `FromStr` so the domain `InvalidPostFormat` message is preserved.
impl From<PostFormat> for String {
    fn from(format: PostFormat) -> Self {
        format.as_ref().to_owned()
    }
}

impl TryFrom<String> for PostFormat {
    type Error = InvalidPostFormat;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// HTML **produced by [`render`]**. This is a *provenance* marker, not a safety
/// guarantee: `render` does **no** sanitization (see #445), so this type means
/// "came out of our renderer", NOT "safe / XSS-free". Its value is structural — the
/// unescaped view sink accepts only `RenderedHtml`, so a raw `String`/body cannot
/// reach it by accident.
///
/// The only ways to obtain one are [`render`] (mints new HTML) and
/// [`RenderedHtml::from_trusted`] (rebuilds a value already produced by `render`
/// and round-tripped through our own storage or wire); the latter is enforced by
/// the `rendered-html-from-trusted` static check. Reading *out* is convenient —
/// `Display`, `AsRef<str>`, `Borrow<str>`, `Deref<Target = str>`, `PartialEq<str>`,
/// and `From<RenderedHtml> for String` (an *outbound* move of the inner) — but there
/// is deliberately no *inbound constructor*: no `From<String>`/`TryFrom`/`FromStr`/
/// `Deserialize`, so a raw `String` can never become a `RenderedHtml` (deref coercion
/// is one-way — it reads out, never in).
///
/// Constructing one from an arbitrary string does not compile:
/// ```compile_fail
/// let _ = common::render::RenderedHtml("<p>x</p>".to_string()); // private field
/// ```
/// ```compile_fail
/// // no inbound `From<String>` (only the outbound `From<RenderedHtml> for String`)
/// let _: common::render::RenderedHtml = "<p>x</p>".to_string().into();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedHtml(String);

impl RenderedHtml {
    /// Rebuild a `RenderedHtml` from a string the caller asserts is prior
    /// [`render`] output round-tripped through our own store or wire. This is the
    /// single trusted-rebuild door; grep it to enumerate every rebuild site. Takes
    /// `impl Into<String>` so callers (esp. fixtures) don't need `.to_string()`.
    #[must_use]
    pub fn from_trusted(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}

impl fmt::Display for RenderedHtml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for RenderedHtml {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// Read-only deref to `str` so callers can use `str` methods (`.contains()`, …)
// without `.as_ref()`. One-way (reads out, never in): it cannot turn a `&str`
// into a `RenderedHtml`, so the trust boundary is untouched.
impl std::ops::Deref for RenderedHtml {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

// Reading out is always safe; deliberately NO `Deserialize` — the wire uses a
// `deserialize_with` helper that routes through `from_trusted`.
impl serde::Serialize for RenderedHtml {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

// The rest of the StrNewtype read-out trailer (#502), hand-written to preserve the
// carve-outs: `Borrow`/`PartialEq` are read-only, and `From<Self> for String` moves the
// inner out (it does not turn a `String` *into* a `RenderedHtml`), so the trust boundary
// is untouched.
impl std::borrow::Borrow<str> for RenderedHtml {
    fn borrow(&self) -> &str {
        &self.0
    }
}

// Move the inner `String` out — a free move, unlike `.to_string()` (a clone plus format
// machinery). Mirrors every derive-trailer newtype's `From<Self> for String`.
impl From<RenderedHtml> for String {
    fn from(v: RenderedHtml) -> Self {
        v.0
    }
}

impl PartialEq<str> for RenderedHtml {
    fn eq(&self, other: &str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<&str> for RenderedHtml {
    fn eq(&self, other: &&str) -> bool {
        self.0 == **other
    }
}

// Write-side sqlx bridge (#502): `RenderedHtml` is a first-class TEXT bind parameter,
// delegating to the inner `String` — so storage binds it directly (`.bind(&rendered_html)`)
// rather than via an `.as_ref()` str-strip.
//
// Deliberately NO `Decode`: a decode could only route through `from_trusted`
// (`RenderedHtml` has no validating `FromStr`), which would bless ANY text column decoded
// into it — e.g. a raw, un-rendered `body` — as trusted, unescaped HTML, invisible to the
// `rendered-html-from-trusted` gate. Reads stay explicit: the `rendered_html` column
// decodes as `String` and is rebuilt via the gated `from_trusted` in `build_post_record`.
// `Type::compatible` is omitted (its trait default suffices) because it is consulted only
// on that absent decode path.
#[cfg(feature = "sqlx")]
const _: () = {
    impl<DB: sqlx::Database> sqlx::Type<DB> for RenderedHtml
    where
        String: sqlx::Type<DB>,
    {
        fn type_info() -> <DB as sqlx::Database>::TypeInfo {
            <String as sqlx::Type<DB>>::type_info()
        }
    }

    impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for RenderedHtml
    where
        String: sqlx::Encode<'q, DB>,
    {
        fn encode_by_ref(
            &self,
            buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
        ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
            <String as sqlx::Encode<'q, DB>>::encode_by_ref(&self.0, buf)
        }
        fn size_hint(&self) -> usize {
            <String as sqlx::Encode<'q, DB>>::size_hint(&self.0)
        }
    }
};

// ---------------------------------------------------------------------------
// Pure rendering functions
// ---------------------------------------------------------------------------

/// Renders `body` to HTML based on `format`. Pure, infallible function. The output
/// is a [`RenderedHtml`] — this is the only door that mints new rendered HTML.
#[must_use]
pub fn render(body: &PostBody, format: &PostFormat) -> RenderedHtml {
    let html = match format {
        PostFormat::Markdown => render_markdown(body),
        PostFormat::Org => render_org(body),
        PostFormat::Html => body.to_string(),
    };
    RenderedHtml(html)
}

/// Metadata derived from a post body used for slug generation and display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedPostMetadata {
    pub title: Option<PostTitle>,
    pub slug_seed: String,
    pub summary_label: PostSummary,
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
        let label = fallback_label(body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(PostTitle::from(title.clone())),
            slug_seed: title,
            summary_label: PostSummary::truncated(&label),
        });
    }

    let extracted_title = match format {
        PostFormat::Markdown => extract_markdown_title(body).map(|(title, _)| title),
        PostFormat::Org => extract_org_title(body).map(|(title, _)| title),
        PostFormat::Html => None,
    };

    if let Some(title) = extracted_title {
        let label = fallback_label(body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(PostTitle::from(title.clone())),
            slug_seed: title,
            summary_label: PostSummary::truncated(&label),
        });
    }

    let label = fallback_label(body)?;
    Some(DerivedPostMetadata {
        title: None,
        slug_seed: label.clone(),
        summary_label: PostSummary::truncated(&label),
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
            // A `strip_prefix("# ")` match always leaves a non-empty remainder
            // because `trimmed` has no trailing whitespace, so no empty-title
            // guard is needed here.
            if let Some(title) = trimmed.strip_prefix("# ") {
                found = Some(title.trim().to_owned());
                continue;
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
        if found.is_some() {
            output.push(line);
            continue;
        }

        let trimmed = line.trim();

        if trimmed.is_empty() {
            // Blank lines in the header block (before any title) are skipped.
            continue;
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

        // * Top-level heading (exactly one asterisk followed by space). As with
        // the Markdown case, a match always leaves a non-empty heading because
        // `trimmed` has no trailing whitespace.
        if let Some(heading) = trimmed.strip_prefix("* ") {
            found = Some(heading.trim().to_owned());
            continue;
        }

        // Any other non-blank, non-KV, non-heading content means no title
        return None;
    }

    found.map(|title| (title, output.join("\n").trim().to_owned()))
}

/// Canonicalize an ingested Org body (ADR-0024): remove the body's title-source
/// line (a `#+TITLE:` header, or a leading top-level `* heading` when there is no
/// `#+TITLE:`) and strip leading blank lines, while preserving every other line —
/// including unrecognized `#+FOO:` headers and content headings — verbatim. Output
/// is byte-deterministic and idempotent so reconcile (Unit D) never sees false
/// divergence.
///
/// A top-level `* heading` is treated as the title source only when it is the very
/// first content of the body (nothing kept before it and no `#+TITLE:` seen). This
/// gate is what makes the function idempotent: once the title source is stripped, a
/// later `* heading` left behind sits after kept header lines and so is content, not
/// a new title source on the next pass. (This is a deliberate, test-pinned refinement
/// of `extract_org_title`'s precedence; see the `canon_*` unit tests.)
#[must_use]
pub fn canonicalize_org_body(body: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut in_header = true; // still scanning the leading blank/#+/title region
    let mut saw_title = false;

    for line in body.lines() {
        if !in_header {
            // Past the header region everything is preserved verbatim — except we
            // keep dropping blank lines while nothing has been kept yet, because a
            // dropped title-source heading turns its trailing blanks into leading
            // blanks.
            if kept.is_empty() && line.trim().is_empty() {
                continue;
            }
            kept.push(line);
            continue;
        }
        let t = line.trim_start();
        if t.is_empty() {
            // Drop leading blank lines; preserve a blank once a header line is kept.
            if !kept.is_empty() {
                kept.push(line);
            }
            continue;
        }
        if t.to_ascii_lowercase().starts_with("#+title:") {
            saw_title = true; // recognized title header → drop
            continue;
        }
        if t.starts_with("#+") {
            kept.push(line); // unrecognized header → preserve verbatim
            continue;
        }
        // The first non-blank, non-`#+` line ends the header region. A top-level
        // `* heading` at the very start of the body (nothing kept before it and no
        // `#+TITLE:` seen) is the title source → drop it; anything else is content.
        in_header = false;
        if !saw_title && kept.is_empty() && t.starts_with("* ") {
            continue;
        }
        kept.push(line);
    }

    kept.join("\n").trim_end().to_string()
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
fn render_org(body: &str) -> String {
    orgize::Org::parse(body).to_html()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_html_display_and_as_ref_expose_inner() {
        let h = RenderedHtml::from_trusted("<p>hi</p>");
        assert_eq!(h.to_string(), "<p>hi</p>");
        assert_eq!(h.as_ref(), "<p>hi</p>");
    }

    #[test]
    fn rendered_html_serializes_as_the_raw_string() {
        let h = RenderedHtml::from_trusted("<b>x</b>");
        assert_eq!(serde_json::to_string(&h).unwrap(), "\"<b>x</b>\"");
    }

    #[test]
    fn rendered_html_into_string_moves_inner() {
        let h = RenderedHtml::from_trusted("<p>move me</p>");
        let s: String = h.into();
        assert_eq!(s, "<p>move me</p>");
    }

    #[test]
    fn rendered_html_borrows_as_str() {
        fn takes_borrow<T: std::borrow::Borrow<str>>(t: &T) -> &str {
            t.borrow()
        }
        let h = RenderedHtml::from_trusted("<p>b</p>");
        assert_eq!(takes_borrow(&h), "<p>b</p>");
    }

    #[test]
    fn rendered_html_partial_eq_str_and_ref() {
        let h = RenderedHtml::from_trusted("<p>x</p>");
        assert!(h == "<p>x</p>"); // PartialEq<&str>
        assert!(h == *"<p>x</p>"); // PartialEq<str>
        assert!(h != "<p>y</p>"); // PartialEq<&str>, unequal
        assert!(h != *"<p>y</p>"); // PartialEq<str>, unequal
    }

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
        let debug_str = format!("{fmt:?}");
        assert_eq!(debug_str, "Markdown");

        let fmt2 = PostFormat::Org;
        let debug_str2 = format!("{fmt2:?}");
        assert_eq!(debug_str2, "Org");
    }

    #[test]
    fn post_format_html_roundtrips_via_display_and_from_str() {
        assert_eq!("html".parse::<PostFormat>().unwrap(), PostFormat::Html);
        assert_eq!(PostFormat::Html.to_string(), "html");
    }

    #[test]
    fn post_format_serde_json_round_trips() {
        assert_eq!(
            serde_json::to_string(&PostFormat::Markdown).unwrap(),
            "\"markdown\""
        );
        assert_eq!(serde_json::to_string(&PostFormat::Org).unwrap(), "\"org\"");
        assert_eq!(
            serde_json::to_string(&PostFormat::Html).unwrap(),
            "\"html\""
        );
        assert_eq!(
            serde_json::from_str::<PostFormat>("\"markdown\"").unwrap(),
            PostFormat::Markdown
        );
        assert_eq!(
            serde_json::from_str::<PostFormat>("\"html\"").unwrap(),
            PostFormat::Html
        );
        assert!(serde_json::from_str::<PostFormat>("\"bogus\"").is_err());
    }

    #[test]
    fn post_format_variants_and_editor_labels() {
        use strum::{EnumMessage, VariantArray};
        assert_eq!(
            PostFormat::VARIANTS,
            &[PostFormat::Markdown, PostFormat::Org, PostFormat::Html]
        );
        assert_eq!(PostFormat::Markdown.get_message(), Some("Markdown"));
        assert_eq!(PostFormat::Org.get_message(), Some("Org"));
        assert_eq!(PostFormat::Html.get_message(), None); // renderer-internal → not offered
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
        let html = render_org("* H1\n** H2");
        assert!(html.contains("H1"));
        assert!(html.contains("H2"));
    }

    #[test]
    fn org_paragraph() {
        let html = render_org("Hello, org world!");
        assert!(html.contains("Hello, org world!"));
    }

    #[test]
    fn org_bold_italic_code() {
        let html = render_org("*bold* /italic/ ~code~");
        assert!(html.contains("<b>bold</b>"));
        assert!(html.contains("<i>italic</i>"));
        assert!(html.contains("<code>code</code>"));
    }

    #[test]
    fn org_list() {
        let html = render_org("- alpha\n- beta");
        assert!(html.contains("alpha"));
        assert!(html.contains("beta"));
    }

    #[test]
    fn org_code_block() {
        let html = render_org("#+BEGIN_SRC rust\nfn main() {}\n#+END_SRC");
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn org_link() {
        let html = render_org("[[https://example.com][example]]");
        assert!(
            html.contains("<a href=\"https://example.com\""),
            "expected an anchor element, got: {html}"
        );
        assert!(html.contains("example"));
    }

    #[test]
    fn org_empty_input() {
        let html = render_org("");
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
        let result = render(&PostBody::from("**bold**"), &PostFormat::Markdown);
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[test]
    fn render_dispatches_org() {
        let result = render(&PostBody::from("*bold*"), &PostFormat::Org);
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

    #[test]
    fn extract_org_title_handles_level1_heading() {
        let result = extract_org_title("* My Title\n\nBody text");
        assert_eq!(
            result,
            Some(("My Title".to_string(), "Body text".to_string()))
        );
    }

    #[test]
    fn extract_markdown_title_skips_leading_blanks_then_finds_heading() {
        // Leading blank lines before the heading exercise the blank-skip branch.
        // (`derive_post_metadata` trims the body first, so this branch is only
        // reachable by calling the helper directly.)
        let result = extract_markdown_title("\n\n# Title\n\nBody");
        assert_eq!(result, Some(("Title".to_string(), "Body".to_string())));
    }

    #[test]
    fn extract_org_title_heading_after_kv_lines() {
        let result = extract_org_title("#+AUTHOR: Me\n* My Title\n\nBody");
        assert_eq!(result, Some(("My Title".to_string(), "Body".to_string())));
    }

    #[test]
    fn extract_org_title_skips_blank_lines_inside_kv_block() {
        // Blank lines in the header block (before any title) must be skipped so
        // the following heading is still recognized as the title.
        let result = extract_org_title("\n#+AUTHOR: Me\n\n#+DATE: today\n* My Title\n\nBody");
        assert_eq!(result, Some(("My Title".to_string(), "Body".to_string())));
    }

    #[test]
    fn extract_org_title_blank_lines_after_heading_are_appended_to_body() {
        // Once a heading is found, every later line (including blank lines) is
        // appended to the body, which is then trimmed.
        let result = extract_org_title("#+AUTHOR: Me\n* Heading\n\nBody\n\nMore");
        assert_eq!(
            result,
            Some(("Heading".to_string(), "Body\n\nMore".to_string()))
        );
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
        assert_eq!(
            render(&PostBody::from(body), &PostFormat::Html).as_ref(),
            body
        );
    }

    // -- canonicalize_org_body tests (ADR-0024; load-bearing, user-flagged) --

    #[test]
    fn canon_strips_title_header_keeps_unknown_and_later_heading() {
        // #+TITLE: present → strip it; keep #+FOO:; a LATER * heading is content → keep.
        let out = canonicalize_org_body("#+TITLE: My Post\n#+FOO: keepme\n\n* Section\nBody\n");
        assert_eq!(out, "#+FOO: keepme\n\n* Section\nBody");
    }

    #[test]
    fn canon_strips_leading_heading_when_no_title_header() {
        // No #+TITLE: → the leading * heading IS the title source → strip it.
        let out = canonicalize_org_body("* My Title\n\nBody line\n");
        assert_eq!(out, "Body line");
    }

    #[test]
    fn canon_strips_title_amidst_other_headers_and_leading_blanks() {
        let out = canonicalize_org_body("\n\n#+FOO: x\n#+title: T\n#+BAR: y\n\nbody\n");
        assert_eq!(out, "#+FOO: x\n#+BAR: y\n\nbody");
    }

    #[test]
    fn canon_no_title_source_preserves_headers_and_content() {
        let out = canonicalize_org_body("#+FOO: x\n\njust content\n");
        assert_eq!(out, "#+FOO: x\n\njust content");
    }

    #[test]
    fn canon_non_top_level_heading_is_not_a_title_source() {
        // "** Sub" is not a top-level heading → not the title → keep.
        let out = canonicalize_org_body("** Sub\n\nBody\n");
        assert_eq!(out, "** Sub\n\nBody");
    }

    #[test]
    fn canon_heading_after_body_text_is_content_not_title() {
        let out = canonicalize_org_body("intro\n* Later\nmore\n");
        assert_eq!(out, "intro\n* Later\nmore");
    }

    #[test]
    fn canon_is_idempotent() {
        for body in [
            "#+TITLE: T\n#+FOO: x\n\n* H\nText\n",
            "* My Title\n\nBody\n",
            "#+FOO: x\n\ncontent\n",
        ] {
            let once = canonicalize_org_body(body);
            assert_eq!(
                canonicalize_org_body(&once),
                once,
                "idempotent for {body:?}"
            );
        }
    }
}
