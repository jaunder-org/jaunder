# Issue #1585: bounded Post shortcodes

## Outcome

Authors can place a bounded YouTube or Vimeo Post Shortcode in Markdown or Org
source and receive a responsive embedded player everywhere Jaunder renders the
Post. AtomPub continues to expose the unchanged native source.

## Load-bearing decisions

- A **Post Shortcode** is a Jaunder source construct, not an Emacs-only
  transformation and not a general template language.
- A shortcode occupies one top-level paragraph source line. The line permits
  zero to three leading ASCII spaces, then `{{<`, one or more ASCII spaces or
  tabs between every token, and `>}}`, followed only by ASCII spaces or tabs.
  Provider names are lowercase and each shortcode accepts exactly one positional
  identifier.
- YouTube accepts exactly 11 case-sensitive ASCII letters, digits, `_`, or `-`.
  Vimeo accepts 1–20 ASCII digits, begins with `1`–`9`, and preserves the digit
  string. Other identifier shapes remain literal.
- YouTube derives `https://www.youtube-nocookie.com/embed/VIDEO_ID` and fallback
  `https://www.youtube.com/watch?v=VIDEO_ID`. Vimeo derives
  `https://player.vimeo.com/video/VIDEO_ID` and fallback
  `https://vimeo.com/VIDEO_ID`. No source text contributes any other URL part.
- Recognition applies to Markdown and Org Posts. HTML Posts do not gain template
  processing.
- Only a top-level paragraph is eligible. Markdown inline code, backtick and
  tilde fences, indented code, lists, and block quotes remain literal. Org
  inline code/verbatim, source/example/export/quote/verse blocks, fixed-width
  forms, drawers, tables, lists, and comments remain literal.
- Unknown providers, invalid identifiers, unsupported arguments, malformed
  syntax, and shortcode-looking text in any ineligible context remain visible as
  literal source; they neither fail publication nor disappear.
- YouTube and Vimeo are provider implementations behind one bounded Post
  Shortcode processor. Tokenization, literal fallback, and trusted-output
  assembly are common policy; a future provider adds one isolated
  validator/renderer plus its tests. This is not a runtime plugin API.
- The renderer emits only fixed, Jaunder-owned iframe markup from validated
  provider identifiers. Author-supplied raw iframes remain forbidden and are
  removed by sanitization.
- Players are lazy-loaded, titled, fullscreen-capable, responsive 16:9,
  constrained to Post width, and accompanied by their canonical fallback link.
- Expansion is canonical rendered-HTML behavior shared by Local, Home, Post
  pages, and Syndication Feeds. It does not rewrite stored source or AtomPub
  Collection responses.
- Provider embed URLs are external presentation resources, not Jaunder Media
  references.

## Acceptance

- Renderer tests prove valid YouTube and Vimeo shortcodes expand in both
  Markdown and Org without mutating the input `PostBody`.
- Storage/AtomPub integration tests create and update Markdown and Org Posts
  containing shortcodes, then prove stored body bytes and AtomPub GET content
  remain the canonical native source rather than generated markup.
- Grammar tests cover both identifier languages, every token boundary, permitted
  indentation/trailing whitespace, case sensitivity, extra arguments, malformed
  delimiters, unknown providers, and invalid identifiers.
- Context tests prove Markdown inline, backtick-fenced, tilde-fenced, indented,
  list, and block-quote forms remain literal; Org inline code/verbatim,
  source/example/export/quote/verse blocks, fixed-width forms, drawers, tables,
  lists, and comments remain literal; HTML Posts never expand.
- Security tests prove raw and disguised author-supplied iframes remain
  stripped, unsafe URL/attribute injection cannot enter generated markup, and
  unknown shortcodes remain inert text.
- Sanitizer/reference classification tests explicitly account for generated
  iframe URLs without treating them as Jaunder Media.
- A Syndication Feed serialization test proves its rendered Post content
  contains the same generated player and canonical fallback link.
- End-to-end coverage demonstrates a rendered player and fallback link from a
  representative published Post without adding another document boot.
- The provider boundary is covered by a fixture showing that one provider can be
  added without changing tokenizer or trusted-assembly policy.

## Boundaries

- No generic shortcode language, runtime plugins, nested shortcodes, closing
  tags, named arguments, author-controlled dimensions, autoplay, or arbitrary
  iframe attributes.
- No click-to-load consent layer in this issue; privacy-enhanced hosts and lazy
  loading are the initial policy.
- No Emacs advice, implicit Org export hook, or new Protocol Client command.
- No shortcode processing for HTML Posts and no localization or downloading of
  third-party video resources.
