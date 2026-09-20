# ADR-DRAFT: Bounded Post Shortcodes Produce Trusted Provider Embeds

- Status: proposed
- Date: 2026-09-19
- Issue: [#1585](https://github.com/jaunder-org/jaunder/issues/1585)

## Context

Jaunder stores a Post's native source and derives one canonical rendered-HTML
representation for web presentation and Syndication Feeds. Markdown and Org
source can contain raw HTML, so
[ADR-0079](../0079-rendered-html-sanitization.md) made
`common::render::sanitize` the only public production door to `RenderedHtml` and
gave the type a no-active-markup invariant. Raw iframes are therefore stripped.

Authors migrating or editing publication source also need a small, stable way to
express provider-hosted video without storing hand-authored iframe markup. The
first required providers are YouTube and Vimeo. The mechanism must preserve
native AtomPub source, remain literal in code examples, fail safely for unknown
or malformed input, and make future providers local additions rather than
parser-wide changes.

An iframe is active third-party content even when its attributes are fixed.
Simply admitting `iframe[src]` to the sanitizer would let author-controlled raw
HTML cross the boundary and would contradict the reason `RenderedHtml` exists.
Treating shortcodes as a client-only rewrite would also make rendering depend on
the publishing client and break the canonical server rendering model.

## Decision

Jaunder owns a bounded Post Shortcode processor for Markdown and Org source. Its
initial complete-line grammar recognizes only `{{< youtube VIDEO_ID >}}` and
`{{< vimeo VIDEO_ID >}}` as top-level paragraphs, with zero to three leading
ASCII spaces and ASCII horizontal whitespace at token boundaries and after the
closer. YouTube identifiers are exactly 11 case-sensitive ASCII letters, digits,
`_`, or `-`; Vimeo identifiers are 1–20 ASCII digits with no leading zero. Every
other shape and every shortcode-looking construct in a code, literal, list,
quote, or other non-top-level-paragraph context remains text. HTML Posts are not
shortcode inputs.

The host-owned processor owns source recognition and dispatch, separating shared
tokenization and fallback policy from a deliberately closed
`common::render::TrustedProviderEmbed` type. Its private closed provider state
owns validated identifier fields and each provider-specific constructor,
canonical embed and fallback URLs, and fixed presentation metadata. Adding a
provider extends that closed type, host dispatch, and their tests; it does not
add a generic template evaluator, runtime registration, or an author-controlled
markup path.

Amend ADR-0079's `RenderedHtml` invariant from “contains no active markup” to
“contains no author-controlled active markup.” Untrusted parser output is still
sanitized through the existing common-owned boundary. A separate common-owned
trusted assembly path may substitute only `TrustedProviderEmbed` values. It
emits fixed iframe structure and attributes; it accepts neither a raw HTML
fragment nor an arbitrary iframe URL. Author content cannot bypass provider
validation or choose markup/URL parts, sanitized content and typed embeds retain
document order, and no internal assembly artifact survives.

Recognition and provider dispatch remain in `host` under
[ADR-0159](../0159-common-host-target-closure.md). Extend that decision's narrow
`common/sanitize` exception only as required by `RenderedHtml`'s private minting
boundary: the host-feature-gated `TrustedProviderEmbed` validation, fixed
markup, and structured assembly door live beside `sanitize`, while source
recognition and general rendering machinery remain host-owned.

Generated players use provider-owned HTTPS embed origins, lazy loading, fixed
responsive presentation, descriptive titles, fullscreen support, and an ordinary
provider link. YouTube derives only
`https://www.youtube-nocookie.com/embed/VIDEO_ID` with
`https://www.youtube.com/watch?v=VIDEO_ID` as its fallback; Vimeo derives only
`https://player.vimeo.com/video/VIDEO_ID` with `https://vimeo.com/VIDEO_ID` as
its fallback. Authors cannot supply dimensions, URL parameters, attributes, or
alternate origins.

Classify generated `iframe[src]` as an external embed resource rather than a
Jaunder Media-bearing pair. It creates no `MediaReference`; the typed provider
boundary, not URL-shaped source markup, is its authority.

## Consequences

Every rendered surface, including Syndication Feeds, receives the same player,
while AtomPub continues to round-trip unchanged Markdown or Org source. Unknown
future shortcodes remain visible and harmless until a provider is deliberately
added.

`RenderedHtml` can contain narrowly bounded active third-party frames, so its
safety claim is more precise but no longer absolute. Review and tests must prove
that all such frames originate from typed provider values and that author HTML
cannot reach the trusted assembly path. This decision amends ADR-0079, the
bounded non-executable surface in
[ADR-0179](../0179-rendered-html-media-elements.md), and ADR-0159's host-only
`common/sanitize` exception; it does not generally admit iframes to the
sanitizer or move general host rendering machinery into `common`.

Loading a player contacts a third party. Privacy-enhanced YouTube URLs, lazy
loading, and fixed origins reduce exposure but do not create a consent gate. A
click-to-load policy would be a separate user-experience decision.

The first grammar deliberately excludes HTML Posts, inline and nested forms,
named arguments, closing tags, runtime plugins, autoplay, and author-controlled
presentation. Supporting any of those requires an explicit extension rather than
accidental parser permissiveness.
