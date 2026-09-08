# Remove the empty subscription dialect

## Outcome

Subscription storage uses one backend-common implementation and one shared
source for its SQL without an empty dialect abstraction. SQLite and PostgreSQL
retain identical externally observable subscription behavior and their existing
storage aliases and construction paths.

## Load-bearing decisions

- Delete `SubscriptionDialect`; subscriptions have no backend-specific operation
  that justifies a dialect under ADR-0019.
- Make the generic subscription storage implementation depend directly on
  `Backend`, while keeping only the SQLx bounds required by its operations.
- Keep every subscription SQL statement private to the shared subscription
  implementation and expressed exactly once.
- Preserve all current SQL text, bind order, write-transaction shape, and
  subscription semantics. In particular, subscribe remains one
  `INSERT ... RETURNING` operation, unsubscribe remains one `DELETE`, and
  local-viewer resolution continues to select the seeded `local` channel inside
  SQL.
- Preserve the object-safe public `SubscriptionStorage` trait unchanged.
- Preserve `SqliteSubscriptionStorage` and `PostgresSubscriptionStorage`, their
  module exports, and all construction paths.
- Remove the SQL-constant parity test. Replace it with focused observable
  coverage of the local-viewer `is_subscriber` path under the shared
  dual-backend harness, homed beside the generic subscription store as ADR-0053
  requires.
- Keep the existing server-level dual-backend subscription coverage; the new
  generic-home contract test specifically replaces the deleted
  implementation-string assertion.
- This is a direct application of ADR-0019, ADR-0020, ADR-0021, ADR-0033, and
  ADR-0053, not a new architectural or domain decision. No ADR or `CONTEXT.md`
  change is required.

## Acceptance

- The production and test Rust workspace no longer defines or references
  `SubscriptionDialect`; frozen historical documentation remains unchanged.
- `SubscriptionStore<DB>` implements `SubscriptionStorage` with `DB: Backend`
  and the existing operation-specific SQLx bounds.
- The seven subscription statements remain single-source in the shared
  subscriptions module; no backend-specific copy or replacement marker trait is
  introduced.
- The public storage trait remains object-safe, and both backend aliases and
  generic `AppState` construction continue to compile.
- Focused dual-backend subscription tests pass, including a generic-home
  contract test that observes the local `ViewerIdentity` subscriber path before
  and after unsubscribe.
- The repository's static check surface passes without lint suppression.

## Boundaries

- Behavior-preserving storage refactor only: no schema, migration, public API,
  policy, visibility, or transaction changes.
- Do not rename subscription domain concepts or alter typed `SubscriberRef`
  decoding and invalid-row handling.
- Do not broaden this change into other storage dialects, backend abstractions,
  or test-harness restructuring.
- Do not duplicate SQL or replace the removed dialect with another zero-behavior
  adapter.
