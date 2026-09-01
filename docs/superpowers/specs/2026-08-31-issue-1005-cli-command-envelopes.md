# Issue #1005: Centralize CLI test command envelopes

## Outcome

`server/src/main.rs` tests use one local constructor for ordinary non-verbose
CLI command envelopes and one explicit helper for successfully initialized
temporary storage. Commands, result expectations, and initialized/uninitialized
scenarios remain visible in each test.

## Load-bearing decisions

- Keep both helpers inside the existing `#[cfg(test)]` module in
  `server/src/main.rs`; add no production or cross-module test API.
- Add `test_cli(command: Commands) -> Cli`, setting `command: Some(command)` and
  `verbose: false`.
- Migrate every current non-verbose wrapper: the 29 historically audited
  top-level wrappers plus the three nested subprocess-child wrappers found in
  the current tree.
- Keep the sole `verbose: true` tracing test as an explicit `Cli` literal; do
  not add a verbosity parameter or second wrapper constructor.
- Add `initialized_storage(base: &TempDir) -> StorageArgs` as an async helper.
  It constructs temporary storage, runs non-skipping `Init` through `test_cli`,
  requires successful initialization, and returns the storage arguments.
- Use `initialized_storage` only where successful initialization is a
  prerequisite for the command under test, including source and target storage
  for restore.
- Keep invalid initialization, production Serve-before-init, development Serve
  auto-init, and other intentionally uninitialized scenarios explicit with
  `test_storage_args` or their custom storage arguments.
- Keep the command under test and every success/error/message assertion at its
  call site. Only the successful prerequisite `Init` is encapsulated by
  `initialized_storage`; helpers encode no expected result or production
  dispatch policy.

## Acceptance

- No `verbose: false` `Cli` literal remains in the test module; all 32 current
  non-verbose wrappers use `test_cli`.
- Repeated successful initialization prerequisites use `initialized_storage`
  without changing which tests begin initialized.
- The verbose tracing case and all uninitialized, auto-init, invalid-path,
  subprocess, spawn/abort, and error-result contracts remain observably
  unchanged.
- Clap definitions, production `run`, command dispatch, and public/test
  interfaces are unchanged.
- Focused `server/src/main.rs` tests pass.
- `cargo xtask check` passes.

## Boundaries

- Do not introduce a builder, command-specific constructors, expected-exit
  abstraction, generic harness, or new test-support module.
- Do not change command arguments, storage lifetime, subprocess isolation,
  environment handling, timing, or assertions.
- Do not consolidate production CLI parsing or dispatch.
