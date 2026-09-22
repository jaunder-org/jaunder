# ADR-DRAFT: Active Post slug uniqueness and historical permalink aliases

- Status: proposed
- Date: 2026-09-22
- Issue: [#1616](https://github.com/jaunder-org/jaunder/issues/1616)

## Context

A Post's public identity includes its User, publication date, and slug, so the
original schema enforces uniqueness only on `(user_id, permalink date, slug)`.
An AtomPub Collection and the Emacs Protocol Client also expose the slug as the
canonical local filename. Legacy data can therefore contain several active Posts
for one User with the same slug on different dates. Stable Post IDs still
distinguish those Members, but the inventory correctly treats their target
filenames as ambiguous and blocks reconciliation.

The creation service already retries a storage-reported slug conflict with a
numeric suffix, but the date-scoped index does not report collisions across
dates and its first retry skips `-1`. Strengthening the constraint requires
repairing existing installations before the new index can be installed. Those
repairs change canonical permalinks, which must not silently break previously
shared links.

[Current-publication-state slug freezing](../0130-current-publication-state-slug-freeze.md)
keeps author-driven slug mutability tied to the current Post state. The repair
is a one-time correction of invalid identity data, not a new reason to rename an
existing Post when later content is created.

## Decision

- An active Post slug is unique per User. SQLite and PostgreSQL enforce
  `(user_id, slug)` where `deleted_at IS NULL`; Deleted Posts remain outside the
  active identity set.
- Creation keeps an existing slug owner stable and allocates the first available
  numeric suffix beginning at `-1`. Unique-constraint conflicts remain the
  concurrency authority. Explicit updates never rename another Post to satisfy a
  requested slug.
- Migration repairs each legacy duplicate group deterministically. The newest
  `(created_at, post_id)` member keeps the base slug solely to match the
  filenames already established by the Emacs Protocol Client. Older members,
  ordered oldest first, receive the first unoccupied `-1`, `-2`, … candidates
  that satisfy the existing slug length boundary. The new unique index is
  installed only after repair succeeds.
- Before repair changes a slug, storage records a permanent Historical Post
  Permalink Alias from the old User-qualified date-and-slug identity to the Post
  ID. The destination is always derived from the Post's current canonical state;
  no URL string is stored.
- Each repaired Post follows the ordinary meaningful-mutation contract: storage
  captures exactly one complete prior-state Post Revision and its child state,
  then advances `updated_at` to a migration observation strictly later than its
  prior value. Independently, the canonical AtomPub strong-ETag input tuple
  includes the slug, so the repaired representation cannot validate against the
  pre-repair ETag.
- The User-qualified public permalink route tries current canonical identity
  first. On a miss, it may resolve one historical alias, reapply current
  anonymous visibility and publication-time predicates, and return a no-store
  `302` preserving the query string. An unavailable target remains the existing
  shell miss.
- The User-omitting compatibility route from
  [the WordPress-compatible permalink alias decision](../0189-wordpress-compatible-permalink-alias.md)
  remains a separate lookup and never treats historical aliases as candidate
  Posts.
- AtomPub and Emacs continue to use Post ID as resource identity. AtomPub emits
  only the repaired current slug; the Emacs duplicate-slug check remains as a
  fail-closed corruption detector.

## Consequences

- Existing duplicate groups become reconcilable without changing Post IDs or
  silently choosing one Member by storage order.
- New duplicates are prevented by both databases, including concurrent writes,
  while different Users and Deleted Posts retain their existing independence.
- Remediation changes some canonical slugs, creates one owner-visible Revision
  per repaired Post, advances their modification times and strong validators,
  and invalidates derived feed/cache content.
- Historical links remain usable, but storage gains a durable alias table and
  the public projector gains one canonical-miss lookup. Aliases are not a second
  editable identity and do not broaden anonymous visibility.
- The migration needs backend-specific SQL with identical deterministic
  allocation behavior, including occupied suffixes and maximum-length Unicode
  slugs. Failure aborts the migration rather than partially repairing data.
