# Issue #832 — Empty Atom title for untitled Posts

## Outcome

Untitled Posts in the public Atom Syndication Feed intentionally render an empty
`atom:title` element. RSS and JSON Feed continue to omit item titles when the
Post has no title.

## Load-bearing decisions

- RFC 4287 requires exactly one `atom:title` per `atom:entry`; the existing
  empty element satisfies that grammar. The RFC's non-empty-title language is
  interoperability guidance, not a conformance requirement.
- Jaunder preserves the author's intent: `Post.title = None` means the author
  did not title the Post, and the Syndication Feed must not synthesize one from
  body text, slug, permalink, or placeholder text.
- `FeedItem.title` remains `Option<PostTitle>` because the same `FeedItem` is
  rendered by Atom, RSS, and JSON Feed. Adding a required title-like neighbor
  would encode one renderer's presentation workaround into shared feed data.
- This is a Syndication Feed rule only. It does not change AtomPub Collection
  serialization, Post storage, or edit surfaces.
- The existing empty `atom:title` output is deliberate, not an accidental
  fallback; a regression test should pin it so a future change does not silently
  synthesize a title.

## Acceptance

- An Atom Syndication Feed item for an untitled Post contains exactly one empty
  `atom:title` element.
- An Atom Syndication Feed item for a titled Post continues to use the explicit
  Post title.
- RSS and JSON Feed output for an untitled Post remain title-less.
- No `FeedItem` fallback-title field, storage backfill, or presentation
  placeholder is introduced.
- Focused tests cover the Atom titled and untitled cases plus the existing
  RSS/JSON titleless behavior.

## Boundaries

- No public permalink, slug-generation, Post naming, AtomPub, or editor-facing
  behavior changes.
- No schema migration and no stored backfill.
- No new feed-wide title policy beyond documenting and testing the Atom
  empty-title rule for untitled Posts.
