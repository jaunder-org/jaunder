# Issue #1717: Pulled Posts converge in reconciliation

## Outcome

After a successful pull, an unedited local Post classifies as `unchanged`
against the accepted remote Member. Slow Media acquisition or Collection
verification must not manufacture a local edit.

## Load-bearing decisions

- Pull synchronization describes the installed local representation, not the
  earlier start of remote staging. Its saved timestamp and filesystem timestamp
  must agree sufficiently for existing reconciliation rules to recognize it.
- Keep the accepted strong Member ETag and Post identity authoritative. A later
  remote change still classifies as `server-ahead`; a later local edit still
  classifies as `local-ahead`; changes on both sides still classify as
  `conflict`.
- Preserve the existing two-second filesystem timestamp tolerance. Do not widen
  it to hide slow operations, disable local-change detection, or automatically
  publish a fetched Post to make it appear synchronized.
- Cover successful server-only pull, matched server-ahead replacement, and
  keep-remote conflict resolution wherever they share the installation contract.
- Preserve ADR-0024's local/served representation mapping, ADR-0200's reviewed
  digest, ETag, clean-buffer and destination checks, and ADR-0211's explicit
  conflict choices and honest partial outcomes.
- Installation and rename remain recoverable rather than a claimed transaction
  across HTTP and the filesystem. A failed operation must not checkpoint an
  untouched local Post or discard authored changes.

## Acceptance

- A regression exercises actual Member staging, complete fresh Collection
  verification, local replacement, and fresh classification. Verification
  lasting longer than the timestamp tolerance currently fails with
  `local-ahead`; after the fix it succeeds with `unchanged` and the accepted
  ETag.
- Representative slow Media/staging and shared installation paths also converge,
  including server-only creation, keep-remote and canonical-slug rename.
- After successful pull, a genuine subsequent local edit outside the existing
  tolerance is detected. A subsequent remote ETag change is detected; both
  changes produce a conflict.
- Existing stale remote/local evidence, modified buffers, destination collision,
  Media integrity and recoverable rename behavior retain their safety proofs.
- Focused ERT proofs run first; applicable repository checks and PR CI verify
  integration. Reproduction logs stay in ignored `.xtask/` storage or the PR;
  only maintained regression tests and necessary current documentation land.

## Boundaries

- No change to Collection request counts or batch caching: that is issue #1716.
- No new content-digest synchronization model, protocol or storage changes.
- No automatic repair of historical Posts already reported `local-ahead`:
  classification alone cannot prove that they have no genuine authored edits.
