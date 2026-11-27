# ADR-0278: Compiling dev steps label their kache events and record its counts

- Status: accepted
- Date: 2026-11-27
- Issue: [#2146](https://github.com/jaunder-org/jaunder/issues/2146)

## Context

[ADR-0254](0254-dev-trace-step-accounting.md) made each `dev` step span time one
kind of work. It gave Nix steps a cache source, but Cargo steps recorded nothing
about the kache compile cache ([ADR-0256](0256-kache-compile-cache.md)). A
retrospective could not tell how much compilation kache spared a Cargo step.

kache 2.1.1 labels every compile's event with a build tree, by default the Cargo
workspace. `kache stats --json --last-build --root <tree>` reports that tree's
latest session: its hits, misses and estimated compile time saved. One session
covers every Cargo command in a tree until it has been idle for five minutes. So
a query by workspace merges consecutive steps, and builds in the same checkout
that run at the same time. A query by time window also mixes in concurrent
builds, and the window can only be set to whole seconds. The event log's own
format is internal to kache.

## Decision

- **The compiling Cargo steps opt in.** These are the Clippy leaves,
  `tools-tests-build`, `tools-tests`, `image-worker`, `nextest-build`, `nextest`
  and `csr-wasm`. `cargo fmt` and `cargo deny` compile nothing and do not opt
  in.
- **Each run of such a step is its own build tree.** When `RUSTC_WRAPPER` is
  `kache` (every host step; a sandbox has none), the step's child gets
  `KACHE_EVENT_ROOT=<checkout>/.xtask/kache/<trace id>-<span id>`. That path is
  never created. kache uses the variable only to label events, not in cache
  keys.
- **After the child exits, the step asks kache.** Inside the step span and
  unless the step was signalled, `dev` runs
  `kache stats --json --last-build --root <tree>`. This query is not a step of
  its own, like the Nix queries behind `jaunder.cache`. `dev` records
  `jaunder.kache.hits` (local, prefetch and remote), `jaunder.kache.misses` and
  `jaunder.kache.avoided_compile_ms` (kache's `time_saved_ms`). It records them
  also for a failed build, and records zero for a session of rustc probes alone.
- **A query that does not answer records nothing.** That covers a nonzero exit,
  as for a tree with no events (nothing compiled, or `KACHE_DISABLED=1`), and a
  report of any other shape. The step's outcome never changes.

This extends ADR-0254's step accounting to Cargo steps, and amends ADR-0256's
"everything else is kache's default" by this one per-step variable.

## Consequences

- **Query time.** Each compiling step takes about 0.2-0.6 s longer, measured
  against a 34 MB event log. kache rotates the log by size, which bounds that
  cost.
- **Less from kache's own tools.** To kache each step run is its own tree:
  - `kache explain` and the wrapper's recorded key diff find no earlier build of
    the same tree for a `dev` step;
  - `kache stats --last-build --root <checkout>` returns only the latest step;
  - the monitor lists each step as its own build.

  A stable tree per step name would have kept explain's baseline. But a gate
  re-run within five minutes would then merge into the earlier session and count
  its compiles twice.

- **Avoided compile time is not wall time.** It is summed over compiles that run
  in parallel, so it can exceed the step's duration.
- **A kache upgrade means checking the report again.** If the report's fields
  change shape, nothing is recorded rather than a wrong value.
