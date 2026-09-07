# ADR-0179: Rendered HTML admits bounded media elements

- Status: accepted
- Date: 2026-09-06
- Issue: [#743](https://github.com/jaunder-org/jaunder/issues/743)

## Context

[ADR-0079](0079-rendered-html-sanitization.md) made `common::render::sanitize`
the only public production door to `RenderedHtml` and fixed its allowlist to
ammonia's audited default plus filtered fenced-code classes. That policy
prevents stored XSS, but ammonia's default strips `audio`, `video`, `source`,
and `track`, so an author cannot publish ordinary playable media or accessible
WebVTT captions in any Post format.

These elements fetch or play resources but do not execute author-supplied code.
Admitting them still expands a security boundary: URL attributes must retain
scheme filtering, event handlers and playback-triggering attributes must remain
absent, and [ADR-0090](0090-media-references-extracted-at-render.md) requires
every admitted attribute to be classified for Media reference extraction.

## Decision

Amend ADR-0079's single allowlist with this bounded non-executable media
surface:

- `audio`: `src`, `controls`;
- `video`: `src`, `controls`, `poster`, `width`, `height`;
- `source`: `src`, `type`;
- `track`: `src`, `kind`, `srclang`, `label`, `default`;
- ammonia's existing generic `lang` and `title` attributes on each element.

The sanitizer keeps ammonia 4.1.4's default URL schemes and relative-URL policy
unchanged. It does not admit `autoplay`, `loop`, `muted`, `preload`, `srcset`,
or event-handler attributes. `srcset` remains excluded because its multi-URL
grammar fits neither ammonia's URL-attribute filtering nor ADR-0090's one-URL
reference table.

Classify `audio[src]`, `video[src]`, `video[poster]`, `source[src]`, and
`track[src]` as Media-bearing pairs. Classify every other newly admitted
attribute as inert for reference extraction. Existing Media identity parsing and
[ADR-0154](0154-media-reference-live-ownership.md) remain authoritative; this
decision changes where references may appear, not what they identify or how live
ownership is resolved.

## Consequences

Authors can publish audio, video, alternate encodings, posters, and caption or
chapter tracks through HTML and raw HTML embedded in Markdown or Org. Controls
are available but not required, and the browser retains ordinary fallback
content behavior.

`RenderedHtml` continues to guarantee no executable markup: scripts, event
handlers, unsafe URL schemes, and unapproved attributes are stripped. Every
future sanitizer expansion remains a security decision and must satisfy the
sanitizer/reference classification gate.

External media may render under the existing URL policy. A non-Media-shaped URL
creates no Media reference; a Media-shaped absolute or scheme-relative URL
remains subject to ADR-0154's live ownership resolution before guarded deletion.
