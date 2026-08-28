# Issue #849: Isolated no-server web test clippy

## Outcome

The gate compiles and lints `web`'s host test targets with no features enabled,
so server-feature gating mistakes cannot hide behind workspace feature
unification. The wasm lint remains limited to wasm library targets and does not
pull host-only test dependencies into wasm.

## Load-bearing decisions

- Add a named host compiling check, `web-no-server-clippy`, whose Cargo contract
  is `clippy -p web --no-default-features --all-targets -- -D warnings`.
- `tools/devtool` owns the command definition. The host xtask ladder and the
  hermetic Nix static-check derivation consume that same definition under the
  existing ADR-0146 architecture.
- Keep `wasm-clippy` unchanged and without `--all-targets`; `web`'s
  unconditional dev-dependencies include host-only storage/Tokio networking that
  cannot compile for `wasm32-unknown-unknown`.
- Keep the generic workspace clippy step. It remains broad host coverage, but it
  cannot replace this isolated step because workspace feature unification
  enables `web/server`.
- Fix existing no-feature warnings in `web` by correcting ownership or cfg
  placement, never by suppressing lints or weakening `-D warnings`.
- No ADR is required: this applies the existing devtool-owned static-check
  architecture rather than changing it.

## Acceptance

- Host and sandbox devtool command specs name `web-no-server-clippy` with the
  exact isolated Cargo arguments.
- The xtask compile/type phase runs the named step, and static-check inventory
  and command-contract tests pin its presence and cacheable Rust compilation
  role.
- A temporary test-only item whose use is gated behind `feature = "server"`
  fails `web-no-server-clippy`; the probe is removed afterward.
- The current isolated no-feature build is warning-free, including the existing
  `edit_post_url` and `UtcInstant` findings.
- `wasm-clippy` retains its current arguments and remains green.
- Contributor and architecture documentation distinguish the isolated host test
  feature check from wasm target linting.
- `cargo xtask validate` passes.

## Boundaries

- Do not make `web`'s host-oriented dev-dependencies wasm-compatible or add wasm
  execution for `web` tests; browser tests remain owned by `client`.
- Do not add `--all-targets` to `wasm-clippy`, enable `server` in the new step,
  or remove the generic workspace clippy step.
- Do not change shipped server or CSR feature sets.
