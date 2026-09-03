# Issue #1026: Centralize local subscription fixtures

## Outcome

Fixture-only tests create local subscriptions through one storage test-support
helper instead of repeating channel, identity, transaction, and confirmation
setup. Existing subscription behavior and test interfaces remain unchanged.

## Load-bearing decisions

- Add
  `storage::test_support::seed_local_subscription(&AppState, author: UserId, subscriber: UserId) -> SubscriptionId`
  in a dedicated test-support sibling module and re-export it from the
  test-support assembly module.
- The helper derives the local channel and local subscriber identity internally;
  callers cannot vary locality or stored status through this seam.
- The helper runs `SubscriptionStorage::subscribe` through the state's write
  scope, treats setup failure as a test panic, confirms the write with the
  repository's fixture convention, and returns the resulting `SubscriptionId`.
- Migrate all 16 currently matching fixture-only subscription creations across
  15 setup sites: the backup fixture, six audience test setups, two
  post-visibility setups, four storage-audience setups, one storage-listing
  setup, and two creations in the resolution matrix.
- Remove superseded file-local subscription helpers and their now-unused
  imports.
- Keep subscription contract tests explicit because they exercise identity
  variants, idempotence, status preservation, invalid references, ordering, and
  unsubscribe behavior.
- This is test-support consolidation only; local subscription admission and
  persistence semantics do not change.

## Acceptance

- The shared helper is the only construction seam used by all 16 in-scope
  fixture-only local subscriptions.
- Each migrated caller still seeds the same author/subscriber relationship and
  uses the returned identifier wherever its assertions require it.
- Subscription contract tests in `server/tests/storage/subscriptions.rs` retain
  their explicit setup.
- A source census finds no superseded file-local helper or matching fixture-only
  setup left behind.
- Focused affected tests pass through the dual-backend harness.
- `cargo xtask check` passes.

## Boundaries

- No production storage API, subscription status policy, local reference
  representation, schema, or public interface changes.
- Do not migrate tests whose observable contract is subscription creation,
  identity resolution, idempotence, status, listing, or removal itself.
- Do not expand coordination issues #750, #950, or #963 into this change.
- No ADR or domain-glossary update is required; the existing local subscription
  model remains authoritative.
