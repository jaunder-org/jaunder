# Active Post slug uniqueness

Issue: [#1616](https://github.com/jaunder-org/jaunder/issues/1616)

## Outcome

A User cannot have two active Posts with the same slug. Existing duplicate data
is repaired deterministically to match the filenames already produced by the
Emacs Protocol Client, historical public permalinks continue to redirect, and
`jaunder-reconcile` can again join those Posts by stable Post ID without an
`inventory-conflict`.

## Load-bearing decisions

- The database is authoritative: active Post slugs are unique per User. Both
  backends enforce a partial unique index on `(user_id, slug)` where
  `deleted_at IS NULL`.
- Slugs are not globally unique. Different Users may use the same slug, and a
  Deleted Post releases its active slug under the existing Post lifecycle.
- Runtime creation never renames an existing Post. It tries the derived or
  explicit base slug first, then the first available `-1`, `-2`, … suffix,
  truncating the base through the existing slug-length rules. The database
  constraint arbitrates concurrent attempts.
- An explicit draft update that requests another active Post's slug fails as a
  slug conflict. Published and Scheduled Post slug freezing remains governed by
  ADR-0130.
- A one-time migration repairs each existing active `(user_id, slug)` duplicate
  group atomically. This remediation-only rule keeps the newest Post by
  `(created_at, post_id)` on the base slug because the observed Emacs inventory
  already gives that Post the unsuffixed filename. Older Posts receive `-1`,
  `-2`, … in ascending `(created_at, post_id)` order.
- Remediation chooses the first suffix not already occupied by any active Post
  for that User, including a pre-existing suffixed slug, and preserves the
  maximum slug length. It never relies on migration row order.
- Before changing a Post's slug, remediation records its old User-qualified
  date-and-slug identity as a Historical Post Permalink Alias. The alias stores
  the source identity and Post ID, not a destination URL.
- A canonical `GET /~username/YYYY/MM/DD/slug` lookup always wins. Only after
  that lookup misses may the server resolve a Historical Post Permalink Alias,
  reapply the same current anonymous visibility and publication-time rules, and
  issue a same-origin `302` to the Post's current canonical permalink.
- Historical redirects preserve the raw query string and send
  `Cache-Control: no-store`. Missing, hidden, future, or Deleted targets retain
  the existing indistinguishable public shell-miss behavior.
- Historical aliases are durable and backend-neutral. A later legitimate slug
  change redirects every recorded historical identity to the current permalink;
  aliases do not become AtomPub Member identities or Emacs matching keys.
- Each repaired Post is one meaningful lifecycle mutation: before changing the
  slug, remediation captures exactly one complete prior-state Post Revision and
  its tag, audience, and media children. It advances `updated_at` to one
  migration observation clamped strictly later than that Post's prior value.
  Separately, the canonical AtomPub strong-ETag input tuple includes the slug so
  any canonical-slug change invalidates a prior validator. Derived feed/cache
  state that embeds canonical permalinks is invalidated.
- SQLite and PostgreSQL migrations have identical data outcomes and install the
  uniqueness constraint only after repair succeeds. Any unrecoverable repair
  condition aborts migration rather than dropping Posts or weakening the
  constraint.
- `duplicate-target-slug` remains in the Emacs inventory as a corruption
  detector. Reconciliation continues to use Post ID as the identity join and
  does not gain an automatic conflict-resolution escape hatch.
- The durable decision is recorded in
  `docs/adr/drafts/active-post-slug-uniqueness.md` and projected into the
  architecture view.

## Acceptance

- SQLite and PostgreSQL migration fixtures reproduce two-, three-, and four-Post
  duplicate groups matching issue #1616 and yield the same repaired slugs:
  newest unsuffixed, older Posts numbered from `-1` in chronological order.
- Migration coverage proves pre-existing occupied suffixes are skipped,
  near-limit Unicode slugs remain valid, multiple Users remain independent,
  Deleted Posts do not participate, and a second migration run is inert.
- Both schemas reject duplicate active `(user_id, slug)` rows while allowing the
  same slug for another User and reuse after soft deletion.
- Sequential creation and shared SQLite/PostgreSQL backend-matrix race coverage
  prove an existing base owner is unchanged and newcomers receive the first
  available suffix beginning at `-1`.
- Update coverage proves an occupied explicit draft slug returns a bounded slug
  conflict without changing either Post; published/scheduled freeze behavior is
  unchanged. A shared-backend-matrix race in which two draft updates request the
  same free slug proves exactly one succeeds, the loser receives that conflict,
  both Post IDs remain stable, and neither losing content nor other state is
  partially changed.
- Every repaired Post retains its Post ID and content, gains exactly one
  complete prior-state Revision, has a strictly advanced `updated_at` and
  changed strong AtomPub ETag, and exposes a unique slug through its AtomPub
  Member. ETag unit coverage proves otherwise-identical Members with different
  slugs have different validators; revalidating the pre-migration ETag returns
  the repaired representation. The issue's representative inventory shapes join
  into ordinary matches rather than `duplicate-target-slug` conflicts.
- User-qualified canonical route coverage proves canonical precedence,
  historical redirect target derivation, query preservation, no-store caching,
  alias chaining to a later canonical slug, and shell misses for unavailable or
  anonymously hidden targets.
- Existing User-omitting `/YYYY/MM/DD/slug` compatibility behavior remains
  governed by ADR-0189 and does not consult the historical alias as a second
  ambiguous identity source.
- Feed/cache and representation tests prove repaired canonical links and strong
  validators do not retain stale pre-migration slugs.

## Boundaries

- This work does not make slugs globally unique across Users.
- It does not automatically rename an existing Post when a later Post is
  created.
- It does not remove or weaken Emacs inventory conflict detection.
- It does not make historical aliases editable, enumerable through application
  APIs, or valid AtomPub Member edit URLs.
- It does not change title-derived slug normalization, Post ID identity,
  publication-state slug freezing, or soft-deletion semantics.
