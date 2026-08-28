# Feed Cache Body Coupling Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a task when
> useful. This outline exists because typed reconstruction must preserve a
> non-obvious storage invariant across both database backends.

## Scope

In:

- A closed `common::feed` representation for rendered RSS, Atom, and JSON Feed
  bodies, with format and content type derived from its variant.
- An invariant-bearing `FeedCacheRow` construction/readback boundary.
- End-to-end caller migration and focused common, dual-backend storage, and
  server integration coverage.

Out:

- Schema changes, payload reparsing, cache repair, new observability, route or
  wire-format changes, and unrelated feed refactors.

## Task outline

- [x] Task 1: Establish the typed rendered Syndication Feed representation
  - Contract: `common::feed::SyndicationFeedRepresentation` is a public struct
    with private format/body state. RSS, Atom, and JSON renderers return it
    through renderer-owned constructors that establish in-memory provenance. A
    separate fallible `try_from_stored(FeedFormat, ContentType, String)` door
    establishes metadata agreement only. Accessors expose/consume the body and
    derive `FeedFormat` and `ContentType`; no API retags an existing body.
  - Verification: focused common feed tests prove each serializer produces the
    expected format, content type, and unchanged wire body, while
    `try_from_stored` accepts matching metadata and rejects every mismatched
    format/content-type pair.
- [x] Task 2: Make feed-cache storage enforce metadata agreement
  - Depends on: Task 1's representation contract.
  - Contract: `FeedCacheRow` fields are not independently forgeable;
    construction verifies `FeedPath` format against the representation, while
    readback reconstructs only when persisted `FeedPath` and `ContentType`
    agree. Mismatch is a typed storage error, not `None`.
  - Verification: focused construction coverage rejects a representation whose
    format conflicts with its `FeedPath`; `#[apply(backends)]` coverage proves
    coherent round-trip and replacement on SQLite/PostgreSQL, plus rejection of
    directly inserted mismatched metadata on both backends.
- [x] Task 3: Carry the representation through regeneration and HTTP serving
  - Depends on: Tasks 1-2 contracts.
  - Contract: regeneration, worker, handler, fixtures, and direct callers use
    the typed row API; valid hits serve the representation's derived
    `ContentType`; a corrupt hit propagates failure without regeneration,
    serving, or rewrite.
  - Verification: focused server feed tests prove all three regenerated formats,
    valid cache-hit MIME/body behavior, and integration-level corrupt-hit
    failure with the stored row unchanged.

## Risk checks

- The type distinguishes in-memory renderer provenance from storage readback's
  weaker metadata-agreement proof; no API claims persisted bytes were parsed or
  serializer-produced.
- Every `FeedCacheRow` constructor, fixture, storage mapper, regeneration
  caller, and response seam migrates in the clean cutover; no compatibility
  constructor or raw production body argument remains.
- Storage errors retain enough typed context to distinguish a semantic
  path/content-type mismatch from primitive decode failures.
- SQLite and PostgreSQL execute the same mismatch contract without dialect-only
  constraints or migrations.
- ETag bytes, validator behavior, serialized feed bodies, and coherent-row HTTP
  responses remain unchanged.
- The implementation references issue #697 and ADR-0063 at the deliberate
  primitive-to-type boundary; no new ADR, `CONTEXT.md` term, or lint suppression
  is introduced.
