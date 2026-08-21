# Define the sandboxed cargo-deny policy for devtool

- Issue: [#1074](https://github.com/jaunder-org/jaunder/issues/1074)
- Milestone: Developer tooling & DX
- Parent: [#276](https://github.com/jaunder-org/jaunder/issues/276)
- Prerequisites: [#1072](https://github.com/jaunder-org/jaunder/issues/1072) and
  [#1073](https://github.com/jaunder-org/jaunder/issues/1073)

## Problem

`cargo-deny` is still split between the host ladder and a crane `deny`
derivation. Issue #276 cannot move that check behind `devtool check` until the
sandbox behavior is explicit.

The host command, `cargo deny check`, includes `advisories`. That path may fetch
the RustSec advisory database. A Nix sandbox must not depend on network access,
and the current crane `deny` derivation already reflects that boundary by
running an offline-safe subset instead of the full host advisory behavior.

Unifying the command without naming this policy would either break hermetic
builds or hide a security-gate difference behind identical command names.

## Decisions

| ID  | Decision                                                                                                                                                                                                                          |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Add `cargo-deny` as a `devtool check` name backed by the root product Cargo workspace.                                                                                                                                            |
| D2  | In normal host mode, `devtool check cargo-deny` runs the full host policy: `cargo deny check`.                                                                                                                                    |
| D3  | In sandbox Cargo mode, `devtool check cargo-deny` skips `advisories` and runs only the offline-safe cargo-deny checks: `bans`, `licenses`, and `sources`.                                                                         |
| D4  | Sandboxed cargo-deny must use the product workspace's offline Cargo configuration from #1073 and must force Cargo offline before spawning `cargo-deny`; a missing product Cargo home is an error before any command runs.         |
| D5  | Add `cargo-deny` to `devtool check --all`, so the Nix `static-checks` derivation exercises the sandbox policy now.                                                                                                                |
| D6  | Leave `xtask/src/steps/static_checks.rs` running its current native `cargo deny check` StepSpec in this issue; #276 owns the later host-ladder unification and removal of temporary duplication with the crane `deny` derivation. |
| D7  | Record the host/sandbox cargo-deny split in a draft ADR because it is a security-relevant trade-off between advisory freshness and hermetic reproducibility.                                                                      |

## Acceptance criteria

- **AC1 - check name exists.** `devtool check cargo-deny` is a valid check name,
  and unknown-check errors list it with the other known checks.
- **AC2 - host behavior is full cargo-deny.** In host mode,
  `devtool check cargo-deny` constructs `cargo deny check` with no offline Cargo
  env overrides and no explicit check subset.
- **AC3 - sandbox behavior is offline-safe.** In sandbox mode,
  `devtool check cargo-deny` constructs a command that cannot run `advisories`;
  it checks only `bans`, `licenses`, and `sources`.
- **AC4 - sandbox Cargo routing is product-scoped.** The sandbox command uses
  the product workspace's offline Cargo home, sets `CARGO_NET_OFFLINE=true`, and
  errors before spawning when `JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` is absent or
  empty.
- **AC5 - `--all` includes the policy.** `devtool check --all --sandbox-cargo`
  includes `cargo-deny`, so the Nix `static-checks` derivation exercises the
  offline policy.
- **AC6 - xtask remains native.** The host `cargo xtask check` static-check
  order and native `cargo-deny` StepSpec remain unchanged in this issue.
- **AC7 - decision is documented.** A numberless ADR draft records the
  cargo-deny host/sandbox split, and `docs/ARCHITECTURE.md` cites that draft
  path so `cargo xtask adr promote` can rewrite the citation at ship.
- **AC8 - gates pass.** Targeted `tools/devtool` tests pass,
  `devtool check cargo-deny --sandbox-cargo` passes in the Nix `static-checks`
  environment, and `cargo xtask check` passes.

## Out of scope

- Vendoring or otherwise providing the RustSec advisory database hermetically.
- Moving the host `xtask` `cargo-deny` StepSpec behind `devtool check`.
- Removing or changing the crane `deny` derivation.
- Moving `clippy`, `wasm-clippy`, or `tools-clippy` behind `devtool check`.
- Changing `deny.toml` policy except where a test fixture or command proof
  requires a local implementation detail.
