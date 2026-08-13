# #341 / #340 — direct author-unpublished permalink lookup

Issues: [#341](https://github.com/jaunder-org/jaunder/issues/341),
[#340](https://github.com/jaunder-org/jaunder/issues/340). Milestone:
Correctness & data integrity.

## Summary

An author cannot open a future-scheduled Post at its canonical permalink when
the publication date differs from the creation date. The authenticated fallback
pages through as many as 10,000 not-yet-live Posts and compares the requested
date only with `created_at`, while `PostRecord::permalink()` correctly uses
`published_at.unwrap_or(created_at)`.

This cycle fixes the correctness bug and its coupled query-shape defect
together. `PostStorage` gains one author-scoped lookup for a not-yet-live Post,
keyed by user, slug, and canonical permalink date. The web fallback calls it
directly. The paginated helper and its 200-page safety bound disappear.

## Decisions

- **D1 — One coupled cutover closes both issues.** #341 is caused inside the
  scan that #340 requires replacing. Shipping a date-comparison patch first
  would preserve an acknowledged path that can issue 200 queries and materialize
  10,000 rows. One direct lookup fixes both defects without an intermediate
  implementation to remove later.
- **D2 — The operation is author-unpublished, not a general owned-Post lookup.**
  It returns only the named user's non-deleted true drafts
  (`published_at IS NULL`) and future-scheduled Posts (`published_at > now`).
  Already-live Posts remain the responsibility of the visibility-filtered public
  lookup. The web fallback remains narrow and cannot disclose another user's
  unpublished Post.
- **D3 — One request-scoped time is injected into both lookups.** The web `get`
  operation captures `now` once and passes that same value to the public
  visibility-filtered lookup and, if needed, the author-unpublished fallback.
  The storage operations therefore agree at the scheduled/live boundary, and
  deterministic tests cover both sides without sleeping.
- **D4 — The lookup uses the canonical permalink date.** It compares the
  requested date with `COALESCE(published_at, created_at)` in UTC: a true draft
  resolves by creation date; a scheduled Post resolves by scheduled publication
  date. The query also keys by `user_id` and `slug`.
- **D5 — Reuse the existing indexed identity.** Both backend schemas already
  define the partial unique index `posts_user_date_slug` over user, canonical
  UTC date, and slug for non-deleted Posts. No migration or new index is needed.
  Backend-specific date syntax remains isolated in `PostDialect` per ADR-0019.
- **D6 — Prove the storage and user-facing contracts.** Dual-backend storage
  tests cover scheduled and true-draft hits, a miss, and exclusion of deleted
  and already-live Posts. The existing scheduled-Post Playwright flow is
  extended to follow the drafts-page permalink and observe the scheduled Post
  rendered for its authenticated author.

No domain term changes and no novel hard-to-reverse architectural decision
result; `CONTEXT.md` and the ADR log remain unchanged.

## Acceptance criteria

- **AC1.** An authenticated author can navigate from the drafts page to a
  future-scheduled Post's canonical permalink, whose date comes from
  `published_at`, and the Post renders instead of returning the not-found view.
- **AC2.** A true draft remains resolvable for its author at its canonical
  permalink, whose date comes from `created_at`.
- **AC3.** Resolving an unpublished author permalink performs one direct storage
  query. The paginated scan, 200-iteration bound, and in-memory Post matching
  are removed.
- **AC4.** The direct lookup returns only a non-deleted true draft or
  future-scheduled Post owned by the supplied user. It returns `None` for a
  missing permalink, another user's Post, a deleted Post, and a Post already
  live at the injected `now`.
- **AC5.** One `get` request captures one `now` and supplies it to both the
  public and author-unpublished lookups. At `published_at == now`, the public
  lookup resolves the Post and the unpublished fallback excludes it; there is no
  instant at which both paths reject the Post because they sampled different
  times.
- **AC6.** The direct query matches the canonical UTC date expression used by
  the existing `posts_user_date_slug` index on SQLite and PostgreSQL; no schema
  migration is added.
- **AC7.** Backend-common storage regression tests run through the shared
  `#[apply(backends)]` harness and cover true-draft and scheduled hits,
  creation/publication dates that differ, missing and wrong-owner lookups,
  deleted and already-live exclusions, and the scheduled/live boundary on both
  lookup contracts.
- **AC8.** Ship review verifies the structural requirements: one direct query
  with no paginated scan (AC3), one request-scoped timestamp shared by both
  lookups (AC5), the indexed date expression, and the absence of a migration
  (AC6).
- **AC9.** The existing Playwright scheduled-Post scenario covers AC1 without
  adding a second document boot.
- **AC10.** `cargo xtask validate` is green.

## Out of scope

- Changing when a scheduled Post becomes publicly visible.
- Changing canonical permalink construction or slug-allocation rules.
- Allowing non-authors to discover unpublished Posts.
- Adding a migration, compatibility alias, or deprecated lookup path.
