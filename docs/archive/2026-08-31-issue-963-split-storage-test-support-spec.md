# Split storage test support by concern

## Outcome

`storage::test_support` is organized into cohesive leaf modules while retaining
its existing public test interface and observable behavior. Each helper and its
contract tests live with the concern that owns them.

## Load-bearing decisions

- Replace `storage/src/test_support.rs` with a wiring-only
  `storage/src/test_support/mod.rs` that contains module declarations, module
  documentation, attributes, and explicit re-exports only, per ADR-0128.
- Preserve the `#[cfg(any(test, feature = "test-support"))]` module gate and
  every existing `storage::test_support::*` path.
- Split the implementation into `backend.rs`, `postgres.rs`, `users.rs`,
  `posts.rs`, `media.rs`, `post_service.rs`, `mail.rs`, `feeds.rs`, and
  `invites.rs`.
- Keep `Backend`, `CloseablePool`, fault-injection lock guards, `TestEnv`,
  `TestBase`, backend setup, the SQLite URL helper, and all four rstest
  templates together in `backend.rs`.
- Keep PostgreSQL configuration, URL construction, provisioning, template
  cloning, teardown, and guards together in `postgres.rs`.
- Keep user fixtures in `users.rs`, post fixture builders and batch seeding in
  `posts.rs`, and production-path post create/update helpers in
  `post_service.rs`.
- Keep media fixtures and media backup/raw-file inspection helpers together in
  `media.rs`; keep the mailer, canonical feed-path parser, and invite-code
  parser in their focused domain leaves.
- Move each existing unit test beside the implementation or contract it proves;
  do not change test homing or backend coverage.
- Preserve exported macro and rstest-template resolution, including bare-name
  `#[apply(...)]` use across crates under ADR-0124, ADR-0033, and ADR-0053.
- Do not redesign `Backend::setup` from #841 or alter the `PostWriteLock`
  behavior tracked by #874.

## Acceptance

- Every implementation leaf has one named responsibility; `mod.rs` only
  assembles and documents the module surface.
- Existing storage and server test call sites compile without import-path
  migration or compatibility aliases.
- Existing backend templates, `with_closeable_pool!`, fault injection, fixture
  builders, PostgreSQL lifecycle helpers, and post-service helpers retain their
  current behavior.
- Tests move with their owning implementation or contract and continue to
  exercise both storage backends where they do today.
- The test-enabled repository gate (`cargo xtask check`) passes on the complete
  split.

## Boundaries

- No production storage behavior, schema, protocol, or domain vocabulary
  changes.
- No new public test helpers, interface renames, deprecations, compatibility
  shims, or opportunistic cleanup outside the split.
- No ADR change: the refactor implements existing module, harness, and
  test-homing decisions.
