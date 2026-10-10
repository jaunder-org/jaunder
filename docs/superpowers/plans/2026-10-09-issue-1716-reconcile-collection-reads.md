# Issue #1716: Operation-owned reconciliation implementation outline

> Execute with jaunder-iterate; use jaunder-dispatch for bounded delegated
> tasks. Trigger: shared evidence ownership, write-side invalidation and
> concurrency. Contract:
> [approved expanded spec](../specs/2026-10-09-issue-1716-reconcile-collection-reads.md).

## Scope

In: one shared operation module for push, pull, keep-local, keep-remote and
delete; nested Post-link resolution; short-lived merge preparation/completion;
public-command request/safety proofs and maintained documentation.

Out: server/storage/protocol changes, cross-operation caches, parallel rows,
standalone command semantic changes, and the UI work in #1718/#1721. The shared
fix's PR must include `Closes #1716` and `Closes #1722`; do not close either
issue before merge.

## Interface contracts

`elisp/jaunder-reconcile-operation.el` owns the operation's private state. Its
callers supply scalar scope and existing inventory/proof data, not a context or
heterogeneous dependency holder. Use the existing config, inventory, Atom and
HTTP seams; keep UI rows, rendering, Ediff and action-specific Media/install
work outside this module. The operation does not replace conditional-write
authority.

- Lifecycle: run work for an explicit root and captured active origin/User in an
  independent dynamic scope. Report refresh runs after this scope ends. A nested
  confirmed command starts a new scope; ordinary publish invoked by a row uses
  the row's existing scope rather than opening another operation per Post.
- Inventory/matching: acquire complete remote discovery lazily once; join fresh
  local files and judge a requested identity/path's unique match.
  Complete-empty, partial/failure diagnostics and root/blog mismatch are
  distinct. Revalidation does not overwrite the caller's reviewed ETag or source
  snapshot.
- Link proof: resolve required Member/alternate evidence inside the operation,
  sharing discovery with matching. Revalidate invalidated identities with exact
  targeted Member GETs and namespace-aware parsing; retain original failures.
  Publish supplies its known target IDs; inverse pull also needs the current
  canonical-href mapping. Unknown/invalid candidates do not become invented
  Members or URLs. Callers retain direction-specific target validation/failure.
- Write observation: mark a possible create/update/delete at send time, classify
  its response or lost outcome, and update/invalidate affected proof before
  fallible local completion. Recognize confirmed deletion as absent, not a
  reusable Member. A new ID is learned only from trustworthy returned identity
  or existing durable recovery, not inference. Authoritative response metadata
  may replace affected discovery; otherwise defer a targeted read until needed.
  A rejected conditional request is not a commit; an uncertain transport is not
  unchanged state. Scope retains enough write-phase evidence to classify a later
  local failure as partial or a lost update/delete response as unknown.

## Task outline

- [x] Task 1: Consolidate read-only operation evidence and pull/conflict
      consumers.
  - Produce the module's lifecycle, inventory/matching and link-proof interface;
    migrate/remove the pull-specific batch cache/provider helpers rather than
    layering a second state owner. Bind it in the row-processing portion of
    `jaunder--reconcile-execute-batch`; release it before final report refresh.
  - Route matched pull and both keep-remote conflict preflights through it.
    Retain targeted Member reads, final local uniqueness after Media, then the
    digest/buffer/destination guard and recoverable replacement/rename. Pull
    staging uses current locals with shared remote proof; standalone staging
    keeps its established supplied-inventory/fallback contract.
  - Verification: public pull/keep-remote one-row and three-row paginated
    budgets; server-only/mixed links; earlier local creation/canonical rename
    and target ambiguity; remote/local/Media drift, late duplicate identities
    and delayed checkpoint convergence. Prove empty/failed/partial discovery,
    cancellation, same-report retry, root/blog isolation and separate failed
    final refresh.
- [x] Task 2: Integrate writing actions and invalidate at the remote-write seam.
  - Implement the write-observation contract around the existing conditional
    send paths and durable create orchestration. In `jaunder-publish`, observe
    accepted create/update responses before `jaunder--write-back` or rename. In
    keep-local, observe before its post-commit drift/checkpoint handling; in
    delete, observe before local removal. Transport errors retain their source
    while operation-owned phase evidence informs terminal unknown/partial
    results.
  - Route keep-local's two conflict preflights and publish-time Local Post Link
    discovery through the same owner. Push without links and delete retain no
    discovery walk; delete retains its pre-confirmation reviewed validators. Own
    writes must never silently refresh a selected row's reviewed precondition.
  - Verification: public push/keep-local/delete one-row/multi-row budgets and
    literal conditional validators; create replay and stale-precondition
    rejection; earlier-row create/update/delete/rename with later Post links;
    failed local completion after confirmed writes, lost PUT/DELETE responses
    and unknown create identity. Required invalidated proof is refreshed
    targetedly or blocks safely, never via a full walk, guessed permalink,
    rollback or unsafe retry.
- [x] Task 3: Bound merge scopes and prove cross-action lifetime/outcomes.
  - Wrap `jaunder--reconcile-merge-stage` preparation and authorized finish work
    in separate operation scopes. Preserve initial/final conflict checks and
    existing `jaunder--reconcile-merge-record` terminal refresh outside the
    scope. Never store operation evidence in `jaunder-reconcile-merge-session`
    or bind it across Ediff's human editing interval; do not add a no-mutation
    refresh.
  - Verification: change remote/local state while editing, then finish; stale
    proof blocks while scratch remains recoverable. Exercise partial/unknown
    finish and later retry, nested confirmed commands and different roots/blogs,
    including a nested same-blog write invalidating its caller's affected proof
    without lending the nested operation's cached evidence to that caller.
    Independent rows/results survive dependent failures and final refresh
    errors.
- [x] Task 4: Finish observable regression coverage and project delivered
      contracts.
  - Remove obsolete private-cache tests where operation/command tests replace
    them; retain reusable minimized HTTP/filesystem fixtures rather than timing
    ledgers tied to deleted implementation shapes. Verify all five actions' page
    budgets with initial report reads separate and differing final page counts.
  - Verification: focused red/green, complete pure ERT and applicable live ERT
    publish/pull/conflict proofs; compare request counts and bounded timing
    without equating synthetic execution to production latency. Update
    `elisp/README.md`, the operation ADR draft and architecture projection to
    delivered behavior; captures and temporary probes remain ignored/session
    evidence.

## Risk checks

- Do not mistake a missing/invalid target for an infrastructure failure:
  preserve unexpected I/O/decode sources and the action's honest
  failure/degradation policy.
- Invalidating at batch-result time is too late: local completion can throw
  after remote commitment. Instrument the send/response seam, including safe
  create replay, rather than assuming a failed publish means no remote change.
- Fresh local checks remain necessary after earlier rows and long Media work.
  Preserve each action's source authority and client-managed metadata writes; do
  not make ordinary push's own metadata look like a forbidden user edit.
- One operation walk is not an atomic Collection snapshot. Repeated global fault
  detection is reduced as approved, never substituted for selected-Member or
  conditional-write checks. Unknown writes never authorize automatic retry.
