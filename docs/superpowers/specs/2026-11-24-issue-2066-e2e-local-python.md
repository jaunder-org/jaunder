# Issue #2066: move the `e2e-local` lifecycle into `dev` as Python

## Outcome

`dev e2e-local` keeps its command line and its behaviour: browsers, filters,
visual update mode, per-worker server instances (#2076), retained capture, panic
gates, the step summary and exit codes. It now runs as Python inside the `dev`
process, and each lifecycle phase appears as a span in dev's trace. The Rust
`e2e-local` executable and its private support modules are deleted. With them
goes host-proof's direct use of `tokio`, `processkit`, `xshell` and `jiff`. The
crates stay in the tools lockfile, because traced-run, repository-policy and
`host` still use them.

## Approach and tradeoffs

**Where it runs.** A new `tools/dev/dev/e2e_local.py` is registered in `cli.py`
as an ordinary dev command, removed from the host-proof forwarding dictionary.
It has the same options and conflicts:

- `--browser chromium|firefox|webkit`;
- an optional `spec-or-file:line` filter;
- `--update-visual-snapshots`, which conflicts with both.

The help text keeps its guidance: focused proof, rerun the same command, and
hermetic `dev e2e-producer` for parity. The command runs under `steps.gate`, so
it produces one trace.

**Report.** The Rust binary printed one JSON `CommandResult`, and its only
reader was dev's `report.render`. The Python module builds the same step list as
a dict and prints it through the renderer. `report.render`'s parsing is split
from a public render-from-dict function, so the dict is never serialised only to
be parsed again. Each step is also recorded as a span.

Each browser lifecycle reports its steps in Rust's order:

1. `e2e-local-<b>-collector`
2. per instance: `-instance-<i>-server` and `-instance-<i>-seed`
3. `-collector-shutdown`
4. `-capture`
5. `-playwright`
6. per instance: `-instance-<i>-panic-gate`

Rust's failure-only steps are kept by name: `-instance-<i>-tmpdir`,
`-instance-<i>-server-log`, `-trace-context` and `-instance-map`.

The setup refusals (`e2e-local-platform`, `e2e-local-env`) print as a one-step
failed summary. Admission and the Nix build are dev steps of their own, so a
failure there ends the gate the way it ends every dev workflow, without a
summary. `e2e-local-signals` and `e2e-local-artifacts` cease to exist; the dev
steps `e2e-local-admission` and `e2e-local-build` replace the latter.

Exit codes:

- 0 when every step passes;
- 1 when any step fails;
- 128+n on a signal, raised as `StepFailed(code, signalled=True)`, so the gate
  span records `signalled`.

Every child's stdout goes to stderr, so stdout carries only the summary. This
amends ADR-0247, which lists e2e-local as a tool that writes a JSON report.

**Setup** runs under the gate's normal signal semantics.

1. **Refusals.** The run refuses:
   - a set `JAUNDER_CSR_BUNDLE_DIR`;
   - visual update mode off Linux;
   - a `JAUNDER_E2E_WORKERS` that is not a positive integer (the default is 1).
2. **Admission.** `admit-inputs .#sandboxArtifacts[ReleaseCsr].sourceAdmits`, as
   `csr-admission` already does.
3. **Build.** Register removal of a fresh per-run GC root,
   `.xtask/gcroots/e2e-local-<unique>`, before building, so the removal runs on
   every exit path and tolerates a missing link. Then build the artifacts:
   - `.#sandboxArtifacts`, or `.#sandboxArtifactsReleaseCsr` in visual update
     mode;
   - through `steps.nix_build`, with `(*NIX_FLAGS, *NIX_LOCK_FLAGS)`;
   - with its out-link at that root. A small `nix_build` extension accepts a
     caller-chosen link, so concurrent runs never repoint each other's root.

   `bin/jaunder` and `bin/test-support` must resolve to executable regular
   files.

After the build, e2e-local installs its own signal handler once, for the rest of
the run (below).

**Spawning.** One helper starts every child:

- in its own process group (`start_new_session`);
- with the environment
  `steps.changed(steps.changed(trace.child_environment(), WITHOUT_GIT_OVERRIDES), <child changes>)`,
  which is what Rust's children got as children of a dev step;
- with stdout and stderr drained continuously by reader threads. Each line goes
  to stderr, and server and collector stderr are also teed into their capture
  logs. Nothing is buffered, so a child that writes more than a pipe holds
  cannot block. This is #2076's hang fix.

The helper never raises on a non-zero exit; callers record steps.
`test-support capture-path`, `seed-e2e` and `verify-no-panics` all run through
it, so a terminal Ctrl-C never reaches them directly, and every instance's panic
gate is attempted and recorded.

**Lifecycle.** This ports #2076's Rust flow with its order and failure handling.
Each planned browser runs once: one browser by default, or Chromium then Firefox
in visual update mode.

- **Retained capture.**
  - Directory: `.xtask/e2e-local/run-XXXXXX/<browser>/capture`, kept.
  - Trace path: from `test-support capture-path otel`, run with
    `JAUNDER_CAPTURE_DIR` set and the OTLP endpoint variables unset. Its parent
    must be the capture directory.
- **Collector.**
  - Command:
    `otelcol-contrib --config end2end/otel-collector.yaml --config nix/otel-processors.yaml --set=service.telemetry.metrics.level=none`.
  - Environment: `OTELCOL_GRPC_ENDPOINT`, `OTELCOL_HTTP_ENDPOINT` and its temp
    `JAUNDER_CAPTURE_DIR`.
  - Stdin is empty, and its stderr is teed to `otelcol-contrib.stderr.log`.
  - Ports: two freshly reserved loopback ports.
  - Ready when both ports accept connections within 5 s.
  - A readiness failure stops it with zero grace. If its stderr says
    `address already in use`, the start is retried with new ports, up to 8
    attempts.
- **Instances.** N instances, one per worker. Each one gets:
  - a temp storage directory;
  - `sqlite:<storage>/jaunder.db`;
  - the capture subdirectory `instance-<i>/`, with `server-stderr.log` inside.

  Each runs `<jaunder> serve` with this environment:
  - `JAUNDER_BIND=127.0.0.1:0`;
  - `JAUNDER_STORAGE_PATH`, `JAUNDER_DB` and
    `JAUNDER_RUNTIME_FILE=<storage>/runtime.json`;
  - `JAUNDER_CAPTURE_DIR=<instance capture>`;
  - `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT=http://<grpc>`;
  - `RUST_LOG=info`.

  It is ready when both of these hold:
  - `runtime.json` appears within 15 s and its `pid` matches the spawned child;
  - an HTTP GET of `http://<ip>:<port>/` succeeds within 30 × 500 ms.

  Both polls check for a pending signal on every tick and abort with "E2E server
  startup interrupted". Each instance is then seeded with
  `<test-support> seed-e2e --db <db> --test-support-bin <ts> --jaunder-bin <j>`,
  with `JAUNDER_STORAGE_PATH`, `JAUNDER_CAPTURE_DIR` and the OTLP endpoint set.
  An instance whose process exists joins teardown, ready or not.

- **Playwright.** A single
  `playwright test --project <browser> --reporter=html,line [filter]` runs from
  `end2end/`. In visual mode the arguments are
  `--grep @visual --update-snapshots` instead. Its environment:
  - `JAUNDER_E2E_INSTANCES`: `[{baseUrl, db, storagePath, captureDir}]`, with
    `localhost` base URLs;
  - `JAUNDER_E2E_PRODUCER_INPUT`;
  - `JAUNDER_TEST_EXECUTABLE`;
  - `JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://<http>/v1/traces`;
  - `JAUNDER_E2E_TRACE_ID`: a fresh, non-zero 128-bit hex value;
  - `JAUNDER_E2E_WORKERS`;
  - `PLAYWRIGHT_HTML_OPEN=never`;
  - `PATH` with the selected `bin/` prepended.
- **Teardown.**
  1. Stop each server in turn.
  2. Stop the collector. It must still be alive when shutdown starts (a 1 s
     probe; otherwise "exited prematurely") and must exit 0.
  3. Copy the capture, only when every writer has stopped. Otherwise keep the
     live source and fail `capture`.
  4. Run `verify-no-panics --capture-dir <i> --server-log <i>/server-stderr.log`
     for each instance.

The `instance-<i>` name is spelled in Python, as `nix/checks.nix` already spells
it. `host::capture::instance_directory` remains the Rust readers' definition.

**Readiness identity.** Rust also compared `runtime.json`'s `start_time` with
the child's. In Python it is redundant, for two reasons:

- dev holds the server's `Popen` unreaped until teardown, so its pid cannot be
  reused;
- `runtime.json` lives in a fresh per-run directory, so it cannot be stale.

The port checks the pid alone. That removes the platform split, and with it the
aarch64-darwin libproc loss that ADR-0261 accepts. This narrows the issue's "pid
and start time" wording.

**Stopping and signals.** We decided this in the design interview. No Python
library contains a process tree the way processkit's cgroup does, and macOS has
no usable equivalent. The port therefore uses process groups plus `psutil`.

Stopping a child:

1. Snapshot its descendants with `psutil`. This catches Playwright's browsers,
   which run in their own groups (`detached: true`).
2. Send TERM to its group.
3. Wait up to 10 s.
4. KILL the group, and every snapshot process still alive.
5. Reap the child.

The e2e handler covers INT, TERM and HUP (the gate's handled set), and keeps
Rust's semantics:

- **While a seed or Playwright consumer runs:** the first signal is recorded as
  the exit code and stops the consumer as above. A further signal during that
  stop escalates it to KILL at once.
- **During server stops:** any signal, including the first, sets the exit code
  if none is set and escalates the stop under way to KILL.
- **Collector shutdown** is not escalated, as in Rust.
- **Readiness polls and between steps:** the first signal is noticed there. No
  further step starts, and teardown always runs in full.

Exit codes are 130, 143 or 129. Teardown is registered on an `ExitStack`, so
exceptions also run it.

**Provenance.** The file is
`.xtask/diagnostics/e2e-host-local/producer-checkout-host-local.json`.

When it is written:

- At start, both `producer-input-host-local.json` and
  `producer-checkout-host-local.json` are cleared.
- It is written before Playwright starts and rewritten at exit.

It keeps the fields that carry information:

- `schema_version` 1, `lane`, `request_started_unix_ms`, `recorded_unix_ms`;
- `requesting_checkout` and `checkout_after`, each holding `head`, `tree`,
  `parents`, `reference` and `clean`;
- `ci_request_annotations` and `acquisition_errors` (bounded as today);
- `build_succeeded`, meaning the whole run succeeded without a signal;
- `marker_destination_cleared`;
- `input_attestation`, with the existing precedence: `marker-clearing-failed`,
  `checkout-unavailable`, `dirty-committed-base-only`, `checkout-changed`, and
  otherwise `selection-unavailable`.

Dropped are the fields that are constant on the host: `selection_before_build`,
`selected_marker_*`, `guest_input`, `execution_observation` and
`original_producing_runtime`. `visual-diagnostics.ts` reads none of them.

**Removed.**

- In `tools/host-proof/src`:
  - the executable: `bin/e2e-local.rs`;
  - the lifecycle: `steps/e2e_local.rs` and `steps/e2e_local/`;
  - its support modules: `steps/host_server.rs`, `steps/host_signals.rs`,
    `steps/process.rs` and `e2e_provenance.rs`.
- `E2eLocalBrowser`, `e2e-local` in `hostProofBin`, and host-proof's direct
  `tokio`, `processkit`, `xshell` and `jiff` dependencies.

`checkout.rs` stays, because `source-diagnostics` uses it. host-proof's
`core.hooksPath` setter is not ported, because it is not part of the lifecycle.

**Costs accepted.**

- A new `psutil` dependency, plus `types-psutil` for `mypy --strict`.
- **macOS:** no containment beyond process groups and the snapshot. That is the
  same as today's processkit fallback.
- **Linux:** a descendant re-parented to init before its stop began (for
  example, a browser orphaned by a crashed Playwright worker) is no longer
  contained. processkit's cgroup, when writable, did catch those.

## Acceptance

Proof to execute. `tools/dev` keeps its 100% coverage gate.

- **pytest units** cover:
  - Playwright arguments for each browser, with and without a filter, and in
    visual mode;
  - the instance map JSON;
  - worker parsing and the platform and `JAUNDER_CSR_BUNDLE_DIR` refusals;
  - `runtime.json` matching: wrong pid, missing key and malformed JSON;
  - the collector bind-collision retry;
  - every attestation branch.
- **pytest process tests** use fake `nix`, `admit-inputs`, `jaunder`,
  `otelcol-contrib`, `test-support` and `playwright` scripts, and show:
  - a consumer writing 1 MiB to stdout runs to exit;
  - INT and TERM yield 130 and 143, with no fake child left alive and the GC
    root removed;
  - a signal during readiness aborts startup and still stops the server;
  - a second signal escalates a TERM-ignoring consumer to KILL well inside the
    10 s grace;
  - a signal during a server stop escalates it;
  - a detached grandchild of a killed consumer is no longer alive afterwards;
  - a startup failure of instance 1 still stops instance 0 and panic-gates it;
  - one failing panic gate does not skip the next instance's gate;
  - the capture is not copied while a writer is alive;
  - step names and order match the list above.
- **Real runs:**
  - `traced-run -- dev e2e-local auth.spec.ts` passes;
  - a site-global spec passes with `JAUNDER_E2E_WORKERS=2`;
  - Ctrl-C during Playwright exits 130, and no `jaunder`, collector or browser
    process survives;
  - `dev e2e-local --update-visual-snapshots` completes with no PNG changes.
- **`dev check`** is green.

Deliverables to commit:

- **Code.** The module and its tests. The `cli.py` registration and forwarding
  removal. The `nix_build` link extension and render-from-dict.
- **Dependencies.** `psutil` in `devPythonDeps` and in
  `tools/dev/pyproject.toml`, and `types-psutil` in `devPythonCheck`.
- **Dev test updates.** The `e2e-local` forwarding cases in
  `tools/dev/tests/test_workflows.py` and `conftest.py` change.
- **Rust.** The deletions and the `Cargo.toml` and lockfile updates.
- **ADR amendments.**
  - ADR-0246, lines 64-69: e2e-local is dev Python, with no ProcessKit/Tokio
    adapters.
  - ADR-0247: e2e-local is no longer a JSON-report tool.
  - ADR-0258: `host-proof` no longer ships `e2e-local`.
  - The draft `per-worker-e2e-server-instances.md`: its `HostServerSession`
    wording.
- **Docs.** Name the Python module in `docs/ARCHITECTURE.md` (around 4299 and
  4337-4341, 5372, 5712), `CONTRIBUTING.md`, `docs/observability.md`, and the
  `nix/dev-shells.nix` and `nix/nixos.nix` comments.

## Boundaries

- **Out of scope:** no change to `end2end/`, the Nix VM lanes,
  `scripts/e2e-workflow`, the lane catalog, or #2076's `JAUNDER_E2E_INSTANCES`
  contract.
- **No new options:** Playwright flags are still not forwarded.
- **host-proof's other binaries stay:** `csr-prerequisite`, `admit-inputs` and
  the rest (#1974).
- **Untouched:** `test-local`, `pg-run` and the sandbox.
