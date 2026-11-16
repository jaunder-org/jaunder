# `dev` runner with step tracing — Implementation Outline

> Execute with dev-cycle-iterate. Outline needed because: this changes a durable
> architecture boundary. Every hook, CI lane and Nix check changes entrypoint,
> and two languages (Python and Rust) must keep writing one trace record format.

## Execution summary

Spec: `docs/superpowers/specs/2026-11-16-issue-1965-dev-runner-step-tracing.md`.

The first slice carries one real step end to end, `tsc`. It runs on the host as
`dev tsc` and in `checks.static-typescript` as `dev static tsc --sandbox`. This
tests the riskiest assumptions before anything is ported in bulk:

- the OpenTelemetry Python SDK can write start and end records
- the merged Chrome JSON loads in Perfetto
- the same `dev` code runs inside a Nix sandbox check

Next, host-proof's Rust spans join the same trace, and Nix cache classification
is added. Only then do the gates and the rest of the recipes move to `dev`.

The spec stages host-proof's OpenTelemetry work with the caller switch. This
outline pulls it forward into deliverable A instead, so the cross-language
record format is proven early. Until Task 6, the old `just test-local` path
loses host-proof's spans. That is a tracing loss only; nothing else depends on
those spans.

Just keeps driving the hooks and CI while `dev` is built up beside it, so every
commit through Task 5 passes the existing pre-commit gate. Task 6 switches all
callers in one commit, and Task 7 deletes Just.

#1958 (the `_host` split) and #1976 (pg-run) must be on `main` before Task 4.
The branch rebases onto them there.

## Cross-task contracts

These are fixed by Task 1 or Task 2, and every later task relies on them.

**Span record:** one JSON object per line, in
`.xtask/run/<run_id>.trace.d/<pid>-<random>.jsonl`, with these fields:

| Field        | Value                            |
| ------------ | -------------------------------- |
| `kind`       | `"start"` or `"end"`             |
| `trace_id`   | hex string                       |
| `span_id`    | hex string                       |
| `parent_id`  | hex string, or null at the root  |
| `name`       | span name                        |
| `time_ns`    | nanoseconds since the Unix epoch |
| `attributes` | object, present on end records   |

Each process appends one record per line to its own file and flushes after every
line, so a SIGKILL loses only what had not yet been written.

**Golden file:** `tools/dev/tests/golden/span-records.jsonl`. Both the pytest
suite and a host-proof Rust test read it.

**Span attributes:**

- `process.command_line`, capped at 1 KiB
- `process.exit.code`
- `jaunder.status`: `ok`, `failed`, `signalled` or `incomplete`. The merge sets
  `incomplete`.
- `jaunder.cache`, for Nix steps only: `reused`, `built` or `substituted`

**Environment:**

- `TRACEPARENT` and `TRACESTATE` carry the current span to child processes, via
  the W3C environment carrier.
- `JAUNDER_TRACE_DIR` names the run directory. The root is the process that
  finds it unset.

**Outputs:** the merged trace is `.xtask/run/<run_id>.trace.json`.
`dev trace summary [PATH] [--markdown]` reads a run directory and defaults to
the latest run.

**CLI shape:** each public Justfile recipe `foo a b` becomes `dev foo a b`. Each
private `_x` recipe becomes an internal function. The `recipes/static.just` and
`recipes/prettier.just` recipes become `dev static <name> [--fix] [--sandbox]`,
which runs in verify mode unless `--fix` is given. The #1958 groups become
`dev format`, `dev docs-shape`, `dev source-contracts`, `dev compile-checks`,
`dev elisp-checks` and `dev source-contract <name>`.

**Nix wiring:**

- In `nix/packages.nix`:
  - `devPythonDeps`: the one dependency list
  - `devPython`: `python3.withPackages devPythonDeps`
  - `packages.dev`: `buildPythonApplication` from the filtered `tools/dev/`
    source
  - `devShim`: the PATH shim
- In `nix/dev-shells.nix`: `devShim` on the `default`, `ci` and `ci-e2e` shells.
- In `nix/checks.nix`: `checks.dev-python`, running ruff, `mypy --strict`, and
  pytest under coverage at 90% branch.

## Task outline

Review deliverable A comprises the trace layer, the runner skeleton and the Rust
join.

- [x] **Task 1: tracer bullet.** `dev tsc` is traced on the host, and
      `dev static tsc --sandbox` runs inside `checks.static-typescript`.
  - Adds:
    - `tools/dev/`: the click group, the trace module, the internal step runner
      that records status, signal handling, and `trace summary`
    - the Nix wiring above, with the shim in all three shells
    - `__pycache__/` in `.gitignore`
    - `checks.dev-python` in the Justfile's `_static` set, so
      `validate-hermetic` builds it
    - ruff, mypy and pytest in the Justfile's `_host-tests`
  - Ports both `tsc` recipes:
    - the root one (Justfile:26-42) as `dev tsc`. It keeps
      `setup-e2e-node-modules` and drops the recorder.
    - `recipes/static.just`'s as `dev static tsc [--sandbox]`, without setup,
      because Nix runs setup itself (`nix/checks.nix:3505`)
  - `checks.static-typescript` runs `dev static tsc --sandbox`. Its source
    filter and the `assert` lines keyed on `recipes/static.just`
    (`checks.nix:3462`, `:3478`) are widened to the `tools/dev/` source.
  - `tools/traced-run/tests/tsc_recipe.rs` keeps passing against the Just recipe
    until Task 4.
  - Contract: the span record, the attributes, the environment and the CLI shape
    above.
  - Verification:
    - pytest covers:
      - the record writer, including records surviving a SIGKILLed writer
      - the merge, including `incomplete` for a missing end and for a missing
        parent
      - the summary, without summing overlapping children
      - an unwritable trace directory leaving output and status byte-identical
      - SIGTERM forwarding and the 128+n exit
    - `nix build` of `.#checks.x86_64-linux.static-typescript` and of
      `.#checks.x86_64-linux.dev-python` passes
    - `just check` passes
    - determine whether nixpkgs `perfetto` ships `trace_processor_shell`:
      - if it does, fix a host-side proof command that loads a merged trace and
        counts its spans, and record it in the task handoff
      - if not, use the ui.perfetto.dev screenshot method
      - perfetto stays out of every check closure either way
- [x] **Task 2: host-proof joins the trace.** `dev test-local -- <selector>`
      shows host-proof's phase spans nested under the `dev` step.
  - Replaces `tools/host-proof/src/invocation_trace.rs` (used by
    `bin/test-local.rs:23`):
    - a tracing-opentelemetry layer on the existing `opentelemetry` and
      `opentelemetry_sdk` 0.32 crates
    - a record writer for both start and end records
    - a `TRACEPARENT` extractor
  - `dev test-local` renders the step report in Python, which replaces piping
    the report through `scripts/step-report.jq` for that recipe.
  - Contract: the Rust writer passes the shared golden file.
  - Verification:
    - the Rust golden-file test passes
    - a pytest test runs `dev test-local` with a fake host-proof that emits one
      span, and asserts the parent link
    - `cargo xtask test-local -- -p host-proof` passes
    - one real `dev test-local -- -p storage` run produces a nested `trace.json`
- [ ] **Task 3: Nix cache classification.** The internal Nix-check step records
      `jaunder.cache` and adds no public subcommand.
  - Contract: none new.
  - Verification: pytest with a fake `nix` covers `reused`, `built` and
    `substituted`, and checks that non-Nix steps carry no `jaunder.cache`
    attribute. The real-run proofs are in Tasks 4 and 8.

Review deliverable B is the full port.

- [ ] **Task 4: gates on `dev`.** These gates run on `dev` with the same step
      order and fail-fast behaviour:
  - `check`, `check-no-test`, `prepush`, `validate-no-e2e`, `validate`
  - `ci-validate-{host,hermetic,test-checks,coverage}`
  - `precommit`, `precommit-markdown-host`, `precommit-broad-host`
  - the #1958 groups
  - Rebase onto main first, which must have #1958 and #1976.
  - Port the contract tests to pytest in `tools/dev/tests/`:
    - `just_workflows.rs`, including `:1007` (devtool launch reflects current
      source)
    - `static_recipes.rs`, `prettier_recipes.rs`, `tsc_recipe.rs`
    - `scoped_leptosfmt.rs`
    - the precommit parts of `native.rs`, `org_frontdoor.rs` and
      `ert_request.rs`
  - Delete only what tests Just or Justfile text: `just_workflows.rs:832` and
    the literal `just X` string assertions.
  - Contract: each gate's step list equals its current recipe order, with the
    Python checks added at the points Task 1 put them. The ported order tests
    assert this.
  - Verification:
    - pytest passes, and `dev check-no-test` passes
    - `dev ci-validate-hermetic` passes, and a second run shows `reused` for its
      Nix checks
    - `just check` still passes, because Just remains until Task 7
- [ ] **Task 5: remaining recipes on `dev`.** These run on `dev`:
  - `e2e-local`, `e2e-producer`, `e2e-reconcile` and the other e2e recipes
  - `perf` and `perf-compare`
  - `coverage` and `coverage-gate`, `doctests`, `pr-watch`, `issue-discover`
  - the qualification recipes
  - every other remaining Justfile recipe
  - Port the contract tests to pytest. They come from:
    - `e2e_workflow.rs`, `e2e_topology.rs`
    - `performance/tests/just_workflow.rs`, `performance_workflow.rs`,
      `performance/tests/baseline.rs:170-173,711-729`
    - `coverage_workflows.rs`
    - `devtool/tests/issue_frontdoors.rs:53,84,448`,
      `devtool/tests/pr_frontdoors.rs:44,63`
  - Contract: none new.
  - Verification:
    - pytest passes
    - `dev e2e-local <one spec>` passes and leaves a trace
    - one-off parity check, recorded in the task handoff rather than committed:
      every recipe in `just --summary` has a `dev` counterpart

Review deliverable C covers the switch, the removal and the proof.

- [ ] **Task 6: callers switch to `dev`.** In one commit, every row of the
      spec's caller table now calls `dev`:
  - `.githooks`
  - `ci.yml` and `performance.yml`
  - `git-index-safety` in `safety.rs`
  - `devtool` in `check.rs`
  - `scoped_leptosfmt.rs`
  - `scripts/e2e-workflow` (including its `step-report.jq` pipe at `:89`, `:92`)
    and `scripts/pr-land.sh`
  - the remaining `checks.nix` call sites, the source filters and their asserts,
    `coverageAuxiliarySource` and `packages.nix:774`
  - the path lists in `git.rs` and `checkout.rs`
  - the user-facing help and error strings
  - In the same commit, CI:
    - uploads `.xtask/run/` from every validation and E2E job (`if: always()`,
      hidden files included, 14-day retention)
    - removes uploads from jobs that produce nothing
    - appends `dev trace summary --markdown` to `$GITHUB_STEP_SUMMARY`
  - Contract: none new.
  - Verification: `dev validate` passes, and the pre-commit hook runs
    `dev precommit`.
- [ ] **Task 7: Just is gone.**
  - Delete:
    - `Justfile` and `recipes/`
    - `pkgs.just` from every devShell
    - traced-run's launcher, span, recorder and workflow-folding code, and its
      tests
    - `scripts/step-report.jq`, once its remaining users are gone: the test
      copies in `coverage_workflows.rs:27` and `e2e_workflow.rs:31`, and the
      reference in `host-tool/src/report.rs:3`
    - `scripts/run-report`, if no user remains after the port
  - Add the committed `rg` guard from Acceptance 7 as a repository-policy check.
  - Update CONTRIBUTING.md, covering:
    - the gate commands
    - the observation and trace policy at `:941-947` and `:1165-1239`
    - the report-rendering references at `:843` and `:1424`
    - what `cache` does and does not claim
  - Project the ADR draft into ARCHITECTURE.md (jaunder-adr-projection).
  - Contract: none new.
  - Verification:
    - `dev validate` passes
    - the Rust coverage gate passes with no lowered threshold. If it drops, add
      Rust unit tests.
    - the `rg` guard is clean
- [ ] **Task 8: acceptance proof.**
  - Local runs:
    - `dev check` succeeds
    - `dev check` with an injected failure
    - `dev check` with a SIGKILLed child
    - each run's summary and trace load in Perfetto, using the Task 1 method
  - Cache: a real Nix check shows `built` or `substituted` after an input
    change, following its `reused` run.
  - After the PR's CI run: download one validation artifact and one E2E
    artifact, run `dev trace summary <dir>` on each, and check the step summary
    tables.
  - Post baselines for `dev prepush` and `dev check` on #1965.
  - List the commands in the out-of-repo `jaunder-develop` skills that need
    updating, for the owner.
  - Verification: the evidence is attached to the PR, and the baseline comment
    is posted.

## Risk checks

- **Fixed per-invocation behaviour.** These come from the Justfile's top-level
  settings and must hold on every `dev` call:
  - `SCCACHE_BASEDIRS` is computed on every call
  - `RUSTC_WRAPPER=sccache` and `CARGO_INCREMENTAL=0` are set
  - inherited `GIT_*` overrides are cleared before calling git
- **Ported recipe behaviour:**
  - The `leptosfmt` recipe's tools build and PATH prepend is preserved.
  - The sandbox cargo environment is admitted before any work: `--offline`,
    CARGO_HOME from the product or tools home, `CARGO_NET_OFFLINE=true`.
  - Arguments are passed as argv lists, never through a shell string. Prettier
    globs arrive unexpanded.
- **Precommit:** `git-index-safety` still selects the host step by change class,
  and precommit's exit status reaches the hook unchanged.
- **Hermetic checks:** every check whose filter changes keeps its `assert` lines
  true, and no check gains the whole repository as a source.
- **Python and Rust records:** the two writers agree, and only the golden file
  defines the format.
- **Tracing never changes outcomes:** no step's stdout or stderr gains trace
  output, and tracing failures produce a single stderr warning.
