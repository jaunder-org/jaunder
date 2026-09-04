# Complete Syndication Feed HTTP Validators Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for isolated slices.
> This outline exists because the approved specification changes a dual-backend
> schema, an atomic cache-write contract, and public HTTP conditional semantics.

Authoritative specification:
`docs/superpowers/specs/2026-09-04-issue-1054-feed-http-validators.md`

## Scope

In:

- Complete RSS, Atom, and JSON semantic identity and strong ETags.
- Persisted semantic fingerprints and atomic same-identity cache preservation.
- Whole-second representation modification time in storage and wire formats.
- RFC 9110 conditional GET/HEAD behavior and response metadata.
- Dual-backend migration, focused behavior tests, route coverage, and
  architecture projection updates.

Out:

- HybridWindow membership or continuous age expiry.
- Cache max-age changes.
- Ordering commits from different Post snapshots beyond the existing publisher
  generation fence.
- Validators for non-Syndication-Feed resources.

## Task outline

- [x] Task 1: Establish complete representation identity in the host layer
  - Contract: introduce one closed semantic fingerprint type; derive it from
    format, that format's explicit serializer revision, complete feed metadata,
    and every ordered item field and tag. Derive the public strong ETag from the
    same tuple plus the selected representation modification time.
  - Contract: Atom `feed.updated` and RSS `channel.lastBuildDate` consume the
    representation time; JSON Feed adds no root timestamp and retains item
    `date_modified` values.
  - Verification: focused host tests prove stability and sensitivity for every
    tuple member, ordering, all formats, independent revision changes, empty
    input, and modification-time changes.

- [x] Task 2: Implement RFC 9110 conditional response semantics
  - Contract: a byte-level parser evaluates repeated `If-None-Match` values,
    weak entity-tag comparison, bounded empty elements, wildcard, malformed
    syntax, and precedence over `If-Modified-Since`; HTTP dates accept all three
    wire forms and emit IMF-fixdate.
  - Contract: response construction has one source for current ETag,
    Last-Modified, and Cache-Control; 200 GET includes body and Content-Type,
    200 HEAD omits body with GET-equivalent headers, and 304 GET/HEAD omit body
    and Content-Type while retaining validator/cache headers.
  - Verification: focused handler tests exercise the complete header grammar,
    precedence, date, status, header, and body matrix from the specification.

- [x] Task 3: Cut over atomic cache identity and every regeneration caller
  - Depends on: Task 1's fingerprint and ETag contracts.
  - Contract: the next paired SQLite/PostgreSQL migration adds the required
    fingerprint column and invalidates legacy cache rows instead of inferring
    identity from old ETags.
  - Contract: `FeedCacheRow` carries fingerprint, representation, strong ETag,
    whole-second modification time, and generation time. Generation-fenced
    commit returns `Committed(effective_row)` or `StaleGeneration`.
  - Contract: an equal fingerprint preserves the stored representation, ETag,
    and modification time while advancing `generated_at`; a different
    fingerprint replaces all candidate identity fields in the same transaction.
  - Contract: regeneration captures one whole-second current instant and builds
    the candidate row. Worker and inline cache-miss paths consume the effective
    committed row; stale publisher generations retain their retry behavior.
  - Contract: every `FeedCacheRow` producer, fixture, mock, cache-write caller,
    and commit-outcome consumer migrates in this task with no compatibility
    path. No-op regeneration advances `generated_at` for go-live accounting
    without changing externally visible identity.
  - Verification: `#[apply(backends)]` storage tests cover migration state,
    round trips, equal/different fingerprints, effective-row return,
    whole-second precision, generation advancement, and a forced concurrent
    equal-fingerprint race. Focused server tests cover empty feeds,
    metadata-only changes, removals, transitions to/from empty, byte-identical
    regeneration, effective cache-miss responses, and generation-fence outcomes.

- [x] Task 4: Prove the public contract and update maintained architecture
  - Depends on: Tasks 1–3.
  - Contract: no new compatibility aliases, deprecated paths, lint suppressions,
    or backend-specific semantics.
  - Verification: dual-backend route tests cover GET/HEAD 200/304 behavior and
    cached regeneration scenarios; the maintained Syndication Feed architecture
    projection describes the completed validator and cache identity flow.
  - Gate: each observable slice reaches `jaunder-commit`; the commit hook owns
    the staged-tree `precommit` run, with no `Co-Authored-By` trailer.

## Risk checks

- Every serializer input is represented exactly once and in serializer order;
  adding a byte-affecting serializer change requires its format revision bump.
- The semantic fingerprint excludes only the derived representation time; the
  public ETag includes it.
- Atomic equality is decided by storage, never by a preceding read susceptible
  to inline/worker races.
- The effective committed row crosses every cache-write callsite; no caller
  serves a discarded candidate.
- `generated_at` remains regeneration bookkeeping and is never an HTTP validator
  input.
- Request entity-tags are bytes, not UTF-8 strings; `obs-text` is valid while
  malformed input can neither match nor activate `If-Modified-Since` fallback.
- HEAD and 304 body absence is verified at the routed HTTP boundary, not
  inferred from Axum defaults.
- Existing feed cache callers, test support, migrations, both storage backends,
  and `docs/ARCHITECTURE.md` migrate in the same clean cutover.
