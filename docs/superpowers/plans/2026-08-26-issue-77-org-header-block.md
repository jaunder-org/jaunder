# Org Header Metadata Block Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks with
> `jaunder-dispatch`. This outline exists because issue #77 changes AtomPub and
> web write contracts, canonical Post storage, and cross-backend slug
> concurrency.

## Scope

In:

- One shared Org normalization interface for every create/update ingress.
- Atomic persistence-dependent bookkeeping checks on SQLite and PostgreSQL.
- Web and AtomPub adapters, native errors, integration coverage, and browser
  flows.
- Compatibility checks for Emacs-generated and pulled Org metadata.

Out:

- Non-Org formats, new metadata aliases, schema migrations, and changing the
  canonical metadata-free body decision.
- Making bookkeeping authoritative or exposing foreign named audiences.
- Rewriting accepted ADR-0024; the proposed draft and architecture projection
  record the evolved decision.

## Task outline

- [x] Task 1: Normalize Org source and metadata behind one deep interface
  - Contract: a `common` Org module accepts source, structured field/lifecycle
    presence, operation context, and one injected request clock; it returns a
    typed effective metadata set, canonical metadata-free `PostBody`, and typed
    bookkeeping or one `OrgMetadataError`. Parsing, merge order, lifecycle/date
    conversion, stripping, and metadata-only rejection are not separate caller
    interfaces.
  - Contract: use `orgize` for the leading element boundary; reuse `PostTitle`,
    `PostSummary`, `TagLabel`, ID/slug/time types; add pinned IANA timezone data
    through the workspace dependency set. Audience ownership and final stored
    value checks stay outside `common`.
  - Contract: extract the existing content ETag projection into one reusable,
    transport-neutral interface consumed by normalization/finalization and
    AtomPub response/precondition handling; preserve its byte algorithm.
  - Migration: replace the Org-specific sequencing of
    `derive_post_naming`/`canonicalize_body`; preserve their non-Org behavior
    and migrate every reference using LSP before removing obsolete Org helpers.
  - Verification: focused `common` tests cover byte grammar, precedence and
    presence, duplicate/empty rules, unknown preservation, idempotent stripping,
    tag rules, the full lifecycle matrix, weekday/DST edges, one-clock equality,
    and metadata-only rejection.

- [x] Task 2: Make persistence-dependent finalization atomic on both backends
  - Depends on: Task 1 typed normalization/bookkeeping result.
  - Contract: shared post orchestration captures exactly one request clock and
    passes that instant through normalization and explicit persistence values;
    ingress adapters and storage adapters never capture another publication
    clock for the same write.
  - Contract: creation carries optional expected bookkeeping slug, format, and
    publication UTC. Each candidate is inserted inside the existing transaction;
    after a successful `INSERT`, compare the final slug/format/publication
    instant before child writes or commit. A mismatch rolls back and proves the
    first free candidate differed; a unique conflict before the expected slug
    retries, while conflict at the expected candidate fails. No expectations
    preserve current suffix retries.
  - Contract: update checks expected target ID, frozen/overridden final slug,
    format, publication instant, and current ETag after the ownership lock but
    before revision insertion or mutation. Mutable metadata and named audiences
    are fully parsed/authorized before either create or update persistence.
  - Verification: `#[apply(backends)]` tests prove first-free slug matching,
    rollback on earlier-free or occupied-expected candidates, create
    format/publication-UTC mismatch rollback, published slug freeze, no
    revision/body mutation on update mismatch, unchanged no-expectation retry
    behavior, explicit publication-clock persistence, and identical
    SQLite/PostgreSQL results.

- [x] Task 3: Adapt web create/update without duplicating Org policy
  - Depends on: Tasks 1-2.
  - Contract: `PostInputs` mapping preserves per-field collection presence and
    treats lifecycle as one source; transport defaults do not become header
    presence. Resolve `named:<id>` through the author-scoped `AudienceStorage`
    interface and pass only typed context to shared orchestration, which owns
    the request clock and content ETag comparison.
  - Contract: consume the transport-neutral content ETag projection from Task 1;
    map metadata failures to existing web Validation and stale sync to Conflict.
  - Verification: extend backend-parametrized web create/update integration
    suites for precedence, explicit empty collections, current omission/default
    behavior, audiences, bookkeeping, atomic rejection, and error kinds.

- [x] Task 4: Adapt AtomPub POST/PUT and preserve protocol behavior
  - Depends on: Tasks 1-2; may execute alongside Task 3.
  - Contract: `entry_to_post_fields` preserves actual Atom element/lifecycle
    presence and remains only a wire adapter. Inject `AudienceStorage` into
    `PostServices`; resolve named IDs author-scoped; preserve create default
    audience and update audience when both Atom and header omit it.
  - Contract: explicit Atom fields win, header bookkeeping validates against
    final values/current ETag, `If-Match` remains an independent precondition,
    metadata errors map to 400, and stale sync maps to 412.
  - Verification: extend existing AtomPub POST/PUT/ETag integration cases for
    full Org metadata, precedence, audience masking, collision-resolved slug,
    successful canonical native-source reads, 400/412 failures, and no mutation.

- [ ] Task 5: Prove client and browser compatibility; finish projections
  - Depends on: Tasks 3-4.
  - Contract: Emacs pull/publish output remains accepted by the stricter server;
    change Emacs serialization only where the approved grammar requires it. Keep
    `CONTEXT.md` unchanged unless implementation reveals new domain language.
  - Verification: add focused browser cases to `posts.spec.ts` and
    `atompub.spec.ts` that exercise Org-header precedence, canonical
    metadata-free readback, and atomic validation/stale-sync errors on
    create/update. Run focused Emacs publish/pull integration coverage,
    `devtool run -- cargo xtask e2e-local posts.spec.ts`, and
    `devtool run -- cargo xtask e2e-local atompub.spec.ts`, then the applicable
    branch gate through `jaunder-commit`.
  - Documentation: reconcile the already-authored ADR draft and
    `docs/ARCHITECTURE.md` projection with final symbol names; do not edit
    `docs/README.md` or promote/number the draft on this feature branch.

## Risk checks

- One parser/normalizer interface owns ordering; web and Atom adapters contain
  no Org grammar or canonicalization copies.
- All exported-symbol changes begin with LSP references and migrate every
  caller; obsolete Org helpers and duplicate ETag implementations are removed.
- The unique index and transaction, not a preflight existence query, arbitrate
  final create slugs on both backends.
- Every validation mismatch is checked before a commit or revision; malformed
  metadata cannot leave a Post or revision behind.
- Audience lookup is author-scoped and errors do not distinguish foreign from
  nonexistent IDs.
- One injected clock governs lifecycle comparison and persisted timing; DST fold
  and gap behavior is deterministic.
- Existing web omission/default, Atom create-default/update-preserve audience,
  published-slug freeze, ETag projection, and non-Org behavior remain covered.
- Each task certifies with `devtool run -- cargo xtask precommit` before commit;
  no lint suppression lands without explicit approval and no commit carries a
  `Co-Authored-By` trailer.
