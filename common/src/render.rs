//! Dual-target post format, rendered-HTML value, and naming normalization.
//!
//! Host-only sanitization lives here behind the optional `sanitize` feature;
//! pure transformations remain available to both CSR and host callers.

use crate::post_summary::truncate_at_text_boundary;
use std::fmt;

use crate::post_body::{InvalidPostBody, PostBody};
use crate::post_title::PostTitle;
use crate::slug::{Slug, slugify_title};

/// The format/markup language used to author a post body.
///
/// A closed string enum (`#[text_enum]`, ADR-0075 as amended by #746):
/// `serialize_all = "snake_case"` gives the wire/DB token, `VariantArray` the
/// enumeration, and `EnumMessage` the editor label (absent = not user-authored). The
/// attribute injects strum's token/`Display`/`FromStr` derives and generates the named
/// `InvalidPostFormat`, the serde bridge, and — via `sqlx` — the typed TEXT column.
///
/// serde is the attribute's own bridge, NOT the derived enum (de)serializer: deserialize
/// goes `String` → `FromStr`, so an invalid token surfaces the domain
/// `InvalidPostFormat` message (asserted at the web boundary,
/// `server/tests/web/posts/create.rs`) rather than serde's generic "unknown variant", and so
/// `serde_qs` form transport decodes a bare form value. The wire token is single-sourced
/// in `serialize_all` (no `rename_all`).
#[macros::text_enum(
    sqlx,
    error = InvalidPostFormat,
    message = "post format must be \"markdown\", \"org\", or \"html\""
)]
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Default, strum::VariantArray, strum::EnumMessage,
)]
#[strum(serialize_all = "snake_case")]
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

/// HTML that is **safe to emit unescaped** — the type's invariant is "contains no
/// active markup", established by scrubbing against an allowlist (#445). It is a
/// guarantee, not merely a provenance marker, and it is structural: the unescaped
/// view sink accepts only `RenderedHtml`, so a raw `String`/body cannot reach it
/// by accident.
///
/// The feature-gated [`sanitize`] function is the only public production door:
/// it establishes this invariant by scrubbing outside input. Common-private `SQLx`
/// decoding and field-specific server DTO deserialization reconstruct persisted
/// Jaunder-owned representations without re-sanitizing them. Exact fixtures use
/// [`crate::test_support::rendered_html`] only when that test-only surface is
/// enabled.
///
/// Reading *out* is convenient — `Display`, `AsRef<str>`, `Borrow<str>`,
/// `Deref<Target = str>`, `PartialEq<str>`, and `From<RenderedHtml> for String`
/// (an *outbound* move of the inner) — but there is deliberately no public
/// *inbound constructor*: no tuple syntax, `From<String>`, `TryFrom`, `FromStr`,
/// `Deserialize`, or trusted-string constructor.
///
/// Positive companion: the public type resolves and remains readable.
/// ```
/// # use common::render::RenderedHtml;
/// fn reads(html: &RenderedHtml) -> &str {
///     html.as_ref()
/// }
/// ```
///
/// No public tuple construction:
/// ```compile_fail
/// # use common::render::RenderedHtml;
/// let _ = RenderedHtml("<p>x</p>".to_string()); // private field
/// ```
///
/// No public raw-string conversion (only the outbound `From<RenderedHtml> for
/// String`):
/// ```compile_fail
/// # use common::render::RenderedHtml;
/// let _: RenderedHtml = "<p>x</p>".to_string().into();
/// ```
///
/// No blanket deserialization:
/// ```compile_fail
/// # use common::render::RenderedHtml;
/// let _: RenderedHtml = serde_json::from_str("\"<p>x</p>\"").unwrap();
/// ```
///
#[derive(Clone, Debug, PartialEq, Eq, macros::SqlxBridge)]
pub struct RenderedHtml(pub(crate) String);

/// The single allowlist every [`sanitize`] call scrubs against. It is ammonia's
/// audited default, widened only to retain fenced-code language markers.
///
/// Re-admitting `class` without narrowing its values would let attacker-supplied
/// markup borrow application CSS. Only `language-*` tokens survive on `<pre>` and
/// `<code>`; expanding this policy is a security decision.
#[cfg(feature = "sanitize")]
static SANITIZER: std::sync::LazyLock<ammonia::Builder<'static>> = std::sync::LazyLock::new(|| {
    let mut builder = ammonia::Builder::default();
    builder.add_tag_attributes("code", ["class"]);
    builder.add_tag_attributes("pre", ["class"]);
    builder.attribute_filter(|_element, attribute, value| {
        if attribute != "class" {
            return Some(value.into());
        }
        // Only reachable for `pre`/`code`: ammonia runs this filter solely for
        // allowlisted tag/attribute pairs, and `class` is permitted nowhere else.
        let kept = value
            .split_whitespace()
            .filter(|token| token.starts_with("language-"))
            .collect::<Vec<_>>()
            .join(" ");
        (!kept.is_empty()).then_some(kept.into())
    });
    builder
});

/// Sanitizes untrusted HTML into a [`RenderedHtml`].
///
/// This is the only public production door that establishes the type's safety
/// invariant. It is feature-gated because client-side builds do not process
/// outside HTML and must not acquire the sanitizer dependency.
#[cfg(feature = "sanitize")]
#[must_use]
pub fn sanitize(raw: &str) -> RenderedHtml {
    RenderedHtml(SANITIZER.clean(raw).to_string())
}
/// The sanitizer's complete permitted `(element, attribute)` surface.
///
/// This test-only inspection seam lets host media-reference coverage classify
/// common's policy without exposing a mutable builder or allocating in production.
#[cfg(all(feature = "sanitize", any(test, feature = "test-support")))]
#[must_use]
pub fn sanitizer_permitted_attribute_pairs() -> Vec<(&'static str, &'static str)> {
    let tags = SANITIZER.clone_tags();
    let generic_attributes = SANITIZER.clone_generic_attributes();
    let tag_attributes = SANITIZER.clone_tag_attributes();

    tags.iter()
        .flat_map(|tag| generic_attributes.iter().map(move |attr| (*tag, *attr)))
        .chain(
            tag_attributes
                .iter()
                .flat_map(|(tag, attrs)| attrs.iter().map(move |attr| (*tag, *attr))),
        )
        .collect()
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

// Reading out is always safe; deliberately NO `Deserialize` on the type itself.
// Server-authored wire DTOs opt into the field-specific helper below instead.
impl serde::Serialize for RenderedHtml {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
/// Rebuilds a server-authored rendered-HTML field during common-owned DTO
/// deserialization.
///
/// This field-specific serde hook deliberately does not make `RenderedHtml`
/// generally deserializable: only common wire models opt into it. It reconstructs
/// without sanitizing or rewriting the rendered bytes.
pub(crate) fn deserialize_rendered_html<'de, D>(deserializer: D) -> Result<RenderedHtml, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    String::deserialize(deserializer).map(RenderedHtml)
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

// Storage decode: the `rendered_html` column decodes straight into `RenderedHtml`
// via the plain `#[derive(SqlxBridge)]` bridge — deliberately NOT a sanitizing
// decode, with an accepted blessing risk the gate cannot see (#701). Re-read
// docs/adr/0123-rendered-html-storage-decode.md before changing this.

/// Derives a post's public title and its slug.
///
/// Total: a body with no non-blank line is unrepresentable ([`PostBody`], #811),
/// so the slug source can always be found and there is no nothing-to-store case
/// left to report.
///
/// The body is stored by the caller — this function never mutates it, and the
/// caller derives naming from the *original* body before canonicalizing, because
/// Org's title source is stripped by canonicalization.
#[must_use]
pub fn derive_post_naming(
    explicit_title: Option<&PostTitle>,
    body: &PostBody,
    format: &PostFormat,
) -> (Option<PostTitle>, Slug) {
    let trimmed = body.trim();

    // An explicit title wins outright, so the body is only parsed for one when
    // there is none — hence `or_else`, not `or`.
    let title = explicit_title.cloned().or_else(|| {
        let extracted = match format {
            PostFormat::Markdown => extract_markdown_title(trimmed).map(|(title, _)| title),
            PostFormat::Org => extract_org_title(trimmed).map(|(title, _)| title),
            PostFormat::Html => None,
        };

        // Extracted titles are non-blank by construction — both extractors reject
        // empty-after-trim — but the compiler cannot see that. So a failed parse
        // falls through to the untitled path rather than panicking on an invariant we
        // believe but cannot prove here (#830).
        extracted.and_then(|title| title.parse::<PostTitle>().ok())
    });

    // A titled post seeds its slug from the title; an untitled one — including one
    // whose extracted title failed that parse — from the body's first non-blank line.
    let seed = match title.as_ref() {
        Some(title) => title.to_string(),
        None => first_meaningful_line(body),
    };

    // `slugify_title` never fails (it falls back to "post") and emits an
    // already-normalized value, so feeding it back through `Slug::from_str` is
    // idempotent — see its rustdoc. Deriving the slug here rather than at each call
    // site is what makes that guarantee usable (#785).
    let Ok(slug) = slugify_title(&seed).parse::<Slug>() else {
        unreachable!("slugify_title's output always re-parses as a Slug")
    };

    (title, slug)
}

/// The body's first non-blank line, trimmed and capped at 100 characters.
///
/// Total on a [`PostBody`]: the type's invariant is *exactly* this search's
/// predicate — at least one line is non-empty after trimming (#811).
fn first_meaningful_line(body: &PostBody) -> String {
    let Some(line) = body.lines().map(str::trim).find(|line| !line.is_empty()) else {
        unreachable!("a PostBody always has a non-blank line")
    };
    truncate_at_text_boundary(line, 100)
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
///
/// # Errors
///
/// Returns [`InvalidPostBody`] when canonicalization consumes the whole body — a
/// title-only post, whose sole content was the title source. See #811 decision 2.
///
/// Private on purpose: [`canonicalize_body`] is the crate's only door to body
/// canonicalization (ADR-0105), so a new format extends that one match instead of
/// giving callers a second per-format entry point.
fn canonicalize_org_body(body: &PostBody) -> Result<PostBody, InvalidPostBody> {
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

    normalize_body_whitespace(&kept.join("\n")).parse()
}

/// The whitespace half of body canonicalization, shared by Markdown and Org: drop leading
/// all-whitespace lines, trim the tail, and restore the single terminating newline.
///
/// Every clause was established by measuring real rendered output (#811), and each one is
/// load-bearing:
///
/// - **Leading _horizontal_ whitespace on a content line is never touched.** Four leading
///   spaces is a `CommonMark` indented code block; stripping them renders a `<p>` where the
///   author wrote a `<pre><code>`.
/// - **Interior blank lines are never touched.** They decide `CommonMark` loose-vs-tight
///   lists — `"- a\n\n- b\n"` renders `<li><p>a</p></li>`, `"- a\n- b\n"` renders `<li>a</li>`.
/// - **The terminating newline is restored**, because a bare `trim_end` eats it and it is
///   significant inside `<pre><code>` and inside Org paragraphs.
///
/// One case is knowingly lossy: a body ending *inside an unclosed code region* has trailing
/// blank lines that are content, and trimming drops them. Detecting that needs a format
/// parser rather than a whitespace rule, so it is accepted — the input is malformed and the
/// loss is confined to trailing blanks inside it. Pinned by
/// `canonicalize_truncates_trailing_blanks_in_unclosed_fence`.
fn normalize_body_whitespace(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut started = false;
    for line in body.trim_end().lines() {
        if !started && line.trim().is_empty() {
            continue;
        }
        started = true;
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The stored form of an authored body: one seam over every [`PostFormat`], so a format is
/// added by extending this match rather than by editing call sites in `storage`.
///
/// Markdown and Org share [`normalize_body_whitespace`]; Org additionally has its
/// title-source line stripped (ADR-0024). **HTML is exempt** — it is verbatim passthrough,
/// so any whitespace edit is a byte change, and a body ending inside an unclosed `<pre>`
/// would lose content outright.
///
/// # Errors
///
/// Returns [`InvalidPostBody`] when canonicalization consumes the whole body — a title-only
/// Org post. See #811 decision 2.
pub fn canonicalize_body(
    body: &PostBody,
    format: &PostFormat,
) -> Result<PostBody, InvalidPostBody> {
    match format {
        PostFormat::Html => Ok(body.clone()),
        PostFormat::Markdown => normalize_body_whitespace(body).parse(),
        PostFormat::Org => canonicalize_org_body(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_html_test_fixture_preserves_exact_bytes() {
        let raw = "<script>fixture markup</script>";
        assert_eq!(crate::test_support::rendered_html(raw).as_ref(), raw);
    }

    // The sanitizer is a public feature-gated seam. Assert only its observable
    // policy, not ammonia's incidental escaping and attribute serialization.
    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_removes_active_markup() {
        let html = sanitize(r#"<p>safe</p><script>alert(1)</script><img src="x" onerror="x">"#);
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("alert(1)"), "{html}");
        assert!(!html.contains("onerror"), "{html}");
        assert!(html.contains("<p>safe</p>"), "{html}");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_preserves_allowed_safe_markup() {
        let html = sanitize(
            "<h2>Heading</h2><p><em>em</em> <strong>strong</strong></p>\
             <ul><li>item</li></ul>\
             <table><thead><tr><th>h</th></tr></thead>\
             <tbody><tr><td>c</td></tr></tbody></table>\
             <blockquote><p>quoted</p></blockquote>\
             <a href=\"https://example.com\">link</a>\
             <img src=\"https://example.com/a.png\">",
        );
        for expected in [
            "<h2>",
            "<em>",
            "<strong>",
            "<ul>",
            "<li>",
            "<table>",
            "<thead>",
            "<th>",
            "<td>",
            "<blockquote>",
            "https://example.com",
            "<a ",
            "<img",
        ] {
            assert!(
                html.contains(expected),
                "{expected} was stripped from: {html}"
            );
        }
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_keeps_only_language_classes_on_code_blocks() {
        let html = sanitize(r#"<pre><code class="language-rust j-anon-only">x</code></pre>"#);
        assert!(html.contains("language-rust"), "{html}");
        assert!(!html.contains("j-anon-only"), "{html}");

        let non_code = sanitize(r#"<p class="j-anon-only">x</p>"#);
        assert!(!non_code.contains("j-anon-only"), "{non_code}");

        let no_language = sanitize(r#"<code class="j-anon-only">x</code>"#);
        assert!(!no_language.contains("j-anon-only"), "{no_language}");
        assert!(!no_language.contains("class"), "{no_language}");
    }

    #[test]
    fn rendered_html_display_and_as_ref_expose_inner() {
        let h = crate::test_support::rendered_html("<p>hi</p>");
        assert_eq!(h.to_string(), "<p>hi</p>");
        assert_eq!(h.as_ref(), "<p>hi</p>");
    }

    #[test]
    fn rendered_html_serializes_as_the_raw_string() {
        let h = crate::test_support::rendered_html("<b>x</b>");
        assert_eq!(serde_json::to_string(&h).unwrap(), "\"<b>x</b>\"");
    }

    #[test]
    fn rendered_html_into_string_moves_inner() {
        let h = crate::test_support::rendered_html("<p>move me</p>");
        let s: String = h.into();
        assert_eq!(s, "<p>move me</p>");
    }

    #[test]
    fn rendered_html_borrows_as_str() {
        fn takes_borrow<T: std::borrow::Borrow<str>>(t: &T) -> &str {
            t.borrow()
        }
        let h = crate::test_support::rendered_html("<p>b</p>");
        assert_eq!(takes_borrow(&h), "<p>b</p>");
    }

    #[test]
    fn rendered_html_partial_eq_str_and_ref() {
        let h = crate::test_support::rendered_html("<p>x</p>");
        assert_eq!(h, "<p>x</p>"); // PartialEq<&str>
        assert_eq!(h, *"<p>x</p>"); // PartialEq<str>
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

    /// A naming fixture. The bodies below are all valid `PostBody` values —
    /// there is no empty case to test, because #811 made it unrepresentable.
    fn naming(
        explicit_title: Option<&str>,
        body: &str,
        format: PostFormat,
    ) -> (Option<PostTitle>, Slug) {
        let explicit_title = explicit_title.and_then(|title| title.parse::<PostTitle>().ok());
        derive_post_naming(
            explicit_title.as_ref(),
            &crate::test_support::parse_post_body(body),
            &format,
        )
    }

    #[test]
    fn derive_post_naming_prefers_explicit_title() {
        let (title, slug) = naming(
            Some(" Explicit "),
            "# Body Heading\ntext",
            PostFormat::Markdown,
        );
        assert_eq!(title.as_deref(), Some("Explicit"));
        assert_eq!(slug, "explicit");
    }

    #[test]
    fn derive_post_naming_extracts_markdown_h1() {
        let (title, slug) = naming(None, "\n# Article Title\n\nBody text", PostFormat::Markdown);
        assert_eq!(title.as_deref(), Some("Article Title"));
        assert_eq!(slug, "article-title");
        // the body is not returned — the caller retains the original
    }

    #[test]
    fn derive_post_naming_extracts_org_title() {
        let (title, slug) = naming(None, "#+title: Org Title\n\nBody text", PostFormat::Org);
        assert_eq!(title.as_deref(), Some("Org Title"));
        assert_eq!(slug, "org-title");
        // the body is not returned — the caller retains the original
    }

    #[test]
    fn derive_post_naming_for_html_extracts_no_title_and_slugs_the_body() {
        let (title, slug) = naming(None, "<p>Hello world</p>", PostFormat::Html);
        assert_eq!(title, None);
        assert_eq!(slug, "p-hello-world-p");
    }

    #[test]
    fn derive_post_naming_allows_titleless_notes() {
        let (title, slug) = naming(None, "A compact note\nwith more text", PostFormat::Markdown);
        assert_eq!(title, None);
        assert_eq!(slug, "a-compact-note");
    }

    #[test]
    fn derive_post_naming_treats_blank_explicit_title_as_absent() {
        // A blank explicit title means "no title supplied", not an error (#830): the
        // body is consulted instead, exactly as if the field had been omitted.
        let (title, slug) = naming(Some("   "), "body line", PostFormat::Markdown);
        assert_eq!(title, None);
        assert_eq!(slug, "body-line");
    }

    #[test]
    fn derive_post_naming_untitled_slug_seed_prefers_word_boundary() {
        let body = format!("{}trailingword\nsecond line", "slug     ".repeat(10));
        let (title, slug) = naming(None, &body, PostFormat::Html);
        let expected = [
            "slug", "slug", "slug", "slug", "slug", "slug", "slug", "slug", "slug", "slug",
        ]
        .join("-");

        assert_eq!(title, None);
        assert_eq!(slug.as_ref(), expected);
    }

    #[test]
    fn derive_post_naming_untitled_slug_seed_hard_caps_long_token() {
        let body = format!("{}\nsecond line", "é".repeat(150));
        let (title, slug) = naming(None, &body, PostFormat::Html);

        assert_eq!(title, None);
        assert_eq!(slug.as_ref(), "é".repeat(crate::slug::MAX_SLUG_CHARS));
    }

    #[test]
    fn derive_post_naming_falls_back_to_post_when_nothing_slugifies() {
        // `slugify_title`'s fallback reached through the derivation: a body of
        // symbols is a valid `PostBody` but yields no slug characters. The pair is
        // still total — the caller's collision retry disambiguates the fallback.
        let (title, slug) = naming(None, "🚀🎉\n", PostFormat::Markdown);
        assert_eq!(title, None);
        assert_eq!(slug, "post");
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
        // (`derive_post_naming` trims the body first, so this branch is only
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
    fn derive_post_naming_extracts_org_level1_heading() {
        let (title, slug) = naming(None, "* Org Heading\n\nBody text", PostFormat::Org);
        assert_eq!(title.as_deref(), Some("Org Heading"));
        assert_eq!(slug, "org-heading");
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

    // -- canonicalize_org_body tests (ADR-0024; load-bearing, user-flagged) --
    //
    // Every expectation below ends with a terminating "\n" on purpose (#811): the
    // body's final newline is significant inside <pre><code> and inside Org
    // paragraphs, so canonicalization must not eat it.

    /// Canonicalize a fixture that is expected to survive — the inputs below all keep
    /// some content, so a failure here is a bug rather than the title-only rejection.
    fn canon(body: &str) -> PostBody {
        canonicalize_org_body(&crate::test_support::parse_post_body(body))
            .expect("fixture retains content after canonicalization")
    }

    #[test]
    fn canon_strips_title_header_keeps_unknown_and_later_heading() {
        // #+TITLE: present → strip it; keep #+FOO:; a LATER * heading is content → keep.
        let out = canon("#+TITLE: My Post\n#+FOO: keepme\n\n* Section\nBody\n");
        assert_eq!(out, "#+FOO: keepme\n\n* Section\nBody\n");
    }

    #[test]
    fn canon_strips_leading_heading_when_no_title_header() {
        // No #+TITLE: → the leading * heading IS the title source → strip it.
        let out = canon("* My Title\n\nBody line\n");
        assert_eq!(out, "Body line\n");
    }

    #[test]
    fn canon_strips_title_amidst_other_headers_and_leading_blanks() {
        let out = canon("\n\n#+FOO: x\n#+title: T\n#+BAR: y\n\nbody\n");
        assert_eq!(out, "#+FOO: x\n#+BAR: y\n\nbody\n");
    }

    #[test]
    fn canon_no_title_source_preserves_headers_and_content() {
        let out = canon("#+FOO: x\n\njust content\n");
        assert_eq!(out, "#+FOO: x\n\njust content\n");
    }

    #[test]
    fn canon_non_top_level_heading_is_not_a_title_source() {
        // "** Sub" is not a top-level heading → not the title → keep.
        let out = canon("** Sub\n\nBody\n");
        assert_eq!(out, "** Sub\n\nBody\n");
    }

    #[test]
    fn canon_heading_after_body_text_is_content_not_title() {
        let out = canon("intro\n* Later\nmore\n");
        assert_eq!(out, "intro\n* Later\nmore\n");
    }

    #[test]
    fn canon_is_idempotent() {
        for body in [
            "#+TITLE: T\n#+FOO: x\n\n* H\nText\n",
            "* My Title\n\nBody\n",
            "#+FOO: x\n\ncontent\n",
        ] {
            let once = canon(body);
            // Now also proves the second pass is `Ok` — a canonical body is still a body.
            let twice = canonicalize_org_body(&once).expect("canonical body stays a body");
            assert_eq!(twice, once, "idempotent for {body:?}");
        }
    }

    #[test]
    fn canon_rejects_title_only_body() {
        // The whole body was the title source, so nothing is left to store (#811).
        let body = crate::test_support::parse_post_body("* My Title\n");
        assert!(canonicalize_org_body(&body).is_err());
    }

    // -- canonicalize_body: the one seam over every format (#811) --

    fn canonicalized(body: &str, format: PostFormat) -> PostBody {
        canonicalize_body(&crate::test_support::parse_post_body(body), &format)
            .expect("fixture retains content after canonicalization")
    }

    #[test]
    fn canonicalize_leaves_html_verbatim() {
        // Verbatim passthrough, so normalization would be a byte change for no gain —
        // and would eat content from a body ending inside an unclosed <pre>.
        let raw = "\n\n  <pre>a\n\n\n";
        assert_eq!(canonicalized(raw, PostFormat::Html), raw);
    }

    #[test]
    fn canonicalize_drops_leading_blank_lines_but_not_leading_indent() {
        // The indent on the first content line is a CommonMark code block; the blank
        // lines above it are not content.
        assert_eq!(
            canonicalized("\n\n    fn main() {}\n", PostFormat::Markdown),
            "    fn main() {}\n"
        );
    }

    #[test]
    fn canonicalize_restores_the_terminating_newline() {
        // A bare trim_end eats it, and it is significant inside <pre><code>.
        assert_eq!(
            canonicalized("    code\n\n  \n", PostFormat::Markdown),
            "    code\n"
        );
    }

    #[test]
    fn canonicalize_preserves_interior_blank_lines() {
        // Interior blanks decide CommonMark loose-vs-tight lists, so "tidying" them
        // would change rendered output. Only the leading and trailing runs go.
        assert_eq!(
            canonicalized("\n- a\n\n- b\n\n", PostFormat::Markdown),
            "- a\n\n- b\n"
        );
    }

    #[test]
    fn canonicalize_preserves_interior_hard_line_break() {
        // Two trailing spaces mid-body are a hard break; only the body's tail is trimmed.
        assert_eq!(
            canonicalized("foo  \nbar  \n", PostFormat::Markdown),
            "foo  \nbar\n"
        );
    }

    #[test]
    fn canonicalize_truncates_trailing_blanks_in_unclosed_fence() {
        // ACCEPTED LOSS, not a bug: these trailing blanks are inside an unclosed fence,
        // so they are content, but seeing that needs a format parser rather than a
        // whitespace rule. The input is malformed and the loss is confined to it (#811).
        assert_eq!(
            canonicalized("```\ncode\n\n\n", PostFormat::Markdown),
            "```\ncode\n"
        );
    }

    #[test]
    fn canonicalize_is_idempotent_for_every_format() {
        // This is what lets the sqlx decode door re-check a stored body without a second
        // normalization pass.
        for format in [PostFormat::Markdown, PostFormat::Org, PostFormat::Html] {
            for body in [
                "\n\n    indented\n\n",
                "#+TITLE: T\n\nbody\n",
                "* My Title\n\nBody\n",
                "- a\n\n- b\n",
                "  <pre>x</pre>  ",
            ] {
                let once = canonicalized(body, format);
                let twice = canonicalize_body(&once, &format).expect("canonical stays a body");
                assert_eq!(twice, once, "idempotent for {format:?} {body:?}");
            }
        }
    }

    #[test]
    fn canonicalize_rejects_title_only_org_through_the_seam() {
        let body = crate::test_support::parse_post_body("* My Title\n");
        assert!(canonicalize_body(&body, &PostFormat::Org).is_err());
        // The same bytes are ordinary content in the other formats.
        assert!(canonicalize_body(&body, &PostFormat::Markdown).is_ok());
        assert!(canonicalize_body(&body, &PostFormat::Html).is_ok());
    }
}
