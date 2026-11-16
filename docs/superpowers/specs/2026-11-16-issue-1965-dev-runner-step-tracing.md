# Issue #1965 — `dev` runner with OpenTelemetry step tracing

## Outcome

A developer or CI job runs a gate such as `dev prepush` and afterwards can read
how long each step took, how it ended and whether its Nix output came from the
cache, with `dev trace summary`. They can also open the same run as a timeline
in Perfetto. Just is gone: one Python command, `dev`, replaces the Justfile and
`recipes/*.just` for developers, hooks, CI and the Nix sandbox.

## Why the direction changed

Milestone 33 and this issue's text said to build on traced-run's span mechanism
and add no second one. The owner reopened that. Just has no per-run or
per-recipe hook. `set shell` applies to every line (including backticks), and a
line can run with no root span and silently record nothing. Wrapping each recipe
line therefore traces unreliably. In a Just, bash and Python comparison
(artifact JBMbkE6f8bHd9c8ooknqXf), Python with click made steps, nesting, exit
status and argv handling ordinary code. Bash still needed a span helper, and
invoke loses argv quoting. Tracing uses the OpenTelemetry API/SDK instead of
custom span plumbing. The ADR draft records this decision
(`docs/adr/drafts/dev-runner-replaces-just.md`).

## Approach

**One command.** `tools/dev/` holds a Python package. Its dependencies come from
the pinned nixpkgs: python 3.14, click 8.3 and opentelemetry-api/-sdk 1.43, plus
ruff, mypy, pytest and coverage for its own checks. It reaches its two kinds of
caller differently:

- **On the host,** the devShell (`default`, `ci` and `ci-e2e`) puts `dev` on its
  PATH as a `writeShellApplication` shim. The shim finds the checkout with
  `git rev-parse --show-toplevel` and runs `dev` from that checkout's
  `tools/dev/` with `python -P`. It adds `tools/dev/` to that one interpreter's
  `sys.path`, not to `PYTHONPATH`, so steps inherit the caller's environment
  unchanged. The interpreter is a `python3.withPackages` environment built from
  the same dependency list as the sandbox package, so there's no virtualenv.
  Hooks and gates always run the working tree's code, the way Just reads the
  Justfile today. Bytecode caching stays on, and `__pycache__/` is added to
  `.gitignore`.
  - **Why not `nix run .#dev`:** measured on Nix 2.34.8 with hyperfine:
    - Nothing changed since the last call: the shim took 92–98 ms in every case,
      and `nix run .#dev` took about 185 ms.
    - First call after any tracked-file edit: `nix run` took 1.3 s.
    - First call after an edit under `tools/dev/`: `nix run` took 4.0 s, because
      the package rebuilds.
- **In the sandbox,** the Nix checks that ran a `recipes/*.just` recipe use a
  `dev` built from a narrowly filtered `tools/dev/` source. Today those checks
  rebuild only when their recipe file changes. Now any edit to `tools/dev/`
  rebuilds all of them, and we accept that cost.

Each Justfile recipe becomes a `dev` subcommand with the same name and
arguments, for example `dev check`, `dev test-local -- -p storage foo`,
`dev ci-validate-host` and `dev perf medium`. Private `_x` recipes become
internal functions. #1958's host groups (`format`, `docs-shape`,
`source-contracts`, `compile-checks`, `elisp-checks`, `source-contract <name>`)
port with their names and order. A step is a function call. Composition is
ordinary Python, and the steps themselves are unchanged.

Callers that change:

| Caller                                                                                                                                                                                                                         | Today                                                  | After                                    |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------ | ---------------------------------------- |
| `.githooks/pre-commit`, `pre-push`                                                                                                                                                                                             | `just precommit` / `just prepush`                      | `dev precommit` / `dev prepush`          |
| CI `ci.yml`, `performance.yml`                                                                                                                                                                                                 | `nix develop .#ci -c just …`                           | `… -c dev …`                             |
| `git-index-safety precommit`                                                                                                                                                                                                   | `just precommit-*-host`                                | `dev precommit-*-host`                   |
| `devtool check ert`, `scoped-leptosfmt`                                                                                                                                                                                        | `just --justfile recipes/static.just …`                | `dev static …`                           |
| Nix checks (~15 call sites)                                                                                                                                                                                                    | `just --justfile recipes/static.just X verify sandbox` | `dev static X --sandbox`                 |
| `scripts/pr-land.sh`                                                                                                                                                                                                           | `just pr-watch`                                        | `dev pr-watch`                           |
| `scripts/e2e-workflow`                                                                                                                                                                                                         | `just e2e-producer` / `e2e-reconcile`                  | `dev e2e-producer` / `dev e2e-reconcile` |
| Nix source filters (`nix/packages.nix`, `coverageAuxiliarySource`, the per-check filters and their asserts in `nix/checks.nix`)                                                                                                | include `Justfile` / `recipes/*.just`                  | include the filtered `tools/dev/` source |
| Path lists in Rust (`repository-policy` `git.rs` docs inputs, `host-proof` `checkout.rs` freshness inputs)                                                                                                                     | name `Justfile` / `recipes/prettier.just`              | name `tools/dev/`                        |
| User-facing text (`source_inventory.rs`, `pr-observer` `watch.rs`, `server_fn_coverage/io.rs`, help text in trace-analysis, wasm-audit, coverage-check, source-diagnostics, e2e-local, and the e2e-local test that asserts it) | `just X`                                               | `dev X`                                  |

`pg-run` (#1976) and the Rust tools otherwise stay as they are, and `dev` calls
them.

**Spans.** Each gate opens a root span, and each step opens a child span. A
child span's attributes are its exit status (`ok`, `failed` with code,
`signalled`), its command text (capped at 1 KiB) and, where it applies, a
`cache` value:

- Nix checks evaluate the check's top-level output path before building it
  (`nix eval --raw <installable>.outPath`, an extra evaluation for each step).
  They record `reused` if that path was already valid. Otherwise they record
  `built` if `nix path-info` marks it `ultimate` after the build, and
  `substituted` if not. The value describes only the top-level output, not its
  dependencies.
- Steps that can't use a cache record no `cache` attribute.

Child processes join the trace through the OpenTelemetry environment carrier
(`TRACEPARENT`/`TRACESTATE`), plus `JAUNDER_TRACE_DIR`, which names the run
directory. We write a small getter/setter, because Python's is private. The root
is the `dev` process that finds `JAUNDER_TRACE_DIR` unset. Outside tools may set
`TRACEPARENT`, so its absence does not mark the root.

**Span records, then one Chrome JSON file.** Perfetto can't import OTLP, and no
maintained exporter writes its formats. Every process writes plain span records
as JSON Lines into `.xtask/run/<run_id>.trace.d/`:

- a _start_ record, written by a `SpanProcessor.on_start`
- an _end_ record, written by a custom `SpanExporter`

Each record carries the trace id, span id, parent id, name, start or end time in
nanoseconds since the Unix epoch, and attributes. Each process writes its own
uniquely named file (its pid plus a random suffix), so processes that run in
parallel, such as producers under `xargs -P`, never share one.

When the root ends, it merges the records into `.xtask/run/<run_id>.trace.json`
(Chrome trace JSON):

- it assigns Perfetto tracks from the span tree, giving overlapping siblings
  separate tracks
- it keeps the span ids in `args`
- it marks a span as `incomplete` when the span has a start record but no end,
  or when its parent is missing, as happens when a child is killed

The merged file is always written, never silently withheld. Spans go only to
files; `dev` adds no OTLP export.

**host-proof joins the same trace.** host-proof gets a Rust OpenTelemetry layer
built on the `opentelemetry`/`opentelemetry_sdk` 0.32 crates already in
`tools/Cargo.lock`. Its processor and exporter write the same span records, and
a roughly 20-line extractor reads `TRACEPARENT`. A shared golden-file test locks
the record format for both languages. This replaces `invocation_trace.rs` and
`JAUNDER_TRACED_RUN_CONTEXT`. Product telemetry (`host::telemetry`, OTLP to the
collector) is unchanged.

**traced-run shrinks.** Its output-parking runner (`traced-run -- cmd`) stays.
The launcher, span, recorder (`record-start`/`record-finish`) and
workflow-folding code is deleted. The `scripts/step-report.jq` rendering moves
into `dev`.

**Summary.** `dev trace summary [PATH]` defaults to the latest run. It reads the
span records from a run directory, including one downloaded from CI. It prints
the slowest spans with their depth, status and cache, plus each parent's
untraced time. Overlapping children are not summed. `--markdown` prints the same
table for `$GITHUB_STEP_SUMMARY`.

**CI.** Every validation and E2E job uploads `.xtask/run/` with `if: always()`,
hidden files included and 14-day retention. Each job appends the summary to its
step summary. The summary is diagnostic only and decides no job's result.

**Invariants.**

- Tracing never changes a step's exit status, stdout or stderr. If the trace
  directory can't be written, the gate warns once on stderr and otherwise
  behaves the same.
- A gate stops at the first failing step and exits with that child's status.
- When SIGTERM or SIGHUP is sent to `dev` alone, `dev` forwards it to the
  running child and waits for the child to exit.
  - A terminal's SIGINT already reaches the whole foreground process group, so
    `dev` doesn't forward it again. It only waits for the child.
  - When a child dies from signal n, `dev` closes the spans as `signalled` and
    exits with 128+n. Python reports such a child's status as -n.
- Unknown flags or arguments are refused before any work starts.
- In the sandbox, trace files land in the build directory and are discarded. The
  host-side Nix step records the cache result.

**Contracts carried over.** The current Just tests run the real recipes against
fake `nix`/`cargo`/`git`. About 70 of them check our behaviour, and those are
rewritten as pytest tests against `dev` with the same fake tools:

- step order for every gate, CI lane and precommit host
- fail-fast and exit status
- verify/fix flag mapping, Prettier globs passed unexpanded, and arguments
  containing shell syntax kept as one argument
- the sandbox cargo environment (`--offline`, CARGO_HOME, CARGO_NET_OFFLINE,
  every home admitted before work) versus the unchanged host environment
- the sccache environment (RUSTC_WRAPPER, CARGO_INCREMENTAL=0, SCCACHE_BASEDIRS)
- GIT_* overrides cleared, dirty-tree admission, and the qualification preflight
- children finish cleanup before `dev` returns
- perf defaults and overrides, with finished producer output kept
- the devtool launch reflects current source
- ERT timing events

Tests that only checked Just itself (its argument parser), Justfile text, or
literal `just X` strings are deleted.

**Python quality gate.** A new hermetic check, `checks.dev-python`, runs:

- ruff format/check
- `mypy --strict`
- pytest under coverage.py, failing below 90% branch coverage of `tools/dev/`

Subprocess paths are covered by the fake-tool tests, not excluded. The check
joins the static set that `validate-hermetic` builds. On the host, the
host-tests step runs ruff, mypy and pytest without coverage.

Deleting the Rust tests that drove real recipes may lower Rust coverage of the
code they exercised. The existing Rust coverage gate must still pass without
lowering any threshold. Where it drops, Rust unit tests make up the gap.

**Cost accepted.**

- Python grows from one tool (the ADR promoter, `nix/packages.nix`
  `adrPromoter`) to owning every workflow.
- The sandbox closure gains python3 and three libraries.
- Edits to `tools/dev/` rebuild the static checks.
- We maintain two small span-record writers, one in Python and one in Rust.

## Delivery

One PR on top of #1958 and #1976, in staged commits that each pass the
pre-commit gate:

1. the trace layer, exporter, merge and summary
2. `dev` with the gates
3. the remaining recipes
4. the Nix/CI/hooks/caller switch, including host-proof OpenTelemetry
5. deleting the Justfile, `recipes/` and Just from the devShell, retiring
   traced-run's span code, and updating the docs

## Acceptance

Proof to run:

1. Run `dev check` once on a clean change and once with an injected failing
   step, and a third time with a child killed by SIGKILL. Each run prints a
   summary and writes a `trace.json` in which nesting, statuses, the failing
   span and the `incomplete` span are visible. The outline's first task settles
   which loader the proof uses:
   - if the pinned nixpkgs `perfetto` 58.3 ships `trace_processor_shell`, a
     fixed command loads the file and queries the span count
   - otherwise the file is loaded in ui.perfetto.dev and a screenshot is
     attached to the PR
2. Do the same for one CI validation job and one E2E lane: download the
   artifact, run `dev trace summary <dir>`, and check that the job's step
   summary shows the table.
3. Post first baselines for `dev prepush` and `dev check` on #1965.
4. A Nix step shows `reused` on a second run and `built` or `substituted` after
   an input change. A non-Nix step has no `cache` field.
5. A pytest test shows that an unwritable trace directory leaves exit status and
   output byte-identical.
6. The pytest contract suite, ruff, `mypy --strict` and coverage ≥90% branch
   pass. `dev validate` passes.

Deliverables to commit:

7. No `just` remains on any devShell PATH. A committed `rg` check finds no
   `just` command text (invocations, file names `Justfile`/`*.just`, or `just X`
   help strings) in `.githooks`, `.github`, `nix`, `scripts` or `tools/*/src`.
8. CONTRIBUTING.md (gate commands, observation and trace policy) and
   ARCHITECTURE.md are updated. The ADR draft is marked as superseding the
   Just-ownership decisions of ADR-0237/0238/0239/0240 and the traced-run span
   mechanism.

## Boundaries

- The content of each step and the set of gate steps stay the same, except for
  the new Python checks. No new parallelism.
- Product telemetry, the E2E collector and the OTLP endpoints are unchanged.
- `pg-run` is #1976's, and `dev` creates no span inside it.
- The skills outside the repo (`jaunder-develop` policy that names `just …`
  commands) are the owner's to update. The PR lists the commands to change.
- After approval, the #1965 issue body's "use the traced-run span mechanism"
  constraint is edited to point at this spec.
