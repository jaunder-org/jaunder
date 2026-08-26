# Issue #784: Raise the per-Post tag limit

## Outcome

A Post may carry up to 256 distinct canonical tag slugs through both web and
AtomPub authoring. A request with 257 or more remains a hard, pre-mutation
validation failure.

## Load-bearing decisions

- `MAX_TAGS_PER_POST` becomes 256. The limit is a finite resource safety rail,
  not an editorial judgement that a Post with many tags is probably mistaken.
- The limit counts distinct canonical tag slugs after validation and
  case-insensitive deduplication. Duplicate spellings retain the first label's
  casing and do not consume additional capacity.
- Web and AtomPub create/update paths continue to enforce the same limit before
  mutating the Post or its tag associations.
- `TagValidationError::TooMany` and each protocol's existing error mapping
  remain the over-limit contract. This issue does not redesign web or AtomPub
  errors.
- Tag persistence remains one atomic transaction and storage remains
  policy-free. A finite caller-side bound preserves ADR-0092's
  bounded-by-construction invariant without chunking or partial-write semantics.
- 256 follows the repository's existing bounded-batch precedent while leaving
  substantial authoring headroom. The analogous feed prototype is not treated as
  a direct benchmark of tag writes.
- Canonical-slug deduplication remains correctness policy independent of the
  cardinality limit.
- The threshold change does not establish a new architectural boundary, so it
  needs no ADR, glossary entry, or architecture projection.

## Acceptance

- The exported per-Post tag limit is exactly 256 and its documentation explains
  that it is a finite resource bound.
- Shared validation accepts exactly 256 distinct canonical slugs and rejects 257
  with `TooMany { count: 257, max: 256 }`.
- Case-insensitive duplicates are removed before the count and first label
  casing remains unchanged.
- Web and AtomPub over-limit tests derive their inputs from the shared constant,
  retain their current protocol error behavior, and prove validation occurs
  before creation or replacement of tag associations.
- Tests and names no longer encode the former value 25 as current policy.
- `cargo xtask precommit` passes; PR CI supplies the authoritative boundary
  gate.

## Boundaries

- No unbounded tag list, chunked persistence, transaction change, schema change,
  request-body limit, or tag-label length policy.
- No client-side tag-count enforcement or new user-interface copy.
- No change to malformed AtomPub category handling, tag identity, casing,
  deduplication, storage APIs, or error status/message mapping.
- Historical archived documents remain unchanged.
