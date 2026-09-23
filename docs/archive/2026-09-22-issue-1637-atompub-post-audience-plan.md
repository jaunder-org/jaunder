# AtomPub Post Audience Round-Trip Implementation Outline

> Execute with `jaunder-iterate`; delegate an individual task with
> `jaunder-dispatch` when useful. This outline exists because issue #1637
> changes the public AtomPub protocol and visibility-sensitive conditional-write
> semantics.

## Scope

In:

- Repeated `j:audience` request/response elements and exact capability
  discovery.
- ADR-0020 union preservation, strict wire validation, and canonical output.
- Audience-aware strong Member ETags and Atom/Org precedence.
- Emacs publish, pull, reconciliation, pure ERT, live integration, and upgrade
  recovery guidance.
- The approved ADR draft and architecture projection.

Out:

- Storage migrations or audience-model changes.
- Browser audience controls or Default Audience configuration.
- Friendly Named audience discovery, labels, completion, or selection UI.
- Automatic reconciliation conflict resolution.
- Syndication Feed representation changes.

## Task outline

- [x] **Task 1: Establish one canonical Atom audience projection and validator**
  - Contract: the AtomPub mapping seam owns repeated Jaunder-namespace
    `audience` values and returns structured presence separately from absence.
    It accepts deduplicated unions of Public, Subscribers, and canonical
    positive Named IDs; Private stands alone. Canonical output is Public,
    Subscribers, then ascending Named IDs, retaining targets dominated by
    Public.
  - Contract: the projection is the single input shape later used by Entry
    serialization and the strong Member ETag; this task does not wire an
    audience-aware ETag before responses expose the same state.
  - Verification: pure `common`/`host` tests cover token grammar, namespace,
    duplicates, Private exclusivity, union retention, ordering, and structured
    Atom-presence-over-Org-header precedence. Pure Service Document tests prove
    the exact Jaunder namespace, version `1`, and `audience` advertisement.

- [x] **Task 2: Apply the wire contract and ETag at every server boundary**
  - Contract: incoming `j:audience` becomes structured audience presence before
    Org normalization, wins completely over Org-header audience, and continues
    through existing author authorization. Absence keeps header/default/create
    and preserve/update behavior unchanged.
  - Contract: authenticated Member and Collection Entries emit the complete
    target set; the Service Document advertises `audience` under the existing
    Jaunder extension version `1`. In the same slice, the canonical target
    projection enters `etag::post_content_etag`, so responses and validators
    change together and input order cannot perturb the ETag.
  - Verification: pure ETag tests prove stability across input order and changes
    across target sets. Backend-parametric server integration tests prove
    create, replacement update, omission, Atom-over-Org precedence, complete
    response round-trip, invalid/foreign target rejection, audience-only ETag
    change, and stale `If-Match` rejection on SQLite and PostgreSQL. Use the
    focused AtomPub lane through
    `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/^atompub::/)'`
    while iterating.

- [x] **Task 3: Carry audience through the Emacs Protocol Client's pure seams**
  - Contract: `jaunder-entry` carries the complete target set;
    `jaunder--org->atom` maps repeated local `JAUNDER_AUDIENCE`; the Atom
    serializer/harvester maps repeated `j:audience`; pull writes deterministic
    repeated properties; reconciliation observes the server's audience-aware
    ETag.
  - Contract: capability recognition requires the Jaunder namespace, supported
    version `1`, and `audience` token. An explicit local audience without that
    exact advertisement fails before Local Post Link, Media, or Post mutation;
    omission retains compatibility.
  - Verification: pure ERT covers mapping both directions, exact Named ID
    grammar, union preservation, canonical ordering, malformed response
    handling, foreign/wrong/missing capability evidence, and pre-mutation
    failure.

- [x] **Task 4: Prove the live workflow and document authoring and upgrades**
  - Contract: real-server Emacs tests cover explicit create, Private-to-Public
    replacement, multi-target union retention, omission behavior, pull, and
    audience-only reconciliation states.
  - Contract: user guidance documents the complete property grammar, exact
    omission behavior, capability requirement, raw Named-ID limitation, and the
    one-time ETag rebaseline. Unchanged `server-ahead` Posts are fetched. For
    conflicts, preserve the local file outside the managed root, fetch current
    remote content/audience, reapply the intended local edit, and conditionally
    publish; never guess which side wins.
  - Contract: change the architecture projection's audience passages from
    committed direction to current reality only when the implementation and
    proofs exist.
  - Verification: `devtool run -- cargo xtask elisp-integration` proves the live
    path; documentation examples and link/format checks pass at the commit gate.

## Key contracts

- Atom namespace: `https://jaunder.org/ns/atompub`; extension version remains
  `1`; feature token is `audience`.
- Named wire spelling: `named:[1-9][0-9]*`, within signed 64-bit range.
- Presence is semantic: no `j:audience` means omitted; `j:audience` value
  `private` means the explicit empty target set.
- A response exposes the complete stored target set, not merely effective
  visibility.
- Strong ETags vary with the canonical target set, not source order.

## Risk checks

- Preserve every ADR-0020 target even when Public dominates; never normalize the
  stored/wire set down to effective visibility.
- Keep existing raw-Org `JAUNDER_AUDIENCE` ingestion and structured-over-header
  precedence intact.
- Reject unsupported capability evidence before any Media upload or Post write.
- Ensure ETag changes are identical across SQLite and PostgreSQL and are applied
  consistently to GET responses and `If-Match` checks.
- Treat the first post-upgrade ETag mismatch as an explicit reconciliation
  rebaseline; document safe recovery rather than weakening conflict detection.
- Keep `CONTEXT.md` unchanged unless implementation discovers genuinely new
  domain vocabulary; this design reuses Post audience terms already established
  by ADR-0020.
- Add no lint suppression without explicit approval and no commit trailer.
