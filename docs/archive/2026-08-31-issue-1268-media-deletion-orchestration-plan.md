# Media Deletion Evidence Orchestration Implementation Outline

> Execute with `jaunder-iterate`; delegate with `jaunder-dispatch` when useful.
> This outline exists because the change deepens a storage interface, changes
> composition-root DI, and preserves a lock-ordering security invariant across
> two public transports.

## Scope

In:

- Deepen `MediaManager` to own deletion evidence construction and reporting
  derivation.
- Root-inject one manager into AtomPub and web.
- Cleanly migrate all four existing upload/delete manager consumers.
- Manager-seam ordering/classification tests and retained endpoint/backend
  verification.

Out:

- Deletion/retention policy, storage dialect, schema, migration, response, force
  UX, and #755 changes.
- New resolver adapters, service locators, or dependency bundles.

## Task outline

- [x] Task 1: Deepen the manager deletion interface
  - Contract: `MediaManager` owns media/posts/site configuration/write
    scope/content locks/resolver/current instance;
    `delete_media(user_id, &MediaRef, force)` returns a named result with the
    mutation outcome and private immutable snapshot/evidence.
  - Contract: result behavior derives deterministic deduplicated owner Post IDs
    while excluding references proven foreign.
  - Verification: manager-seam tests prove one snapshot, exact ownership
    classification, same evidence, and resolver completion before lock
    acquisition.
- [x] Task 2: Inject one manager and cut over all consumers
  - Contract: the server composition root constructs one `Arc<MediaManager>`
    from explicit dependencies and injects it independently into Axum and
    Leptos; AtomPub/web upload and delete callers no longer construct managers.
  - Verification: AtomPub force/status/authentication and web
    optional-force/reporting behavior pass focused tests; upload contracts
    remain green.
- [x] Task 3: Reconcile architecture and verify retained storage behavior
  - Contract: architecture documentation describes the real root-injected
    manager seam; duplicate evidence orchestration and ordering comments are
    deleted from both handlers.
  - Verification: dual-backend guarded deletion, concurrency, retained-history,
    file/quota reclamation, and shared-path suites pass; no duplicated handler
    orchestration remains; Standards/Spec reviews pass.

## Risk checks

- Exactly one global bounded reference snapshot is used for ownership
  resolution, guarded deletion, and web reporting.
- Ownership resolution completes before the manager acquires the content lock or
  enters its write scope.
- The same current `InstanceId` and immutable `MediaReferenceEvidence` reach
  guarded deletion and reclaimability.
- Unknown, failed, malformed, or timed-out ownership remains protected.
- AtomPub remains unconditional-force with current 404/409/204 behavior pending
  #755.
- Web refusal reporting remains owner-scoped, excludes proven-foreign rows, and
  uses the pre-lock snapshot.
- Upload callers use the injected manager without behavioral change.
- `AppState` remains storage-only; the manager is independently injected per
  ADR-0016.
- No compatibility constructor, alias, duplicate local manager construction, or
  obsolete ordering comment remains.
