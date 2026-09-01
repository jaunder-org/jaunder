# Media Deletion Evidence Orchestration

Issue: #1268 Status: Approved

## Problem

AtomPub and web independently assemble the same security- and race-sensitive
media deletion evidence: site identity, one global reference snapshot, resolved
ownership, and the immutable proof passed into guarded deletion. Drift can
weaken retained-reference protection or move ownership resolution inside the
storage lock.

## Design

- Deepen `storage::MediaManager` rather than add a second deletion service.
- Add `PostStorage`, `MediaReferenceOwnershipResolver`, and `InstanceId` to its
  constructor dependencies alongside the existing media, site configuration,
  write scope, and content locks.
- Construct one `Arc<MediaManager>` at the server composition root and inject it
  independently into Axum extensions and Leptos contexts.
- Migrate all four existing production manager consumers—AtomPub upload/delete
  and web upload/delete—to the injected manager. Upload behavior remains
  unchanged.
- Change deletion to accept only endpoint policy inputs: user, exact `MediaRef`,
  and force.
- Inside `MediaManager::delete_media`, in order:
  1. load the current `SiteIdentity`;
  2. load exactly one bounded global `MediaReferenceSnapshot` for the exact
     media address;
  3. resolve ownership with the injected resolver, current `InstanceId`, and
     identity base URL;
  4. only after resolution completes, acquire the existing content/storage lock
     and perform guarded deletion and reclaimability using the same instance and
     evidence.
- Return a named `MediaDeletionResult`. Keep its snapshot and evidence private;
  expose the mutation outcome and a method deriving the authenticated owner's
  referenced Post IDs while excluding rows proven foreign.

## Preserved behavior

- AtomPub authentication and username match, fixed `MediaSource::Upload`,
  unconditional `force = true`, and current 404/409/204 mapping pending #755.
- Web optional-force default, request/response DTOs, exact `referenced_in_posts`
  reporting, resource invalidation, and force UX.
- Fail-closed ownership classification and bounded global snapshot behavior.
- SQLite and PostgreSQL guarded-deletion, concurrency, retained-reference,
  forced override, file/quota reclamation, retained-history, and shared-path
  behavior.
- Existing upload behavior on both transports.

## Verification

- Manager-seam tests prove one snapshot, ownership classification, use of the
  same evidence, and resolution completion before content/storage lock
  acquisition.
- Result tests prove owner Post reporting excludes rows proven foreign and
  remains deterministic/deduplicated.
- Preserve focused AtomPub authentication/force/not-found/status contracts and
  web optional-force/reporting contracts.
- Preserve existing dual-backend guarded-deletion, concurrency, and
  `MediaManager` reclamation suites.
- Run the repository commit and PR gates through the normal Jaunder lifecycle.

## Exclusions

- No schema, migration, storage dialect, retention/deletion policy, public
  response, AtomPub override, force UX, protocol, or wire-format change.
- No service locator, heterogeneous dependency bundle, or new resolver adapter.
- No implementation of #755.
