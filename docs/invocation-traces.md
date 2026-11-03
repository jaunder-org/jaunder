# Inspecting local `cargo xtask` traces

`cargo xtask test-local` can capture a Chrome Trace Event JSON timeline for
investigating one local workload. From the repository's supported Nix devshell,
run the normal packaged command with bounded selectors, for example:

```sh
cargo xtask test-local -- -p storage site_config_primitives_round_trip
```

The normal runner-owned trace is written to `.xtask/run/<id>.trace.json`; the
command reports its path. If runner publication is unavailable, the standalone
fallback is `.xtask/traces/<run-id>/trace.json`. Retain useful traces outside
`.xtask/run/`, which is runner history subject to pruning. Open the reported
JSON file at <https://ui.perfetto.dev> (or use Perfetto's Open trace file
control). The `cargo-xtask` logical track separates acquisition from execution,
with nested orchestration and CSR prerequisite phases. Instrumented phases
expose an `outcome`; root and execution slices also expose `exit_code` and
`exit_override` (not every slice has outcome metadata). Concurrent work is
retained on its observed tracks; the logical grouping is not an OS thread.

Capture preserves command behavior and raw trace records. Only bounded,
allowlisted workload metadata is admitted; no arbitrary argv, environment,
working directory, or subprocess output is captured. Developer-selected bounded
selectors are **not guaranteed secret-free**: do not include secrets in them.
Traces and logs may contain diagnostic data; handle them accordingly and avoid
committing transient captures.
