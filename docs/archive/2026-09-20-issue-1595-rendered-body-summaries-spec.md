# Rendered-body fallback summaries (#1595)

## Outcome

When a Post lacks an authored summary, Jaunder derives metadata from its
rendered text instead of exposing markup or leaving useful metadata empty. Full
Post content remains present everywhere it is present today.

## Load-bearing decisions

- An authored Post summary remains authoritative and is preserved unchanged.
- A fallback summary is disposable presentation metadata. It is derived on read
  and is never persisted as authored Post content.
- Derivation starts from `RenderedHtml`, never from Markdown, Org, or HTML
  source. Equivalent rendered text streams therefore produce the same fallback,
  and source markup cannot appear in it.
- Derivation is host-owned beside Post rendering. The host strips elements with
  `ammonia::Builder::empty()`, decodes the sanitizer serialization's entities,
  replaces each maximal Unicode-whitespace run with one ASCII space, and trims
  both ends. It does not infer separators from element boundaries and does not
  build another HTML parser.
- Sentence selection is deliberately lexical, not linguistic. Within the first
  500 Unicode scalars, the first `.`, `!`, or `?` ends the fallback; immediately
  following closing quotes or brackets (`'`, `"`, `’`, `”`, `)`, `]`, or `}`)
  are included. This rule also applies to abbreviations and decimals. If no such
  terminator fits, the fallback cuts at the last Unicode-whitespace boundary
  within the limit, then at the scalar limit only when necessary.
- The effective summary is authored when present, otherwise derived; a rendering
  with no text has no effective summary.
- Permalink description and Open Graph description metadata use the effective
  summary. They remain empty only when neither authored nor rendered text
  exists.
- Atom and JSON Syndication Feeds expose the effective summary in their optional
  summary fields. All three feed formats retain their complete rendered article
  bodies; RSS continues to use that body as its description.
- A titleless draft or scheduled Post uses a compact projection of the effective
  summary as its list label. For this projection only, authored or derived text
  receives the same Unicode-whitespace normalization and sentence-first
  truncation with a 100-scalar limit. If no textual summary exists, the label is
  the Post slug.
- Existing cached Syndication Feed representations are invalidated so the new
  summary projection applies after deployment without waiting for another Post
  event.

## Acceptance

- A titleless HTML Post whose body begins with elements displays readable text,
  not literal tags, in the drafts or scheduled-Posts list.
- Markdown, Org, and HTML Posts with equivalent serialized text and whitespace
  derive equivalent fallback summaries; adjacent elements do not manufacture a
  separator absent from the rendered HTML stream.
- Entity references become their intended characters; maximal Unicode-whitespace
  runs, including decoded non-breaking spaces, become one ASCII space; output is
  trimmed; UTF-8 remains intact; and truncation never splits a Unicode scalar.
- The lexical sentence rule is pinned for short multi-sentence text,
  abbreviations, decimals, punctuation at the 500-scalar boundary, and trailing
  closing quotes or brackets. Without a fitting terminator, text ends at a
  Unicode-whitespace boundary when possible and never exceeds 500 scalars.
- Authored summaries win wherever an effective summary is used. A titleless
  text-only draft label never exceeds 100 scalars; a titleless image-only draft
  uses its slug.
- A permalink without an authored summary emits derived description and Open
  Graph description metadata.
- Atom and JSON Feed items gain derived summary metadata while Atom, JSON, and
  RSS all retain their complete rendered HTML content.
- Cached feeds created before this behavior change cannot continue to be served
  after the deployment migration.
- Focused tests cover extraction, entity decoding, whitespace and boundary
  rules, Unicode limits, precedence, textless behavior, and metadata/feed
  projection. Regression assertions prove the exclusions below, including
  absence from Post and revision storage.

## Boundaries

- Do not synthesize public-site summary paragraphs or AtomPub summaries; those
  surfaces continue to distinguish authored metadata from absence.
- Do not shorten, excerpt, or remove complete RSS, Atom, or JSON Feed content.
- Do not persist derived text or change permalink-title, Site Tagline, or
  site-level metadata behavior.
