# Issue #1968: `dev` runs Nix-built host tools

## Outcome

Every Rust host tool that `dev` runs comes from a Nix package, and CI
substitutes that package from Cachix instead of compiling it. This covers
`source-contracts`, `coverage-check`, `wasm-audit`, `perf-data` and the rest.

A CI job compiles no tool unless the tool package's own Nix source changed. That
source is the tool crate, its path dependencies, the toolchain and `flake.lock`.
Jobs that run no cargo stop restoring the 1 GB `tools/target` cache.

This absorbs #2027 and supersedes #2026.

## Why

Measured on main run 44842863313 with a warm cache. Each `<bin>-build` span is a
`cargo build` of one tool:

| Lane        | Tool compile spans            |
| ----------- | ----------------------------- |
| host        | `scoped-leptosfmt-build` 27 s |
| coverage    | `coverage-check-build` 56 s   |
| test-checks | `coverage-check-build` 54 s   |
| hermetic    | `wasm-audit-build` 29 s       |

The cause is the shared cache key
`tools-<os>-hash(tools/Cargo.lock, flake.lock)`:

- `actions/cache` saves only when the key misses, and then from whichever job
  finishes first.
- That job is normally `E2E catalog`, which never runs cargo.
- So the cache never gains what the validation lanes build.
- 17 e2e jobs restore the cache and never use it.

## Approach and tradeoffs

- **One way to get a tool.** `steps.tool(T)` calls `steps.nix_build` on T's
  flake package, once per package per `dev` process, and returns
  `<out>/bin/<binary>`.
  - The Nix build is the traced step. The `<bin>-build` cargo spans go away.
  - `e2e.py`'s direct `nix_build(".#host-proof")` is routed through `steps.tool`
    too.
  - Deleted: `Tool.build`, `TOOLS_TARGET` and `forget_tools`.
  - This is what ADR-0225 decided for host tooling. It is the same locally and
    in CI.
- **Packages.** Existing packages are reused where they already build the
  binary:
  - `.#repository-policy`: source-contracts, repository-docs,
    org-timezone-manifest, scoped-leptosfmt.
  - `.#system-artifacts`.
  - `.#host-proof`, extended from `e2e-evidence` to all its binaries, so
    `dev host-proof <binary>` forwarding keeps working.
  - New `mkHostTool` packages for `coverage-check` (feature `host`), `perf-data`
    and `pr-observer`. `mkHostTool` gains a `features` argument.
  - **Dependencies built once.** Each tool package takes a dependencies-only
    build (crane `buildDepsOnly`) as its `cargoArtifacts`, replacing today's
    `cargoArtifacts = null`. That build changes only with `tools/Cargo.lock`,
    the toolchain or features, so Cachix almost always serves it. A PR that
    changes `common/` or `host/` then recompiles only the workspace crates, not
    the roughly 400 dependencies. That is about what cargo does today after a
    fresh checkout. `repository-policy` moves onto `mkHostTool` if its
    source-admission environment fits there; otherwise its builder takes the
    same dependencies artifact.
  - Package names must not match setup-ci's Cachix `pushFilter`
    (`jaunder-coverage|jaunder-e2e`). Otherwise they would never be pushed, and
    so never substituted.
  - The new packages and `repository-policy` join `nix/platform-contract.nix`'s
    portable list, because `dev` now needs them on every developer system.
- **Scoped leptosfmt.** `dev` puts `<repository-policy out>/bin` first on
  leptosfmt's PATH, replacing `tools/target/debug`.
- **Edited and untracked sources.** The flake source is the working tree, so
  editing a tool changes its derivation and Nix rebuilds it. A file Git does not
  track is left out of the flake source.
  - **repository-policy binaries:** they already refuse to run while any
    admitted input is untracked (`JAUNDER_REPOSITORY_POLICY_SOURCE`, per
    ADR-0225). That refusal now applies to `dev` gates, including
    `dev precommit`, whenever an untracked file sits under `nix/`, `flake.*`,
    `common/` or the policy crate. This is intended: the commit would omit that
    file.
  - **`mkHostTool` tools:** they do not refuse. A missing module fails the
    build. An untracked data file is simply absent, the same as for every Nix
    check of the working tree. CI checkouts have no untracked files.
  - The ADR draft amends ADR-0225's refusal clause for these tools rather than
    adding admission machinery.
- **mutants verification test.** `tools/coverage/tests/mutants_verify.rs`
  currently drives `.mutants-loop/verify.sh` and stubs `dev`'s `cargo build`, so
  that `dev` runs the binary under test.
  - The test instead puts a fake `dev` on PATH that runs `$CHECK_BIN` with the
    `coverage-check` arguments.
  - It still exercises the real script and the binary under test. It no longer
    depends on how `dev` obtains tools.
  - The launch-failure case becomes the fake `dev` exiting 127.
- **CI cache.**
  - setup-ci's "Cache host tools build" step drops `tools/target`.
  - It becomes opt-in through an input, and covers only `~/.cargo/registry` and
    `~/.cargo/git`, keyed on both `Cargo.lock` files.
  - The registry's contents are a function of the lockfiles, so saving only on a
    key miss is correct.
  - Only the jobs that still run host cargo opt in: `validate-host`
    (tools/product Clippy and tests) and the scheduled `mutants` job.
  - #1972's persisted kache store serves the host lane's tools compiles.
  - ci.yml's resource-baseline probes of `tools/target` are removed, with
    #2008's agreement, since #2008 owns ci.yml.
- **Ordering.** This lands after #1972, so the host lane always has a dependency
  cache. #2008 builds on top of it: lanes whose shells have no cargo need these
  packages.
- **Alternative rejected:** fixing the Actions cache with per-lane keys over the
  tools sources plus mtime restore (#2026). It adds key and freshness logic,
  keeps compiling in every lane, and blocks #2008's cargo-free shells.
- **Cost accepted:**
  - **Closure-changing rebuilds.** When a PR touches `common/`, `host/` or
    migrations (about 27 of the last 100 merges), the hermetic, coverage and
    test-checks lanes may each rebuild host-proof or coverage-check, in
    parallel. Kache cannot help here: Nix builds run sandboxed, with no
    persistent compile cache.
  - **Evaluation per package.** Each tool package costs one flake evaluation per
    `dev` process. That measured about 5.6 s warm on a clean tree. A dirty tree
    defeats Nix's evaluation cache, so it costs more locally, including in the
    precommit hook. Today a no-op cargo build costs about 0.5 s.
  - **Local tool edits.** Editing a tool locally means a Nix rebuild of its
    package. The tool's own `cargo test` and `cargo clippy` loop is unchanged.
  - **Other jobs.** `performance.yml` now Nix-builds `perf-data`.
- **Decision record.** A `jaunder-adr` draft records this:
  - It amends ADR-0251's "host workflows reach `system-artifacts` through
    `cargo run`" and its rule that tools are built from the checkout with cargo.
  - It amends ADR-0225's untracked-input clause for `mkHostTool` tools.
  - `docs/ARCHITECTURE.md` and `CONTRIBUTING.md` (the `<bin>-build` paragraph)
    are updated in the same change.

## Acceptance

Proof to execute:

- `dev` pytest covers `steps.tool`:
  - It returns `<nix out>/bin/<binary>`.
  - It builds each package once per process.
  - A failed build fails the step, with no fallback.
- Locally, `nix build` succeeds for each tool package. The gates that use them
  pass: `dev check`, and `dev test-local -p coverage --test mutants_verify` or
  the equivalent tools test.
- Smoke commands for tools that no gate runs:
  - `dev pr-watch <merged PR>`
  - `dev perf-compare` or `perf-data --help`
  - `dev host-proof trace-analysis --help`
  - `dev census`
  - org-timezone-manifest through its `dev` caller
- On a warm CI run of the PR (a re-run):
  - Every validation lane's trace shows its tool steps as Nix-build spans whose
    `jaunder.cache` is `substituted` or `reused`.
  - There are no `<bin>-build` cargo spans. `tools-tests-build` remains on
    purpose.
- No e2e job restores a cargo cache.
- Recorded on #1968:
  - Per-lane job and span times, before and after. "Before" is the first main
    run after #1972 lands.
  - One closure-changing measurement: a run whose change touches `common/`. Each
    lane's tool build substitutes the dependencies artifact and compiles only
    workspace crates, and its time is recorded per lane.

Deliverables to commit: the code, Nix and CI changes; the test change; the ADR
draft; the doc updates.

## Boundaries

- No change to which lanes run tools-clippy or the tools tests, or to what they
  check. #2010 owns that.
- No devShell or lane-shell changes; those belong to #2008. No kache changes;
  those belong to #1972.
- The only ci.yml changes are the cache opt-in inputs and removing the
  `tools/target` probes.
- No change to the in-derivation `mkToolsBin` producers.
