# Issue #988: Split xtask CLI grammar from dispatch

## Outcome

Replace the mixed responsibilities in `xtask/src/lib.rs` with four focused leaf
modules: `cli.rs`, `gate.rs`, `dispatch.rs`, and `lifecycle.rs`. Keep `lib.rs`
as the small crate facade: module declarations plus explicit re-exports of the
existing public API. Existing commands, arguments, help text, result envelopes,
sidecar/sentinel behavior, gate membership and ordering, failure behavior, and
public/test interfaces remain unchanged.

## Load-bearing decisions

- `xtask/src/cli.rs` owns the top-level Clap grammar, every embedded subcommand
  grammar currently in `lib.rs`, and grammar-derived metadata: `Cli`, `Command`,
  `E2eBackend`, `E2eBrowser`, `PrWatchUntil`, `PrCommand`,
  `ServerFnCoverageCommand`, `AdrCommand`, `CoverageCommand`, `TracesCommand`,
  backend/browser string values, `Cli::command_name`, and
  `Command::produces_json_payload`. The already-cohesive nested
  `issue::IssueCommand` remains with its issue workflow owner. Existing types
  remain available at their current `xtask::*` paths through explicit `lib.rs`
  re-exports.
- `xtask/src/gate.rs` owns gate execution policy and ordered orchestration:
  `ExecutionPolicy`, the command-to-policy mapping consumed by dispatch,
  `run_with_policy`, `HostGateStep`, the host-step catalogs, host-gate runners,
  Markdown precommit routing, `PrepushPhase`, and the prepush plan. The mapping
  is a crate-visible gate function rather than a `Command` method, preserving
  the one-way `gate -> cli` dependency. It changes no catalog membership,
  ordering, mode, fail-fast boundary, or result detail.
- `xtask/src/dispatch.rs` owns `run(Cli)`, command-specific result adaptation,
  and the exhaustive mapping from parsed commands to domain runners. It
  coordinates existing modules; it does not absorb their implementation. The
  `Validate` and `E2e` arms retain their distinct server-function coverage paths
  from #824: local full `validate` verifies the realized authoritative
  combination after the aggregate, while each matrix `e2e` invocation verifies
  its own uncollided authoritative combo.
- `xtask/src/lifecycle.rs` owns cross-command lifecycle and preconditions:
  best-effort hook installation, clean-tree classification, precommit
  snapshot/reconciliation orchestration, and final duration/timestamp recording.
  The issue command continues to use this single finalization implementation; no
  duplicate lifecycle helper is introduced.
- `xtask/src/lib.rs` contains the existing domain/module declarations and
  explicit re-exports only. It re-exports `Cli` and all currently public command
  types from `cli`, `run` from `dispatch`, `ensure_hooks_installed` from
  `lifecycle`, and `CommandResult`, `Mode`, and `StepResult` from `result`. The
  four new leaf modules remain private; no compatibility aliases or new public
  module paths are added.
- The gate/static-check dependency remains one-way. `steps::static_checks`
  retains the pure phase catalogs and individual `StepSpec` execution. `gate`
  owns applying `ExecutionPolicy` to those specs, replacing the current
  `static_checks::{run_phase_with, run_markdown_phase_with}` callbacks rather
  than leaving `static_checks` dependent on `gate`.
- The dependency direction is acyclic: `cli` depends only on Clap, path types,
  the existing nested issue grammar, and stable command-name constants;
  `lifecycle` depends only on `git` and `result`; `gate` depends on `cli`,
  `lifecycle`, `result`, and `steps`; `dispatch` depends on all three leaves
  plus the existing domain runners and owns command-specific result adaptation
  such as malformed trace attributes. `steps` does not depend on `gate`; neither
  `cli` nor the lifecycle/result layer calls dispatch.
- ADR-0028 remains exact: all command dispatch remains host-side in `xtask`; no
  behavior moves into the in-sandbox `devtool` boundary.
- ADR-0029 remains exact: precommit retains conservative staged-subset routing
  and reconciliation, prepush retains its clean-tree-first fail-fast path, and
  explicit check/validate retain exhaustive execution.
- ADR-0034 remains exact: `validate` remains the full local gate and CI retains
  per-`{backend}×{browser}` `e2e` command dispatch with the same derivations and
  coverage verification.
- No `mod.rs` is introduced. Existing `mod.rs` files remain assembly-only under
  ADR-0128.

## Test ownership

- Clap parsing, defaults, conflicts, help/metadata, stable command names, and
  JSON-support tests move to `cli.rs`.
- Gate catalog membership/order, execution-policy, fail-fast/exhaustive,
  Markdown routing, and prepush plan tests move to `gate.rs`. This includes the
  static-phase fail-fast/exhaustive tests because the policy-applying
  `run_phase_with` behavior moves from `steps::static_checks` into `gate`.
- Pure static catalog/spec-construction tests remain in `steps/static_checks.rs`
  beside their implementation.
- Precommit snapshot/reconciliation and clean-tree lifecycle tests move to
  `lifecycle.rs`.
- Tests that exercise full command-to-runner behavior or result assembly move to
  `dispatch.rs`.
- The Git environment-scrubbing test moves to `git.rs`, whose implementation it
  proves. Tests already owned by `result.rs`, other individual `steps`, and
  domain modules remain there.
- Every pre-split test and assertion remains present under its semantic owner;
  changes are relocation and path updates only unless compilation exposes a test
  that currently spans two owners, in which case each assertion moves to the
  owner of the contract it proves.

## Acceptance

- `xtask/src/lib.rs` is a small facade containing module declarations and
  explicit public re-exports, with no command grammar, dispatch match, gate
  catalog, lifecycle implementation, or inline tests.
- `cli.rs`, `gate.rs`, `dispatch.rs`, and `lifecycle.rs` each contain only the
  responsibility named above; no catch-all utility module is introduced.
- The public root paths used by `xtask/src/main.rs` and tests remain valid:
  `xtask::{Cli, run}`, `xtask::ensure_hooks_installed`, all currently public
  command grammar types, and the existing result types.
- `cargo xtask --help` and every subcommand's parsing, flags, defaults,
  conflicts, validation, examples, command names, and `--json` acceptance or
  rejection are unchanged.
- Check, precommit, prepush, and validate retain exact step membership, order,
  modes, preconditions, fail-fast/exhaustive behavior, result details, and
  timing/finalization.
- The #824 parity fix remains intact: `validate` with e2e runs
  `verify_after_validate` after realizing all combinations, and `e2e` retains
  `verify_after_combo`.
- Result serialization, human reporting, `.xtask/last-result.json`,
  `xtask-done:`, exit codes, and hard-error propagation are unchanged.
- Focused xtask tests and the repository pre-commit gate pass.

## Boundaries

- No CLI option, help copy, command name, step catalog, gate policy, check
  authority, result schema, sidecar format, sentinel, exit code, or domain
  behavior changes.
- No redesign of `result`, `steps`, `issue`, `pr`, traces, coverage, e2e, Nix,
  or any domain runner.
- No per-command dispatch hierarchy: the exhaustive command match remains one
  readable owner in `dispatch.rs`.
- No new abstraction around command execution and no generic command framework.
- No ADR is needed: this is a behavior-preserving projection of ADR-0028,
  ADR-0029, ADR-0034, and ADR-0128 into a more cohesive module layout.
