//! Pure post-body rendering and title/metadata derivation.
//!
//! Format-driven transformation of post bodies to HTML plus extraction of
//! titles, slug seeds, and summary labels. No storage or database concerns.

use std::fmt;

// Only the sanitize-gated half of this module (`render`, the media-reference walk) uses
// these.
#[cfg(feature = "sanitize")]
use crate::media::{parse_media_url, MediaRef};
#[cfg(feature = "sanitize")]
use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;

/// The format/markup language used to author a post body.
///
/// A `strum` string enum (ADR: `docs/adr/0075-adopt-strum-retire-str-enum.md`):
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

crate::strum_enum::parse_error!(
    InvalidPostFormat,
    post_format_parse_err,
    "post format must be \"markdown\", \"org\", or \"html\""
);

// serde `into`/`try_from = "String"` proxy so a bad token surfaces the named
// `InvalidPostFormat` message (deserialize routes through `FromStr`).
crate::strum_enum::impl_string_serde_proxy!(PostFormat);

// Typed `sqlx` bind/decode (feature = "sqlx"): stores/loads the TEXT token as a
// `PostFormat` value, like the newtypes (#438) — not a stringly `.to_string()` strip.
crate::db_enum::impl_text_column_enum!(PostFormat);

/// HTML that is **safe to emit unescaped** — the type's invariant is "contains no
/// active markup", established by scrubbing against an allowlist. Before #445 this
/// was only a *provenance* marker ("came out of our renderer"), which did not imply
/// safety because nothing sanitized; it now carries the guarantee its name suggests.
/// Its structural value is unchanged: the unescaped view sink accepts only
/// `RenderedHtml`, so a raw `String`/body cannot reach it by accident.
///
/// Two doors, meaning different things:
///
/// - [`RenderedHtml::sanitize`] **establishes** the invariant by scrubbing. This is
///   the door for anything from outside jaunder — a rendered post body (via
///   [`render`]), an ingested feed entry, any future inbound producer.
/// - [`RenderedHtml::from_trusted`] **inherits** it, rebuilding a value we already
///   sanitized and round-tripped through our own storage or wire. Confined to an
///   allowlist of call sites by the `rendered-html-from-trusted` static check, so a
///   new inbound path cannot quietly use it in place of `sanitize`.
///
/// Reading *out* is convenient —
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

/// The single allowlist every [`RenderedHtml::sanitize`] call scrubs against —
/// defined once here so no caller can drift to a different policy.
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
/// attribute permitted here but unknown to the extractor is a silent blind spot in the
/// `post_media` table — the exact failure mode #711 exists to fix. So every
/// `(element, attribute)` pair this builder permits must appear in either
/// [`MEDIA_URL_ATTRS`] (its value names media) or [`KNOWN_INERT_ATTRS`] (it does not,
/// and here is why). `sanitizer_surface_is_fully_classified` fails the build until each
/// newly permitted pair is classified, so this cannot be forgotten — but it is a
/// judgement call, not a formality.
///
/// One shape does not fit: a **multi-URL** attribute such as `srcset` carries a list,
/// and [`MEDIA_URL_ATTRS`] is one attribute → one URL. Permitting it means widening
/// that table to carry a per-attribute parse mode *first*; classifying it as-is would
/// silently record a wrong single-URL parse.
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

impl RenderedHtml {
    /// Sanitize untrusted HTML into a `RenderedHtml` — the door for anything
    /// originating **outside** jaunder: an authored post body's rendered output, an
    /// ingested feed entry (#282), any future inbound producer.
    ///
    /// This door *establishes* the type's invariant ("contains no active markup")
    /// by scrubbing against an allowlist, which is what distinguishes it from
    /// [`RenderedHtml::from_trusted`] — that one only *inherits* an invariant
    /// something else already established. Outside data must come through here.
    ///
    /// Host-only: gated on the `sanitize` feature, which is never enabled for wasm.
    /// With the feature off this door does not exist, so there is no build in which
    /// it silently degrades to a passthrough.
    #[cfg(feature = "sanitize")]
    #[must_use]
    pub fn sanitize(raw: &str) -> Self {
        Self(SANITIZER.clean(raw).to_string())
    }

    /// Rebuild a `RenderedHtml` we already sanitized, round-tripped through our own
    /// store or wire. This door **inherits** the invariant rather than establishing
    /// it — it asserts, it does not check — so it is only correct where the value's
    /// safety was established earlier by [`RenderedHtml::sanitize`].
    ///
    /// **Not for anything from outside jaunder.** Ingested feed content, remote
    /// channel data, or any future inbound producer must use `sanitize`; reaching
    /// for this door instead is what the `rendered-html-from-trusted` static check
    /// fails the build over. Its allowlist is down to a single production call site
    /// (the seed-DTO wire rebuild), so `grep` still enumerates every rebuild.
    ///
    /// Takes `impl Into<String>` so callers (esp. fixtures) don't need `.to_string()`.
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

/// Deserializes a wire `String` into a `RenderedHtml` via `from_trusted` — the
/// deserialize counterpart to `RenderedHtml`'s deliberate lack of a `Deserialize`
/// impl (it is server-rendered, trusted output; see the note above). Used by the
/// seed DTOs' `#[serde(deserialize_with = ...)]`.
pub(crate) fn deserialize_rendered_html<'de, D>(deserializer: D) -> Result<RenderedHtml, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    String::deserialize(deserializer).map(RenderedHtml::from_trusted)
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
// `Decode` (#445) constructs the private field directly — it needs neither door, since
// this impl lives in the same module as the type. So the `rendered_html` column decodes
// straight into `RenderedHtml`, like every other domain column (#438/#572), and
// `build_post_record` no longer rebuilds via `from_trusted`.
//
// This reverses the previous "deliberately NO `Decode`" stance, so the reasoning is worth
// keeping rather than deleting. That stance rested on a decode being able to "bless ANY
// text column decoded into it — e.g. a raw, un-rendered `body`". **That risk is real and
// is accepted here**: decoding some other column into this type would still bless it.
//
// It rests on one argument only — that typing a column as `RenderedHtml` is a deliberate,
// reviewable act. Note what does *not* back it: the `rendered-html-from-trusted` gate does
// **not** catch this. That gate matches `from_trusted` call sites in expression position;
// a `FromRow` field typed `RenderedHtml` over the wrong column names no door at all and is
// invisible to it. Widening the gate to flag `RenderedHtml`-typed row fields outside an
// allowlist would close the hole — filed as #701.
//
// A *sanitizing* decode would have removed the risk outright and healed any pre-#445 row
// on read. It was rejected: no deployed instance holds data, so it would guard only
// against a write path that forgot to sanitize — which the gate already catches — at the
// cost of an html5ever parse on every post read, forever. Revisit only if an instance ever
// accumulates rows written by a pre-#445 build.
#[cfg(feature = "sqlx")]
const _: () = {
    impl<DB: sqlx::Database> sqlx::Type<DB> for RenderedHtml
    where
        String: sqlx::Type<DB>,
    {
        fn type_info() -> <DB as sqlx::Database>::TypeInfo {
            <String as sqlx::Type<DB>>::type_info()
        }
        // Delegated like every other newtype bridge (#438/#572). Previously omitted
        // because `compatible` is consulted only on the decode path, which did not
        // exist; the trait default would accept only the exact `type_info`, rejecting
        // an equally-valid `VARCHAR` column.
        fn compatible(ty: &<DB as sqlx::Database>::TypeInfo) -> bool {
            <String as sqlx::Type<DB>>::compatible(ty)
        }
    }

    impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for RenderedHtml
    where
        String: sqlx::Decode<'r, DB>,
    {
        fn decode(
            value: <DB as sqlx::Database>::ValueRef<'r>,
        ) -> Result<Self, sqlx::error::BoxDynError> {
            // `Self(..)` — the private constructor, reachable because this impl lives
            // in the type's own module. Neither door is involved: this is not new
            // outside data (so not `sanitize`), and routing it through `from_trusted`
            // would put a gate-policed door on a path the gate cannot inspect.
            <String as sqlx::Decode<'r, DB>>::decode(value).map(Self)
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
/// is a [`RenderedHtml`], minted through [`RenderedHtml::sanitize`] — a post body is
/// author-supplied, so it is outside input and every format's output is scrubbed.
///
/// All three formats need that scrub, not just [`PostFormat::Html`]: the Markdown and
/// Org parsers both pass embedded raw HTML through untouched, so `<script>` in a
/// Markdown body reaches the output just as readily as in an HTML one (#445).
///
/// Host-only: gated on `sanitize`, like the door it mints through. With the feature
/// off this function does not exist, so there is no build that renders unsanitized.
#[cfg(feature = "sanitize")]
#[must_use]
pub fn render(body: &PostBody, format: &PostFormat) -> RenderedHtml {
    let html = match format {
        PostFormat::Markdown => render_markdown(body),
        PostFormat::Org => render_org(body),
        PostFormat::Html => body.to_string(),
    };
    RenderedHtml::sanitize(&html)
}

// ---------------------------------------------------------------------------
// Media references in rendered output (#711)
// ---------------------------------------------------------------------------

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
/// is data, and reviewable as data. Its counterpart is [`KNOWN_INERT_ATTRS`], and
/// between them they must cover everything [`SANITIZER`] permits —
/// `sanitizer_surface_is_fully_classified` enforces that.
#[cfg(feature = "sanitize")]
pub const MEDIA_URL_ATTRS: &[(&str, &str)] = &[("a", "href"), ("img", "src")];

/// Permitted pairs deliberately *not* treated as media references. Present so
/// `sanitizer_surface_is_fully_classified` can tell "considered and excluded" from
/// "nobody looked" — every pair [`SANITIZER`] permits must be in exactly one of these
/// two tables, and a newly permitted one fails that test until a human classifies it.
///
/// An element of `"*"` means "on every element", which is how the *generic* attributes
/// (permitted on every tag, so `tags × generic_attributes` pairs) are classified without
/// writing out that product. The wildcard is an inert-table notion only: the walk in
/// `extract_media_refs_with` matches element names literally, so `("*", …)` in
/// [`MEDIA_URL_ATTRS`] would match nothing — pinned by
/// `media_url_attrs_names_elements_literally`.
#[cfg(feature = "sanitize")]
pub const KNOWN_INERT_ATTRS: &[(&str, &str)] = &[
    // Generic attributes, permitted on every tag: human-language and advisory text.
    ("*", "lang"),
    ("*", "title"),
    // Link metadata that is not itself a link — the language of the *target*.
    ("a", "hreflang"),
    // Text direction.
    ("bdo", "dir"),
    // Quotation provenance. A URL, but excluded as a deliberate scope call (spec D2):
    // it says where a quote came from, and no browser fetches, displays or navigates
    // it, so it points no reader at anything. Revisit deliberately, not by accident.
    ("blockquote", "cite"),
    ("del", "cite"),
    ("ins", "cite"),
    ("q", "cite"),
    // Edit timestamps.
    ("del", "datetime"),
    ("ins", "datetime"),
    // Presentational table/column geometry and alignment.
    ("col", "align"),
    ("col", "char"),
    ("col", "charoff"),
    ("col", "span"),
    ("colgroup", "align"),
    ("colgroup", "char"),
    ("colgroup", "charoff"),
    ("colgroup", "span"),
    ("table", "align"),
    ("table", "char"),
    ("table", "charoff"),
    ("table", "summary"),
    ("tbody", "align"),
    ("tbody", "char"),
    ("tbody", "charoff"),
    ("td", "align"),
    ("td", "char"),
    ("td", "charoff"),
    ("td", "colspan"),
    ("td", "headers"),
    ("td", "rowspan"),
    // `tfoot` is in ammonia's attribute map but not its tag allowlist. Enumerating the
    // map wholesale is the conservative reading — classifying a pair that cannot occur
    // costs nothing, while skipping one that can is the hole this table exists to close.
    ("tfoot", "align"),
    ("tfoot", "char"),
    ("tfoot", "charoff"),
    ("th", "align"),
    ("th", "char"),
    ("th", "charoff"),
    ("th", "colspan"),
    ("th", "headers"),
    ("th", "rowspan"),
    ("th", "scope"),
    ("thead", "align"),
    ("thead", "char"),
    ("thead", "charoff"),
    ("tr", "align"),
    ("tr", "char"),
    ("tr", "charoff"),
    ("hr", "align"),
    ("hr", "size"),
    ("hr", "width"),
    // Image presentation. `src` is the reference; the rest is geometry and alt text.
    ("img", "align"),
    ("img", "alt"),
    ("img", "height"),
    ("img", "width"),
    // List numbering.
    ("ol", "start"),
    // `SANITIZER`'s one widening: the `language-*` marker on a fenced code block, whose
    // values the attribute filter already narrows.
    ("code", "class"),
    ("pre", "class"),
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
#[cfg(feature = "sanitize")]
#[must_use]
pub fn extract_media_refs(html: &str) -> Vec<MediaRef> {
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
/// `&str -> Vec<MediaRef>` that the coupling test and future reclamation work reuse.
///
/// Uses `html5ever`'s tokenizer, not its tree builder: only start tags and their
/// attributes are needed, and the input is already well-formed sanitizer output.
#[cfg(feature = "sanitize")]
fn extract_media_refs_with(html: &str, pairs: &[(&str, &str)]) -> Vec<MediaRef> {
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
        refs: RefCell<BTreeSet<MediaRef>>,
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
/// The reference set is private and [`RenderOutput::render`] is the only constructor, so a
/// value whose set disagrees with its HTML is unrepresentable rather than merely
/// discouraged (spec D1). Everything downstream only *carries* the pair: a post
/// create/update input on its way to storage cannot substitute a set of its own, correct
/// or not, and a caller with no way to render has no way to invent one either.
///
/// The set is derived, never supplied:
/// ```
/// # use common::render::{PostFormat, RenderOutput};
/// let out = RenderOutput::render(&"hello".to_owned().into(), &PostFormat::Markdown);
/// assert!(out.media().is_empty());
/// ```
/// and a struct literal cannot smuggle one in:
/// ```compile_fail
/// # use common::render::{render, PostFormat, RenderOutput};
/// let html = render(&"hello".to_owned().into(), &PostFormat::Markdown);
/// let _ = RenderOutput { html, media: vec![] }; // private field
/// ```
#[cfg(feature = "sanitize")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOutput {
    /// The sanitized HTML, as [`render`] produced it.
    pub html: RenderedHtml,
    /// What that HTML points a reader at. Private — see the type's docs.
    media: Vec<MediaRef>,
}

#[cfg(feature = "sanitize")]
impl RenderOutput {
    /// Renders `body` and extracts what the result references, as one step.
    ///
    /// A thin composition over [`render`] and [`extract_media_refs`], which stay public in
    /// their own right — the sanitisation tests and the extractor tests each exercise one
    /// half, and rendering without needing the references is still a legitimate thing to do.
    #[must_use]
    pub fn render(body: &PostBody, format: &PostFormat) -> Self {
        let html = render(body, format);
        let media = extract_media_refs(html.as_ref());
        Self { html, media }
    }

    /// The media the HTML references — sorted and deduplicated, as
    /// [`extract_media_refs`] returns them.
    #[must_use]
    pub fn media(&self) -> &[MediaRef] {
        &self.media
    }
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
///
/// Gated with [`render`], its only caller: on a build without `sanitize` there is no
/// renderer, so this would be dead code.
#[cfg(feature = "sanitize")]
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
///
/// Gated with [`render`], its only caller (see [`render_markdown`]).
#[cfg(feature = "sanitize")]
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

    // `sanitize` is the establishing door (#445): it is what makes the type's
    // invariant — "contains no active markup" — true rather than asserted. These
    // assert on the *absence* of the dangerous token rather than exact output,
    // because ammonia's escaping details are not our contract.
    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_strips_script_element() {
        let h = RenderedHtml::sanitize("<p>hi</p><script>alert(1)</script>");
        assert!(!h.contains("<script"), "{h}");
        assert!(!h.contains("alert(1)"), "{h}");
        assert!(h.contains("<p>hi</p>"), "{h}");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_strips_event_handler_attributes() {
        let h = RenderedHtml::sanitize(r#"<img src="x" onerror="alert(1)">"#);
        assert!(!h.contains("onerror"), "{h}");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_strips_javascript_urls() {
        let h = RenderedHtml::sanitize(r#"<a href="javascript:alert(1)">x</a>"#);
        assert!(!h.contains("javascript:"), "{h}");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_preserves_formatting_markup() {
        // The shapes `pulldown-cmark` and `orgize` actually emit. If the default
        // allowlist eats any of these, posts render degraded — widen deliberately
        // rather than letting it pass.
        let h = RenderedHtml::sanitize(
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

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_keeps_only_language_classes_on_code() {
        // Re-admitting `class` must not become arbitrary class injection: a post
        // could otherwise borrow the app's own CSS to mimic or hide UI. Only
        // `language-*` tokens survive, and only on `pre`/`code`.
        let h = RenderedHtml::sanitize(
            r#"<pre><code class="language-rust j-anon-only">x</code></pre>"#,
        );
        assert!(h.contains("language-rust"), "{h}");
        assert!(
            !h.contains("j-anon-only"),
            "non-language class survived: {h}"
        );

        // …and `class` stays disallowed everywhere else — dropped by the tag
        // allowlist before the attribute filter is consulted at all.
        let other = RenderedHtml::sanitize(r#"<p class="j-anon-only">x</p>"#);
        assert!(
            !other.contains("j-anon-only"),
            "class survived on <p>: {other}"
        );

        // The filter's drop-entirely branch: on `code`, where `class` *is*
        // allowlisted, a value with no `language-*` token must lose the attribute
        // outright rather than surviving as an empty `class=""`.
        let none_kept = RenderedHtml::sanitize(r#"<code class="j-anon-only">x</code>"#);
        assert!(
            !none_kept.contains("j-anon-only"),
            "non-language class survived on <code>: {none_kept}"
        );
        assert!(
            !none_kept.contains("class"),
            "empty class attribute left behind: {none_kept}"
        );
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitize_preserves_safe_links_and_images() {
        let h = RenderedHtml::sanitize(
            r#"<a href="https://example.com">link</a><img src="https://example.com/a.png">"#,
        );
        assert!(h.contains("https://example.com"), "{h}");
        assert!(h.contains("<a "), "{h}");
        assert!(h.contains("<img"), "{h}");
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

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_headings() {
        let html = render_markdown("# H1\n## H2\n### H3");
        assert!(html.contains("<h1>H1</h1>"));
        assert!(html.contains("<h2>H2</h2>"));
        assert!(html.contains("<h3>H3</h3>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_paragraph() {
        let html = render_markdown("Hello, world!");
        assert!(html.contains("<p>Hello, world!</p>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_bold_italic_strikethrough() {
        let html = render_markdown("**bold** *italic* ~~strike~~");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
        assert!(html.contains("<del>strike</del>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_code_block() {
        let html = render_markdown("```rust\nfn main() {}\n```");
        assert!(html.contains("<code"));
        assert!(html.contains("fn main()"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_links() {
        let html = render_markdown("[example](https://example.com)");
        assert!(html.contains("<a href=\"https://example.com\">example</a>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_ordered_list() {
        let html = render_markdown("1. first\n2. second\n3. third");
        assert!(html.contains("<ol>"));
        assert!(html.contains("<li>first</li>"));
        assert!(html.contains("<li>second</li>"));
        assert!(html.contains("<li>third</li>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_unordered_list() {
        let html = render_markdown("- alpha\n- beta");
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>alpha</li>"));
        assert!(html.contains("<li>beta</li>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_table() {
        let input = "| A | B |\n|---|---|\n| 1 | 2 |";
        let html = render_markdown(input);
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>A</th>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_empty_input() {
        let html = render_markdown("");
        assert!(html.is_empty());
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_multiple_paragraphs() {
        let html = render_markdown("First paragraph.\n\nSecond paragraph.");
        // Two separate <p> tags
        let count = html.matches("<p>").count();
        assert_eq!(count, 2);
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn markdown_tasklist() {
        let html = render_markdown("- [x] done\n- [ ] todo");
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("checked"));
    }

    // -- Org-mode tests --

    #[cfg(feature = "sanitize")]
    #[test]
    fn org_headings() {
        let html = render_org("* H1\n** H2");
        assert!(html.contains("H1"));
        assert!(html.contains("H2"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn org_paragraph() {
        let html = render_org("Hello, org world!");
        assert!(html.contains("Hello, org world!"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn org_bold_italic_code() {
        let html = render_org("*bold* /italic/ ~code~");
        assert!(html.contains("<b>bold</b>"));
        assert!(html.contains("<i>italic</i>"));
        assert!(html.contains("<code>code</code>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn org_list() {
        let html = render_org("- alpha\n- beta");
        assert!(html.contains("alpha"));
        assert!(html.contains("beta"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn org_code_block() {
        let html = render_org("#+BEGIN_SRC rust\nfn main() {}\n#+END_SRC");
        assert!(html.contains("fn main()"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn org_link() {
        let html = render_org("[[https://example.com][example]]");
        assert!(
            html.contains("<a href=\"https://example.com\""),
            "expected an anchor element, got: {html}"
        );
        assert!(html.contains("example"));
    }

    #[cfg(feature = "sanitize")]
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
    #[cfg(feature = "sanitize")]
    #[test]
    fn render_preserves_real_renderer_output() {
        let md = render(
            &PostBody::from(
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
            &PostBody::from("* Heading\n\nSome *bold* text and [[https://example.com][a link]].\n"),
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

    #[cfg(feature = "sanitize")]
    #[test]
    fn render_dispatches_markdown() {
        let result = render(&PostBody::from("**bold**"), &PostFormat::Markdown);
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn render_dispatches_org() {
        let result = render(&PostBody::from("*bold*"), &PostFormat::Org);
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
    #[cfg(feature = "sanitize")]
    fn assert_no_active_markup(html: &str) {
        assert!(!html.contains("<script"), "script element survived: {html}");
        assert!(!html.contains("onerror"), "event handler survived: {html}");
        assert!(
            !html.contains("javascript:"),
            "javascript: URL survived: {html}"
        );
    }

    /// The three vectors as raw HTML, for embedding in each format's body.
    #[cfg(feature = "sanitize")]
    const ACTIVE_MARKUP: &str = concat!(
        "<script>alert(1)</script>",
        r#"<img src=x onerror=alert(1)>"#,
        r#"<a href="javascript:alert(1)">x</a>"#,
    );

    #[cfg(feature = "sanitize")]
    #[test]
    fn render_markdown_strips_embedded_script() {
        let result = render(
            &PostBody::from(format!("Hello\n\n{ACTIVE_MARKUP}").as_str()),
            &PostFormat::Markdown,
        );
        assert_no_active_markup(&result);
        assert!(!result.contains("alert(1)"), "{result}");
        assert!(result.contains("Hello"), "{result}");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn render_org_strips_embedded_script() {
        // `@@html:…@@` is Org's inline-export escape hatch — the form that actually
        // reaches the output as raw HTML. (A `#+begin_export html` block is escaped
        // by orgize itself, so it never needed us.) Assert on the executable form:
        // the literal text `alert(1)` surviving *escaped* is harmless.
        let result = render(
            &PostBody::from(format!("Hello\n\n@@html:{ACTIVE_MARKUP}@@").as_str()),
            &PostFormat::Org,
        );
        assert_no_active_markup(&result);
        assert!(result.contains("Hello"), "{result}");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn render_html_strips_embedded_script() {
        let result = render(
            &PostBody::from(format!("<p>hi</p>{ACTIVE_MARKUP}").as_str()),
            &PostFormat::Html,
        );
        assert_no_active_markup(&result);
        assert!(!result.contains("alert(1)"), "{result}");
        assert!(result.contains("<p>hi</p>"), "{result}");
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

    // Was `render_html_format_is_identity` before #445: the `Html` format used to be
    // a verbatim passthrough. It is now sanitized like every other format, so the
    // guarantee is "safe markup survives unchanged", not "the input survives".
    #[cfg(feature = "sanitize")]
    #[test]
    fn render_html_format_preserves_safe_markup() {
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

    // -----------------------------------------------------------------------
    // Media references (#711)
    // -----------------------------------------------------------------------

    #[cfg(feature = "sanitize")]
    use crate::media::MediaSource;

    /// A realistic lowercase SHA-256 hex digest (the digest of the empty input). The
    /// same value `media.rs`'s tests use, restated because that one is private to its
    /// own test module.
    #[cfg(feature = "sanitize")]
    const CANONICAL: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[cfg(feature = "sanitize")]
    fn media_url_for(name: &str) -> String {
        format!("/media/upload/e3/b0/{CANONICAL}/{name}")
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn extract_finds_a_markdown_image() {
        // Rendered via the real renderer, so this pins end-to-end behaviour rather than a
        // hand-written fragment.
        let body: PostBody = format!("![alt]({})", media_url_for("photo.jpg")).into();
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].filename.as_ref(), "photo.jpg");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn extract_finds_a_raw_img_embedded_in_a_markdown_body() {
        // The rendered-HTML choice (spec D2): raw HTML passes through the Markdown parser.
        let body: PostBody = format!("<img src=\"{}\">", media_url_for("photo.jpg")).into();
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].filename.as_ref(), "photo.jpg");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn extract_finds_a_raw_filename_spelling() {
        // The #675 regression, at the extractor level: a post addressing the file by the
        // name a person types must resolve to the stored, encoded spelling.
        let body: PostBody = format!("<img src=\"{}\">", media_url_for("my photo.jpg")).into();
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].filename.as_ref(), "my%20photo.jpg");
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn extract_finds_an_atompub_member_url_in_a_link() {
        let body: PostBody =
            format!("<a href=\"/atompub/alice/media/{CANONICAL}/photo.jpg\">doc</a>").into();
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].source, MediaSource::Upload);
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn extract_ignores_media_in_stripped_elements_and_code_blocks() {
        // Sanitisation removes <video>, so it can never load and is not a reference.
        let video: PostBody =
            format!("<video src=\"{}\"></video>", media_url_for("clip.mp4")).into();
        assert!(extract_media_refs(render(&video, &PostFormat::Markdown).as_ref()).is_empty());

        // A URL displayed as literal text points nobody at anything (spec D2's deliberate
        // narrowing away from the old substring search over the source body).
        let fenced: PostBody = format!("```\n{}\n```", media_url_for("photo.jpg")).into();
        assert!(extract_media_refs(render(&fenced, &PostFormat::Markdown).as_ref()).is_empty());
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn extract_deduplicates_and_sorts() {
        let one = media_url_for("a.jpg");
        let two = media_url_for("b.jpg");
        let body: PostBody =
            format!("<img src=\"{two}\"><img src=\"{one}\"><img src=\"{one}\">").into();
        let refs = extract_media_refs(render(&body, &PostFormat::Markdown).as_ref());
        assert_eq!(refs.len(), 2, "duplicate references collapse to one row");
        assert!(
            refs[0] < refs[1],
            "output is sorted for deterministic writes"
        );
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn extract_ignores_non_media_links() {
        let body: PostBody = "<a href=\"https://example.com/page\">x</a>"
            .to_owned()
            .into();
        assert!(extract_media_refs(render(&body, &PostFormat::Markdown).as_ref()).is_empty());
    }

    #[cfg(feature = "sanitize")]
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

    #[cfg(feature = "sanitize")]
    #[test]
    fn media_url_attrs_names_elements_literally() {
        // The `"*"` element is a KNOWN_INERT_ATTRS notion — a way to classify the generic
        // attributes without writing out `tags × generic_attributes`. The walk compares
        // element names literally, so a wildcard in MEDIA_URL_ATTRS would match nothing
        // and silently extract nothing; keep it out of that table.
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

    #[cfg(feature = "sanitize")]
    #[test]
    fn render_output_derives_its_media_from_its_html() {
        let body: PostBody = format!("<img src=\"{}\">", media_url_for("photo.jpg")).into();
        let out = RenderOutput::render(&body, &PostFormat::Markdown);
        assert_eq!(
            out.media(),
            extract_media_refs(out.html.as_ref()).as_slice()
        );
        assert_eq!(out.media().len(), 1);
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn render_output_media_is_empty_for_a_body_referencing_nothing() {
        let out = RenderOutput::render(&"plain text".to_owned().into(), &PostFormat::Markdown);
        assert!(out.media().is_empty());
    }

    /// Whether `(tag, attr)` appears in either classification table. `KNOWN_INERT_ATTRS`
    /// honours the `"*"` wildcard element; `MEDIA_URL_ATTRS` does not (see
    /// `media_url_attrs_names_elements_literally`).
    #[cfg(feature = "sanitize")]
    fn is_classified(tag: &str, attr: &str) -> bool {
        MEDIA_URL_ATTRS
            .iter()
            .any(|&(element, attribute)| element == tag && attribute == attr)
            || KNOWN_INERT_ATTRS.iter().any(|&(element, attribute)| {
                (element == "*" || element == tag) && attribute == attr
            })
    }

    /// Permitted `(element, attribute)` pairs appearing in neither classification table.
    ///
    /// The enumeration is `tags × generic_attributes ∪ tag_attributes` — `generic_attributes`
    /// applies to every tag, so omitting that product would leave a hole. It is deliberately
    /// *not* filtered by any URL-attribute predicate: ammonia's `is_url_attr` is private, and
    /// a hand-written substitute would not recognise `srcset` as URL-bearing — the exact
    /// attribute this coupling is most likely to be widened with. So the assertion is
    /// inverted: every permitted pair must be classified, whether or not it looks like a URL.
    #[cfg(feature = "sanitize")]
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
    /// `MEDIA_URL_ATTRS` nor `KNOWN_INERT_ATTRS` mentions — i.e. someone widened the
    /// allowlist without saying whether the new attribute's value names media.
    ///
    /// To resolve: classify each reported pair. Into `MEDIA_URL_ATTRS` if its value is a
    /// URL a reader is pointed at, otherwise into `KNOWN_INERT_ATTRS` with a one-line
    /// reason. Silence is not an option, which is the whole point: separating the
    /// extractor's surface from the sanitiser's allowlist would otherwise recreate #711's
    /// own failure mode — widen the sanitiser, forget the extractor, and `post_media`
    /// quietly acquires a blind spot.
    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitizer_surface_is_fully_classified() {
        let unclassified = unclassified_sanitizer_pairs(&SANITIZER);
        assert!(
            unclassified.is_empty(),
            "SANITIZER permits {unclassified:?}, which appear in neither MEDIA_URL_ATTRS nor \
             KNOWN_INERT_ATTRS. Classify each: add it to MEDIA_URL_ATTRS if its value names \
             media, otherwise to KNOWN_INERT_ATTRS with a reason."
        );
    }

    #[cfg(feature = "sanitize")]
    #[test]
    fn sanitizer_coupling_test_bites_when_the_allowlist_widens() {
        // Prove the guard can fail. A widened builder with an unclassified URL-bearing
        // attribute must be reported — otherwise the check above is decorative.
        let mut widened = ammonia::Builder::default();
        widened.add_tags(["video"]);
        widened.add_tag_attributes("video", ["src"]);
        // A *generic* attribute too: it applies to every tag, so it is only reported if
        // the enumeration really forms `tags × generic_attributes`. Without that product
        // the check would look healthy while missing a whole axis.
        widened.add_generic_attributes(["data-poster"]);
        let unclassified = unclassified_sanitizer_pairs(&widened);
        assert!(
            unclassified.contains(&("video".to_owned(), "src".to_owned())),
            "the coupling check must flag a newly permitted, unclassified pair"
        );
        assert!(
            unclassified.contains(&("img".to_owned(), "data-poster".to_owned())),
            "a newly permitted generic attribute must be flagged on every tag"
        );
    }
}
