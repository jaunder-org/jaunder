# Idempotency key domain type implementation outline

> Execute with dev-cycle-iterate. This outline exists because a new shared
> domain type changes storage/service and AtomPub contracts in dependency order.

## Scope

In:

- Shared `IdempotencyKey` type and standard trailer.
- Typed post-service/storage contracts and existing-schema SQLx use.
- Compatible AtomPub header parsing and replay behavior.
- Focused type, dual-backend storage, and AtomPub boundary coverage.
- ADR-0063 and current architecture projection updates.

Out:

- Schema changes, retention, payload fingerprints, new response statuses, client
  format changes, and edits to frozen #697 artifacts.

## Task outline

- [x] Task 1: Establish the shared idempotency-key contract
  - Contract: `common::idempotency_key::IdempotencyKey` is a non-secret
    SQLx-enabled string newtype; `FromStr` trims, rejects empty, and otherwise
    preserves the trimmed string. Its named error and standard trailer follow
    existing ADR-0063 types.
  - Verification: focused `common` tests prove canonicalization,
    arbitrary-string acceptance, empty rejection, serde/owned-borrowed behavior,
    and the repository's standard trailer requirements.

- [x] Task 2: Carry typed keys from AtomPub through persistence
  - Depends on: Task 1.
  - Contract: the AtomPub handler preserves `HeaderValue::to_str` compatibility,
    maps missing/whitespace-only/unreadable values to `None`, and parses
    readable non-empty text once into owned `IdempotencyKey`. Borrowed
    service/orchestration and lookup seams use `Option<&IdempotencyKey>` /
    `&IdempotencyKey`; lifetime-free content/input structs own
    `Option<IdempotencyKey>`; SQL lookup and insert bind the type directly.
    Every caller migrates in this task, with no primitive compatibility overload
    or migration.
  - Verification: dual-backend storage/service tests prove typed SQLx behavior
    and atomic rollback on collision. Dual-backend router tests prove valid
    first/reused keys, different content on reuse, per-user scope,
    whitespace-only, non-ASCII UTF-8 bytes, invalid UTF-8 bytes, and no-key
    behavior through real HTTP requests.

- [ ] Task 3: Record the completed domain contract
  - Depends on: Tasks 1–2.
  - Contract: ADR-0063 and `docs/ARCHITECTURE.md` describe the type,
    compatibility boundary, per-user persistence, and replay semantics; frozen
    #697 artifacts remain unchanged.
  - Verification: documentation/gate checks resolve all live paths and
    references without modifying archived history.

## Risk checks

- Every idempotency-specific exported symbol migration includes all callsites
  before its task is committed; no raw-string shim or deprecated path remains.
- AtomPub unreadable-header cases are constructed as raw `HeaderValue` bytes so
  tests exercise `to_str` rejection rather than only the domain parser.
- SQLx decode validates stored rows; both existing database dialects remain
  schema-identical.
- Collision tests distinguish the original committed post from every row
  belonging to the rolled-back attempt.
- Slug collision retries remain separate from idempotency conflicts and retain
  their current retry behavior.
- Integrated verification runs the repository's changed-contract checks and
  normal commit gate after all tasks land.
