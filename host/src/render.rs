//! Host-side rendering, HTML sanitization, and media-reference extraction.

use common::media::{MediaReference, parse_media_url};
use common::post_body::PostBody;
use common::render::{PostFormat, RenderedHtml};

/// The single allowlist every [`sanitize`] call scrubs against — defined once
/// here so no caller can drift to a different policy.
///
/// ammonia's audited default, widened in exactly one place: `class` on `<pre>` and
/// `<code>`, so a fenced code block keeps the language marker `pulldown-cmark`
/// emits (`class="language-rust"`). Because the tag/attribute allowlist alone would
/// permit *any* class on attacker-supplied content — letting a post borrow our app's
/// CSS to mimic or hide UI — an attribute filter narrows the values to `language-*`
/// tokens and drops everything else. Widening this list is a security decision;
/// `sanitize_*` tests pin both halves.
///
/// # Widening also obliges you to classify the new attributes (#711)
///
/// A post's media references are extracted from *this* allowlist's output, so an
/// attribute permitted here but unknown to the extractor is a silent blind spot in
/// the `post_media` table — the exact failure mode #711 exists to fix. So every
/// `(element, attribute)` pair this builder permits must be classified: either the
/// **pair** appears in [`MEDIA_URL_ATTRS`] (its value names media) or the
/// **attribute name** appears in [`INERT_ATTRS`] (it does not, on any element).
/// `sanitizer_surface_is_fully_classified` fails the build until each newly
/// permitted pair is classified, so this cannot be forgotten — but it is a
/// judgement call, not a formality.
///
/// One shape does not fit: a **multi-URL** attribute such as `srcset` carries a
/// list, and [`MEDIA_URL_ATTRS`] is one attribute → one URL. Permitting it means
/// widening that table to carry a per-attribute parse mode *first*; classifying it
/// as-is would silently record a wrong single-URL parse.
pub(super) static SANITIZER: std::sync::LazyLock<ammonia::Builder<'static>> =
    std::sync::LazyLock::new(|| {
        let mut builder = ammonia::Builder::default();
        builder.add_tag_attributes("code", ["class"]);
        builder.add_tag_attributes("pre", ["class"]);
        builder.attribute_filter(|_element, attribute, value| {
            if attribute != "class" {
                return Some(value.into());
            }
            // Only reachable for `pre`/`code`: ammonia runs this filter solely for
            // (tag, attribute) pairs the allowlist above already permits, and `class`
            // is permitted nowhere else — a `class` on any other element is dropped
            // before it gets here.
            let kept = value
                .split_whitespace()
                .filter(|token| token.starts_with("language-"))
                .collect::<Vec<_>>()
                .join(" ");
            (!kept.is_empty()).then_some(kept.into())
        });
        builder
    });

/// Sanitizes untrusted HTML into a common-owned `RenderedHtml`.
#[must_use]
pub fn sanitize(raw: &str) -> RenderedHtml {
    // rendered-html-from-trusted:allow host sanitizer establishes RenderedHtml's safety invariant (#847)
    RenderedHtml::from_trusted(SANITIZER.clean(raw).to_string())
}
/// Renders `body` to HTML based on `format`. Pure, infallible function. The
/// output is a [`RenderedHtml`], minted through [`sanitize`] — a post body is
/// author-supplied, so it is outside input and every format's output is scrubbed.
///
/// All three formats need that scrub, not just [`PostFormat::Html`]: the Markdown and
/// Org parsers both pass embedded raw HTML through untouched, so `<script>` in a
/// Markdown body reaches the output just as readily as in an HTML one (#445).
///
/// Host-only: this module is owned by the host crate, so no build exposes a
/// weaker unsanitized implementation.
#[must_use]
pub fn render(body: &PostBody, format: &PostFormat) -> RenderedHtml {
    let html = match format {
        PostFormat::Markdown => render_markdown(body),
        PostFormat::Org => render_org(body),
        PostFormat::Html => body.to_string(),
    };
    sanitize(&html)
}

/// Renders Markdown to HTML using pulldown-cmark with common extensions.
pub(super) fn render_markdown(body: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};

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
pub(super) fn render_org(body: &str) -> String {
    orgize::Org::parse(body).to_html()
}

/// The `(element, attribute)` pairs whose values name media. Adding an element to
/// [`SANITIZER`] means adding its URL-bearing attributes here — the walk knows no tag
/// names of its own, so extending to `<video>`/`<audio>` is a **data edit**
/// (`("video", "src")`, `("video", "poster")`, `("audio", "src")`, …) with no change to
/// `extract_media_refs_with`, which `extract_walk_is_table_driven_not_tag_hardcoded`
/// keeps a checked claim rather than a hope.
///
/// One attribute, one URL. A multi-URL attribute such as `srcset` does not fit this
/// shape and needs the table widened to carry a per-attribute parse mode before it can
/// be listed here.
///
/// Public because it is the contract of [`extract_media_refs`]: what the walk looks at
/// is data, and reviewable as data. Its counterpart is [`INERT_ATTRS`], and between them
/// they must cover everything [`SANITIZER`] permits —
/// `sanitizer_surface_is_fully_classified` enforces that.
pub const MEDIA_URL_ATTRS: &[(&str, &str)] = &[("a", "href"), ("img", "src")];

/// Attribute *names* that name no media **wherever they appear**. The counterpart to
/// [`MEDIA_URL_ATTRS`]: a permitted `(element, attribute)` counts as classified when the
/// **pair** is in that table, or the **name** is here.
///
/// Names rather than pairs because that is the shape of the fact. `align` on `<td>` is
/// inert for the same reason it is inert on `<tr>`, `<col>` and `<hr>` — the reason is a
/// property of the attribute, not of the element carrying it. Spelling the product
/// instead mirrored ammonia's default table pair for pair, which meant ~50 entries that
/// drifted the moment ammonia changed one, and buried the handful of real judgements
/// under ten repetitions of `char`/`charoff`. It also covers the *generic* attributes
/// (permitted on every tag) without writing out `tags × generic_attributes`.
///
/// # The invariant
///
/// **No name listed here is URL-bearing on any element.** That is what makes the
/// name-level rule sound; a name that is a URL somewhere would be silently excused
/// everywhere. So `src`, `href`, `poster`, `srcset`, `data`, `action`, `formaction` and
/// `ping` must never appear here — each is either an entry in [`MEDIA_URL_ATTRS`] for the
/// element that carries it, or (for a tag [`SANITIZER`] does not permit) simply absent,
/// which is what keeps the coupling test biting when the allowlist widens.
///
/// `cite` is the one entry a reader should question: it *is* URL-valued. It is here as a
/// deliberate scope call (spec D2) — it records where a quotation came from, and no
/// browser fetches, displays or navigates it, so it points no reader at anything. Revisit
/// that deliberately, not by accident.
pub const INERT_ATTRS: &[&str] = &[
    // Advisory text and human-language metadata (`lang`/`title` are generic — permitted
    // on every tag; `hreflang` is the language of a link's *target*, not a link).
    "alt", "hreflang", "lang", "title",
    // Quotation provenance — URL-valued, and deliberately out of scope; see above.
    "cite",     // Edit timestamps (`<del>`/`<ins>`).
    "datetime", // Text direction (`<bdo>`).
    "dir",
    // Presentational geometry and alignment, on tables, columns, rules and images.
    "align", "char", "charoff", "colspan", "headers", "height", "rowspan", "scope", "size", "span",
    "summary", "width", // List numbering (`<ol>`).
    "start",
    // `SANITIZER`'s one widening: the `language-*` marker on a fenced code block, whose
    // values the attribute filter already narrows.
    "class",
];

/// Extracts the media a sanitized HTML fragment references, deduplicated and sorted.
///
/// The input is a [`RenderedHtml`]'s own text (it derefs to `&str`) — the stored,
/// already-sanitized output. So what is extracted is what a reader is actually pointed
/// at: a raw `<img>` embedded in a Markdown body counts (both parsers pass raw HTML
/// through), and anything sanitisation stripped, or that survived only as literal text
/// inside a code block, does not (spec D2).
///
/// Each value is handed to [`parse_media_url`], which decides what names a stored entry;
/// this function contributes no URL knowledge of its own, only *where in the document*
/// to look — [`MEDIA_URL_ATTRS`].
///
/// The output is sorted and deduplicated (it is collected through a `BTreeSet`), so a
/// byte-identical body yields a byte-identical set of rows.
#[must_use]
pub fn extract_media_refs(html: &str) -> Vec<MediaReference> {
    extract_media_refs_with(html, MEDIA_URL_ATTRS)
}

/// Table-driven core of [`extract_media_refs`]; separate so a test can drive it with a
/// synthetic pair table and prove no tag name is baked into the walk.
///
/// Re-parses the sanitized string rather than collecting during ammonia's clean pass
/// (spec D6): ammonia permits only one attribute filter, the existing one already
/// enforces the `language-*` class policy, and its ordering against URL-scheme filtering
/// would have to be verified and then depended on. A second parse of the final string is
/// the literal reading of "extract from the rendered, sanitized HTML", and yields a pure
/// `&str -> Vec<MediaReference>` that the coupling test and future reclamation work reuse.
///
/// Uses `html5ever`'s tokenizer, not its tree builder: only start tags and their
/// attributes are needed, and the input is already well-formed sanitizer output.
pub(super) fn extract_media_refs_with(html: &str, pairs: &[(&str, &str)]) -> Vec<MediaReference> {
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    use html5ever::tendril::StrTendril;
    use html5ever::tokenizer::{
        BufferQueue, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
        TokenizerOpts,
    };

    /// Collects the references named by the `(element, attribute)` pairs it was given.
    /// `TokenSink::process_token` takes `&self`, so the set lives behind a `RefCell`.
    struct MediaRefSink<'a> {
        pairs: &'a [(&'a str, &'a str)],
        refs: RefCell<BTreeSet<MediaReference>>,
    }

    impl TokenSink for MediaRefSink<'_> {
        type Handle = ();

        fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
            let TagToken(tag) = token else {
                return TokenSinkResult::Continue;
            };
            // End tags carry no attributes, and a reference is something the *opening*
            // tag points at.
            if tag.kind != StartTag {
                return TokenSinkResult::Continue;
            }
            // Element and attribute names arrive ASCII-lowercased from the tokenizer, so
            // the tables are matched case-sensitively against a normalized name.
            let element: &str = &tag.name;
            self.refs.borrow_mut().extend(
                tag.attrs
                    .iter()
                    .filter(|attr| {
                        let name: &str = &attr.name.local;
                        self.pairs
                            .iter()
                            .any(|(el, at)| *el == element && *at == name)
                    })
                    .filter_map(|attr| parse_media_url(&attr.value)),
            );
            TokenSinkResult::Continue
        }
    }

    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(
        MediaRefSink {
            pairs,
            refs: RefCell::new(BTreeSet::new()),
        },
        TokenizerOpts::default(),
    );
    // One `feed` drains the queue: the sink always answers `Continue`, so nothing
    // suspends tokenization. `end` flushes the tokenizer's final state.
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.refs.take().into_iter().collect()
}

/// A rendered post body and the media it references — derived together, never separately.
///
/// The reference set is private and [`render_with_media`] is the only constructor, so a
/// value whose set disagrees with its HTML is unrepresentable rather than merely
/// discouraged (spec D1). Everything downstream only *carries* the pair: a post
/// create/update input on its way to storage cannot substitute a set of its own, correct
/// or not, and a caller with no way to render has no way to invent one either.
///
/// The set is derived, never supplied. The companion imports the **same** names the
/// two negatives below hide — including the free `render`, which they both call — so
/// each fails for the private field rather than for an unresolved path:
/// ```
/// # use common::post_body::PostBody;
/// # use common::render::PostFormat;
/// # use host::render::{RenderOutput, render, render_with_media};
/// # let body: PostBody = "hello".parse().unwrap();
/// let other: PostBody = "different".parse().unwrap();
/// let out = render_with_media(&body, &PostFormat::Markdown);
/// assert!(out.media().is_empty());
/// let _direct = render(&body, &PostFormat::Markdown); // `render` resolves
/// let _other = render(&other, &PostFormat::Markdown); // the last negative's fixture
/// ```
/// and a struct literal cannot smuggle one in:
/// ```compile_fail
/// # use common::post_body::PostBody;
/// # use common::render::PostFormat;
/// # use host::render::{RenderOutput, render};
/// # let body: PostBody = "hello".parse().unwrap();
/// let html = render(&body, &PostFormat::Markdown);
/// let _ = RenderOutput { html, media: vec![] }; // private field
/// ```
/// nor can the HTML be swapped out from under the set that describes it — the same
/// desynchronisation reached from the other side, which a `pub html` would have left open:
/// ```compile_fail
/// # use common::post_body::PostBody;
/// # use common::render::PostFormat;
/// # use host::render::{RenderOutput, render, render_with_media};
/// # let body: PostBody = "hello".parse().unwrap();
/// # let other: PostBody = "different".parse().unwrap();
/// let mut out = render_with_media(&body, &PostFormat::Markdown);
/// out.html = render(&other, &PostFormat::Markdown); // private field
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOutput {
    /// The sanitized HTML, as [`render`] produced it. Private for the same reason
    /// `media` is: a `pub` field here would let a caller assign a *different* HTML over
    /// the references derived from the original, which is the same desynchronisation the
    /// private `media` field exists to prevent — reached from the other side.
    // rendered-html-from-trusted:allow RenderOutput is minted only by render/sanitize before pairing media refs (#701)
    html: RenderedHtml,
    /// What that HTML points a reader at. Private — see the type's docs.
    media: Vec<MediaReference>,
}

/// Renders a body and derives its media references from its sanitized HTML.
#[must_use]
pub fn render_with_media(body: &PostBody, format: &PostFormat) -> RenderOutput {
    let html = render(body, format);
    let media = extract_media_refs(html.as_ref());
    RenderOutput { html, media }
}

impl RenderOutput {
    /// The sanitized HTML.
    #[must_use]
    pub fn html(&self) -> &RenderedHtml {
        &self.html
    }

    /// The media the HTML references — sorted and deduplicated, as
    /// [`extract_media_refs`] returns them.
    #[must_use]
    pub fn media(&self) -> &[MediaReference] {
        &self.media
    }

    /// Consumes the pair, yielding the HTML alone.
    ///
    /// The one legitimate way to take the HTML *out*: by consuming the value, the
    /// reference set it was derived with goes away with it, so nothing is left holding a
    /// set that no longer describes anything.
    #[must_use]
    pub fn into_html(self) -> RenderedHtml {
        self.html
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::render::{PostFormat, canonicalize_body};
    use common::test_support::parse_post_body;

    // The load-bearing guard for the no-trim half of the whitespace rule (#811).
    // Every test in `post_body.rs` still passes if a "tidy-up" trim is added to the
    // constructor; this one does not, because it asserts on what the reader sees.
    #[test]
    fn markdown_body_with_leading_indent_still_renders_as_code_block() {
        let body = parse_post_body("    fn main() {}\n");
        let canonical = canonicalize_body(&body, &PostFormat::Markdown).expect("body survives");
        let html = render(&canonical, &PostFormat::Markdown);
        assert!(html.contains("<pre><code>"), "{html}");
        assert!(!html.contains("<p>fn main()"), "{html}");
    }

    // The other half: canonicalization must not perturb what the reader sees. A bare
    // trim_end would drop the newline inside the code block, and stripping leading
    // blank lines must not disturb the indent that makes it a code block at all.
    #[test]
    fn canonicalizing_markdown_does_not_change_rendered_output() {
        for raw in [
            "\n\n    fn main() {}\n",
            "- a\n\n- b\n\n",
            "foo  \nbar\n",
            "# Heading\n\ntext\n\n\n",
        ] {
            let body = parse_post_body(raw);
            let canonical = canonicalize_body(&body, &PostFormat::Markdown).expect("body survives");
            assert_eq!(
                render(&canonical, &PostFormat::Markdown),
                render(&body, &PostFormat::Markdown),
                "canonicalization changed rendered output for {raw:?}"
            );
        }
    }

    // `sanitize` is the establishing door (#445): it is what makes the type's
    // invariant — "contains no active markup" — true rather than asserted. These
    // assert on the *absence* of the dangerous token rather than exact output,
    // because ammonia's escaping details are not our contract.
    #[test]
    fn sanitize_strips_script_element() {
        let h = sanitize("<p>hi</p><script>alert(1)</script>");
        assert!(!h.contains("<script"), "{h}");
        assert!(!h.contains("alert(1)"), "{h}");
        assert!(h.contains("<p>hi</p>"), "{h}");
    }

    #[test]
    fn sanitize_strips_event_handler_attributes() {
        let h = sanitize(r#"<img src="x" onerror="alert(1)">"#);
        assert!(!h.contains("onerror"), "{h}");
    }

    #[test]
    fn sanitize_strips_javascript_urls() {
        let h = sanitize(r#"<a href="javascript:alert(1)">x</a>"#);
        assert!(!h.contains("javascript:"), "{h}");
    }

    #[test]
    fn sanitize_preserves_formatting_markup() {
        // The shapes `pulldown-cmark` and `orgize` actually emit. If the default
        // allowlist eats any of these, posts render degraded — widen deliberately
        // rather than letting it pass.
        let h = sanitize(
            "<h2>Heading</h2><p><em>em</em> <strong>strong</strong></p>\
             <ul><li>item</li></ul>\
             <pre><code class=\"language-rust\">let x = 1;</code></pre>\
             <table><thead><tr><th>h</th></tr></thead>\
             <tbody><tr><td>c</td></tr></tbody></table>\
             <blockquote><p>quoted</p></blockquote>",
        );
        for expected in [
            "<h2>",
            "<em>",
            "<strong>",
            "<ul>",
            "<li>",
            "<pre>",
            "<code",
            "<table>",
            "<thead>",
            "<th>",
            "<td>",
            "<blockquote>",
        ] {
            assert!(h.contains(expected), "{expected} was stripped from: {h}");
        }
        // `pulldown-cmark` puts the fence's language in `class="language-rust"` on
        // the `<code>`; ammonia's default drops `class` entirely, so `SANITIZER`
        // re-admits it for `pre`/`code`.
        assert!(h.contains("language-rust"), "code-block language lost: {h}");
    }

    #[test]
    fn sanitize_keeps_only_language_classes_on_code() {
        // Re-admitting `class` must not become arbitrary class injection: a post
        // could otherwise borrow the app's own CSS to mimic or hide UI. Only
        // `language-*` tokens survive, and only on `pre`/`code`.
        let h = sanitize(r#"<pre><code class="language-rust j-anon-only">x</code></pre>"#);
        assert!(h.contains("language-rust"), "{h}");
        assert!(
            !h.contains("j-anon-only"),
            "non-language class survived: {h}"
        );

        // …and `class` stays disallowed everywhere else — dropped by the tag
        // allowlist before the attribute filter is consulted at all.
        let other = sanitize(r#"<p class="j-anon-only">x</p>"#);
        assert!(
            !other.contains("j-anon-only"),
            "class survived on <p>: {other}"
        );

        // The filter's drop-entirely branch: on `code`, where `class` *is*
        // allowlisted, a value with no `language-*` token must lose the attribute
        // outright rather than surviving as an empty `class=""`.
        let none_kept = sanitize(r#"<code class="j-anon-only">x</code>"#);
        assert!(
            !none_kept.contains("j-anon-only"),
            "non-language class survived on <code>: {none_kept}"
        );
        assert!(
            !none_kept.contains("class"),
            "empty class attribute left behind: {none_kept}"
        );
    }

    #[test]
    fn sanitize_preserves_safe_links_and_images() {
        let h = sanitize(
            r#"<a href="https://example.com">link</a><img src="https://example.com/a.png">"#,
        );
        assert!(h.contains("https://example.com"), "{h}");
        assert!(h.contains("<a "), "{h}");
        assert!(h.contains("<img"), "{h}");
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

    /// AC6/D6 asks that the allowlist strip nothing our renderers *legitimately*
    /// emit. The `sanitize_preserves_*` tests above feed hand-written HTML, which
    /// proves the allowlist but not that it matches what `pulldown-cmark` and
    /// `orgize` actually produce. This one closes that gap by driving real
    /// renderer output through the real `render()` door.
    #[test]
    fn render_preserves_real_renderer_output() {
        let md = render(
            &parse_post_body(
                "# Heading\n\n\
                 Some **bold** and *emphasis* and a [link](https://example.com).\n\n\
                 ```rust\nfn main() {}\n```\n\n\
                 | a | b |\n|---|---|\n| 1 | 2 |\n",
            ),
            &PostFormat::Markdown,
        );
        for expected in [
            "<h1>",
            "<strong>bold</strong>",
            "<em>emphasis</em>",
            r#"<a href="https://example.com""#,
            r#"<pre><code class="language-rust">"#,
            "<table>",
            "<thead>",
            "<th>",
            "<td>",
        ] {
            assert!(md.contains(expected), "markdown lost {expected}: {md}");
        }

        let org = render(
            &parse_post_body(
                "* Heading\n\nSome *bold* text and [[https://example.com][a link]].\n",
            ),
            &PostFormat::Org,
        );
        for expected in [
            "<h1>",
            "<b>bold</b>",
            r#"<a href="https://example.com""#,
            "a link",
        ] {
            assert!(org.contains(expected), "org lost {expected}: {org}");
        }

        // Known and intended: orgize wraps its output in `<main><section>`, and
        // neither tag is in ammonia's default allowlist, so both are dropped while
        // their children survive (asserted above). That is not a regression to fix
        // — the rendered HTML is injected into a page that already has its own
        // `<main>`, so keeping orgize's would nest a document-level landmark, and
        // no stylesheet targets either tag. Pinned so the drop stays deliberate.
        assert!(!org.contains("<main>"), "unexpected <main> wrapper: {org}");
        assert!(
            !org.contains("<section>"),
            "unexpected <section> wrapper: {org}"
        );
    }

    // -- Cross-format dispatch tests --

    #[test]
    fn render_dispatches_markdown() {
        let result = render(&parse_post_body("**bold**"), &PostFormat::Markdown);
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[test]
    fn render_dispatches_org() {
        let result = render(&parse_post_body("*bold*"), &PostFormat::Org);
        assert!(result.contains("<b>bold</b>"));
    }

    // -- Sanitization at the mint point (#445, AC1) --
    //
    // Every format must neutralize active markup. Markdown and Org both pass
    // embedded raw HTML straight through their parsers, and `Html` is a verbatim
    // passthrough, so all three need their own assertion rather than one shared one.

    /// AC1's three vectors — a `<script>` element, an event-handler attribute, and
    /// a `javascript:` URL — asserted as one invariant so every format test covers
    /// the same ground instead of each restating a subset.
    fn assert_no_active_markup(html: &str) {
        assert!(!html.contains("<script"), "script element survived: {html}");
        assert!(!html.contains("onerror"), "event handler survived: {html}");
        assert!(
            !html.contains("javascript:"),
            "javascript: URL survived: {html}"
        );
    }

    /// The three vectors as raw HTML, for embedding in each format's body.
    const ACTIVE_MARKUP: &str = concat!(
        "<script>alert(1)</script>",
        r#"<img src=x onerror=alert(1)>"#,
        r#"<a href="javascript:alert(1)">x</a>"#,
    );

    #[test]
    fn render_markdown_strips_embedded_script() {
        let result = render(
            &parse_post_body(format!("Hello\n\n{ACTIVE_MARKUP}").as_str()),
            &PostFormat::Markdown,
        );
        assert_no_active_markup(&result);
        assert!(!result.contains("alert(1)"), "{result}");
        assert!(result.contains("Hello"), "{result}");
    }

    #[test]
    fn render_org_strips_embedded_script() {
        // `@@html:…@@` is Org's inline-export escape hatch — the form that actually
        // reaches the output as raw HTML. (A `#+begin_export html` block is escaped
        // by orgize itself, so it never needed us.) Assert on the executable form:
        // the literal text `alert(1)` surviving *escaped* is harmless.
        let result = render(
            &parse_post_body(format!("Hello\n\n@@html:{ACTIVE_MARKUP}@@").as_str()),
            &PostFormat::Org,
        );
        assert_no_active_markup(&result);
        assert!(result.contains("Hello"), "{result}");
    }

    #[test]
    fn render_html_strips_embedded_script() {
        let result = render(
            &parse_post_body(format!("<p>hi</p>{ACTIVE_MARKUP}").as_str()),
            &PostFormat::Html,
        );
        assert_no_active_markup(&result);
        assert!(!result.contains("alert(1)"), "{result}");
        assert!(result.contains("<p>hi</p>"), "{result}");
    }

    // The `Html` format is sanitized like every other format (#445), so the
    // guarantee is "safe markup survives unchanged", not "the input survives".
    #[test]
    fn render_html_format_preserves_safe_markup() {
        let body = "<p>hi <b>there</b></p>";
        assert_eq!(
            render(&parse_post_body(body), &PostFormat::Html).as_ref(),
            body
        );
    }

    // -----------------------------------------------------------------------
    // Media references (#711)
    // -----------------------------------------------------------------------

    use common::media::MediaSource;
    use common::test_support::MEDIA_TEST_SHA256;

    fn media_url_for(name: &str) -> String {
        format!("/media/upload/e3/b0/{MEDIA_TEST_SHA256}/{name}")
    }

    #[test]
    fn extract_finds_a_markdown_image() {
        // Rendered via the real renderer, so this pins end-to-end behaviour rather than a
        // hand-written fragment.
        let body = parse_post_body(&format!("![alt]({})", media_url_for("photo.jpg")));
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media().filename.as_ref(), "photo.jpg");
    }

    #[test]
    fn extract_finds_a_raw_img_embedded_in_a_markdown_body() {
        // The rendered-HTML choice (spec D2): raw HTML passes through the Markdown parser.
        let body = parse_post_body(&format!("<img src=\"{}\">", media_url_for("photo.jpg")));
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media().filename.as_ref(), "photo.jpg");
    }

    #[test]
    fn extract_finds_a_raw_filename_spelling() {
        // The #675 regression, at the extractor level: a post addressing the file by the
        // name a person types must resolve to the stored, encoded spelling.
        let body = parse_post_body(&format!("<img src=\"{}\">", media_url_for("my photo.jpg")));
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media().filename.as_ref(), "my%20photo.jpg");
    }

    #[test]
    fn extract_finds_an_atompub_member_url_in_a_link() {
        let body = parse_post_body(&format!(
            "<a href=\"/atompub/alice/media/{MEDIA_TEST_SHA256}/photo.jpg\">doc</a>"
        ));
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media().source, MediaSource::Upload);
    }

    #[test]
    fn extract_ignores_media_in_stripped_elements_and_code_blocks() {
        // Sanitisation removes <video>, so it can never load and is not a reference.
        let video = parse_post_body(&format!(
            "<video src=\"{}\"></video>",
            media_url_for("clip.mp4")
        ));
        assert!(extract_media_refs(render(&video, &PostFormat::Markdown).as_ref()).is_empty());

        // A URL displayed as literal text points nobody at anything (spec D2).
        let fenced = parse_post_body(&format!("```\n{}\n```", media_url_for("photo.jpg")));
        assert!(extract_media_refs(render(&fenced, &PostFormat::Markdown).as_ref()).is_empty());
    }

    #[test]
    fn extract_deduplicates_and_sorts_complete_references() {
        let local = media_url_for("photo.jpg");
        let absolute = format!("https://example.com{local}?download=1");
        let scheme_relative = format!("//example.com:8443{local}?download=1");
        let body = parse_post_body(&format!(
            "<img src=\"{scheme_relative}\"><img src=\"{local}\"><img src=\"{absolute}\"><img src=\"{absolute}\">"
        ));
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 3, "only complete duplicate references collapse");
        assert_eq!(
            refs.iter()
                .map(common::media::MediaReference::reference_form)
                .collect::<Vec<_>>(),
            vec![local.as_str(), absolute.as_str(), scheme_relative.as_str()]
        );
    }

    #[test]
    fn extract_ignores_non_media_links() {
        let body = parse_post_body("<a href=\"https://example.com/page\">x</a>");
        assert!(extract_media_refs(render(&body, &PostFormat::Markdown).as_ref()).is_empty());
    }

    #[test]
    fn extract_walk_is_table_driven_not_tag_hardcoded() {
        // Drive the walk with a pair absent from MEDIA_URL_ATTRS. This fails if any tag
        // name is baked into the walk, which is what makes "adding <video> is a data edit"
        // a checked claim rather than a hope.
        let html = format!("<span data-src=\"{}\"></span>", media_url_for("photo.jpg"));
        let refs = extract_media_refs_with(&html, &[("span", "data-src")]);
        assert_eq!(refs.len(), 1);
        assert!(
            extract_media_refs(&html).is_empty(),
            "the real table does not pick it up"
        );
    }

    #[test]
    fn media_url_attrs_names_elements_literally() {
        // The walk compares element names literally, so a `"*"` in MEDIA_URL_ATTRS would
        // match nothing and silently extract nothing — a wildcard is not the way to say
        // "on every element" here. (`INERT_ATTRS` says that by listing bare names, which
        // the walk never consults.)
        let html = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        assert!(
            extract_media_refs_with(&html, &[("*", "src")]).is_empty(),
            "the walk must not honour a wildcard element"
        );
        assert!(
            MEDIA_URL_ATTRS.iter().all(|&(element, _)| element != "*"),
            "MEDIA_URL_ATTRS must name elements literally"
        );
    }

    #[test]
    fn render_output_derives_its_media_from_its_html() {
        let body = parse_post_body(&format!("<img src=\"{}\">", media_url_for("photo.jpg")));
        let out = render_with_media(&body, &PostFormat::Markdown);
        assert_eq!(
            out.media(),
            extract_media_refs(out.html().as_ref()).as_slice()
        );
        assert_eq!(out.media().len(), 1);
    }

    #[test]
    fn render_output_media_is_empty_for_a_body_referencing_nothing() {
        let out = render_with_media(&parse_post_body("plain text"), &PostFormat::Markdown);
        assert!(out.media().is_empty());
    }

    /// Whether `(tag, attr)` is classified: the **pair** is in `MEDIA_URL_ATTRS`, or the
    /// attribute **name** is in `INERT_ATTRS` (which is element-agnostic by
    /// construction — see its docs for the invariant that rests on).
    fn is_classified(tag: &str, attr: &str) -> bool {
        MEDIA_URL_ATTRS
            .iter()
            .any(|&(element, attribute)| element == tag && attribute == attr)
            || INERT_ATTRS.contains(&attr)
    }

    /// Permitted `(element, attribute)` pairs appearing in neither classification table.
    ///
    /// The enumeration is `tags × generic_attributes ∪ tag_attributes` — `generic_attributes`
    /// applies to every tag, so omitting that product would leave a hole. It is deliberately
    /// *not* filtered by any URL-attribute predicate: ammonia's `is_url_attr` is private, and
    /// a hand-written substitute would not recognise `srcset` as URL-bearing — the exact
    /// attribute this coupling is most likely to be widened with. So the assertion is
    /// inverted: every permitted pair must be classified, whether or not it looks like a URL.
    fn unclassified_sanitizer_pairs(builder: &ammonia::Builder<'_>) -> Vec<(String, String)> {
        let tags = builder.clone_tags();
        let generic_attributes = builder.clone_generic_attributes();
        let tag_attributes = builder.clone_tag_attributes();

        let generic_pairs = tags
            .iter()
            .flat_map(|tag| generic_attributes.iter().map(move |attr| (*tag, *attr)));
        let specific_pairs = tag_attributes
            .iter()
            .flat_map(|(tag, attrs)| attrs.iter().map(move |attr| (*tag, *attr)));

        let mut unclassified: Vec<(String, String)> = generic_pairs
            .chain(specific_pairs)
            .filter(|&(tag, attr)| !is_classified(tag, attr))
            .map(|(tag, attr)| (tag.to_owned(), attr.to_owned()))
            .collect();
        // Sorted so a failure reads the same on every run — the two ammonia collections are
        // hash sets, so iteration order is not stable.
        unclassified.sort();
        unclassified
    }

    /// Fails when `SANITIZER` permits an `(element, attribute)` pair that neither
    /// `MEDIA_URL_ATTRS` nor `INERT_ATTRS` classifies — i.e. someone widened the
    /// allowlist without saying whether the new attribute's value names media.
    ///
    /// To resolve: classify each reported pair. Into `MEDIA_URL_ATTRS` if its value is a
    /// URL a reader is pointed at, otherwise into `INERT_ATTRS` with a one-line reason —
    /// and only if the name names no URL on *any* element, which is that table's
    /// standing invariant. Silence is not an option, which is the whole point:
    /// separating the extractor's surface from the sanitiser's allowlist would otherwise
    /// recreate #711's own failure mode — widen the sanitiser, forget the extractor, and
    /// `post_media` quietly acquires a blind spot.
    #[test]
    fn sanitizer_surface_is_fully_classified() {
        let unclassified = unclassified_sanitizer_pairs(&SANITIZER);
        assert!(
            unclassified.is_empty(),
            "SANITIZER permits {unclassified:?}, which appear in neither MEDIA_URL_ATTRS nor \
             INERT_ATTRS. Classify each: add the pair to MEDIA_URL_ATTRS if its value names \
             media, otherwise add the attribute name to INERT_ATTRS with a reason."
        );
    }

    #[test]
    fn sanitizer_coupling_test_bites_when_the_allowlist_widens() {
        // Prove the guard can fail. A widened builder with an unclassified URL-bearing
        // attribute must be reported — otherwise the check above is decorative.
        let mut widened = ammonia::Builder::default();
        widened.add_tags(["video"]);
        // `src` is in MEDIA_URL_ATTRS — but paired with `img`, so `("video", "src")` is
        // still unclassified. That is what keeps the *pair* half of the rule meaningful
        // after INERT_ATTRS collapsed to bare names: a media name is never excused
        // element-agnostically. `poster` is a name nothing classifies at all.
        widened.add_tag_attributes("video", ["src", "poster"]);
        // A *generic* attribute too: it applies to every tag, so it is only reported if
        // the enumeration really forms `tags × generic_attributes`. Without that product
        // the check would look healthy while missing a whole axis.
        widened.add_generic_attributes(["data-poster"]);
        let unclassified = unclassified_sanitizer_pairs(&widened);
        for attr in ["src", "poster"] {
            assert!(
                unclassified.contains(&("video".to_owned(), attr.to_owned())),
                "the coupling check must flag the newly permitted, unclassified \
                 (video, {attr})"
            );
        }
        assert!(
            unclassified.contains(&("img".to_owned(), "data-poster".to_owned())),
            "a newly permitted generic attribute must be flagged on every tag"
        );
    }

    #[test]
    fn inert_attrs_lists_no_url_bearing_name() {
        // The invariant the name-level table rests on. An inert *name* excuses that
        // attribute on every element at once, so listing one that carries a URL anywhere
        // would open a blind spot everywhere — and silently, since the coupling check
        // would then read as healthy. These are the names a widening of `SANITIZER` is
        // most likely to bring in.
        for name in [
            "src",
            "href",
            "poster",
            "srcset",
            "data",
            "action",
            "formaction",
            "ping",
            "background",
            "longdesc",
            "usemap",
            "manifest",
        ] {
            assert!(
                !INERT_ATTRS.contains(&name),
                "{name} is URL-bearing somewhere and must never be listed inert"
            );
        }
        // `cite` is the deliberate exception (spec D2) — URL-valued, listed anyway. Pinned
        // so removing it reads as the scope change it is, not as a tidy-up.
        assert!(INERT_ATTRS.contains(&"cite"));
    }
}
