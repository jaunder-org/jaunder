# Publication-State Persistence Projections

Issue: #1248 Status: Approved

## Problem

Web and AtomPub independently project present `PublicationState` values into
persistence inputs across create and update paths. Drift can change whether a
Post is persisted as draft, scheduled, published, or backdated.

## Design

- Add `PublicationState::published_at(self) -> Option<UtcInstant>` in
  `common::org`:
  - `Draft` → `None`
  - `Scheduled(at)` → `Some(at)`
  - `Published(at)` → `Some(at)`
- Add `From<PublicationState> for storage::PublishUpdate` beside
  `PublishUpdate`:
  - `Draft` → `Unpublish`
  - `Scheduled(at)` → `Publish { at: Some(at) }`
  - `Published(at)` → `Publish { at: Some(at) }`
- Use exhaustive matches so a future `PublicationState` variant requires an
  explicit projection decision.
- Migrate only present-state projections in web Org create/update and AtomPub
  create/update.

## Preserved behavior

- Every `Presence::Absent` branch.
- AtomPub `is_draft`, request-clock create fallback, and `Publish { at: None }`
  update fallback.
- Web non-Org lifecycle handling.
- AtomPub `classify_published` and transport parsing.
- Existing `PublishUpdate::Publish { at: None }` storage retention/stamping
  semantics.
- SQL, dialect, schema, migration, protocol, wire-format, and dependency
  behavior.

## Verification

- Table-test all three `PublicationState::published_at` projections in `common`.
- Table-test all three `PublicationState` to `PublishUpdate` conversions in
  `storage`.
- Preserve focused web Org create/update, AtomPub create/update, and
  dual-backend storage publication contracts.
- Run the repository commit gate through the normal `jaunder-commit` workflow.

## Exclusions

- No transport absence unification.
- No non-Org web changes.
- No AtomPub compatibility or classification changes.
- No SQL, schema, migration, dependency, ADR, or protocol changes.
