# Issue #941 — Build RSD XML with quick-xml

## Outcome

The RSD document serializer constructs its XML with `quick_xml::Writer` instead
of interpolating an XML string with `format!`. The rendered document remains
semantically equivalent: same RSD structure, same service/homepage URL roles,
and escaped URL text/attributes.

## Load-bearing decisions

- Use `quick_xml` for this owned XML surface because the crate is already in
  `common`, already used by adjacent AtomPub serializers, and handles
  text/attribute escaping at the writer boundary.
- Preserve the function contract:
  `render_rsd_document(&ServiceDocUrl, &HomepageUrl) -> String` stays infallible
  and keeps the typed URL role ordering that prevents service/homepage
  transposition.
- Preserve the existing RSD wire shape except for writer-owned serialization
  details such as XML declaration encoding spelling or self-closing
  empty-element syntax when XML-equivalent.
- The service URL remains an `apiLink` attribute and the homepage URL remains
  `homePageLink` text.
- No new generic XML abstraction is introduced; this is a local cleanup to align
  `rsd.rs` with the existing AtomPub `service.rs`/`categories.rs` writer style.

## Acceptance

- `render_rsd_document` uses `quick_xml::Writer`/events rather than `format!`
  plus manual `quick_xml::escape::escape` interpolation.
- Tests prove the document still contains the RSD root, engine name, homepage
  link, Atom API declaration, and service URL.
- Tests prove both URL placements are XML-safe: homepage text escapes query
  ampersands and service URL attribute escapes query ampersands.
- The compile-fail typed-URL transposition doc test remains valid.

## Boundaries

- No AtomPub Service Document, Atom Entry, feed, or parser behavior changes.
- No dependency/version changes.
- No broad XML helper refactor beyond using the existing local helper style
  where it directly fits this serializer.
