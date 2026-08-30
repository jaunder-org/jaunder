# Allowlist-free SQLx Decode Approval Implementation Outline

> Execute with dev-cycle-iterate. This outline exists because storage decode
> correctness, cross-backend representation parity, and the gate cutover have
> non-obvious ordering and fail-closed invariants.

## Scope

In:

- Migrate all 53 current decode exemptions to declaration-backed types.
- Preserve SQLite/PostgreSQL semantic parity and corruption handling.
- Remove the decode allowlist and update its conformance documentation.

Out:

- Schema migrations, wire/API changes, scanner-population changes, SQL/receiver
  heuristics, and changes to `sqlx-newtype-bind`.
- Public domain types for storage metadata, corruption states, or test fixtures.

## Task outline

- [x] Task 1: Make shared count and existence decodes semantic and cross-backend
  - Contract: private storage-wide `RowCount` enforces a nonnegative `i64` and
    exposes an explicit lossless conversion for public counts; private
    storage-wide `Exists` wraps `bool`. Portable `EXISTS` queries decode
    `Exists` on both backends, and complete SQL/helpers are shared only when
    their semantics and full statement shape match.
  - Verification: focused type-boundary tests cover valid/negative counts;
    SQLite and PostgreSQL focused storage tests cover media filename existence,
    tag existence, subscriptions, and feed-event counts without integer-to-bool
    or negative-to-zero fallback behavior.

- [x] Task 2: Give catalog, backup, config, opaque, and schema-test values
      explicit roles
  - Contract: each catalog metadata role and each intentionally lossless stored
    value receives a private concern-owned bridge type; closed tokens and
    structured payloads validate during decode, while unknown config keys,
    arbitrary config/diagnostic text, catalog definitions, and stored session
    labels remain lossless through their existing export, deletion, parse, or
    repair boundary. This task owns each backend's complete `database_is_empty`
    migration after Task 1 supplies `Exists`. No generic representation wrapper
    is introduced.
  - Verification: focused dual-backend backup/schema tests cover metadata,
    export, checksum, migration-version, and database-emptiness paths; focused
    config and feed-cache tests prove unknown-key export and deletion,
    opaque-value fidelity, and structured-payload rejection.

- [ ] Task 3: Type production row state and isolate custom row policy
  - Contract: feed attempts use a nonnegative checked type; email verification
    and operator status use distinct bool wrappers; serialized post tags use a
    validating role type. Stored session labels and physical row identities use
    distinct lossless private types. Feed-event claim SQL decodes a fully
    policed intermediate row, then conversion alone owns feed-URL diversion. The
    proven handwritten post decoder remains within the existing strict grammar.
  - Verification: SQLite and PostgreSQL focused tests prove negative and
    out-of-range attempts propagate without row mutation, only feed-URL parse
    failures divert, all other claim-field failures propagate, user flags retain
    behavior, invalid post-tag payloads retain column-scoped diagnostics,
    physical-row identity tests retain meaning, and session-label repair remains
    lossless.

- [ ] Task 4: Cut over the gate and its architectural record
  - Contract: after Tasks 1–3 leave no unapproved leaf, delete `Allowed`,
    `Category`, `ALLOWLIST`, exact-site/count matching, rationale reporting, and
    allowlist self-policing. Retain structural enumeration, declaration/macro
    self-policing, approved-foreign handling, composite delegation, strict
    handwritten-row proof, and unreadable-input failures.
  - Verification: focused xtask fixtures prove scalar/optional/tuple/derived and
    handwritten approvals plus unknown-leaf, unreadable-target, unpoliced-
    composite, incomplete-macro, and novel-bare-primitive failures. Re-run all
    four #728 one-line revert proofs with equivalent diagnostics, update
    ADR-0085 and `docs/ARCHITECTURE.md`, then pass `cargo xtask check` and
    `cargo xtask validate --no-e2e` on the final staged tree.

## Risk checks

- Keep the allowlist operational until every migrated decode has a declared
  target; delete it only in the final cutover task so intermediate commits
  remain gated.
- Do not broaden `APPROVED_FOREIGN` or add a bridge type whose only meaning is
  its primitive representation.
- Treat SQLx SQLite integer-to-bool compatibility as the implementation detail
  beneath `Exists`; shared storage code sees only bool semantics.
- Preserve faithful handling of unknown/orphan site-config keys and lossy stored
  session labels; direct decoding into validated domain types would hide or
  reject data the product currently repairs or exports.
- Preserve feed-event row ownership on every non-feed-URL decode failure,
  including negative and out-of-range attempt values.
- Keep backend-specific catalog SQL explicit where schemas differ; parity means
  matching Rust contracts and behavior, not forced textual identity.
- Update all affected gate tests, storage tests, ADR-0085, architecture docs,
  and stale comments/dependencies that still describe the allowlist.
