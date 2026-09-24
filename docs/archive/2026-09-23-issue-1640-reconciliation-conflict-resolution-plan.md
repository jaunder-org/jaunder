# Issue #1640 — Reconciliation conflict resolution implementation outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for an isolated
> implementation slice. The outline exists because an interactive Ediff session,
> conditional remote write, and local atomic replacement have non-atomic failure
> boundaries that must remain independently reviewable.

Authoritative contract:
[approved spec](2026-09-23-issue-1640-reconciliation-conflict-resolution-spec.md).

## Scope

In: true uniquely matched Emacs reconciliation conflicts, three explicitly
confirmed choices, strong reviewed evidence, honest per-Post results, and pure
plus live ERT proofs. Out: new server protocol/storage, three-way ancestry,
non-Org publishing, automatic conflict decisions, rollback across
HTTP/filesystem or Media, and changes to ordinary non-conflict operations.

## Task outline

- [x] 1. Resolve only reviewed, uniquely matched conflict selections into
      actionable blocked outcomes, before any Post mutation.
  - Contract: share a read-only conflict evidence/preflight seam carrying the
    selected row's path, byte SHA-256, canonical Post ID, Member edit identity,
    and strong ETag; distinguish modified visiting buffers, local drift,
    duplicate identity, missing Member, malformed/stale ETag, and Member
    transport errors. Final guards must be callable again after staging/Ediff;
    do not turn a fresh Collection ETag into mutation authority. Reuse report
    selection and terminal-result conventions, not a second state model.
  - Verification: pure ERT state × action and local/remote preflight matrix;
    assert no write occurs on any blocked path.

- [x] 2. Accept a conflict's remote Post through the existing staged matched
      pull without loosening other states' eligibility.
  - Contract: after confirmation stage Member and Media, require staged and
    final Member identity/ETag to match the reviewed row, and reuse ADR-0200's
    local digest/identity/buffer/destination checks and atomic replacement plus
    recoverable rename. Include conflicts in ordered keep-remote batches with
    explicit blocked/partial outcomes and refreshed report.
  - Verification: pure ERT for final drift, visited buffers, destination
    collision, rename-after-replace partial commit and batch isolation; live
    Emacs coverage for successful replacement and concurrent remote edit.

- [x] 3. Accept reviewed local authored content through a conditional PUT with
      no pre-PUT local Post metadata write.
  - Contract: the publish seam accepts an explicit reviewed strong `If-Match`
    for this operation without changing ordinary publish/create recovery.
    Validate and prepare without `JAUNDER_SYNCED` or timezone write-back before
    the PUT; recheck local and Member evidence after any preparatory Media side
    effects. Report rejected 412 as blocked, lost response as unknown remote
    outcome with local Post unchanged, and confirmed remote success followed by
    failed local checkpoint/rename as partial success with recovery guidance.
    Preserve the ordinary successful write-back/rename path and ordered batch
    results.
  - Verification: pure ERT request/header and write-order tests, including
    deliberate response loss and post-commit local failure; live Emacs proof of
    successful local choice and concurrent remote edit (no unconditional PUT).

- [x] 4. Merge one conflict through two-way Ediff into a client-managed scratch
      result, then finalize only with explicit User action.
  - Contract: stage actual remote Member and Media before opening Ediff; keep
    the local Post, remote Post, reviewed evidence, and report-buffer identity
    separate from editable authored result. Never treat Ediff exit as approval;
    finalization rechecks the same evidence and uses Task 3's conditional send
    before local installation. Preserve edited scratch on cancellation, failed
    completion, unknown remote outcome, or partial commit; require an explicit
    discard choice. Modified visiting buffers and changed local/remote bytes
    while Ediff is open block finalization.
  - Verification: pure ERT with a controlled Ediff boundary for selection,
    authored-field editability and metadata ownership; initial staging failure
    creates no result buffer. Once edited scratch exists, prove it survives
    cancellation, final revalidation block, lost PUT response, and post-commit
    local failure separately. Cover post-launch drift, final conditional header,
    and write order.

- [x] 5. Prove and explain the completed conflict-resolution surface.
  - Contract: keep existing ordered Last batch results across fresh inventory on
    every successful direction; show blocked/unknown/partial recovery without
    claiming two-sided preservation. Update `elisp/README.md` with conflict
    meaning, choices, and Media/partial/unknown outcomes; move the proposed
    architecture paragraph from **Committed direction** into present-tense
    current state alongside its tracked draft ADR. Keep the spec and proposed
    ADR consistent with what ships.
  - Verification: pure ERT injects a failed fresh-inventory refresh after a
    partial or unknown outcome and proves its ordered terminal summary remains
    visible and reviewable. Run full pure ERT and live Emacs integration under
    its self-booting harness; focused `devtool run -- devtool check ert`,
    pre-commit and pre-push hooks, then CI's authoritative hermetic Elisp
    coverage. No browser e2e unless a changed browser-visible path requires it.

## Risk checks

- Never write `JAUNDER_SYNCED`, `JAUNDER_DATE_TZ`, or other local Post metadata
  before an attempted conflict PUT; if ordinary publish does, isolate conflict
  preparation without weakening its normal ID-first recovery behavior.
- `If-Match` uses a fresh strong ETag that equals the reviewed one, not a newer
  ETag automatically promoted from a GET, a Collection value, or Ediff staging.
- PUT response loss and post-commit local failure cannot be flattened into
  `operation-failed` or a claim of unchanged Posts. Carry committed/unknown
  outcomes through batch summary and subsequent refresh, even if refresh fails.
- Keep-remote may install reusable Local Media Copies before a later block; a
  rename failure after atomic replacement leaves the recognizable old-path Post
  allowed by ADR-0200. No overwrite of another destination or modified visiting
  buffer.
- Keep the Ediff session's owned buffers and report reference alive across
  asynchronous interaction; never publish on a `quit` hook, forget edited
  scratch on error, or allow a second row to inherit the first row's evidence.
- For each slice, stage the final intended tree and commit through
  `jaunder-commit` (enforced `cargo xtask precommit`); never bypass coverage or
  lint without the explicit approval required by `CONTRIBUTING.md`.
