# Media elements in Post bodies implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for an isolated task
> when useful. This outline exists because issue #743 changes the security-owned
> rendered-HTML sanitizer allowlist and requires a successor to ADR-0079.

## Scope

In:

- Admit the spec's exact media elements and attributes through the shared
  host-side sanitizer.
- Extend sanitized-HTML Media reference classification for every new permitted
  element/attribute pair.
- Record the revised allowlist decision in a proposed ADR draft and project it
  into the architecture view.
- Prove all three Post formats, rejected active attributes and schemes, Media
  references, and the sanitizer/extractor coupling.

Out:

- Upload, MIME-validation, transcoding, player-UI, proxy, origin-policy, and
  WebVTT-authoring changes.
- `srcset`, autoplay behavior, additional playback attributes, client-side
  sanitization, or changes to Media ownership and deletion semantics.
- Author-side ADR numbering, promotion, or generated `docs/README.md` edits.

## Task outline

- [x] Task 1: Preserve safe media elements and their Media references.
  - Contract: the shared sanitizer admits exactly `audio[src,controls]`,
    `video[src,controls,poster,width,height]`, `source[src,type]`, and
    `track[src,kind,srclang,label,default]`, plus inherited `lang` and `title`.
    The host extractor classifies `audio[src]`, `video[src]`, `video[poster]`,
    `source[src]`, and `track[src]` as one-URL Media-bearing pairs; all other
    new pairs are inert. Existing `parse_media_url` and live ownership
    resolution remain the only identity/ownership authorities.
  - Decision record: add a numberless proposed successor ADR under
    `docs/adr/drafts/` that preserves `RenderedHtml`'s no-executable-markup
    invariant, permits the bounded media surface, and leaves ADR-0090/ADR-0154
    semantics intact; project that current policy into `docs/ARCHITECTURE.md`
    with a promotable path citation in the same feature deliverable.
    `CONTEXT.md` needs no change because no domain term changes;
    `docs/README.md` remains unchanged.
  - Verification: focused common and host render tests prove exact preservation
    and rejection, all-three-format rendering, and zero unclassified sanitizer
    pairs. The five URL-bearing pairs each preserve relative, HTTP, and HTTPS
    values while rejecting `javascript:` and `data:`. Extraction proof covers
    local paths, non-Media-shaped external URLs, and Media-shaped absolute and
    scheme-relative URLs; existing ADR-0154 ownership/deletion tests must prove
    that only proven-foreign references stop protecting local Media. ADR format,
    documentation links, and architecture-view parity accept the draft and
    projection. Browser-drive a representative rendered Post containing video,
    alternate sources, audio, and a WebVTT track, then run the repository
    static/check lane. Obtain the required full security review before shipping.

## Risk checks

- `javascript:`, `data:`, event handlers, autoplay, loop, muted, preload, and
  `srcset` remain absent from sanitized output; relative/HTTP/HTTPS sources
  survive under ammonia 4.1.4's unchanged default URL policy.
- Generic `lang` and `title` pairs are included in the classification surface,
  not hidden by element-specific attribute tables.
- Media-shaped absolute and scheme-relative URLs remain references until
  ADR-0154 live ownership resolution; only proven-foreign references stop
  protecting local Media.
- Every permitted pair is classified without treating URL-bearing names as
  inert, and guarded Media deletion behavior remains unchanged.
- Markdown, Org, and HTML continue to mint `RenderedHtml` only through the
  common host-side sanitizer; no new raw-construction or web sink appears.
