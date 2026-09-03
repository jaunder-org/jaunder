# #1237 — delegate Nix build supervision to processkit

Issue: [#1237](https://github.com/jaunder-org/jaunder/issues/1237). Milestone:
Developer tooling & DX.

## Outcome

`build_check` uses xtask's private synchronous processkit seam for Nix builds.
Processkit owns child-tree containment, stderr pumping, wait/reap, and drop
cleanup while Nix diagnostics and rescue policy remain unchanged.

## Load-bearing decisions

- Extract the generic synchronous `Process` owner from the E2E-local module into
  the private `xtask::steps::process` seam. E2E server and collector wrappers
  stay in `e2e_local::process`; Nix-specific policy stays in `nix`.
- Keep the seam small: start a configured `processkit::Command`, wait for its
  terminal `Outcome`, retain the existing readiness and graceful-shutdown
  operations needed by E2E, and expose no repository-wide command wrapper.
- The owned `RunningProcess` is released before its Tokio runtime on every path.
  This corrects the current field/drop order and makes unwind cleanup safe for
  both existing E2E users and the Nix build user.
- `build_check` still constructs the exact
  `nix build -L --keep-failed --log-lines 50 --accept-flake-config --out-link …`
  invocation and owns GC-root paths, diagnostic paths, failure excerpts,
  failed-outPath rescue, warning aggregation, duration, and `StepResult` policy.
- Preserve the inherited standard streams that `std::process::Command` currently
  supplies: stdin and stdout remain inherited, while only stderr is piped
  through processkit. A focused fixture locks this configuration so the
  migration cannot replace stdin with EOF or silently discard stdout.
- Replace `MultiWriter` plus `drain_build_stderr` with a byte-oriented raw-tee
  sink driven by processkit's stderr pump. For each chunk it writes the
  diagnostic sink first and live stderr second, continues after either failure,
  flushes both, and records diagnostic and primary-output failures separately.
- Diagnostic sink failure remains best-effort: emit the existing sanitized
  warning, omit unreliable log/excerpt paths, and preserve the child result.
- Preserve the current failure precedence after reaping: a processkit wait or
  teardown error wins first; otherwise primary-output failure wins over both
  child success and child failure, returns `failed to stream nix build stderr`,
  and skips excerpting and failed-outPath rescue. Only a reliable primary stream
  reaches terminal child-outcome classification.
- A processkit start or wait/cancellation/teardown error follows the existing
  spawn/wait-error path: report any diagnostic-capture warning, fail the step
  with that error, and do not run child-failure excerpting or failed-outPath
  rescue because no definitive child exit outcome exists.
- A terminal non-zero or signalled `Outcome` follows the existing child-failure
  path. Preserve diagnostic-before-rescue ordering and the structure of the
  failure detail; successful completion remains successful despite a
  diagnostic-only capture failure.
- Tests exercise Jaunder's synchronous adapter and tee/error policy, not a broad
  duplicate of processkit's upstream suite. No ADR: this deepens the private
  processkit seam selected by #802 without introducing a new architectural
  choice.

## Acceptance

- Successful and failed Nix-build fixtures produce byte-identical, ordered live
  stderr and `build.log` output while inheriting stdin and stdout.
- Child success, child failure, diagnostic-tee failure, primary-tee failure, and
  processkit start/wait errors retain the existing `StepResult`, warning,
  reliable-path, and rescue behavior. A combined primary-tee-plus-child-failure
  fixture proves primary-output failure wins and rescue does not run.
- A focused cancellation/drop fixture proves the shared owner tears down the
  parent and descendant while retaining bytes already accepted by the tee.
- `build_check` contains no direct child stderr extraction, `io::copy`, or
  `Child::wait`; processkit owns pipe draining and wait/reap.
- Existing E2E-local server and collector behavior and tests remain green after
  moving them onto the shared process owner.
- The exact Nix build arguments still include `-L`, `--keep-failed`,
  `--log-lines 50`, `--accept-flake-config`, and the per-check out-link and
  installable; stdin and stdout remain inherited.

## Boundaries

- Do not migrate ordinary one-shot Nix evaluation commands.
- Do not change VM artifact-copy or failed-outPath rescue ordering.
- Do not add a public cancellation option or a generic repository-wide command
  wrapper.
- Do not move Nix diagnostic/excerpt/rescue policy into the shared process
  module.
