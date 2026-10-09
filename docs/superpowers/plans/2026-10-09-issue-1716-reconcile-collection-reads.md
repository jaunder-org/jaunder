# Issue #1716: Reconciliation Collection-read implementation outline

> Execute with jaunder-iterate; use jaunder-dispatch only for delegated tasks.
> Trigger: concurrency-sensitive evidence lifetime and pre-install ordering.
> Contract:
> [approved spec](../specs/2026-10-09-issue-1716-reconcile-collection-reads.md).

## Scope

In: confirmed pull and keep-remote batches, including pull-time Local Post Link
proof, request-count regressions, and the approved safety/documentation updates.

Out: server/storage/protocol changes, cross-batch caching, concurrent execution,
push, keep-local, interactive merge, standalone pull, and publish-time links.

## Task outline

- [x] Task 1: Batch matched pull and keep-remote without repeated remote walks.
  - Contract: the row-operation portion of `jaunder--reconcile-execute-batch`
    owns a root/blog-bound context only for pull and keep-remote. Distinguish
    unacquired, complete Members (including an empty Collection), and failed
    acquisition; failure retains actionable evidence rather than nil/defaults.
    Acquire lazily through the existing validated Collection enumerator once;
    preserve page progress and errors. End the binding before final refresh.
  - Contract: `jaunder--reconcile-pull-unique-match` joins freshly scanned local
    files with that context's remote Members. Outside an applicable context,
    retain fresh complete inventory behavior for keep-local and merge callers.
    Never refresh reviewed row ETags from the new Collection.
  - Contract: preserve both keep-remote preflights and targeted Member checks.
    At `jaunder--reconcile-pull-install-staged`, perform a final local identity
    and uniqueness scan after Media finalization, then repeat digest/buffer/
    destination preflight before replacement. This local-only check must not
    trigger another Collection fetch. Retain structured blocked/partial results
    and the existing checkpoint, clean-buffer, and recoverable rename behavior.
  - Verification: confirmed three-row matched pull and keep-remote fixtures
    count four operation page GETs plus four separate refresh page GETs. Inject
    remote deletion/ETag/identity changes and local ID/path/digest/buffer/
    destination changes between rows. Inject a new duplicate local ID during
    Media finalization and after the prior Member check; both operations must
    preserve the reviewed Post. Retain Media and rename recovery regressions.
- [x] Task 2: Share remote proof through pull-time link staging.
  - Contract: reconciliation staging supplies `jaunder--pull-stage-member` with
    an inventory joining Task 1's remote Members to current local files when
    link proof is needed. Do not freeze the report's local evidence or change
    standalone staging's fallback behavior. Server-only rows and matched rows
    use the same batch remote source; link work cannot add per-row walks.
  - Verification: server-only and mixed batches containing Org HTTP(S) links
    retain singleton ID/href and current local target proof. Absent, ambiguous,
    changed, or invalid targets retain canonical URLs. Include a target created
    or renamed by an earlier row and later target-local identity changes.
- [ ] Task 3: Prove lifetime/failure boundaries and document the contract.
  - Contract: final refresh and later invocations acquire new evidence; buffers,
    roots and blogs cannot share it. Failed or partial operation acquisition
    blocks every dependent row without per-row retry or stale-preview fallback.
    Keep cancellation and failed-refresh terminal results reviewable.
  - Verification: duplicate remote Entries, cycles/bad pages, failed
    acquisition, cancellation, subsequent successful retry, root/blog isolation,
    independent row failure, and failed final refresh. Count operation requests
    separately from refresh attempts even when either fails.
  - Verification: run focused regressions, complete pure ERT, and applicable
    live ERT pull/conflict proofs. Update `elisp/README.md` and reconcile the
    decision draft/architecture projection with delivered behavior. Keep run
    evidence in ignored/session storage; commit only maintained docs/tests.

## Risk checks

- Inspect every shared helper caller: optimized dynamic scope must not leak into
  keep-local, merge/Ediff callbacks, publish, standalone pull, or final refresh.
- Local uniqueness scans precede the final path/digest/buffer/destination guard;
  slow Media work cannot separate the last scan from replacement. There is no
  claim of eliminating the filesystem race after the final check.
- Collection Entries authorize neither a newer reviewed ETag nor installation;
  selected-Member checks and Media integrity remain authoritative.
