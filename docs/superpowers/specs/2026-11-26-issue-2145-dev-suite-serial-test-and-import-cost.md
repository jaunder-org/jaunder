# Issue #2145: cut the dev suite's serial test and per-launch import cost

## Outcome

`pytest tools/dev` loses its single 151 s serial test and every `dev` launch
imports less, so the suite's wall clock falls in pre-commit, pre-push, CI's host
lane and the `dev-python` Nix check. Gate failure semantics stay pinned.

## Approach and tradeoffs

**1. Replace the exhaustive fail-fast test.**
`test_gates.py::test_every_step_failure_is_its_status_and_stops_subsequent_work`
launches `dev` once per distinct step of eight gates. It is replaced by one
parametrized test with a failure case for each runner kind. Each case fails a
step that has later steps after it, and asserts what the current test asserts:
exit status 17, the recorded commands stop at the failing step, and stderr ends
with `<tool> failed`. The kinds:

- a plain `steps.run` step
- a rendered-report step (`report.render`, e.g. `coverage-policy` in `validate`)
- a Nix check build (`steps.nix_check`/`nix_build`)
- a `static.run` leaf's host preparation step
- a step `test_local` runs while it holds its host-artifacts lock
  (`nextest-build` in `check`; `nextest` must not run)

Why this is enough: every runner kind reaches a gate's exit status through one
shared `raise StepFailed` path. `steps._run` serves `run`, `output` and `text`,
while `nix_build` and `phase` re-raise. `steps.gate` then maps the failure to
the exit status. Existing tests already pin the rest:

- `test_each_gate_runs_its_complete_population_in_order` pins step order.
- `test_clean_admission_refuses_when_git_fails_before_work` and
  `test_producer_cleanup_finishes_before_dev_returns` pin each gate's mapping
  from step failure to exit status.

Accepted residual: no test stops a future edit to a gate from catching
`StepFailed` or ignoring the status `steps.captured` returns; code review guards
that. The issue suggested a structural syntax check over `gates.py`, which is
rejected: it is a hand-written policy over our own source, not a design that
makes the invalid state unrepresentable. The real fix is declarative gates,
filed as #2151: each gate declares its steps as data, and one runner loop
executes them and stops at the first failure. Steps that pass values to later
steps would read and write a shared, persistent environment. Python's type
system cannot force a caller to check a returned status, so changing what steps
return would not close this gap.

**2. Defer import-time work to the code that needs it.** Profiling
`import dev.__main__` with `-X importtime` shows that the largest avoidable
costs are work done at import time, not the loading of modules:

| Cost at import                                            | Needed by                       |
| --------------------------------------------------------- | ------------------------------- |
| `e2e_local._OPENER`: `build_opener` creates a TLS context | `dev e2e-local` server probes   |
| `urllib.request` and `http.client` imports in `e2e_local` | the same probes                 |
| `yaml` in `pr_watch`                                      | `dev pr-watch` workflow parsing |
| `tabulate` in `report`                                    | `report.table` only             |

Each of these moves to its use site. The two modules become function-local
imports, and the opener is built on first use through `@functools.cache`, which
adds no branch for coverage to cover. `dev.cli` and the command definitions stay
unchanged.

Alternatives: the issue suggests importing modules inside the command bodies in
`dev.cli`, or loading command groups on demand (click's `LazyGroup`). That would
also stop `dev`'s own modules loading, such as `pr_watch`, `e2e_evidence` and
`trace_analysis`. Their combined self time is similar to `yaml` plus `tabulate`.
But both alternatives restructure the CLI. Several decorators read module
constants when the command is defined (`e2e_local.BROWSERS`,
`trace_analysis.BROWSERS`, `pr_watch.Watch` defaults, `static.LEAVES`), so those
constants would have to move into a light module or be duplicated. Deferring at
the source takes the largest single costs for every importer, with a much
smaller change.

Accepted cost: the OpenTelemetry SDK and what it loads at module scope still
load for every command: `logging`, `traceback` and `psutil`, since
`opentelemetry.sdk.resources` imports `psutil`. Every command opens its trace
through `steps.gate`, so tracing starts the same way as today. `dev.pr_watch`,
`dev.e2e_local` and the other command modules still load, just more cheaply.

## Acceptance

Proof to execute, with results posted as a comment on #2145:

- The exhaustive test is gone, and the five runner-kind cases pass.
- #2145's results comment links the declarative-gates follow-up, #2151.
- `python -X importtime -c "import dev.__main__"`, before and after: the minimum
  over repeated runs on the same machine. The after figure is measurably lower.
- A test imports `dev.__main__` in a subprocess and asserts that `yaml` and
  `tabulate` are absent from `sys.modules`. The deferred opener has no module to
  pin; the importtime figure covers it.
- `pytest tools/dev` full-suite wall clock before (at `origin/main`) and after,
  back-to-back on the same machine, plus one CI host-lane `pytest` step duration
  after the change.
- `dev-python` keeps 100% statement and branch coverage.

## Boundaries

- No change to the suite's place in the hooks or CI (#2136, #2144), xdist worker
  counts, the Python version requirement, or `dev.cli`'s command surface.
- No structural source check (pytest or ast-grep) and no new gate step.
- No redesign of `gates.py` or the `steps` API; that is the follow-up's.
