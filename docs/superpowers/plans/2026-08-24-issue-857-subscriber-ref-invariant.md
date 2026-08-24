# Subscriber reference invariant implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independent work.
> This outline exists because the approved change combines a public proc-macro
> cutover with paired schema migrations and backend-parity restore semantics.

## Scope

In:

- Validate `SubscriberRef` at Rust domain, serde, SQLx, and application-read
  boundaries while preserving accepted bytes.
- Add equal sequential SQLite/PostgreSQL schema enforcement and strict upgrade
  behavior.
- Preserve portable backup restore failure classification and rollback.
- Remove the unneeded infallible string-newtype mode completely.
- Land the decision record, architecture projection, operator guidance, and
  observable regression coverage required by the spec.

Out:

- Remote-channel identifier grammar or normalization.
- Automatic repair/deletion of invalid subscriptions.
- A general newtype, schema-constraint, or restore redesign.
- Backward restore across schema versions.

## Task outline

Execution waves: Tasks 1 and 2 may run in parallel. Task 3 starts after Task 1
removes the last production selector. Task 4 integrates the completed slices.

- [x] **Task 1 — Make subscriber identity validate before application use.**
  - Contract: `SubscriberRef` rejects `value.trim().is_empty()` with a typed,
    stable domain error; accepted values remain byte-identical. Local `UserId`
    conversion remains an infallible typed-proof door. Subscriber summary rows
    decode `SubscriberRef` before producing display `String` values.
  - Verification: common-domain tests cover empty, Unicode whitespace, verbatim
    opaque values, serde, error display, and local decimal encoding; focused
    storage tests prove active subscriber listing and summaries cannot bypass
    typed decode on both backends.

- [x] **Task 2 — Enforce the portable storage subset and strict rollout.**
  - Contract: matching next-number migrations retain `NOT NULL` and reject
    zero-length `subscriber_ref`; applied migration 0019 remains untouched.
    Existing invalid data aborts atomically without changing subscriptions or
    dependent audience membership. SQLite and PostgreSQL restore both report
    `BackupError::ConstraintViolation` and leave the target unmodified.
  - Verification: dual-backend migration tests exercise clean upgrade, invalid
    pre-upgrade data, rollback, constraints, indexes, and foreign-key integrity;
    portable backup tests exercise the constraint-error category and rollback.

- [x] **Task 3 — Remove `str_newtype(infallible)` after its adopter migrates.**
  - Contract: delete the parser option, codegen/serde/SQLx branches, public
    rustdoc, fixtures, and mode-specific tests. Preserve ordinary validating,
    `no_ord`, serde, SQLx, and compile-fail behavior. Historical ADR Decision
    text remains immutable; Task 4 owns dated corrections to stale present-state
    claims.
  - Verification: macro unit/integration/doctest coverage proves the retained
    surface; source and active-documentation searches find no remaining mode
    selector or API contract.

- [x] **Task 4 — Integrate the architectural cutover and operator guidance.**
  - Contract: the numberless ADR records the final invariant, portable schema
    limit, strict rollout, restore contract, and ADR-0063/0101 amendment. It is
    also the durable operator guide: include the exact diagnostic query and the
    dependency-safe audience-membership/subscription repair or deletion order.
    Add a dated correction beside ADR-0063's stale “no production type” claim
    without rewriting its historical Decision. `docs/ARCHITECTURE.md` describes
    the implemented state and cites the draft by path. `CONTEXT.md` stays
    unchanged by explicit consideration.
  - Verification: inspect the ADR's runnable query and repair ordering, confirm
    the stale active claim is corrected while historical context remains, review
    the combined diff against every spec acceptance item, then run
    `devtool run -- cargo xtask precommit`; the ship phase owns the full
    SQLite/PostgreSQL validation gate and ADR promotion after rebasing.

## Cross-task contracts

- Task 1 owns the `SubscriberRef` validation/error API and typed summary-row
  shape. Task 2 consumes only the invariant (`blank` in Rust, zero length in
  SQL), not Task 1's private implementation.
- Task 2 owns migration number 0026 for both backends and migration/restore
  tests. No other task edits those migration files.
- Task 3 assumes Task 1 has removed every production `infallible` selector
  before deleting the macro mode; it does not redesign `StrNewtype`'s retained
  trailer.
- Task 4 owns the ADR, its embedded operator procedure, ADR-0063's dated
  correction, and architecture reconciliation after Tasks 1–3; earlier tasks
  provide evidence but do not independently rewrite those documents.

## Risk checks

- Both migration directories retain identical, gap-free number sets.
- SQLite table rebuild preserves both unique constraints, foreign keys, default,
  index, subscription IDs, and `audience_members` references.
- Migration failure is transactional and non-destructive on both backends.
- Database enforcement is deliberately zero-length-only; Rust remains the owner
  of Unicode-blank rejection.
- Backup export remains an exact schema snapshot; only restore classification
  and schema rejection change.
- Every application projection of a subscriber reference validates the domain
  value before converting it to display text.
- Macro removal leaves no second convention, compatibility alias, deprecated
  path, or stale active rustdoc.
- No lint suppression is introduced without explicit approval.
