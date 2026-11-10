# Inspecting local `cargo xtask` traces

## Automatic passive receipts

Entering the existing `test-local` owner automatically attempts a private bundle
under `.xtask/workflow-traces/<invocation-id>/` after resolving the checkout.
There is no opt-in, executable interception, or global fallback sink. This
bounded integration observes only that owner, not native Just discovery, other
commands, or work before entry. It does not change execution, signals, waiting,
or required cleanup.

`start.json` and `completion.json` are separately published JSON objects using
temporary-file/no-overwrite publication. Each bundle is private (0700 on
supported Unix hosts; records 0600). Schema version 1 contains only an opaque
invocation ID, an explicitly supplied parent ID or null, fixed `test-local`
stage, `start`/`completion` observation, UTC Unix lookup milliseconds at start,
process-local elapsed milliseconds, outcome, observed exit code or null, and
capture health. UTC is for lookup, not cross-host duration measurement. Nested
and concurrent observations have separate IDs; no implicit active-root registry
or inferred parentage exists.

Start outcome is `unknown`. A valid completion reports the owner's observed
`success`/`failure` and exit code, separately from `capture_health: available`.
It does not independently establish a signal or timeout cause. A missing,
malformed or non-file completion is incomplete/unknown, including after abrupt
death; dropping the recorder never manufactures a terminal result. Missing
receipts prove neither skips nor absence of work. Source/tool/CI provenance and
phase-level receipts are not captured by this first slice and remain unknown.

Receipts accept no argv, selectors, environment, arbitrary paths, output or
error text. They are not copies of the richer existing timeline below. If start
or completion publication fails, stderr reports the fixed safe gap
`xtask-receipt: status=unavailable reason=capture-gap`; the actual workload
result and mandatory cleanup remain the owner's responsibility and are
unchanged. A previously published start remains useful incomplete evidence;
capture health cannot be recovered from an absent record.

Bundles are ignored worktree-owned evidence, separate from count-pruned
`.xtask/run/` history. There is no age expiry, collector, silent eviction, or
storage-capacity guarantee. Under storage pressure, report the capture gap and
request storage/export intervention, not a workload rerun. Explicitly export and
verify any evidence needed beyond separately authorized worktree removal. This
slice adds no export command, CI publication, or removal authority.

## Existing timeline

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
