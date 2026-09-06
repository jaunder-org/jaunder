# Media elements in Post bodies

## Outcome

Authors can embed playable audio and video, multi-format sources, and accessible
WebVTT tracks in Post bodies. The elements survive rendered-HTML sanitization
without weakening Jaunder's stored-XSS boundary or Media reference protection.

## Load-bearing decisions

- The shared rendered-HTML sanitizer admits `audio`, `video`, `source`, and
  `track`, so the same policy applies to HTML Posts and raw HTML embedded in
  Markdown or Org Posts.
- The admitted attribute surface is intentionally minimal:
  - `audio`: `src`, `controls`;
  - `video`: `src`, `controls`, `poster`, `width`, `height`;
  - `source`: `src`, `type`;
  - `track`: `src`, `kind`, `srclang`, `label`, `default`.
- Each new element also retains ammonia's existing generic `lang` and `title`
  attributes. They are inert for Media reference extraction.
- `controls` is permitted but not required. Authors retain fallback content and
  may compose multiple `source` and `track` children using ordinary HTML.
- `autoplay`, `loop`, `muted`, and `preload` remain stripped. In particular,
  browser autoplay restrictions are not Jaunder's policy boundary.
- `srcset` remains forbidden. It is a multi-URL grammar and is not covered by
  ammonia's URL-attribute scheme filtering or Jaunder's one-URL Media reference
  extraction model.
- Media URLs retain the complete `url_schemes` and relative-URL policy of
  ammonia 4.1.4's default builder unchanged. Relative, HTTP, and HTTPS URLs
  survive; `javascript:` and `data:` URLs do not. Jaunder does not proxy or
  rewrite sources.
- Every admitted URL-bearing pair participates in sanitized-HTML Media reference
  extraction: `audio[src]`, `video[src]`, `video[poster]`, `source[src]`, and
  `track[src]`.
- A `track[src]` value naming stored Jaunder Media protects that Media like any
  other reference, including a stored `.vtt` file. An external URL without a
  Jaunder Media-shaped path produces no reference. An absolute or
  scheme-relative external URL with a Media-shaped path remains a reference
  until ADR-0154's live ownership resolution classifies it as owned, foreign, or
  unknown.
- Non-URL attributes are explicitly classified as inert for Media reference
  extraction. Admission of an element/attribute pair may not create an
  unclassified sanitizer surface.
- The change preserves ADR-0079's no-executable-markup invariant: audio and
  video are non-executable rendered content, while scripts, event handlers, and
  unsafe URL schemes remain excluded. Because ADR-0079 also fixed the accepted
  allowlist to ammonia's default plus fenced-code classes, a successor ADR
  records this security-boundary expansion and updates the architecture view.
  ADR-0090's rule that Media references derive only from sanitized rendered HTML
  remains unchanged.
- The sanitizer expansion receives a full security review despite its small
  implementation size.

## Acceptance

- Sanitized HTML preserves the four media elements, their element-specific
  admitted attributes, and inherited `lang` and `title`, including multiple
  `source` choices and an accessible `track`.
- HTML, Markdown, and Org Post rendering all preserve an allowed representative
  media embed.
- Sanitization removes `autoplay`, `loop`, `muted`, `preload`, `srcset`, event
  handlers, `javascript:` URLs, and `data:` URLs from the new elements.
  Relative, HTTP, and HTTPS URLs survive on every admitted URL-bearing pair.
- Sanitized `audio[src]`, `video[src]`, `video[poster]`, `source[src]`, and
  `track[src]` values naming Jaunder Media produce Media references.
- External HTTP(S) media and WebVTT sources remain renderable. Non-Media-shaped
  URLs produce no reference; Media-shaped external URLs follow ADR-0154 live
  ownership resolution and do not protect local Media when proven foreign.
- The sanitizer/reference coupling check reports no unclassified permitted
  element/attribute pair.
- Existing active-markup rejection and Media deletion protections remain green.

## Boundaries

- No upload workflow, MIME validation, transcoding, media player UI, proxying,
  origin restriction, or WebVTT authoring workflow is added.
- No additional media attributes or URL grammars are admitted.
- No change is made to the Media Upload Capability, Media storage formats,
  ownership resolution, or guarded-deletion semantics.
- No client-side sanitizer or new rendered-HTML construction path is introduced.
