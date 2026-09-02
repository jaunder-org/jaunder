# Issue #1030: Centralize rendered Post translation

## Outcome

`web/src/posts/server.rs` has one owner for translating a `PostRecord` into the
shared `RenderedPost` payload. Public listings and author surfaces retain their
existing draft, author, permalink, timestamp, tag, and source behavior.

## Load-bearing decisions

- `authored_post` remains the canonical translator because it must support both
  draft and published records and already owns the additional source payload.
- `rendered_post` retains the published-only guard before delegating; a draft
  still returns `None` and never reaches a public listing.
- `rendered_post` derives `is_author` by comparing the optional viewer identity
  with the record owner before moving the record into `authored_post`.
- The delegated result is `authored_post(post, is_author).post`; no second
  helper, public API, compatibility alias, or alternate translation path is
  introduced.
- `authored_post` continues to set `published_at` directly from the record and
  exposes a permalink only for published records. Existing field ownership and
  timestamp mappings remain unchanged.

## Acceptance

- `rendered_post` contains the published-only guard and viewer-to-owner author
  check, then delegates the shared translation to `authored_post`.
- `authored_post` remains able to translate drafts with no publication instant
  or public permalink.
- Existing focused tests prove draft exclusion and authored draft behavior.
- `cargo xtask check` passes.

## Boundaries

- No wire, page, or seed schema changes from #804.
- No ownership or authorization policy changes coordinated under #748.
- No changes outside the two translation functions and their focused contract
  tests unless required by compilation or formatting.
