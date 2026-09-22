# Active Post slug uniqueness implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` only for an isolated
> task. This outline exists because the approved specification changes both
> storage dialects, persistent identity, write concurrency, and a public route.

Authoritative specification:
`docs/superpowers/specs/2026-09-22-issue-1616-active-post-slug-uniqueness.md`.

## Scope

In:

- active per-User slug uniqueness and conflict translation;
- deterministic cross-backend legacy repair with complete Revisions;
- Historical Post Permalink Alias persistence and public redirects;
- AtomPub validator, feed/cache, and Emacs inventory conformance;
- the approved ADR, glossary, and architecture projection.

Out:

- global slug uniqueness;
- editable or API-enumerable aliases;
- changing Post ID identity, slug normalization, publication-state freezing, or
  the User-omitting ADR-0189 route;
- weakening Emacs conflict detection.

## Task outline

- [x] Task 1: Migrate legacy data and enforce the storage invariant
  - Contract: matching `0041` SQLite/PostgreSQL migrations create Historical
    Post Permalink Alias storage, identify duplicate active `(user_id, slug)`
    groups, preserve the newest member's base slug, allocate deterministic
    unoccupied suffixes to older members, capture one full prior-state Revision
    and child snapshot per repaired Post, strictly advance `updated_at`, clear
    permalink-bearing cache state, and finally replace the date-scoped index
    with the active per-User partial unique index. Migration failure is atomic.
  - Verification: backend migration fixtures cover representative group sizes,
    occupied suffixes, near-limit Unicode, multiple Users, Deleted Posts,
    revision children, modification clocks, cache invalidation, and an already
    applied migration; schema tests prove rejection, cross-User independence,
    and soft-delete reuse.

- [x] Task 2: Make runtime creation and updates obey the invariant
  - Contract: the shared candidate allocator tries base, `-1`, `-2`, … within
    the existing slug-length rules. Create retries only the new Post candidate;
    update maps the new unique-index violation to a bounded slug-conflict result
    and never retries by mutating another Post. Both dialect implementations and
    service/API boundaries expose equivalent behavior.
  - Verification: `#[apply(backends)]` sequential tests cover first-gap
    allocation and owner stability; the shared backend matrix covers competing
    creates and two draft updates racing for one free slug, including atomic
    loser state and stable Post IDs. Existing published/scheduled freeze tests
    remain green.

- [ ] Task 3: Preserve repaired public paths through historical aliases
  - Contract: `PostStorage` gains one anonymous historical User-qualified
    permalink resolver returning the target Post's current canonical route.
    `PublicProjector::permalink` invokes it only after canonical lookup misses,
    then uses the existing percent-encoded redirect builder with a same-origin
    `302`, raw query preservation, and `Cache-Control: no-store`. Resolver SQL
    joins aliases to current Posts and Users and applies the canonical anonymous
    active, audience, and publication-time predicates. The existing
    User-omitting alias resolver remains unchanged.
  - Verification: backend resolver tests and projector/handler tests prove
    canonical precedence, current-target derivation after another slug change,
    Unicode encoding, query preservation, and indistinguishable misses for
    absent, Deleted, future, and anonymously hidden targets.

- [ ] Task 4: Make protocol projections observe repaired canonical identity
  - Contract: slug joins the canonical AtomPub strong-ETag input tuple without
    changing Post ID Member identity. Migration-updated representations and
    feeds expose repaired canonical links. Emacs continues to join by Post ID
    and retains `duplicate-target-slug` as a corruption detector.
  - Verification: ETag tests distinguish otherwise-identical Members by slug and
    reject a pre-repair validator; AtomPub/feed tests prove repaired links and
    fresh cache output; an inventory regression proves representative repaired
    Members become ordinary matches while genuine duplicate target slugs still
    conflict.

- [ ] Task 5: Finalize durable decision documentation
  - Contract: keep `docs/adr/drafts/active-post-slug-uniqueness.md`,
    `CONTEXT.md`, and `docs/ARCHITECTURE.md` aligned with the implemented
    contracts; do not edit the generated ADR index or promote the draft on this
    branch.
  - Verification: documentation links, formatting, ADR draft format, and
    architecture projection checks pass in the repository gate.

## Ordering and commit boundaries

1. Task 1 establishes the schema contract consumed by Tasks 2 and 3.
2. Task 2 lands runtime write behavior against that contract.
3. Tasks 3 and 4 may be implemented independently after Task 1 but remain
   serialized in this checkout; each gets focused proof before its commit.
4. Task 5 reconciles documentation only after implementation names and behavior
   are final.

Each checked task stages its intended tree and commits through `jaunder-commit`;
the enforced pre-commit gate checks the staged result. No diagnostic or coverage
suppression is authorized.

## Risk checks

- SQLite and PostgreSQL must derive byte-for-byte equivalent valid slugs without
  depending on update order, collation order, or an arbitrary suffix bound.
- Revision insertion must snapshot scalar and child state before mutation and
  associate each child row with exactly the new Revision for its Post.
- The migration clock must be strictly later than each repaired Post's prior
  `updated_at`, including restored future timestamps and differing backend time
  precision.
- Constraint-error classification must identify only the new slug index; other
  storage failures must not be reported as user conflicts.
- Concurrent create retries and update conflict handling must respect
  transaction boundaries under both backend lock/isolation models.
- Alias lookup must not broaden anonymous visibility, redirect a semantic miss,
  loop onto its source, or shadow a current canonical Post.
- Slug in the ETag tuple must be canonical and encoded unambiguously so tuple
  boundaries cannot collide.
- Migration and runtime changes must preserve backup/restore, Post Revision,
  feed invalidation, AtomPub, and Emacs backend-parity coverage.
