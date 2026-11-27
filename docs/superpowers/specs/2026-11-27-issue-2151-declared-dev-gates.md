# Declare `dev` gates as data run by one fail-fast loop

## Outcome

Every gate, CI lane and host check group in `tools/dev/dev/gates.py` becomes a
list of steps instead of a function that calls them. One loop runs a list, and
only that loop runs steps. A failed step's `StepFailed` propagates out of the
loop to `steps.gate`, which is the single place that turns it into the gate's
exit status. A gate therefore has no code that could swallow a failure.

Nothing a user can observe changes: step populations, order, spans, output and
exit statuses stay the same.

## Approach and tradeoffs

- **Declarations.** A step is a callable that takes no arguments, and a
  declaration is a `tuple[Step, ...]`.
  - The existing primitives (`steps.run`, `nix_check`, `verdict`, `phase`, …)
    keep running one traced child or phase. They still report failure by raising
    `StepFailed`.
  - Groups such as `host(mode, docs)` and `host_tests()` become functions that
    return tuples, and gates concatenate them, e.g.
    `check_no_test() = (*host(FIX, HOST), *host_tests())`.
- **The runner** is `run(declaration)`: `for step in declaration: step()`. It
  catches nothing.
  - `steps.gate` keeps its signal handling and trace. It takes a factory that
    builds the declaration, calls it inside the gate after `checkout.enter`, and
    runs the result.
  - Choices made while building a declaration therefore see the checkout and the
    run's environment variables: prepush's platform-dependent Markdown verdict
    (`index.shares_docs()`) and whether `JAUNDER_CSR_BUNDLE_DIR` skips CSR
    admission and the bundle build.
  - Every other `dev` entry point is run as a one-step declaration and is
    otherwise unchanged. That includes perf, upgrade, e2e, pr-watch, issues,
    csr, wasm-audit, trace-analysis and the static leaves.
  - `gates.csr_bundle()` stays a helper that returns the bundle's GC root, for
    `dev wasm-audit report` (`cli.py`).
- **A step that holds a resource wraps a sub-declaration.** test-local becomes
  `holding_host_artifacts_lock(inner)`. That step takes the lock and runs
  `inner` through the same `run`. The lexical `with` releases the lock when
  `inner` ends, fails or is interrupted, so the lock is held exactly as long as
  it is today. Each Cargo step in `inner` computes its Cargo environment when it
  runs: the bundle's resolved root, `CFLAGS` and the test executable. The lock
  stops that root being repointed between the three steps, so all three compute
  the same value.
- **Rejected alternatives.**
  - Steps returning a typed result instead of raising. That would change about
    95 call sites outside `gates.py`, and neither mypy nor ruff flags a
    discarded result.
  - A "tolerate" flag on the runner. It would give the runner two return
    contracts, only to move custody's `try/except` into it.
- **Deliberate failure-combining stays where it is.** `dev precommit` is the
  declaration `(custody,)`. `index.precommit` takes
  `select: Callable[[bool], Declaration]` and calls
  `run(select(markdown_only))`, where `select` returns `docs_checks(FIX)` or
  `check_no_test()`. Its existing `except StepFailed` still:
  - records a failed check,
  - runs the after-snapshot and staging anyway,
  - re-raises a signal.

  Two other deliberate catch sites are unchanged as well: the `static-docs`
  request and post-check in `index.py`, and e2e's `StepFailed` → `EvidenceError`
  refusal in `_inventory`. All three live inside steps, never in a gate. Code
  review guards against new ones, as it does today.

- **Shared environment.** `steps.gate` creates one `Env` per run, the way it
  already creates the run's signal state. `Env` is a typed dataclass. It lives
  in memory in that one `dev` process for the whole gate run, is never written
  to disk, and is dropped when the gate ends.
  - Its one step-to-step value is `packages`, the tool package output paths. It
    replaces `steps.tool`'s module-level cache.
  - A package is still built lazily, as its `<package>-build` step, on the first
    `steps.tool` lookup in the run, and never again. It is never a declared
    step, because a gate's population depends on whether an earlier step already
    built the package.
  - Callers of `steps.tool` keep their signatures. Every other value the issue
    lists is now handled without `Env`: the CSR root is a constant, test-local's
    lock is held by the wrapping step, custody's checks are a declaration, and
    the Markdown verdict is decided when the declaration is built.

## Acceptance

Executed proof:

- `traced-run -- dev check-no-test` and then `dev prepush` pass, with
  `dev-python` at 100% statement and branch coverage.
- One runner test, using fake steps, pins three things: the loop stops at the
  first failed step, later steps never run, and the gate exits with that step's
  status, or 128+n for a step killed by signal n.
- These tests pass with their expected command populations unchanged:
  - the per-gate population tests in `test_gates.py` and
    `test_coverage_workflows.py`
  - `test_index.py` (custody and static-docs)
  - `test_signals.py`
  - `test_test_local.py`, which covers lock waiting, release after a failure,
    and a signal while waiting
- Two tests that proved fail-fast gate by gate are removed, since the runner
  test replaces them: #2145's per-kind `STEP_FAILURES` test and
  `test_workflows_stop_at_each_failing_boundary`. Stopping inside a single step
  stays covered by `test_nix_check.py` (a failed evaluation skips its build),
  `test_static.py` (a failed preparation skips its check) and `test_gates.py` (a
  failed tool build skips the tool, and the wasm budget never runs without its
  bundle).

Committed deliverables:

- `gates.py`: every gate is a declaration, and no gate body calls a step
  directly.
- The `Env` docstring states the environment's lifetime and that it is never
  persisted.
- An amendment to ADR-0251 describing declared gates, the runner, `Env`, the
  lock-holding wrapper and the sanctioned catch sites, with the matching
  projection in `docs/ARCHITECTURE.md` ("Development tooling") in the same
  commit.

## Boundaries

- No change to any gate's step population, order, span names or attributes,
  output or exit status.
- `StepFailed` and `steps.refuse` stay. Workflows outside `gates.py` are not
  converted to declarations.
- Index custody's staging rules and the static-docs semantics are unchanged.
- No structural (AST or source) check for `except StepFailed`.
