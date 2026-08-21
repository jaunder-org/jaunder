# Add a host-native cached test lane

- Issue: [#1106](https://github.com/jaunder-org/jaunder/issues/1106)

## Problem

Day-to-day Jaunder development still pays too much repeated compilation cost.
The project has lighter commit and push checks, and #1071 made host compiling
static checks use `sccache` in the multi-worktree shape. The broad test path,
however, still routes through the Nix coverage derivation when developers want
confidence across the product workspace.

That Nix path is the right authority for hermetic coverage and CI parity, but it
is a poor inner loop. Its project-level derivations invalidate broadly, and
compiler work done inside the Nix sandbox is not shared with host Cargo
invocations or with simultaneous agents working in separate worktrees.

The failed earlier `sccache` attempt was therefore a placement failure, not a
reason to abandon `sccache`: the expensive compilation happened where the cache
could not be effectively shared. For multi-agent development, the project needs
an explicit host-native test lane that uses the Nix dev-shell toolchain while
letting host Cargo and `sccache` reuse compiler work across worktrees.

PostgreSQL remains the main operational wrinkle. Automated tests must not use
the developer's persistent local PostgreSQL instance. The repository already has
the correct isolation primitive: `devtool pg run` starts a throwaway PostgreSQL
cluster, exports the test URLs, and tears the cluster down after the wrapped
command. That primitive is currently shaped for one run at a time; this issue's
multi-agent target requires it to be safe for simultaneous worktrees.

## Decision

Add a first-class xtask command for the host-native inner-loop test lane:

```bash
cargo xtask test-local
```

The command is deliberately separate from `check` and `validate`. `check` and
`validate` remain gate commands with existing semantics; `test-local` is a
developer-confidence command optimized for repeated local and multi-worktree
agent runs.

With no trailing arguments, `test-local` runs the root product workspace tests
through `cargo nextest run --workspace` on the host. It wraps the command in an
ephemeral PostgreSQL runner so both-backend tests keep using isolated PostgreSQL
through `JAUNDER_PG_TEST_URL` and `JAUNDER_PG_BOOTSTRAP_TEST_URL`.

The PostgreSQL wrapper must be concurrency-safe on a single host. Two agents in
two worktrees must be able to run `test-local` at the same time without
colliding on a fixed PostgreSQL port. The implementation may evolve
`devtool pg run` itself, share its Rust helper, or add a host-side wrapper, but
the observable contract is that each run receives a private loopback endpoint
and data directory.

The lane always uses the multi-worktree `sccache` mode:

- `RUSTC_WRAPPER=sccache`
- `CARGO_INCREMENTAL=0`
- `SCCACHE_BASEDIRS` derived from `git worktree list --porcelain`, including the
  current checkout

This is intentionally different from an ordinary hand-run `cargo test` in one
checkout, where Cargo incremental can be attractive. `test-local` is tuned for
Jaunder's common concurrent-agent case, where cross-worktree reuse matters more
than same-target incremental reuse.

## Command contract

The command accepts trailing nextest arguments after `--`:

```bash
cargo xtask test-local -- -p storage
cargo xtask test-local -- -p storage post_creation
cargo xtask test-local -- --run-ignored ignored-only
```

Trailing arguments are appended after the default `cargo nextest run` command.
When trailing arguments are present, the command does not add `--workspace`;
callers own the focused nextest selection. When no trailing arguments are
present, the command adds `--workspace`.

The command reports a normal xtask `StepResult` and exits non-zero when the
wrapped nextest run fails. The step detail should expose the command shape and
any `sccache` worktree-discovery warning, matching the static-check cache
behavior added in #1071.

## Boundaries

`test-local` is not a replacement for any existing gate:

- `cargo xtask check` remains the commit-oriented Fix-mode check.
- `cargo xtask validate --no-e2e` remains the pre-push / pre-PR local gate.
- `cargo xtask validate` and CI remain the full hermetic authority.
- Nix coverage, doctest, wasm, and e2e checks remain intact.

`test-local` does not run coverage, doctests, e2e, `xtask` unit tests, or
`tools` unit tests by default. It covers the root product workspace test lane
only. If later work wants additional host-native lanes, those should be explicit
rather than hidden in this command's default.

Do not introduce `devenv` for this issue. A persistent development service may
be useful someday, but this test lane needs isolated throwaway PostgreSQL, and
`devtool pg run` already owns that primitive.

## Documentation

Update the working docs so developers and agents know which command to choose:

- use `cargo xtask test-local` for repeated host-native product tests during
  day-to-day development;
- use `cargo xtask check --no-test` for static/clippy-focused iteration;
- use `cargo xtask check` for commit confidence under the existing hook model;
- use `cargo xtask validate --no-e2e` before pushing or opening a PR;
- use `cargo xtask validate` when the full local e2e gate is required.

The docs must call out that `test-local` intentionally disables Cargo
incremental to make Rust compiler invocations cacheable by `sccache` across
worktrees.

## Acceptance criteria

- `cargo xtask test-local` exists as a first-class xtask command.
- With no trailing arguments, it runs `cargo nextest run --workspace` for the
  root product workspace on the host.
- With trailing arguments after `--`, it runs `cargo nextest run` with those
  arguments and does not force `--workspace`.
- The command runs under an isolated throwaway PostgreSQL cluster via the shared
  `devtool pg` implementation or equivalent shared Rust helper, not a persistent
  local PostgreSQL instance.
- The PostgreSQL wrapper does not use one fixed host port for all runs; two
  simultaneous `test-local` invocations in separate worktrees can boot
  independent clusters without port collision.
- The wrapped nextest process receives `JAUNDER_PG_TEST_URL` and
  `JAUNDER_PG_BOOTSTRAP_TEST_URL` from the ephemeral PostgreSQL runner.
- Rust compilation in this lane receives `RUSTC_WRAPPER=sccache`,
  `CARGO_INCREMENTAL=0`, and `SCCACHE_BASEDIRS` derived from linked worktree
  roots plus the current checkout.
- The sccache/worktree environment construction is shared with, or factored
  consistently with, the existing host compiling static-check behavior from
  #1071.
- The command exits non-zero when nextest fails and reports the failure through
  the normal xtask result envelope.
- `CONTRIBUTING.md` documents when to use `test-local` versus `check`,
  `check --no-test`, `validate --no-e2e`, and `validate`.
- Existing Nix coverage, doctest, wasm, and e2e gates are not removed or
  weakened.
- Focused tests cover command construction, default versus passthrough argument
  behavior, sccache environment construction, PostgreSQL wrapper invocation, and
  per-run PostgreSQL endpoint selection without needing to start a real
  PostgreSQL server.
- `cargo xtask check` passes.
