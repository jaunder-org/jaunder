# Issue #1002: Centralize Syndication Feed renderer fixtures

## Outcome

The seven audited `FeedMetadata` and `FeedItem` test constructions use one
host-owned typed fixture seam. Format-specific Syndication Feed behavior and
variations remain visible in the Atom, RSS, JSON Feed, and metadata tests.

## Load-bearing decisions

- Follow ADR-0159's current ownership: add `host/src/feed/test_support.rs`, not
  the issue's stale `common/src/feed/test_support.rs` path. `common::feed`
  continues to own only dual-target grammar.
- Wire the sibling module under `#[cfg(test)]`; add no public production API.
- Provide a typed metadata constructor whose required input is the format-local
  `FeedUrl`. It supplies the shared valid title, canonical URL, timestamp, and
  absent optional metadata.
- Callers express description and WebSub Hub presence locally with struct
  update, preserving each format's presence/absence cases.
- Provide a typed item constructor taking `PostId`, `PermalinkUrl`,
  `RenderedHtml`, and one timestamp. It sets both publication timestamps to that
  value and leaves title, summary, and tags absent.
- Callers express title, summary, and tags locally with struct update. This
  keeps Atom/RSS/JSON differences visible and keeps the metadata test's id, URL,
  content, and timestamp load-bearing.
- Preserve JSON Feed's existing typed `item` and `item_with_summary` test-helper
  signatures from #694; they delegate to the shared constructor and continue
  parsing tag literals at the local boundary.
- Preserve #832's format behavior: Atom renders an empty entry title when
  absent, while RSS and JSON Feed omit it.

## Acceptance

- The three audited `FeedMetadata` literals in
  `host/src/feed/{atom,rss,json}.rs` delegate to the shared typed metadata
  constructor.
- The four audited `FeedItem` literals in those files and
  `host/src/feed/metadata.rs` delegate to the shared typed item constructor.
- Atom, RSS, JSON Feed, and metadata assertions remain format-local and
  observably unchanged.
- No compatibility alias, production constructor, builder family, or generic
  fixture abstraction is added.
- Focused host feed tests pass for every affected module.
- `cargo xtask check` passes.

## Boundaries

- Do not alter Syndication Feed serialization, metadata validation/composition,
  title fallback policy, or public/test interfaces.
- Do not move host-owned models back into `common` or broaden `common`'s target
  closure.
- Do not absorb malformed-input helpers or unrelated feed/storage/server
  fixtures.
- Coordination issues #694, #689, and #832 remain substantive-behavior
  exclusions, not blockers.
