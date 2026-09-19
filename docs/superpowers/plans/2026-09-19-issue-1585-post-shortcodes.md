# Bounded Post Shortcodes Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for an isolated task
> when useful. This outline exists because the feature amends the `RenderedHtml`
> security invariant and adds a trusted active-markup assembly boundary.

## Scope

In:

- Exact Markdown/Org shortcode grammar and context-aware recognition.
- Closed YouTube/Vimeo provider values and fixed trusted player markup.
- Sanitizer, media-reference, storage/AtomPub, Syndication Feed, browser, and
  documentation proof.

Out:

- HTML Post expansion, generic templates or plugins, author-controlled iframe
  data, Emacs transformations, click-to-load consent, and downloaded video.

## Task outline

- [x] Task 1: Establish typed provider embeds and trusted HTML assembly
  - Contract: `host::render` owns source recognition and provider dispatch.
    `common::render` owns a deliberately closed `TrustedProviderEmbed` type with
    private validated provider state, provider-specific constructors, fixed
    markup, and the only structured assembly door to `RenderedHtml`. It accepts
    no raw trusted HTML or arbitrary URL. This is an explicit extension of
    ADR-0159's existing sanitizer/minting exception, not a new home for general
    host rendering machinery.
  - Contract: sanitized author content and typed embeds preserve document order;
    author input cannot bypass provider validation or choose markup/URL parts;
    no internal assembly artifact survives. Fixed provider output owns canonical
    embed/fallback URLs, lazy loading, title, fullscreen attributes, responsive
    hooks, and external embed classification. Generated `iframe[src]` never
    enters `MEDIA_URL_ATTRS` and never creates a `MediaReference`.
  - Verification: focused `common` and `host` tests prove identifier grammars,
    fixed output, hostile raw/disguised iframe stripping, assembly non-forgery,
    unsafe-input rejection, no raw constructor or arbitrary-URL door, document
    ordering, absence of assembly artifacts, and sanitizer/reference surface
    classification.

- [x] Task 2: Add format-aware Post Shortcode recognition to canonical rendering
  - Contract: Markdown and Org adapters recognize only the spec's exact
    top-level-paragraph grammar and produce Task 1's ordered typed embed input;
    they preserve each parser's whole-document semantics and never rewrite the
    input `PostBody`. HTML bypasses the processor. The implementation may use
    parser events, structured nodes, or internal markers provided Task 1's
    observable assembly contract holds.
  - Contract: shared tokenization and literal fallback are provider-independent;
    host dispatch maps a recognized provider name to one closed common
    constructor. Unknown, malformed, extra-argument, wrong-case, and
    ineligible-context forms reach the normal parser unchanged.
  - Verification: `devtool run -- cargo xtask test-local -- -p host render`
    covers both valid providers, every grammar boundary, the complete Markdown
    and Org suppression matrices, HTML non-expansion, unchanged input, parser
    cross-document behavior, and a fixture provider added without tokenizer or
    assembly-policy changes.

- [ ] Task 3: Prove persistence, protocol, and browser behavior; project built
      reality
  - Contract: create and update flows continue storing canonical native source;
    AtomPub Member GET returns that source, while Syndication Feeds and web Post
    views consume the one expanded `RenderedHtml` value.
  - Contract: responsive presentation uses Jaunder-owned selectors without
    widening author-controlled Style Contract input; the browser test asserts a
    mounted player and canonical fallback link through one document boot, plus
    Post-width containment and 16:9 geometry at desktop and narrow viewports.
  - Verification: backend-parametric integration coverage in the existing
    AtomPub Post and feed suites proves source bytes and feed HTML; focused
    `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/^(atompub|feed)::/)'`
    proves both backends; focused
    `devtool run -- cargo xtask e2e-local posts.spec.ts` proves the user-visible
    flow.
  - Documentation: move the shortcode decision from `docs/ARCHITECTURE.md`'s
    **Committed direction** into current content-model truth, keeping the draft
    ADR citation; reconcile module docs and `RenderedHtml` invariant prose with
    the implemented boundary.

## Risk checks

- Raw author HTML, source text, provider arguments, and internal-assembly
  lookalikes cannot obtain a trusted iframe or arbitrary URL.
- Trusted assembly occurs only after untrusted parser output is sanitized, and
  whole-document Markdown/Org semantics are not changed by fragment rendering.
- The only iframe surface is fixed typed provider output; sanitizer policy does
  not generally admit `iframe`.
- Every generated URL-bearing attribute is explicitly classified, while provider
  URLs remain outside Jaunder Media ownership and deletion accounting.
- Stored Markdown/Org bytes and AtomPub native-source fidelity remain unchanged
  across create and update on SQLite and PostgreSQL.
- Syndication Feed and web presentation use the same rendered value; no
  serializer grows a second shortcode implementation.
- No lint suppression is introduced without explicit approval. Each task keeps
  its focused proof, then stages and commits through `jaunder-commit` before the
  next task begins.
