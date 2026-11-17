# Inspecting local `test-local` traces

Every `dev` command records a trace, and `dev test-local` is a good way to
investigate one local workload. From the repository's supported Nix devshell,
run it with bounded selectors, for example:

```sh
dev test-local -p storage site_config_primitives_round_trip
```

Each process in the run writes span records under
`.xtask/run/<run_id>.trace.d/`. When the run ends, `dev` merges them into
`.xtask/run/<run_id>.trace.json`, a Chrome trace JSON file. `dev trace summary`
prints the latest run's slowest spans, and `dev trace summary <dir>` reads a
named run directory. Open the JSON file at <https://ui.perfetto.dev> (or use
Perfetto's Open trace file control) to see the timeline. Copy traces you want to
keep beyond the worktree out of `.xtask/run/`, which is local run history.

Each of `test-local`'s steps is a span under `dev test-local`: `hooks-path`
(only when `core.hooksPath` needs writing), `host-artifacts-lock` (the wait for
the checkout's artifact lock), `csr-admission`, `csr-bundle`, `csr-prerequisite`
(with host-proof's manifest and inventory checks nested inside it),
`image-worker` and `nextest`. Each span records `jaunder.status` (`ok`,
`failed`, `signalled`, or `incomplete` when it has no end record or no parent).
Each `dev` step span also records its `process.exit.code` and its command line,
capped at 1 KiB. Spans that ran in parallel get separate Perfetto tracks.

Tracing never changes a command's output or exit status. The recorded command
lines include the selectors you pass, which are **not guaranteed secret-free**:
do not include secrets in them. Traces and logs may contain diagnostic data;
handle them accordingly and avoid committing transient captures.
