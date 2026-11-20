# ADR-DRAFT: Dev trace spans time one thing each and account for the CI job

- Status: proposed
- Date: 2026-11-20
- Issue: [#2005](https://github.com/jaunder-org/jaunder/issues/2005)

## Context

The `dev` traces from [ADR-0251](../0251-dev-runner-replaces-just.md) made step
time visible, but on CI they hid or mislabelled the largest costs. CI run
44383337174 showed several of them.

- **Untraced time before `dev`.** The root span started inside
  `nix develop .#ci`, after setup-ci's cache restores and devShell entry. The
  hermetic job took 243 s of wall time, but its root span covered only 108.9 s.
- **Compiling counted as tool time.** Host tools ran through
  `cargo run --quiet`, so compiling them was counted as tool time.
  `coverage-policy` took 58.6 s for 2.5 s of work, and `tools-tests` took 142.4
  s, about 112 s of it compiling. ADR-0251 recorded that host workflows reach
  `system-artifacts` "through `cargo run`".
- **No cost or origin for Nix steps.** A Nix step's evaluation was folded into
  its build, and a `substituted` step recorded neither its output nor what it
  fetched.
- **No runner identity.** Traces did not record the runner, although the suite
  runs about 40% slower on AWS-hosted runners than on Azure-hosted ones.

## Decision

A span times one kind of work, and a CI trace accounts for the job around it. No
new trace framework is added; the existing span records gain attributes only.

- **Tools are built, then run.** Every tools-workspace binary `dev` launches is
  built first, as its own step `<bin>-build`
  (`cargo build --locked --quiet --manifest-path tools/Cargo.toml --target-dir tools/target -p <package> --bin <bin>`).
  It is built once per `dev` process and then run directly from
  `tools/target/debug/<bin>`. No host workflow reaches a tools binary through
  `cargo run`. This amends the `system-artifacts` sentence of ADR-0251.
- **Tests compile apart from their run.**
  - `tools-tests-build` runs `cargo test --no-run` with `tools-tests`' own
    arguments.
  - `nextest-build` runs `cargo nextest run --no-run --workspace` with
    `nextest`'s environment, but only when `dev` chooses the selection.
  - `image-worker`, `nextest-build` and `nextest` share one environment, so
    build scripts that track it do not rebuild between them.
  - Hand-typed `dev test-local ARGS` compile inside `nextest`, because nextest
    refuses some run options with `--no-run`.
- **A flake step evaluates, then builds.**
  - An `eval` child (`nix path-info --derivation`) evaluates the installable and
    leaves the evaluation cached.
  - A `build` child (`nix build` of the same installable) reuses that cache, so
    the flake is evaluated once and the evaluation is timed.
  - Building the printed `.drv` was rejected: it would drop the flake's
    `nixConfig` substituter.
- **What a Nix step records.** The step records its output path. Unless the
  output was reused, it also records the bytes substituters supplied to its
  build closure since the build started, judged by store registration time and
  `ultimate`, as `jaunder.cache` already was.
- **The root records its runner and the CI job's start.** The root span records
  the CPU model, the usable CPUs and the cloud provider from the DMI vendor,
  wherever the machine exposes them. In CI it also records the job's start,
  which setup-ci's first step exports. Region is not looked up over the network.

## Consequences

- **Build steps.** A gate shows a `<bin>-build` step the first time each tool is
  used. A step that rebuilds tools binaries some other way must make `dev`
  forget what it built.
- **Extra Nix queries.**
  - Every flake step runs one extra `nix path-info` process, about 0.06 s warm.
  - A step whose output was not reused also runs a closure query, about 1.4 s
    for a 5,000-path closure.
- **What `fetched_bytes` measures.** It is unpacked NAR size, not download size.
  Fixed-output downloads are excluded, and a path registered in the start second
  by another step may be counted.
- **Hyper-V.** A local Hyper-V machine reports `azure`.
- **What stays untraced in CI.** "Set up job" and checkout before the mark, and
  the steps after the root ends.
