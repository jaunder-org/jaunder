# Issue #678 - lint feature-gated server code

## Outcome

The `cargo xtask check` and `cargo xtask validate` ladders lint Jaunder's
host-side `web` code behind `feature = "server"`. The gate remains explicit
about feature selection instead of relying on a blanket workspace
`--all-features` pass that does not currently work for this workspace.

## Load-bearing decisions

- This is a gate coverage fix, not a feature-model redesign. `web` remains one
  crate with separate `csr` and `server` Cargo features as documented in
  `docs/ARCHITECTURE.md`.
- Do not use a blanket workspace `cargo clippy --all-features --all-targets`.
  The experiment failed before reaching the `web` question: workspace
  `--all-features` enables `macros` test targets without `sqlx` available in the
  test crate root, producing `cannot find sqlx in the crate root` errors from
  the newtype derives. The Leptos `csr`/`ssr` pairing was not the observed
  blocker.
- Add an explicit host/server clippy surface for the code the issue names:
  `cargo clippy -p web --features server --all-targets -- -D warnings`. A direct
  run of that command passed, so it is a viable gate step.
- Keep the existing wasm clippy surface unchanged:
  `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings`.
- Put the command definition in `devtool check`, because `xtask` delegates
  product compile/type checks to `devtool` and the Nix static-checks derivation
  shares those command definitions.
- Wire the new check into the `xtask` compile/type phase so `check`,
  `check --no-test`, `precommit`, `prepush`, and `validate --no-e2e` inherit it
  through the existing ladder.

## Boundaries

- No change to runtime behavior.
- No change to Cargo feature definitions unless the implementation discovers
  that the exact server clippy command cannot stay stable.
- No `--all-features` replacement for the product clippy step in this issue.
- No attempt to lint every possible optional feature combination. The only
  required new coverage is `web --features server`, because that is the current
  known blind spot for production server-function code.
- No lint suppressions.

## Acceptance

- `devtool check` exposes a named check for host-side `web --features server`
  clippy, and its unit tests lock the exact command arguments.
- `xtask/src/steps/static_checks.rs` includes that check in the compile/type
  phase, marks it rustc-cacheable, and its unit tests lock the
  ordering/presence.
- `cargo xtask check` and `cargo xtask validate --no-e2e` reach the new check
  via the normal ladder.
- A deliberate clippy violation inside a `#[cfg(feature = "server")]` block is
  proven to fail the new check before the violation is removed.
- Final verification includes `devtool run -- cargo xtask check --no-test` and
  the focused deliberate-violation proof.
