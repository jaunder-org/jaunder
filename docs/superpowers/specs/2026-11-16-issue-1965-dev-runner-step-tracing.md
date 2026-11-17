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
| `git-index-safety precommit` (superseded by Task 7e: the binary is gone and `dev precommit` runs the host checks itself)                                                                                                       | `just precommit-*-host`                                | `dev precommit-*-host`                   |
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
untraced time. Every span that did not end `ok` (failed, signalled, incomplete)
is listed as well, however short. Overlapping children are not summed.
`--markdown` prints the same table for `$GITHUB_STEP_SUMMARY`.

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
- pytest under coverage.py with `branch = true`, failing below `fail_under = 90`
  (a combined statement-and-branch figure) for `tools/dev/`

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

## Folded-in issues

Approved 2026-11-16: four open Milestone 33 issues framed against Just are
delivered here on `dev`, and close with this PR. #1961 (move the inline `perf`
recipe body into a script) is already delivered by `perf.py` and closes as
delivered.

- **#1959: ERT without devtool.** Today the chain is `dev` → `devtool check ert`
  → `dev static ert --sandbox` → Emacs, kept only for a timing line that `dev`'s
  spans now provide. Delete `devtool check` and its `devtool-check-timing`
  event, after confirming nothing consumes it. The `all` static group and the
  `static-ert` Nix check run `dev static ert` directly, and `static-ert` drops
  `devtoolBin`. This also removes the host use of `--sandbox` that the
  deliverable C review found.
- **#1960: unused static groups.** Delete the `code` group and its test. Keep
  `all` (`dev static-all`), with its purpose stated in CONTRIBUTING: one command
  that runs every host static leaf in order. The `devtool` build drops out of
  `dev static-all` along with ERT's hop.
- **#1962: hidden gate work becomes `dev` steps.**
  1. `wasm-budget`: `dev` builds `.#csrBundle` as its own Nix step, with a cache
     state (the `csr-bundle` step and GC root test-local also uses), and passes
     the store path to `wasm-audit budget --site-path <p>`. Without a path, the
     gate refuses to run.
  2. Doctests move into crane; no host `cargo test --doc` remains (owner
     decision 2026-11-17):
     - `tools/` has no doctests, so checking its fences against its run proves
       nothing. A plain `craneLib.cargoDocTest` check over the tools workspace
       (`tools-doctests`) replaces the host `doctest-fences` step and does no
       fence reconciliation.
     - Prepush's host `workspace-doctests`, which duplicated `checks.doctests`,
       becomes a `dev` Nix-check step on `checks.doctests` plus `doctests-gate`.
     - The root workspace keeps its ADR-0095 reconciliation in
       `checks.doctests`, because its `compile_fail` fences are real proofs.
     - The `doctest-check` binary and `tools/doctests/src/host.rs` are deleted.
       `roots::HOST` goes, and the ownership test records that `tools/` is
       outside the fence population.
  3. _Superseded by Task 7e (the `git-index-safety` port below): custody moved
     into `dev` itself, with no `dev` → Rust → `dev` nesting. Kept as the
     history of the intermediate step._ Pre-commit: `git-index-safety` keeps
     sole custody of the Git index (ADR-0029): it snapshots, runs
     `dev precommit-*-host`, reconciles staging, and still runs the post-check
     after a failure. Its hand-built `nix build …static-docs` becomes
     `dev nix-check static-docs`, so all Nix check flags have one definition
     (`steps.nix_check`), and the request shows up as a step with a cache state.
     The `dev` → Rust → `dev` nesting stays, because single-process index
     custody is deliberate, and the trace now shows every level.
- **#1964: test-local sequencing in `dev`.** The lock question is answered. The
  lock is a checkout-local `flock` on `.xtask/host-artifacts.lock`, and only
  test-local takes it. It stops a concurrent test-local in the same checkout
  from replacing the `.xtask/gcroots/csr-bundle` out-link, or the
  `target/debug/jaunder` image worker, while a test run is using them. It stays.
  `dev` holds the same `flock`, which is not inherited by children, across these
  steps:
  1. CSR untracked-input admission
  2. `nix build .#csrBundle` rooted at `.xtask/gcroots/csr-bundle`, skipped when
     `JAUNDER_CSR_BUNDLE_DIR` is set
  3. `host-proof csr-prerequisite <root>` (validation stays in Rust)
  4. `cargo build --locked -p jaunder --bin jaunder`, with `CFLAGS` set empty
     when absent
  5. `pg-run cargo nextest run …` with `JAUNDER_CSR_BUNDLE_DIR` and
     `JAUNDER_TEST_EXECUTABLE` set

  The same task stops tests from tracing into the real checkout. A test that
  runs the real `dev` (for example `tools/coverage/tests/mutants_verify.rs`
  through `.mutants-loop/verify.sh`) sets `JAUNDER_TRACE_DIR` empty, so its fake
  runs neither nest in a gate's trace nor leave roots in `.xtask/run`.

  The checkout preparation that test-local does today moves to `dev` too, so
  host-proof's `test-local` binary and `steps/test_local.rs` are deleted.
  Selectors still pass through, and each step is its own span.

- **`git-index-safety` moves into `dev`** (owner decision 2026-11-17, part of
  #1962's visibility and #1977's language review). `dev precommit` itself:
  1. takes the before snapshot
  2. classifies the change as Markdown-only or broad
  3. runs the matching host checks in the same process
  4. reconciles staging even after a failure
  5. for a Markdown-only change, requests `static-docs` with the clean-state
     admission and the mandatory post-check

  `dev prepush` does the `prepush-docs` request the same way. This keeps
  ADR-0029's single-process custody of the index without a `dev` → Rust → `dev`
  bounce. Every git operation stays on the `git` CLI
  (`status --porcelain=v1 -z`, `diff --cached --raw -z`, `hash-object`, `add`),
  so the snapshot always agrees with what `git commit` writes, including a
  hook's temporary `GIT_INDEX_FILE`. A library such as libgit2 could diverge on
  that index, on filters or on fsmonitor. The staging rules, the refusals and
  the "uncertain evidence refuses" rule are unchanged. `safety.rs`, the
  snapshot, plan and admission parts of `git.rs`, and the `git-index-safety`
  binary are deleted, and their `native.rs` tests become pytest tests against
  real temporary repositories. The rest of `git.rs` stays for the other tools.

**Decisions recorded at the deliverable D review (2026-11-17):**

- **Local E2E concurrency.** `dev validate`'s local E2E runs at most three VMs
  at once by default (`scripts/e2e-workflow`, `jobs=${2:-3}`), the peak before
  #1890, when each backend's three Firefox lanes ran together. Running every VM
  at once is an error, not a default.
- **No hook-path self-healing in `dev`.** `dev precommit` deliberately dropped
  the `core.hooksPath` self-healing `git-index-safety` did; a hook that runs has
  its path already. `dev test-local` does not set it either (whole-branch
  review, 2026-11-17): its Python copy duplicated the one owner,
  `repository_policy::checkout::ensure_hooks_installed`, which the
  repository-policy host tools, `coverage-check`, `e2e-local` and
  `source-diagnostics` already call before their work.
- **`tools-doctests` source.** The check runs over its own source
  (`toolsDocTestSrc`), in which every tools member is real, reusing the shared
  tools dependency build. The shared tools source carries `repository-policy`,
  `qualification` and `host-proof` only as placeholders, so a doctest there
  would otherwise never run. Those three have no doctests today; the check now
  compiles them.
- **Issue helpers in Python** (owner decision). `issue-discover` and the issue
  metadata front doors are rewritten in Python as
  `tools/dev/jaunder_dev/issues.py`, and their shell scripts are deleted.
- **pytest runs with pytest-xdist `-n auto`** (owner decision). That is
  parallelism inside one step, the pytest run; gate steps still run one after
  another.
- **Perfetto proof by screenshot.** Task 1 found that the nixpkgs `perfetto`
  package lacks `trace_processor_shell`, so acceptance 1 loads each trace in
  ui.perfetto.dev and attaches screenshots.
- **Coverage threshold as enforced.** "90% branch coverage" means what
  `tools/dev/pyproject.toml` enforces: coverage.py `fail_under = 90` with
  `branch = true`, a combined statement-and-branch figure.
- **E2E producer builds record no cache state (deferred to #1999).** Every Nix
  build `dev` runs as a step records `jaunder.cache`. `scripts/e2e-workflow`
  builds the producers itself, inside the `dev e2e-producer` step, and keeps its
  failed-build diagnostics with `--keep-failed`, `--json` and a tee'd log that
  `steps.nix_build` cannot express simply.
- **Defaults kept.** Prepush keeps building the root doctests (`checks.doctests`
  plus `doctests-gate`). The static-docs request's log and excerpt files under
  `.xtask/diagnostics/static-docs` are dropped: the request is an ordinary
  Nix-check step whose output the trace and Nix's own log keep. A check killed
  by a signal, or a termination request to `dev`, skips the staging
  reconciliation and the docs post-check, and `dev` exits 128+n.

## Delivery

One PR on top of #1958 and #1976, in staged commits that each pass the
pre-commit gate:

1. the trace layer, exporter, merge and summary
2. `dev` with the gates
3. the remaining recipes
4. the Nix/CI/hooks/caller switch, including host-proof OpenTelemetry
5. deleting the Justfile, `recipes/` and Just from the devShell, retiring
   traced-run's span code, and updating the docs
6. the folded-in issues: #1959 with #1960, then #1962, then #1964, then the
   `git-index-safety` port, one commit each

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
6. The pytest contract suite, ruff, `mypy --strict` and coverage.py's
   `fail_under = 90` with `branch = true` pass. `dev validate` passes.

Deliverables to commit:

7. `pkgs.just` is in no Nix input, devShell or check. With that removed, passing
   gates are the proof that nothing needs Just; there is no separate source
   check (owner decision 2026-11-17, which retired the `no-just` contract).
8. CONTRIBUTING.md (gate commands, observation and trace policy) and
   ARCHITECTURE.md are updated. The ADR draft is marked as superseding the
   Just-ownership decisions of ADR-0237/0238/0239/0240 and the traced-run span
   mechanism.

## Boundaries

- The content of each step and the set of gate steps stay the same, except for
  the new Python checks and the steps that the folded-in issues make visible. No
  new parallelism.
- Product telemetry, the E2E collector and the OTLP endpoints are unchanged.
- `pg-run` is #1976's, and `dev` creates no span inside it.
- The skills outside the repo (`jaunder-develop` policy that names `just …`
  commands) are the owner's to update. The PR lists the commands to change.
- After approval, the #1965 issue body's "use the traced-run span mechanism"
  constraint is edited to point at this spec.
