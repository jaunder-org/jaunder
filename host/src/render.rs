//! Host-side rendering and media-reference extraction.

use common::media::{self, MediaReference};
use common::post_body::PostBody;
use common::post_summary::{
    MAX_POST_SUMMARY_CHARS, PostSummary, normalize_summary_whitespace,
    truncate_at_first_sentence_or_word_boundary,
};
use common::post_title::PostTitle;
use common::render::{
    PostFormat, RenderedHtml, RenderedHtmlPart, RenderedPostTitle, TrustedProviderEmbed,
    assemble_rendered_html,
};
/// Renders `body` to HTML based on `format`. Pure, infallible function.
///
/// Parser output is author supplied, so every Markdown, Org, and HTML fragment
/// crosses [`common::render::sanitize`]. Markdown and Org additionally pass
/// only validated [`TrustedProviderEmbed`] values to the typed common-owned
/// assembly boundary; no source HTML or arbitrary URL can enter that path.
///
/// All three formats need sanitization, not just [`PostFormat::Html`]: the
/// Markdown and Org parsers both pass embedded raw HTML through untouched, so
/// `<script>` in a Markdown body reaches the output just as readily as in an
/// HTML one (#445).
///
/// Host-only: this module is owned by the host crate, so no build exposes a
/// weaker unsanitized implementation.
#[must_use]
pub fn render(body: &PostBody, format: &PostFormat) -> RenderedHtml {
    match format {
        PostFormat::Markdown => render_markdown_with_shortcodes(body),
        PostFormat::Org => render_org_with_shortcodes(body),
        PostFormat::Html => common::render::sanitize(body),
    }
}

/// Derives a fallback [`PostSummary`] from already-sanitized rendered HTML.
///
/// This remains host-owned so the browser and storage never acquire ammonia or HTML
/// handling. Ammonia strips elements without inventing separators; `html_escape` then
/// decodes the sanitizer's serialized text entities before Unicode whitespace is
/// normalized for the derived presentation value.
#[must_use]
pub fn summarize_rendered_html(html: &RenderedHtml) -> Option<PostSummary> {
    let stripped = ammonia::Builder::empty().clean(html.as_ref()).to_string();
    let decoded = html_escape::decode_html_entities(&stripped);
    let normalized = normalize_summary_whitespace(&decoded);
    if normalized.is_empty() {
        return None;
    }
    let text = truncate_at_first_sentence_or_word_boundary(&normalized, MAX_POST_SUMMARY_CHARS);
    let Ok(summary) = text.parse() else {
        unreachable!("normalized and bounded rendered text is a valid PostSummary");
    };
    Some(summary)
}

/// A provider-neutral source token. Provider dispatch remains at this host seam;
/// common accepts only the resulting closed validated value.
#[derive(Clone, Copy)]
struct Shortcode<'a> {
    provider: &'a str,
    id: &'a str,
}

fn parse_shortcode(line: &str) -> Option<Shortcode<'_>> {
    let line = line.strip_prefix("{{<")?.strip_suffix(">}}")?;
    let tokens: Vec<_> = line
        .split([' ', '\t'])
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.len() != 2
        || !(line.starts_with(' ') || line.starts_with('\t'))
        || !(line.ends_with(' ') || line.ends_with('\t'))
    {
        return None;
    }
    Some(Shortcode {
        provider: tokens[0],
        id: tokens[1],
    })
}

type ProviderConstructor = fn(&str) -> Option<TrustedProviderEmbed>;

fn youtube_provider(id: &str) -> Option<TrustedProviderEmbed> {
    TrustedProviderEmbed::youtube(id).ok()
}

fn vimeo_provider(id: &str) -> Option<TrustedProviderEmbed> {
    TrustedProviderEmbed::vimeo(id).ok()
}

const POST_SHORTCODE_PROVIDERS: &[(&str, ProviderConstructor)] =
    &[("youtube", youtube_provider), ("vimeo", vimeo_provider)];

fn dispatch_shortcode_with(
    shortcode: Shortcode<'_>,
    providers: &[(&str, ProviderConstructor)],
) -> Option<TrustedProviderEmbed> {
    providers
        .iter()
        .find(|(name, _)| *name == shortcode.provider)
        .and_then(|(_, constructor)| constructor(shortcode.id))
}

fn shortcode_line_with(
    line: &str,
    providers: &[(&str, ProviderConstructor)],
) -> Option<TrustedProviderEmbed> {
    let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
    (indentation <= 3)
        .then(|| line.get(indentation..))
        .flatten()
        .and_then(|line| parse_shortcode(line.trim_end_matches([' ', '\t', '\r', '\n'])))
        .and_then(|shortcode| dispatch_shortcode_with(shortcode, providers))
}

fn shortcode_line(line: &str) -> Option<TrustedProviderEmbed> {
    shortcode_line_with(line, POST_SHORTCODE_PROVIDERS)
}

fn marker(source: &str, index: usize) -> String {
    marker_with_nonce(source, index, rand::random)
}

fn marker_with_nonce(source: &str, index: usize, mut next_nonce: impl FnMut() -> u128) -> String {
    loop {
        let marker = format!("JAUNDER_SHORTCODE_{:032x}_{index}", next_nonce());
        if !source.contains(&marker) {
            return marker;
        }
    }
}

fn assemble_with_markers(html: &str, markers: Vec<(String, TrustedProviderEmbed)>) -> RenderedHtml {
    if markers.is_empty() {
        return common::render::sanitize(html);
    }
    let mut owned_parts = Vec::new();
    let mut remaining = html;
    for (marker, embed) in markers {
        let placeholder = format!("<!--{marker}-->");
        let Some((before, after)) = remaining.split_once(&placeholder) else {
            unreachable!("a generated shortcode marker must be emitted exactly once");
        };
        if after.contains(&placeholder) {
            unreachable!("a generated shortcode marker must be emitted exactly once");
        }
        owned_parts.push((before.to_owned(), Some(embed)));
        remaining = after;
    }
    owned_parts.push((remaining.to_owned(), None));
    let parts: Vec<_> = owned_parts
        .iter()
        .flat_map(|(html, embed)| {
            let mut parts = vec![RenderedHtmlPart::Untrusted(html)];
            if let Some(embed) = embed {
                parts.push(RenderedHtmlPart::Embed(embed));
            }
            parts
        })
        .collect();
    assemble_rendered_html(&parts)
}

fn markdown_options() -> pulldown_cmark::Options {
    use pulldown_cmark::Options;
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);
    options
}

/// Renders Markdown to HTML using one complete pulldown-cmark event stream.
fn render_markdown(body: &str) -> String {
    use pulldown_cmark::{Parser, html};
    let mut html_output = String::new();
    html::push_html(&mut html_output, Parser::new_ext(body, markdown_options()));
    html_output
}

fn render_markdown_with_shortcodes(body: &str) -> RenderedHtml {
    render_markdown_with_shortcodes_using(body, POST_SHORTCODE_PROVIDERS)
}

fn render_markdown_with_shortcodes_using(
    body: &str,
    providers: &[(&str, ProviderConstructor)],
) -> RenderedHtml {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd, html};
    let mut markers = Vec::new();
    let mut events = Vec::new();
    let mut depth = 0_usize;
    let mut skipped = false;
    for (event, range) in Parser::new_ext(body, markdown_options()).into_offset_iter() {
        if skipped {
            if matches!(event, Event::End(TagEnd::Paragraph)) {
                skipped = false;
            }
            continue;
        }
        if let Event::Start(Tag::Paragraph) = event
            && depth == 0
            && let Some(embed) = body
                .get(range)
                .and_then(|line| shortcode_line_with(line, providers))
        {
            let marker = marker(body, markers.len());
            events.push(Event::Html(format!("<!--{marker}-->").into()));
            markers.push((marker, embed));
            skipped = true;
            continue;
        }
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        events.push(event);
    }
    let mut html = String::new();
    html::push_html(&mut html, events.into_iter());
    assemble_with_markers(&html, markers)
}

/// Renders Org-mode to HTML using orgize.
fn render_org(body: &str) -> String {
    orgize::Org::parse(body).to_html()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OrgContainer {
    Document,
    Section,
    Other,
}

/// Intercepts only direct document-section paragraphs while preserving orgize's
/// one complete exporter traversal for every other syntax node.
struct OrgShortcodeExport<'a> {
    source: &'a str,
    html: orgize::export::HtmlExport,
    containers: Vec<OrgContainer>,
    markers: Vec<(String, TrustedProviderEmbed)>,
}

impl OrgShortcodeExport<'_> {
    /// Returns a shortcode-looking line hidden by Org's normal exporter.
    ///
    /// Comments and drawers otherwise remain omitted. This limited fallback
    /// emits only a shortcode-looking source line as sanitized literal text.
    fn hidden_shortcode_literal(container: &orgize::export::Container) -> Option<String> {
        let raw = match container {
            orgize::export::Container::Comment(node) => node.raw(),
            orgize::export::Container::CommentBlock(node) => node.raw(),
            orgize::export::Container::Drawer(node) => node.raw(),
            orgize::export::Container::PropertyDrawer(node) => node.raw(),
            _ => unreachable!("hidden Org shortcode fallback received an unsupported container"),
        };
        let literal = raw
            .lines()
            .filter_map(|line| {
                let line = line.strip_prefix("# ").unwrap_or(line);
                line.find("{{<").and_then(|start| line.get(start..))
            })
            .collect::<Vec<_>>()
            .join("\n");
        (!literal.is_empty()).then_some(literal)
    }

    fn escape_html_text(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn container(container: &orgize::export::Container) -> OrgContainer {
        match container {
            orgize::export::Container::Document(_) => OrgContainer::Document,
            orgize::export::Container::Section(_) => OrgContainer::Section,
            _ => OrgContainer::Other,
        }
    }
}

impl orgize::export::Traverser for OrgShortcodeExport<'_> {
    fn event(&mut self, event: orgize::export::Event, ctx: &mut orgize::export::TraversalContext) {
        match event {
            orgize::export::Event::Enter(
                container @ (orgize::export::Container::Comment(_)
                | orgize::export::Container::CommentBlock(_)
                | orgize::export::Container::Drawer(_)
                | orgize::export::Container::PropertyDrawer(_)),
            ) => {
                if let Some(literal) = Self::hidden_shortcode_literal(&container) {
                    let literal = Self::escape_html_text(&literal);
                    self.html.push_str(format!("<p>{literal}</p>"));
                    ctx.skip();
                    return;
                }
                self.containers.push(Self::container(&container));
                self.html
                    .event(orgize::export::Event::Enter(container), ctx);
            }
            orgize::export::Event::Enter(orgize::export::Container::Paragraph(paragraph))
                if self.containers == [OrgContainer::Document, OrgContainer::Section] =>
            {
                if let Some(embed) = shortcode_line(&paragraph.raw()) {
                    let marker = marker(self.source, self.markers.len());
                    self.html.push_str(format!("<!--{marker}-->"));
                    self.markers.push((marker, embed));
                    ctx.skip();
                    return;
                }
                self.containers.push(OrgContainer::Other);
                self.html.event(
                    orgize::export::Event::Enter(orgize::export::Container::Paragraph(paragraph)),
                    ctx,
                );
            }
            orgize::export::Event::Enter(container) => {
                self.containers.push(Self::container(&container));
                self.html
                    .event(orgize::export::Event::Enter(container), ctx);
            }
            orgize::export::Event::Leave(container) => {
                self.html
                    .event(orgize::export::Event::Leave(container), ctx);
                if self.containers.pop().is_none() {
                    unreachable!("orgize emitted a leave event without an entered container");
                }
            }
            event => self.html.event(event, ctx),
        }
    }
}

fn render_org_with_shortcodes(body: &str) -> RenderedHtml {
    let org = orgize::Org::parse(body);
    let mut export = OrgShortcodeExport {
        source: body,
        html: orgize::export::HtmlExport::default(),
        containers: Vec::new(),
        markers: Vec::new(),
    };
    org.traverse(&mut export);
    let html = export.html.finish();
    assemble_with_markers(&html, export.markers)
}

/// Renders a Post's title and body projections as one inseparable write aggregate.
///
/// The title source, format, and both derived fragments travel together so storage
/// cannot bind a title from one authoring input with derivatives from another.
#[must_use]
pub fn render_post(
    title: Option<PostTitle>,
    body: PostBody,
    format: PostFormat,
) -> PostRenderOutput {
    let rendered_html = render(&body, &format);
    let media = extract_media_refs(rendered_html.as_ref());
    let rendered_title = title.as_ref().map(|title| render_title(title, &format));
    PostRenderOutput {
        title,
        body,
        format,
        rendered_title,
        rendered_html,
        media,
    }
}

/// Renders an authored title through the common ammonia-owned title policy.
///
/// Markdown and Org first use their ordinary renderers. The common boundary then
/// sanitizes every authoring format into the persisted inline fragment grammar.
#[must_use]
pub fn render_title(title: &PostTitle, format: &PostFormat) -> RenderedPostTitle {
    let source = match format {
        PostFormat::Markdown => render_markdown(title.as_ref()),
        PostFormat::Org => render_org(title.as_ref()),
        PostFormat::Html => title.to_string(),
    };
    common::render::sanitize_post_title(&source)
}

/// Projects a persisted Rendered Title into readable text for RSS and JSON Feed.
#[must_use]
pub fn rendered_title_visible_text(title: &RenderedPostTitle) -> String {
    common::render::rendered_post_title_visible_text(title)
}

/// The complete authored and rendered state for one Post write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostRenderOutput {
    title: Option<PostTitle>,
    body: PostBody,
    format: PostFormat,
    rendered_title: Option<RenderedPostTitle>,
    rendered_html: RenderedHtml,
    media: Vec<MediaReference>,
}

impl PostRenderOutput {
    #[must_use]
    pub fn title(&self) -> Option<&PostTitle> {
        self.title.as_ref()
    }
    #[must_use]
    pub fn body(&self) -> &PostBody {
        &self.body
    }
    #[must_use]
    pub const fn format(&self) -> PostFormat {
        self.format
    }
    #[must_use]
    pub fn rendered_title(&self) -> Option<&RenderedPostTitle> {
        self.rendered_title.as_ref()
    }
    #[must_use]
    pub fn rendered_html(&self) -> &RenderedHtml {
        &self.rendered_html
    }
    #[must_use]
    pub fn media(&self) -> &[MediaReference] {
        &self.media
    }
    /// Consumes the aggregate when only its rendered body remains needed.
    #[must_use]
    pub fn into_rendered_html(self) -> RenderedHtml {
        self.rendered_html
    }
}

/// The `(element, attribute)` pairs whose values name media. When common's
/// sanitizer permits an element/attribute pair, its URL-bearing attributes belong
/// here — the walk knows no tag names of its own, so extending to
/// `<video>`/`<audio>` is a **data edit** (`("video", "src")`,
/// `("video", "poster")`, `("audio", "src")`, …) with no change to
/// `extract_media_refs_with`, which `extract_walk_is_table_driven_not_tag_hardcoded`
/// keeps a checked claim rather than a hope.
///
/// One attribute, one URL. A multi-URL attribute such as `srcset` does not fit this
/// shape and needs the table widened to carry a per-attribute parse mode before it can
/// be listed here.
///
/// Public because it is the contract of [`extract_media_refs`]: what the walk looks at
/// is data, and reviewable as data. Its counterpart is [`INERT_ATTRS`], and between them
/// they must cover common's complete sanitizer surface —
/// `sanitizer_surface_is_fully_classified` enforces that.
pub const MEDIA_URL_ATTRS: &[(&str, &str)] = &[
    ("a", "href"),
    ("img", "src"),
    ("audio", "src"),
    ("video", "src"),
    ("video", "poster"),
    ("source", "src"),
    ("track", "src"),
];

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
/// element that carries it, or simply absent when common's sanitizer does not permit it,
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
    // Media presentation, format and caption metadata. URL-bearing `src`/`poster`
    // remain pair-classified in `MEDIA_URL_ATTRS`.
    "controls", "default", "kind", "label", "srclang", "type",
    // Quotation provenance — URL-valued, and deliberately out of scope; see above.
    "cite",     // Edit timestamps (`<del>`/`<ins>`).
    "datetime", // Text direction (`<bdo>`).
    "dir",
    // Presentational geometry and alignment, on tables, columns, rules, images and video.
    "align", "char", "charoff", "colspan", "headers", "height", "rowspan", "scope", "size", "span",
    "summary", "width", // List numbering (`<ol>`).
    "start",
    // Common's fenced-code widening: the `language-*` marker whose values the
    // attribute filter already narrows.
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
                    .filter_map(|attr| media::parse_media_url(&attr.value)),
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
/// The reference set is private and [`with_media`] is the only constructor, so a
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
/// # use host::render::{RenderOutput, render, with_media};
/// # let body: PostBody = "hello".parse().unwrap();
/// let other: PostBody = "different".parse().unwrap();
/// let out = with_media(&body, &PostFormat::Markdown);
/// assert!(out.media().is_empty());
/// let _direct = render(&body, &PostFormat::Markdown); // `render` resolves
/// let _other = render(&other, &PostFormat::Markdown); // the last negative's fixture
/// ```
/// and a struct literal cannot smuggle one in:
/// ```compile_fail
/// # use common::post_body::PostBody;
/// # use common::render::PostFormat;
/// # use host::render::{RenderOutput, render, with_media};
/// # let body: PostBody = "hello".parse().unwrap();
/// let html = render(&body, &PostFormat::Markdown);
/// let _ = RenderOutput { html, media: vec![] }; // private field
/// ```
/// nor can the HTML be swapped out from under the set that describes it — the same
/// desynchronisation reached from the other side, which a `pub html` would have left open:
/// ```compile_fail
/// # use common::post_body::PostBody;
/// # use common::render::PostFormat;
/// # use host::render::{RenderOutput, render, with_media};
/// # let body: PostBody = "hello".parse().unwrap();
/// # let other: PostBody = "different".parse().unwrap();
/// let mut out = with_media(&body, &PostFormat::Markdown);
/// out.html = render(&other, &PostFormat::Markdown); // private field
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOutput {
    /// The sanitized HTML, as [`render`] produced it. Private for the same reason
    /// `media` is: a `pub` field here would let a caller assign a *different* HTML over
    /// the references derived from the original, which is the same desynchronisation the
    /// private `media` field exists to prevent — reached from the other side.
    html: RenderedHtml,
    /// What that HTML points a reader at. Private — see the type's docs.
    media: Vec<MediaReference>,
}

/// Renders a body and derives its media references from its sanitized HTML.
#[must_use]
pub fn with_media(body: &PostBody, format: &PostFormat) -> RenderOutput {
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
    use common::test_support::{parse_post_body, rendered_html};

    #[test]
    fn rendered_body_summary_strips_elements_and_decodes_normalized_text() {
        let html =
            rendered_html("<p>  Hello&nbsp;<strong>world</strong> &amp;\u{2003}friends.  </p>");

        assert_eq!(
            summarize_rendered_html(&html).as_deref(),
            Some("Hello world & friends.")
        );
    }

    #[test]
    fn rendered_body_summary_is_equivalent_across_authoring_formats() {
        // TDD: fallback metadata must derive only from rendered text, not source markup.
        let summaries = [
            ("A *shared* summary. trailing", PostFormat::Markdown),
            ("A *shared* summary. trailing", PostFormat::Org),
            (
                "A <strong>shared</strong> summary. trailing",
                PostFormat::Html,
            ),
        ]
        .map(|(source, format)| {
            let body = parse_post_body(source);
            summarize_rendered_html(&render(&body, &format))
        });

        assert_eq!(
            summaries,
            [
                Some("A shared summary.".parse().unwrap()),
                Some("A shared summary.".parse().unwrap()),
                Some("A shared summary.".parse().unwrap()),
            ]
        );
    }

    #[test]
    fn rendered_body_summary_does_not_invent_element_separators_or_text() {
        assert_eq!(
            summarize_rendered_html(&rendered_html("<span>first</span><span>second</span>"))
                .as_deref(),
            Some("firstsecond")
        );
        assert_eq!(
            summarize_rendered_html(&rendered_html("<img src=\"/image.png\">")),
            None
        );
    }

    #[test]
    fn rendered_body_summary_uses_lexical_sentence_boundaries() {
        assert_eq!(
            summarize_rendered_html(&rendered_html("A sentence.” Then another.")).as_deref(),
            Some("A sentence.”")
        );
        assert_eq!(
            summarize_rendered_html(&rendered_html("Dr. Example continued.")).as_deref(),
            Some("Dr.")
        );
        assert_eq!(
            summarize_rendered_html(&rendered_html("Value 3.14 remains.")).as_deref(),
            Some("Value 3.")
        );
    }

    #[test]
    fn rendered_body_summary_obeys_word_and_scalar_limits() {
        let word_bounded = format!("{} trailing", "word ".repeat(100));
        let summary = summarize_rendered_html(&rendered_html(&word_bounded)).unwrap();
        assert!(summary.chars().count() <= common::post_summary::MAX_POST_SUMMARY_CHARS);
        assert!(!summary.ends_with("trailing"));
        assert!(!summary.ends_with(char::is_whitespace));

        let scalar_bounded = "é".repeat(common::post_summary::MAX_POST_SUMMARY_CHARS + 1);
        let summary = summarize_rendered_html(&rendered_html(&scalar_bounded)).unwrap();
        assert_eq!(
            summary.chars().count(),
            common::post_summary::MAX_POST_SUMMARY_CHARS
        );
        assert!(summary.chars().all(|character| character == 'é'));
    }

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

    // -- Markdown tests --

    #[test]
    fn post_shortcodes_expand_only_as_eligible_markdown_and_org_paragraphs() {
        let youtube = "{{< youtube dQw4w9WgXcQ >}}";
        let vimeo = "   {{<\tvimeo\t123456789\t>}}   ";
        for (format, body) in [
            (
                PostFormat::Markdown,
                format!("before\n\n{youtube}\n\n{vimeo}\n\nafter"),
            ),
            (
                PostFormat::Org,
                format!("before\n\n{youtube}\n\n{vimeo}\n\nafter"),
            ),
        ] {
            let body = parse_post_body(&body);
            let original = body.clone();
            let rendered = render(&body, &format);
            assert_eq!(body, original);
            assert!(
                rendered.contains("youtube-nocookie.com/embed/dQw4w9WgXcQ"),
                "{rendered}"
            );
            assert!(
                rendered.contains("player.vimeo.com/video/123456789"),
                "{rendered}"
            );
            assert!(rendered.find("before").unwrap() < rendered.find("youtube-nocookie").unwrap());
            assert!(
                rendered.find("youtube-nocookie").unwrap() < rendered.find("player.vimeo").unwrap()
            );
            assert!(rendered.find("player.vimeo").unwrap() < rendered.find("after").unwrap());
            assert!(!rendered.contains("JAUNDER_SHORTCODE_"), "{rendered}");
            assert!(extract_media_refs(rendered.as_ref()).is_empty());
        }
    }

    #[test]
    fn post_shortcode_grammar_accepts_only_exact_ascii_token_boundaries() {
        for (line, accepted) in [
            ("{{< youtube dQw4w9WgXcQ >}}", true),
            ("{{<\tyoutube\tdQw4w9WgXcQ\t>}}", true),
            ("{{<  youtube \t dQw4w9WgXcQ  \t>}}", true),
            ("{{<youtube dQw4w9WgXcQ >}}", false),
            ("{{< youtubedQw4w9WgXcQ >}}", false),
            ("{{< youtube dQw4w9WgXcQ>}}", false),
            ("{{< youtube dQw4w9WgXcQ > }}", false),
            ("{{< youtube dQw4w9WgXcQ }}", false),
            ("{{< youtube dQw4w9WgXcQ >}", false),
            ("{{< youtube\u{a0}dQw4w9WgXcQ >}}", false),
            ("{{ youtube dQw4w9WgXcQ >}}", false),
            ("{{< youtube dQw4w9WgXcQ extra >}}", false),
        ] {
            assert_eq!(parse_shortcode(line).is_some(), accepted, "{line}");
        }

        for indentation in 0..=3 {
            let line = format!(
                "{}{{{{< youtube dQw4w9WgXcQ >}}}} \t",
                " ".repeat(indentation)
            );
            assert!(shortcode_line(&line).is_some(), "{line}");
        }
        assert!(shortcode_line("    {{< youtube dQw4w9WgXcQ >}}").is_none());
        for trailing in [" ", "\t"] {
            assert!(
                shortcode_line(&["{{< youtube dQw4w9WgXcQ >}}", trailing].concat()).is_some(),
                "{trailing:?}"
            );
        }

        for source in [
            "{{< YouTube dQw4w9WgXcQ >}}",
            "{{< unknown dQw4w9WgXcQ >}}",
            "{{< youtube dQw4w9WgXcQ?start=1 >}}",
            "{{< youtube dQw4w9WgXc >}}",
            "{{< vimeo 0 >}}",
        ] {
            assert!(shortcode_line(source).is_none(), "{source}");
        }
    }

    #[test]
    fn post_shortcode_grammar_and_ineligible_markdown_contexts_remain_literal() {
        let valid = "{{< youtube dQw4w9WgXcQ >}}";
        for source in [
            "{{< YouTube dQw4w9WgXcQ >}}",
            "{{< youtube dQw4w9WgXcQ extra >}}",
            "{{< youtube dQw4w9WgXcQ?start=1 >}}",
            "{{< unknown dQw4w9WgXcQ >}}",
            "{{< youtube dQw4w9WgXcQ >}",
            &format!("`{valid}`"),
            &format!("```\n{valid}\n```"),
            &format!("~~~\n{valid}\n~~~"),
            &format!("    {valid}"),
            &format!("- {valid}\n  - {valid}"),
            &format!("> {valid}"),
            &format!("ordinary {valid}"),
        ] {
            let rendered = render(&parse_post_body(source), &PostFormat::Markdown);
            assert!(!rendered.contains("<iframe"), "{source:?}: {rendered}");
            assert!(
                rendered.contains("youtube")
                    || rendered.contains("YouTube")
                    || rendered.contains("unknown")
            );
        }
    }

    #[test]
    fn post_shortcode_org_and_html_suppression_preserve_source() {
        let valid = "{{< youtube dQw4w9WgXcQ >}}";
        for source in [
            &format!("~{valid}~"),
            &format!("={valid}="),
            &format!("#+begin_src text\n{valid}\n#+end_src"),
            &format!("#+begin_example\n{valid}\n#+end_example"),
            &format!("#+begin_export html\n{valid}\n#+end_export"),
            &format!("#+begin_quote\n{valid}\n#+end_quote"),
            &format!("#+begin_verse\n{valid}\n#+end_verse"),
            &format!(": {valid}"),
            &format!(":PROPERTIES:\n:VALUE: {valid}\n:END:"),
            &format!(":LOGBOOK:\n{valid}\n:END:"),
            &format!("| {valid} |"),
            &format!("- list item\n  {valid}"),
            &format!("- outer\n  - inner\n    {valid}"),
            &format!("# {valid}"),
            &format!("#+begin_comment\n{valid}\n#+end_comment"),
            &format!("* Headline\n{valid}"),
        ] {
            let body = parse_post_body(source);
            let rendered = render(&body, &PostFormat::Org);
            assert!(!rendered.contains("<iframe"), "{source:?}: {rendered}");
            assert!(rendered.contains("dQw4w9WgXcQ"), "{source:?}: {rendered}");
            assert_eq!(body.as_ref(), source);
        }
        for source in [
            "# {{< unknown opaque >}}",
            ":LOGBOOK:\n{{< youtube not-an-id >}}\n:END:",
        ] {
            let rendered = render(&parse_post_body(source), &PostFormat::Org);
            assert!(!rendered.contains("<iframe"), "{source:?}: {rendered}");
            assert!(rendered.contains("{{&lt;"), "{source:?}: {rendered}");
        }
        let multiple_hidden = render(
            &parse_post_body(":LOGBOOK:\n{{< unknown first >}}\n{{< unknown second >}}\n:END:"),
            &PostFormat::Org,
        );
        assert!(multiple_hidden.contains("first"), "{multiple_hidden}");
        assert!(multiple_hidden.contains("second"), "{multiple_hidden}");

        let hidden_markup = render(
            &parse_post_body(
                "# {{< unknown opaque >}}<img src=\"https://evil.example/active.png\">",
            ),
            &PostFormat::Org,
        );
        assert!(!hidden_markup.contains("<img"), "{hidden_markup}");
        assert!(hidden_markup.contains("&lt;img"), "{hidden_markup}");

        for source in [
            "# ordinary comment",
            "#+begin_comment\nordinary comment\n#+end_comment",
            ":LOGBOOK:\nordinary drawer value\n:END:",
            ":PROPERTIES:\n:VALUE: ordinary property\n:END:",
        ] {
            assert_eq!(
                render(&parse_post_body(source), &PostFormat::Org),
                common::render::sanitize(&render_org(source)),
                "ordinary hidden Org content changed: {source:?}"
            );
        }

        let html = render(&parse_post_body(valid), &PostFormat::Html);
        assert!(!html.contains("<iframe"), "{html}");
    }

    #[test]
    fn markdown_shortcode_preserves_reference_and_footnote_rendering() {
        let shortcode = "{{< youtube dQw4w9WgXcQ >}}";
        let source = format!(
            "before [reference][site] and a footnote[^note]\n\n{shortcode}\n\nafter\n\n[site]: https://example.com/path\n[^note]: retained footnote"
        );
        let rendered = render(&parse_post_body(&source), &PostFormat::Markdown);
        let ordinary = common::render::sanitize(&render_markdown(&source));

        for expected in [
            r#"<a href="https://example.com/path" rel="noopener noreferrer">reference</a>"#,
            "retained footnote",
        ] {
            assert!(ordinary.contains(expected), "ordinary: {ordinary}");
            assert!(rendered.contains(expected), "rendered: {rendered}");
        }
        assert!(rendered.contains("youtube-nocookie.com/embed/dQw4w9WgXcQ"));
    }

    #[test]
    fn fixture_provider_adds_an_isolated_dispatch_entry_without_changing_assembly() {
        fn fixture_provider(id: &str) -> Option<TrustedProviderEmbed> {
            (id == "fixture-id").then(TrustedProviderEmbed::fixture)
        }

        let rendered = render_markdown_with_shortcodes_using(
            "before\n\n{{< fixture fixture-id >}}\n\nafter",
            &[("fixture", fixture_provider)],
        );
        assert!(
            rendered.contains("https://fixture.invalid/player/fixture-video"),
            "{rendered}"
        );
        assert!(rendered.contains("Watch fixture video"), "{rendered}");
        assert!(!rendered.contains("youtube-nocookie.com"), "{rendered}");
        assert!(rendered.find("before").unwrap() < rendered.find("fixture.invalid").unwrap());
        assert!(rendered.find("fixture.invalid").unwrap() < rendered.find("after").unwrap());
    }

    #[test]
    fn org_shortcode_preserves_cross_document_reference_and_footnote_rendering() {
        let shortcode = "{{< youtube dQw4w9WgXcQ >}}";
        let source = format!(
            "before [[https://example.com/path][reference]] and [fn:note]\n\n{shortcode}\n\nafter\n\n[fn:note] retained footnote"
        );
        let rendered = render(&parse_post_body(&source), &PostFormat::Org);
        let ordinary = common::render::sanitize(&render_org(&source));

        for expected in ["reference", "retained footnote"] {
            assert!(ordinary.contains(expected), "ordinary: {ordinary}");
            assert!(rendered.contains(expected), "rendered: {rendered}");
        }
        assert!(rendered.contains("youtube-nocookie.com/embed/dQw4w9WgXcQ"));
    }

    #[test]
    fn provider_dispatch_shares_grammar_and_keeps_unknown_names_literal() {
        let future = parse_shortcode("{{< future-provider opaque-id >}}")
            .expect("provider-neutral grammar accepts future provider tokens");
        assert_eq!(future.provider, "future-provider");
        assert_eq!(future.id, "opaque-id");
        assert!(dispatch_shortcode_with(future, POST_SHORTCODE_PROVIDERS).is_none());

        for (source, provider_url) in [
            (
                "{{< youtube dQw4w9WgXcQ >}}",
                "youtube-nocookie.com/embed/dQw4w9WgXcQ",
            ),
            (
                "{{< vimeo 123456789 >}}",
                "player.vimeo.com/video/123456789",
            ),
        ] {
            let rendered = render(&parse_post_body(source), &PostFormat::Markdown);
            assert!(rendered.contains(provider_url), "{source:?}: {rendered}");
        }
        for source in [
            "{{< unknown dQw4w9WgXcQ >}}",
            "{{< youtube dQw4w9WgXcQ extra >}}",
            "{{< vimeo 123456789 extra >}}",
        ] {
            let rendered = render(&parse_post_body(source), &PostFormat::Markdown);
            assert!(!rendered.contains("<iframe"), "{source:?}: {rendered}");
        }
    }

    #[test]
    fn shortcode_marker_retries_a_source_collision() {
        let colliding_nonce = 7_u128;
        let source = format!("JAUNDER_SHORTCODE_{colliding_nonce:032x}_0");
        let mut nonces = [colliding_nonce, colliding_nonce + 1].into_iter();
        let generated = marker_with_nonce(&source, 0, || {
            nonces.next().expect("two marker attempts suffice")
        });
        assert_eq!(
            generated,
            format!("JAUNDER_SHORTCODE_{:032x}_0", colliding_nonce + 1)
        );
    }

    #[test]
    fn shortcode_markers_cannot_be_forged_and_raw_iframes_stay_stripped() {
        let source = concat!(
            "<!--JAUNDER_SHORTCODE_00000000000000000000000000000000_0-->",
            "<iframe src=\"https://evil.example\"></iframe>\n\n",
            "{{< youtube dQw4w9WgXcQ >}}"
        );
        let rendered = render(&parse_post_body(source), &PostFormat::Markdown);
        assert!(!rendered.contains("evil.example"), "{rendered}");
        assert!(!rendered.contains("JAUNDER_SHORTCODE_"), "{rendered}");
        assert!(!rendered.contains("-->"), "{rendered}");
        assert!(!rendered.contains("--&gt;"), "{rendered}");
        assert!(rendered.contains("youtube-nocookie.com"), "{rendered}");
    }

    #[test]
    fn trusted_provider_frames_are_external_embeds_not_media_references() {
        use common::render::{RenderedHtmlPart, TrustedProviderEmbed, assemble_rendered_html};

        let embed = TrustedProviderEmbed::youtube("dQw4w9WgXcQ").expect("valid YouTube ID");
        let html = assemble_rendered_html(&[RenderedHtmlPart::Embed(&embed)]);

        assert!(html.contains("<iframe"), "{html}");
        assert!(extract_media_refs(html.as_ref()).is_empty());
        assert!(
            !MEDIA_URL_ATTRS.contains(&("iframe", "src")),
            "provider frames are external presentation, not Media references"
        );
    }

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

    #[test]
    fn render_preserves_safe_media_elements_in_every_post_format() {
        let media = concat!(
            r#"<video src="/media/video.mp4" poster="/media/poster.jpg" "#,
            r#"width="640" height="360"><source src="/media/video.webm" "#,
            r#"type="video/webm"><source src="/media/video.ogv" "#,
            r#"type="video/ogg"><track src="/media/captions.vtt" kind="captions" "#,
            r#"srclang="en" label="English" default>Video unavailable</video>"#,
            r#"<audio src="/media/audio.mp3" controls>Audio unavailable</audio>"#,
        );
        let bodies = [
            (PostFormat::Html, media.to_owned()),
            (PostFormat::Markdown, media.to_owned()),
            (PostFormat::Org, format!("@@html:{media}@@")),
        ];

        for (format, body) in bodies {
            let html = render(&parse_post_body(&body), &format);
            for expected in [
                "<video",
                "/media/video.webm",
                "/media/video.ogv",
                "<track",
                "Video unavailable",
                "<audio",
                "controls",
                "Audio unavailable",
            ] {
                assert!(
                    html.contains(expected),
                    "{format:?} stripped {expected} from: {html}"
                );
            }
        }
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
    fn extract_finds_media_in_safe_media_elements_and_ignores_code_blocks() {
        for (element, attribute, filename) in [
            ("audio", "src", "sound.mp3"),
            ("video", "src", "clip.mp4"),
            ("video", "poster", "poster.jpg"),
            ("source", "src", "clip.webm"),
            ("track", "src", "captions.vtt"),
        ] {
            let body = parse_post_body(&format!(
                "<{element} {attribute}=\"{}\"></{element}>",
                media_url_for(filename)
            ));
            let refs = extract_media_refs(render(&body, &PostFormat::Html).as_ref());
            assert_eq!(refs.len(), 1, "{element}[{attribute}] was not extracted");
            assert_eq!(refs[0].media().filename.as_ref(), filename);
        }

        let external_track = parse_post_body(r#"<track src="https://example.com/captions.vtt">"#);
        assert!(
            extract_media_refs(render(&external_track, &PostFormat::Html).as_ref()).is_empty(),
            "a non-Media-shaped external track must not become a Media reference"
        );

        // A URL displayed as literal text points nobody at anything (spec D2).
        let fenced = parse_post_body(&format!("```\n{}\n```", media_url_for("photo.jpg")));
        assert!(extract_media_refs(render(&fenced, &PostFormat::Markdown).as_ref()).is_empty());
    }

    #[test]
    fn extract_deduplicates_and_sorts_complete_references_from_media_elements() {
        let local = media_url_for("photo.jpg");
        let absolute = format!("https://example.com{local}?download=1");
        let scheme_relative = format!("//example.com:8443{local}?download=1");
        let body = parse_post_body(&format!(
            "<video src=\"{scheme_relative}\"></video><source src=\"{local}\">\
             <track src=\"{absolute}\"><audio src=\"{absolute}\"></audio>"
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
        let out = with_media(&body, &PostFormat::Markdown);
        assert_eq!(
            out.media(),
            extract_media_refs(out.html().as_ref()).as_slice()
        );
        assert_eq!(out.media().len(), 1);
    }

    #[test]
    fn render_output_media_is_empty_for_a_body_referencing_nothing() {
        let out = with_media(&parse_post_body("plain text"), &PostFormat::Markdown);
        assert!(out.media().is_empty());
    }

    #[test]
    fn render_output_into_html_consumes_the_derived_media_pair() {
        let out = with_media(&parse_post_body("plain text"), &PostFormat::Markdown);
        assert_eq!(out.into_html().as_ref(), "<p>plain text</p>\n");
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
    /// The common-owned sanitizer exposes its complete output surface as pairs. This
    /// assertion is deliberately inverted: every permitted pair must be classified,
    /// whether or not it looks URL-valued.
    fn unclassified_sanitizer_pairs(
        pairs: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> Vec<(String, String)> {
        let mut unclassified: Vec<(String, String)> = pairs
            .into_iter()
            .filter(|&(tag, attr)| !is_classified(tag, attr))
            .map(|(tag, attr)| (tag.to_owned(), attr.to_owned()))
            .collect();
        // Sorted so a failure reads the same on every run.
        unclassified.sort();
        unclassified
    }

    #[test]
    fn title_projection_renders_authoring_formats_through_ammonia() {
        let cases = [
            (
                PostFormat::Markdown,
                "**bold** [link](https://example.test) ![alt](x)",
                "<strong>bold</strong> link",
            ),
            (
                PostFormat::Org,
                "*bold* /italic/ +strike+",
                "<b>bold</b> <i>italic</i> <s>strike</s>",
            ),
            (
                PostFormat::Html,
                r#"<div>one</div><a href="/">two</a><img alt="three"><br><!-- gone --><script>lost</script>"#,
                "onetwo<br>",
            ),
        ];
        for (format, source, expected) in cases {
            let title: PostTitle = source.parse().unwrap();
            assert_eq!(
                render_title(&title, &format).as_ref(),
                expected,
                "{format:?}"
            );
        }
    }

    #[test]
    fn title_projection_uses_ammonia_for_inline_html_and_text_feeds() {
        let title: PostTitle =
            r#"<strong>A &amp; B</strong><br><a href="/">C</a><script>lost</script>"#
                .parse()
                .unwrap();
        let rendered = render_title(&title, &PostFormat::Html);
        assert_eq!(rendered.as_ref(), "<strong>A &amp; B</strong><br>C");
        assert_eq!(rendered_title_visible_text(&rendered), "A & B C");
        let encoded = common::render::sanitize_post_title("&amp;lt;");
        assert_eq!(rendered_title_visible_text(&encoded), "&lt;");

        let title: PostTitle = r#"<img alt="not retained"><div>block</div>"#.parse().unwrap();
        let rendered = render_title(&title, &PostFormat::Html);
        assert_eq!(rendered.as_ref(), "block");
        assert_eq!(rendered_title_visible_text(&rendered), "block");
    }

    #[test]
    fn title_projection_collapses_content_free_fragments() {
        for (format, source) in [
            (PostFormat::Markdown, "<br>"),
            (PostFormat::Org, "@@html:<br>@@"),
            (PostFormat::Html, "<br>"),
            (PostFormat::Html, "<em></em>"),
            (PostFormat::Html, "<script>discarded</script>"),
        ] {
            let title: PostTitle = source.parse().unwrap();
            assert_eq!(
                render_title(&title, &format).as_ref(),
                "",
                "{format:?} {source:?}"
            );
        }
    }

    #[test]
    fn title_projection_is_total_for_long_and_malformed_titles() {
        for source in [
            "x".repeat(16 * 1024 + 1),
            format!("{}x{}", "<b>".repeat(128), "</b>".repeat(128)),
            "<b><i>x</b>y</i>".to_owned(),
            "<script/>".to_owned(),
            "<b>x".to_owned(),
        ] {
            let title: PostTitle = source.parse().unwrap();
            let rendered = render_title(&title, &PostFormat::Html);
            assert!(
                rendered.as_ref()
                    == common::render::sanitize_post_title(rendered.as_ref()).as_ref(),
                "{source}"
            );
        }
    }

    /// Fails when common's sanitizer permits an `(element, attribute)` pair that
    /// neither `MEDIA_URL_ATTRS` nor `INERT_ATTRS` classifies.
    #[test]
    fn sanitizer_surface_is_fully_classified() {
        let unclassified =
            unclassified_sanitizer_pairs(common::render::sanitizer_permitted_attribute_pairs());
        assert!(
            unclassified.is_empty(),
            "the common sanitizer permits {unclassified:?}, which appear in neither \
             MEDIA_URL_ATTRS nor INERT_ATTRS. Classify each: add the pair to \
             MEDIA_URL_ATTRS if its value names media, otherwise add the attribute \
             name to INERT_ATTRS with a reason."
        );
    }

    #[test]
    fn sanitizer_coupling_test_bites_when_the_policy_widens() {
        // Prove the guard can fail without duplicating or mutating common's policy.
        let widened = [
            ("iframe", "src"),
            ("video", "srcset"),
            ("img", "data-poster"),
        ];
        let unclassified = unclassified_sanitizer_pairs(widened);
        for (element, attribute) in widened {
            assert!(
                unclassified.contains(&(element.to_owned(), attribute.to_owned())),
                "the coupling check must flag newly permitted, unclassified \
                 ({element}, {attribute})"
            );
        }
    }

    #[test]
    fn inert_attrs_lists_no_url_bearing_name() {
        // The invariant the name-level table rests on. An inert *name* excuses that
        // attribute on every element at once, so listing one that carries a URL anywhere
        // would open a blind spot everywhere — and silently, since the coupling check
        // would then read as healthy. These are the names a widening of common's
        // sanitizer is most likely to bring in.
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
