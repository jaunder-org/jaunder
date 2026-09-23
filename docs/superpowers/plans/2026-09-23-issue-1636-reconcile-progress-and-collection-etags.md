# Reconciliation Progress and Collection ETags — Implementation Outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for a bounded task
> if useful. This outline exists because #1636 changes a public AtomPub wire
> contract.

## Scope

In: publicly discoverable per-Member validators in Collection Entries; Emacs
reconciliation preview fast path and fallback; synchronous progress for opening
and refreshing; protocol and client proofs.

Out: stored ETags, a new API, asynchronous reconciliation, Syndication Feeds,
changes to Member write/conditional semantics, and #1641's compression work.

## Task outline

- [x] Publish the Collection validator contract
  - Contract: each Entry of each paginated Posts Collection page has one direct
    `{https://jaunder.org/ns/atompub}etag` child containing exactly the same
    strong quoted validator as its Member `GET`, without attributes or padded
    text; incoming values are ignored. Service Document version 1 advertises
    `member-etag`. Compute from the existing content-and-audience ETag function,
    not a second formula. Preserve upstream Atom serialization and other
    clients' parsing. Author the proposed ADR draft and its
    `docs/ARCHITECTURE.md` projection with this slice.
  - Verification:
    `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/atompub/)'`
    for host integration tests of Collection/Member parity on multiple pages and
    an audience-only update, service discovery, and an ignored incoming
    extension; host Atom serialization tests check namespace identity and
    well-formed output.
- [ ] Parse optional validators without trusting malformed wire data
  - Contract: an inventory Member holds the optional validator only for exactly
    one direct namespace-qualified `etag` element with bare, exact strong-ETag
    text and no attributes or nested content. Absent, duplicate, weak, padded,
    attributed, nested, foreign-namespace, or otherwise malformed values do not
    enter the validator field; preserve other inventory join behavior.
  - Verification:
    `devtool run -- emacs --batch -Q -L elisp -l elisp/test/jaunder-reconcile-test.el --eval '(ert-run-tests-batch-and-exit "jaunder-reconcile-")'`
    for valid and invalid wire fixtures and paginated inventory (narrower test
    selector if a single new test answers the current question).
- [ ] Classify matched Posts from the fast path or existing fallback
  - Contract: a validated inventory Member ETag replaces the preview's
    matched-Post `GET`; an absent validator invokes the existing Member
    request/classifier and retains row-local transport/HTTP failure evidence.
    Preserve all operation-time remote checks, conditional writes, local
    preflights, explicit selection, and conflict/blocked behavior.
  - Verification: focused Emacs ERT as above for same classification matrix with
    and without extensions, per-page rather than per-match request count on
    multiple Posts, and fallback failure rows; live Emacs integration test
    proves the paginated server-to-client path without per-Member preview reads.
- [ ] Make initial reconciliation and refresh visibly synchronous
  - Contract: expose an in-progress status before blocking network work for
    initial `jaunder-reconcile` and `g`, finish truthfully on success or error,
    retain the previous report and selection on refresh failure, and keep
    batch-action progress separate.
  - Verification: focused Emacs ERT as above for initial/refresh start, success,
    failure and report preservation; `devtool run -- devtool check ert` for the
    complete pure suite.
- [ ] Prove the public endpoint and cross-surface result
  - Contract: the extension is usable by non-Emacs AtomPub consumers;
    documentation names `j:etag`, `member-etag`, fallback, and the non-atomic
    nature of preview. Preserve the Collection application endpoint's
    integration and end-to-end coverage obligations.
  - Verification: `devtool run -- cargo xtask e2e-local atompub.spec.ts` checks
    a running app's Collection validators against Member headers across
    pagination; rerun focused server/Emacs tests after fixes, then the
    pre-commit gate via `jaunder-commit` and `devtool run -- cargo xtask check`
    for broad diagnostics. Record multi-Post HTTP request-count evidence. No
    lint suppressions without explicit approval.

## Risk checks

- Per-Entry ETag refers to the mutable Member representation, never a page ETag
  or a cached permission to mutate; compare with current Member and
  conditional-write ETag including audience targets.
- Multiple pages retain every Post once, and missing/invalid metadata degrades
  to a request rather than an assumed unchanged state. Atom namespace URI plus
  local name, not the textual prefix, identifies the extension.
- The progress message must be visible before blocking I/O in Emacs; failure
  must preserve the existing report. Batch operations must still revalidate
  after preview.
- The ADR draft remains proposed on the feature branch; cite it by draft path in
  the architecture view. Do not edit the generated ADR index.
