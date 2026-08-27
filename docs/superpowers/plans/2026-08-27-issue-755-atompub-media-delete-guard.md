# AtomPub Media Delete Guard Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a delegated task
> when useful. This outline exists because issue #755 changes the public AtomPub
> wire contract and must preserve ADR-0154 storage-safety invariants.

## Scope

In:

- Guard AtomPub media Member deletion and return the approved problem document.
- Distinguish owner-reference conflicts from global rowless-reference conflicts.
- Extend AtomPub integration coverage for the complete observable contract.

Out:

- Storage schema, dialect SQL, media ownership probing, and reclamation changes.
- AtomPub force extensions, Emacs client work, or general AtomPub error
  conversion.
- Changes to the web media library's existing confirmation and force behavior.

## Task outline

- [x] Task 1: Add the referenced-media problem response contract
  - Contract: one AtomPub-owned response type emits status 409,
    `application/problem+json`, the specification's exact `type`, `title`,
    conditional `detail`, and unique ascending numeric `post_ids`.
  - Verification: focused response tests prove the exact owner-reference and
    global-safety documents, including content type, ordering, and empty IDs.
- [x] Task 2: Guard AtomPub media Member deletion end to end
  - Contract: `member_delete` uses the existing non-force storage path; owner
    live-Post IDs select the owner detail, while a refusal without reportable
    owner IDs selects the nondisclosing global-safety detail. Existing 204 and
    404 behavior remains unchanged.
  - Verification: focused AtomPub integration coverage proves unreferenced,
    owner-reference, multiple-owner-reference, Deleted Post, proven-foreign,
    unknown/ambiguous ownership, and post-refusal preservation behavior. A
    request-fixture end-to-end case exercises the deployed guarded DELETE,
    asserts the conflict representation, and observes that the media Member
    remains available.

## Risk checks

- `post_ids` disclose only the authenticated owner's live Posts and are unique,
  numeric, and ascending.
- Ownership probe failures and ambiguous results remain fail-closed evidence
  uncertainty, not masked internal errors.
- The handler performs ownership resolution before entering the existing atomic
  conditional delete; SQLite and PostgreSQL continue to share storage semantics.
- AtomPub exposes no force input, and global rowless-reference safety remains
  non-bypassable by the unchanged web force path.
- Unexpected storage and serialization failures retain typed sources and use the
  existing masked AtomPub boundary.
- Update only tests and durable protocol documentation whose current contract is
  changed; do not broaden problem details to unrelated AtomPub failures.
