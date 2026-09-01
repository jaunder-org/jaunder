# Publication-State Persistence Projections Implementation Outline

> Execute with `jaunder-iterate`; delegate with `jaunder-dispatch` when useful.
> This outline exists because the change adds public domain and storage
> interfaces across four transport callsites.

## Scope

In:

- Pure present-state timestamp projection owned by
  `common::org::PublicationState`.
- Pure present-state update-command conversion owned beside
  `storage::PublishUpdate`.
- Clean cutover of web Org and AtomPub create/update present branches.
- Owner-level table tests and focused retained behavior verification.

Out:

- Transport absence, AtomPub compatibility/classification, and web non-Org
  behavior.
- SQL, dialect, schema, migration, dependency, ADR, protocol, and wire-format
  changes.

## Task outline

- [x] Task 1: Add owner-level exhaustive projections
  - Contract: `PublicationState::published_at(self) -> Option<UtcInstant>` and
    `From<PublicationState> for PublishUpdate` use exhaustive three-variant
    matches; `PublishUpdate::Publish { at: None }` remains unreachable from a
    present state.
  - Verification: table tests cover Draft, Scheduled, and Published with exact
    instants in both owner modules.
- [x] Task 2: Cut over all four present-state callers
  - Contract: web Org create calls `published_at`; web Org update converts into
    `PublishUpdate`; AtomPub create/update delegate only
    `Presence::Present(state)`. Every `Presence::Absent` and non-Org branch
    stays behaviorally identical.
  - Verification: focused common/storage unit tests plus retained web Org,
    AtomPub, and dual-backend publication contracts pass.
- [x] Task 3: Reconcile documentation and review the clean cutover
  - Contract: update architecture documentation only if the new public
    interfaces materially change its current ownership description; remove every
    obsolete duplicated present-state match.
  - Verification: callsite search finds no transport-owned present-state
    persistence match, applicable repository checks pass, and Standards/Spec
    reviews pass.

## Risk checks

- `Presence::Absent` must never flow through either new projection.
- AtomPub create absence still uses `is_draft` or `request_clock`; update
  absence still chooses `Unpublish` or `Publish { at: None }`.
- Web non-Org missing/false/true publication controls remain unchanged.
- `classify_published` remains transport-owned and untouched.
- `PublishUpdate::Publish { at: None }` retains existing storage
  stamping/retention semantics on SQLite and PostgreSQL.
- Storage may depend on `common`; `common` must not acquire an upward
  dependency.
- All exported-interface callsites are migrated in one clean cutover; no
  compatibility alias or duplicate helper remains.
