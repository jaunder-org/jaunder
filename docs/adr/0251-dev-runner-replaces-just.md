# ADR-0251: A Python `dev` runner with OpenTelemetry tracing replaces Just

- Status: accepted
- Date: 2026-11-16
- Issue: [#1965](https://github.com/jaunder-org/jaunder/issues/1965)
- Amended: 2026-11-25 — the wasm budget runs in-process in `dev` (#2101)
- Amended: 2026-11-25 — `admit-inputs` moves to host-tool; `csr-prerequisite` is
  retired (#2102)
- Amended: 2026-11-27 — gates are declarations run by one fail-fast loop (#2151)

## Context

[ADR-0237](0237-just-owned-gate-workflows.md),
[ADR-0238](0238-just-e2e-workflow-ownership.md),
[ADR-0239](0239-qualification-workflow-ownership.md) and
[ADR-0240](0240-just-frontdoors-host-pr-observer.md) made the root `Justfile`
and `recipes/*.just` the owner of developer, hook, CI and sandbox workflows.
Milestone 33 then asked for per-step timing, outcome and cache evidence. The
plan was to extend traced-run's custom span mechanism, which passes context in
`JAUNDER_TRACED_RUN_CONTEXT` and writes Chrome JSON fragments.

Just 1.58 has no per-run or per-recipe hook. `set shell` is global and also
wraps backtick evaluation, and a recipe line can run outside any root span and
record nothing. Making every step traced, with its exit status and cache state,
would mean wrapping every line and trusting that no line escapes the wrapper.
Shell and Just recipes also handle argv quoting, nested composition and signal
forwarding poorly.

We compared real recipe ports in Just, bash and Python. Python with click
expressed steps, nesting, exit status and argv as ordinary code. Bash still
needed a hand-written span protocol, and invoke loses argv quoting. The
OpenTelemetry API/SDK already provides spans, context and environment
propagation (`TRACEPARENT`), so a custom span protocol is unnecessary. Perfetto
reads Chrome JSON and its own protobuf, but not OTLP.

## Decision

A Python package in `tools/dev/`, built with click and the OpenTelemetry API/SDK
and packaged by Nix as the `dev` command, owns every workflow the Justfile and
`recipes/*.just` owned. That covers developer gates, Git hooks, CI lanes, the
commands the Nix checks run inside the sandbox, and the remaining utility
recipes. Just is removed from the repository and the devShell.

Each gate and step is an OpenTelemetry span. Its attributes are its exit status,
its command text, and a `cache` value where one applies: `reused`, `built` or
`substituted` for Nix outputs, and nothing for any other step. Spans are
exported only to files: every process writes plain span start and end records
under `.xtask/run/<run_id>.trace.d/`. The root process merges them into one
Chrome trace JSON file, `<run_id>.trace.json`, for Perfetto, and marks spans
with no end or no parent as incomplete. Child processes join through the
OpenTelemetry environment carrier. _Amended by #2102:_ only `dev` processes
write records; host-proof's Rust OpenTelemetry writer, whose only user was
`csr-prerequisite`, is retired. `dev` runs from the checkout on the host and is
built from a filtered source in the sandbox. `dev trace summary` reads the
records locally or from CI artifacts.

Tracing never changes a step's exit status or output. traced-run keeps only its
output-parking runner; its launcher, span, recorder and workflow-folding code is
retired.

_Amended by [#2151](https://github.com/jaunder-org/jaunder/issues/2151):_ a
gate, CI lane or host check group in `tools/dev/dev/gates.py` is a declaration,
the tuple of steps it runs in order, and never runs a step itself. Groups are
functions that return tuples, and gates concatenate them. One loop,
`steps.execute`, runs a declaration. It catches nothing: a failed step's
`StepFailed` leaves the loop, so no later step runs, and `steps.gate` turns it
into the gate's exit status. Fail-fast is therefore one loop's property, pinned
by one test, while per-gate tests pin each population and its order.
`steps.gate` builds the declaration inside the gate, after the checkout root is
entered, so choices that depend on the host or the environment, such as whether
prepush takes the shared Nix verdicts or a given `JAUNDER_CSR_BUNDLE_DIR`, are
made there. Every other `dev` workflow runs as a declaration of one step.

Values a step produces for later steps live in the run's environment (`Env`),
which `steps.gate` creates for each run. It stays in the `dev` process's memory
for that run and is never written anywhere; it holds the output path of each
tool package the run has built, which is still built lazily on its first use. A
step that holds a resource across others wraps a sub-declaration: test-local
runs its steps through the same loop inside the `host-artifacts-lock` it holds,
and each Cargo step resolves the bundle root as it starts. Among the gates'
steps, three catch `StepFailed` on purpose: precommit records a failed check and
still reconciles staging, index custody (precommit's `static-docs` request and
prepush's checks) still runs its post-check, and the e2e server-fn inventory
turns a failed listing into an evidence refusal. Custody, like test-local, runs
the declaration it is given through the same loop. Each re-raises a signal.
Steps keep raising rather than returning a result, because a returned status,
once discarded, is invisible to mypy and ruff.

Work a gate performs is a visible `dev` step, not hidden inside a tool
([#1962](https://github.com/jaunder-org/jaunder/issues/1962)):

- The wasm budget builds `.#csrBundle` as its own Nix step, with its cache state
  (the same `csr-bundle` step and GC root `dev test-local` uses), and passes
  that output to `wasm-audit budget --site-path`. The budget requires the path
  and never runs Nix itself. _Amended by #2101:_ the budget is now `dev`'s own
  Python (`tools/dev/dev/wasm_audit.py`). After the `csr-bundle` step it
  measures that output in-process as the `wasm-budget` phase, and still runs no
  Nix.
- Every Nix check `dev` requests goes through one definition of its build flags,
  the shared `static-docs` verdict of precommit and prepush included; that
  request also leaves `flake.lock` exactly as committed.
- `dev precommit` and `dev prepush` keep custody of the Git index themselves, in
  one process: the snapshot, Markdown-or-broad classification, the host checks,
  the staging reconciliation (also after a failed check) and the `static-docs`
  admission, request and post-check. The snapshots, the staging reconciliation
  and the docs admission, request and post-check are each a span directly under
  the gate; the host checks run under one `precommit-markdown-host` or
  `precommit-broad-host` phase span. A signal is the one exception: a check
  killed by signal n, or a termination request to `dev`, skips the
  reconciliation and the post-check, and `dev` exits 128+n. The staging rules,
  refusals and the rule that uncertain evidence refuses are unchanged. Every
  index query runs the `git` CLI (`status --porcelain=v1 -z`,
  `diff --cached --raw -z`, `ls-files --stage -z`, `hash-object`, `add`) with
  the hook's environment, so the evidence agrees with what `git commit` writes,
  including the temporary `GIT_INDEX_FILE` of `git commit <path>`. A library
  such as libgit2 could diverge on that index, on filters or on fsmonitor. No
  step inherits the hook's `GIT_*` repository overrides, so a test's scratch
  `git init` cannot reinitialise the hooked repository. repository-policy's
  `git-index-safety` executable and its snapshot, plan and admission code are
  retired.
- Doctests run only in Nix. The root workspace's `checks.doctests` producer and
  `doctests-gate` keep the bidirectional fence reconciliation, and `dev prepush`
  requests them as steps. `checks.tools-doctests` runs the `tools/` workspace's
  `cargo test --doc` without reconciliation, over a source in which every tools
  member is real, so no member's doctests can go unrun. The host
  `doctest-fences` and `workspace-doctests` steps and the `doctest-check`
  executable are retired.
- `dev test-local` sequences its own steps
  ([#1964](https://github.com/jaunder-org/jaunder/issues/1964)): CSR input
  admission, the `.#csrBundle` Nix step, CSR validation, the image worker build
  and `pg-run cargo nextest`. Admission and validation stay in Rust as the small
  `host-proof admit-inputs` and `host-proof csr-prerequisite` executables.
  _Amended by #2102:_ `admit-inputs` is now host-tool's executable, and
  `csr-prerequisite` is retired: `dev` passes the bundle's resolved absolute
  root to Cargo, and the server build script, which already ran the same
  validators on a declared root and fails closed, validates it in
  `image-worker`. The separately named early failure and its two sub-phase spans
  are the accepted loss. `dev` holds the checkout-local `flock` on
  `.xtask/host-artifacts.lock`, in a descriptor no child inherits, for the whole
  sequence. The lock stays: it stops a concurrent `test-local` in the same
  checkout from repointing the bundle's GC root or rebuilding
  `target/debug/jaunder` while a run's tests use them. host-proof's `test-local`
  executable is retired.

With xtask gone, the in-sandbox `devtool` crate no longer marks a boundary; it
was a catch-all of unrelated jobs. It is dissolved: each job becomes an
executable of the crate that owns its logic, and `tools/devtool` is deleted.

- `coverage-sandbox` in `tools/coverage`: `emit` (the instrumented suite's
  producer), `validate-status`, `wasm initialize|map|finalize` (the browser
  WebAssembly coverage lifecycle) and `diagnostic-build` (the diagnostic CSR
  build's artifact operations). It needs none of the crate's host-only
  dependencies, which stay behind the `host` feature.
- `doctests` in `tools/doctests`: `emit`, the root workspace's doctest producer.
- `perf-producer` in a new `tools/perf-producer` crate: `storage` and
  `validate-browser`. It links `performance` with the typed product storage. A
  crate of its own keeps that storage out of the `performance` library and
  `perf-data`. It also keeps the producer out of the root workspace's coverage
  population, which `performance` belongs to through `test-support`. A feature
  inside `performance` could do neither.
- `system-artifacts` in a new `tools/system-artifacts` crate, which stages the
  closed system styling inventory through `host`. Its `qualification` feature is
  built only for the system-artifact qualification packages.

Each executable is its own Nix package, built from the shared tools source and
dependency build, and each derivation takes only the executable it runs: the
doctest producer no longer builds the performance or styling code. Sandbox
behaviour, outputs, status files and failure semantics are unchanged. The
docs-only `--no-default-features` devtool build had no consumer and is not
replaced. The executables are on no devShell PATH; host workflows reach
`system-artifacts` through `cargo run`.

This supersedes the Just-ownership parts of ADR-0237, ADR-0238, ADR-0239 and
ADR-0240, the `just perf`, `just perf-compare` and `just perf-import-baseline`
front doors of [ADR-0241](0241-performance-workflow-ownership.md), the
`just sandbox` front door of
[ADR-0244](0244-standalone-ux-sandbox-lifecycle.md), and the traced-run
span-mechanism constraint in milestone 33. Those workflows keep their names and
arguments as `dev` subcommands.

It also amends these decisions:

- [ADR-0095](0095-doctest-gate-enumerates-the-fence-population.md)'s host half:
  `tools/` leaves the fence population. It has no doctests, so its plain Nix
  doctest run proves what reconciliation would. Only the root workspace is
  reconciled, as before.
- [ADR-0029](0029-git-enforced-verify-gate.md)'s #1117 supplement: `prepush` no
  longer runs host `workspace-doctests`. It requests the Nix doctest producer
  and gate, so it is no longer free of Nix derivations.
- [ADR-0029](0029-git-enforced-verify-gate.md)'s #1079 and #1863 supplements:
  the single-process index custody they describe is preserved, but it lives in
  `dev` on the `git` CLI rather than in Rust, and it now reads the hook's index
  (a temporary one included) rather than clearing the hook's overrides.
- [ADR-0246](0246-retire-xtask-devtool-endpoints.md)'s repository-policy home:
  it no longer owns `git-index-safety`.
- [ADR-0247](0247-host-tool-json-report-contract.md)'s list of host tools:
  `doctest-check` and `test-local` are gone. `admit-inputs` and
  `csr-prerequisite` render no verdict report: a refusal is their exit status
  and stderr, and `csr-prerequisite` prints only the root it verified. Its human
  views: `dev` renders the report (`tools/dev/dev/report.py`), and
  `scripts/step-report.jq` and `scripts/run-report` are deleted. The shared
  multi-checkout compile-cache environment moves from `tools/host-tool` into
  `dev` (`tools/dev/dev/checkout.py`), which sets it for every host step.
  _Amended by #2102:_ `admit-inputs` is host-tool's executable and
  `csr-prerequisite` is retired.
- [ADR-0028](0028-devtool-vs-xtask-boundary.md)'s devtool/xtask boundary:
  superseded. Both crates are gone. The litmus test it recorded, _where must
  this code execute?_, still separates the executables above, which run inside
  derivations, from the host concern tools, which never do.
- [ADR-0028](0028-devtool-vs-xtask-boundary.md)'s #1858 addendum: traced-run no
  longer writes Chrome JSON process spans and has no launcher mode, and
  `scripts/cargo-xtask` is deleted. traced-run keeps only captured
  single-program execution; `dev`'s OpenTelemetry spans replace the process
  spans and their recorded-entry links.
- [ADR-0052](0052-devtool-unifies-static-checks.md) and
  [ADR-0146](0146-devtool-owns-compiling-static-check-definitions.md):
  `devtool check` is gone. Its last use, `devtool check ert`, was kept only for
  its `devtool-check-timing` event; both are deleted. `dev static` owns every
  static check definition, ERT included, and its spans give the timing.

## Consequences

- Python, so far used only by the ADR promoter, now owns every workflow and gets
  its own gates: ruff, `mypy --strict`, pytest, and coverage.py's
  `fail_under = 100` with `branch = true`, a combined statement-and-branch
  figure.
- The hermetic checks' closures gain python3, click and opentelemetry.
- Two small span-record writers, Python and Rust, must keep writing the same
  format. A shared golden-file test locks it.
- Any edit to `tools/dev/` rebuilds the hermetic checks that run `dev`.
- Behaviour tests that drove `just` with fake tools move to pytest against
  `dev`. Tests of Just itself and of Justfile text are deleted.
- No source check guards against Just's return. Just is in no Nix input, so the
  gates passing proves that nothing needs it.
- Adding OTLP export later is a configuration choice, not a redesign.
- Pre-push builds the root doctest producer in Nix. A reused output is cheap,
  but a changed root source rebuilds it.
- `tools-doctests` has its own source, so an edit to `repository-policy`,
  `qualification` or `host-proof` (which the shared tools build carries only as
  placeholders) reruns it.
- The in-derivation executables share the tools source, so an edit to any tools
  crate in that source still rebuilds each of them and reruns the checks that
  use them.
