# Account for untraced and mislabelled time in CI traces (#2005)

## Outcome

A CI job's `dev` trace accounts for the job's wall time, and each tool span
measures only the tool. Compiling is timed as separate build steps. A Nix step
says which output it produced, how much it fetched from substituters, and how
long it spent evaluating. The root span identifies the runner.
`dev trace summary` shows all of this, and one CI run demonstrates it on the
issue.

## Approach and tradeoffs

The approach extends the existing span records (`trace.py`, `steps.py`,
`summary.py`) and `setup-ci`. There is no new trace framework, and the record
format gains attributes only.

**1. Time before `dev`.** `setup-ci`'s first step appends
`JAUNDER_CI_JOB_START_NS=$(date +%s%N)` to `$GITHUB_ENV`. `nix develop` passes
it through. The root `dev` process alone (the one that finds `JAUNDER_TRACE_DIR`
unset) records the value on its span as `jaunder.ci.job_start_ns` when it is a
decimal integer. A missing or malformed value records nothing.

`dev trace summary` then shows a `before dev` row at depth 0, outside the row
limit like a failed span. It covers the time from the mark to the root's start,
and is omitted when that interval is not positive. A later `dev` root in the
same job inherits the same mark, so its row honestly spans everything since the
job started.

The row covers setup-ci's cache restores, every job step before `dev`, and
devShell entry. GitHub's step timings split it further, so one mark is enough
(decided).

Two parts of the job stay untraced, and the demonstration states their size:

- before the mark: "Set up job" and checkout;
- after the root ends: the trace-summary step, artifact uploads and post-job
  cleanup.

**2. Compile apart from run.** Every tools-workspace binary that `dev` now
launches with `cargo run --quiet` is built first as its own step, then run
directly from `tools/target/debug/<bin>`. This covers the gate tools and the
one-shot front doors alike (census, coverage-check, the host-proof reports,
perf-data, pr-observer).

Building is
`cargo build --locked --quiet --manifest-path tools/Cargo.toml --target-dir tools/target -p <pkg> [--features …] --bin <bin>`,
always naming the bin (`pr-observer` included), in a step named `<bin>-build`.
This generalizes the existing `scoped-leptosfmt-build` step, which becomes one
more instance.

A `dev` process builds each binary once. `tools-tests` rebuilds package binaries
in `tools/target/debug` with unified features, so it forgets what was built.
Running directly, instead of keeping `cargo run`, is what guarantees the tool
span holds no cargo work at all.

Test steps get a build step with the run's own arguments and environment, so the
run that follows finds everything fresh:

- **`tools-tests-build`** runs `cargo test --no-run` with `tools-tests`'
  arguments.
- **`nextest-build`** runs `cargo nextest run --no-run --workspace` before
  `nextest`, only when `dev` chooses the arguments itself. That is every gate,
  including CI's host lane. Hand-typed `dev test-local ARGS` keeps one combined
  step: nextest rejects some run options alongside `--no-run` (measured:
  `--no-fail-fast`), and parsing user arguments is not worth it for an
  interactive command.
- **Shared environment.** `image-worker`, `nextest-build` and `nextest` share
  one environment: `JAUNDER_CSR_BUNDLE_DIR`, `JAUNDER_TEST_EXECUTABLE`, and
  `CFLAGS` set empty when absent. Today `server/build.rs` (which tracks
  `JAUNDER_CSR_BUNDLE_DIR`) and aws-lc-sys (which tracks `CFLAGS`) rebuild
  alternately between `image-worker` and `nextest`. Sharing the environment
  removes that.
- **`pg-run`.** The build runs without `pg-run`. Its URLs are the only
  difference, and no build script reads them.

**3. Nix step cost and origin.** A flake step (`nix_build`/`nix_check`) gets two
child spans:

- **`eval`** runs `nix path-info --derivation <flags> <installable>`. Its
  stdout, the `.drv` path, is captured and never shown, while its stderr reaches
  the user. It runs under the same signal supervision as any step. If it fails,
  the step ends `failed` (or `signalled`) with its exit code, and no build
  starts. A dirty-tree warning now prints twice, which is accepted.
- **`build`** runs `nix build <installable>` as today. It reuses the eval cache.

The flake is therefore still evaluated once. Measured on `static-code` from a
cold cache: `eval` took 16.1 s, then `nix build` took 0.2 s. Repeated `eval`
calls took 0.07 s from a clean tree and 0.2 s from a dirty one.

An installable that starts with `/` is a derivation path (`perf.py`), so it has
no `eval` child, and its `.drv` is the installable without its `^outputs`
suffix. Building the printed `.drv` instead of the flake installable would also
avoid a second evaluation. But it drops the flake's `nixConfig` substituter
(jaunder-org Cachix) locally, which is why the eval cache is the mechanism.

The registration start second is taken after `eval`, just before `build`. That
is now the reference for `jaunder.cache` too, so downloads made during `eval`
(flake inputs) are not attributed to the build. After a successful build that
prints one path, the step records:

- `jaunder.nix.output`: the printed output path.
- `jaunder.nix.fetched_bytes`: only when `jaunder.cache` is not `reused`. It is
  the sum of `narSize` over the paths in the build closure
  (`nix-store --query --requisites --include-outputs <drv>`, then
  `nix path-info --json --json-format 1 --stdin`) that are not `*.drv`, were
  registered from the start second on, and are not `ultimate`. That means bytes
  from substituters, unpacked. Fixed-output downloads are `ultimate` and are
  excluded.

Skipping reused outputs keeps warm local runs at #2009's speed. A path
registered in the start second by `eval` or by the previous step can be counted,
which is a small accepted overcount. A failed query records nothing and never
changes the step's result or output.

Costs:

- **CI:** about 1.4 s per non-reused step (5,017 paths, measured), so roughly 15
  s per hermetic run at worst.
- **Warm local runs:** about 0.1 s more per Nix step, for its process and `eval`
  launch.
- **Rejected alternative:** a whole-store query (`nix path-info --all`) took 57
  s and produced 270 MB locally.

**4. Runner identity.** The root span carries:

- `host.cpu.model.name`: the first `model name` in `/proc/cpuinfo`.
- `jaunder.host.nproc`: `len(os.sched_getaffinity(0))`, as `nproc` counts.
- `cloud.provider`: `aws` or `azure`, from `/sys/class/dmi/id/sys_vendor`
  ("Amazon EC2" or "Microsoft Corporation"). Any other vendor records nothing.

A missing or unreadable source omits its attribute silently, with no warning and
no change to the run. A local Hyper-V VM also reads as `azure`, which is
accepted and documented. Region is omitted: the runner exposes it only in its
log header (decided).

`summary.py` gains a `Fetched` column (MiB) and the `before dev` row. The root
span's runner attributes appear in one header line above the table. The docs are
updated to match:

- ARCHITECTURE.md and CONTRIBUTING.md describe the new attributes and the
  build-step convention.
- The setup-ci cache comment ("Host tools run through `cargo run`") and
  `nix_build`'s docstring are corrected.

## Acceptance

Proof to execute:

- `tools/dev` tests cover:
  - the root attributes: present, absent, malformed, and missing sources;
  - the `before dev` row: shown, omitted when the interval is not positive, and
    kept beyond the limit;
  - the runner header line and the `Fetched` column;
  - tool build steps: built once and then run directly, rebuilt after
    `tools-tests`, and a build failure stopping the gate before the tool runs;
  - `tools-tests-build` and `nextest-build` receiving their run's arguments and
    environment, and hand-typed `test-local` args getting no build step;
  - the `eval` and `build` child spans: success, eval failure (no build), eval
    output captured, and no `eval` for a derivation installable;
  - `jaunder.nix.output`;
  - `jaunder.nix.fetched_bytes`: recorded, skipped for reused output, `.drv` and
    `ultimate` paths excluded, and a failed query recording nothing.
- The coverage gate stays at 100% (branch).
- `dev ci-validate-hermetic` with a warm store stays within about a second of
  #2009's 1.5 s median (three runs).
- One CI run of the PR has `dev trace summary` output posted on #2005. For the
  hermetic, host and coverage jobs it shows:
  - job wall time (GitHub API) against `before dev` plus the root, with the
    remaining gap stated in seconds;
  - the build steps separate from the tool and test spans;
  - `eval` spans, with `build` spans showing no second evaluation;
  - `Fetched` for substituted steps;
  - the CPU model, nproc and cloud.

## Boundaries

- No new trace backend, no OTLP export, and no change to the record format
  beyond new attributes.
- No network lookup of region, and no substituter origin per path.
- No build split for hand-typed `dev test-local ARGS`.
- Removing the e2e lanes' unneeded `tools/target` restore is #1968.
- E2E phase spans and per-test spans are #1999.
- The validation jobs' own steps are not restructured.
