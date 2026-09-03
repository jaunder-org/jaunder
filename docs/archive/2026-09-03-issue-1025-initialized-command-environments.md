# Issue #1025 — package initialized command environments

## Outcome

Every currently matching non-initialization command test in
`server/tests/misc/commands.rs` uses an owned initialized environment instead of
separately retaining storage arguments, a temporary base directory, and an
optional PostgreSQL guard. Command behavior, backend coverage, and test
interfaces remain unchanged.

## Load-bearing decisions

- Add private `InitializedCommandEnv { args, base, _postgres }` test support in
  `server/tests/misc/commands.rs`.
- `InitializedCommandEnv::new(backend)` creates the `TempDir`, calls the
  existing dual-backend `storage_args`, immediately runs
  `cmd_init(&args, false)`, and preserves the current happy-path `unwrap`
  behavior before returning `Self`.
- The environment owns all resources for its full lexical lifetime. `base`
  retains SQLite files and storage paths; `_postgres` retains the unique
  PostgreSQL database guard until command use is complete.
- Keep the requested field order and private visibility. Callers access `args`
  and `base` directly within the test module; no production or public helper is
  introduced.
- Migrate all currently matching non-init tests, including the nineteen original
  audit callers and fifteen post-audit backup, restore, and SMTP callers. Tests
  needing source and target storage use one independent environment per role.
- A matching caller is a test in this module that creates `storage_args` and
  immediately performs successful `cmd_init(..., false)` solely to prepare later
  command behavior. No such raw setup remains after migration.
- Keep the four leading `cmd_init` contract tests on raw `storage_args`: fresh
  initialization, second initialization failure, skip-if-exists success, and
  invalid-path failure. Their subject is initialization itself.
- Keep uninitialized-command tests, typed fault-injection tests using
  `Backend::setup`, and low-level PostgreSQL/cross-backend fixtures on their
  existing seams.
- Initialization remains real and backend-specific through `storage_args`; do
  not substitute the already-migrated and seeded `Backend::setup` fixture.
- Fixture setup owns no command execution beyond `cmd_init`, no seed data, and
  no assertions. Each test retains those responsibilities.
- Production command files do not move, so coordination issue #977 requires no
  code or layout change.
- This private test-fixture packaging adds no domain terminology or
  architectural decision; `CONTEXT.md`, ADRs, and `docs/ARCHITECTURE.md` remain
  unchanged.

## Acceptance

- Every currently matching non-init caller in `server/tests/misc/commands.rs`
  uses `InitializedCommandEnv::new`; the four `cmd_init` contract tests remain
  raw.
- No duplicated `TempDir` / `storage_args` / immediate successful `cmd_init`
  lifecycle remains outside those four contract tests.
- SQLite bases and PostgreSQL guards remain alive through each command and
  assertion, including distinct source/target restore environments.
- Existing commands, seed setup, failure setup, and assertions are unchanged.
- Focused dual-backend command integration tests pass.
- `cargo xtask check` passes.

## Boundaries

- No production command, storage, CLI, or public test-support interface change.
- No migration of uninitialized tests, typed fault-injection tests using
  `Backend::setup`, low-level PostgreSQL fixtures, or cross-backend
  interoperability fixtures.
- No command-module split or other work from coordination issue #977.
