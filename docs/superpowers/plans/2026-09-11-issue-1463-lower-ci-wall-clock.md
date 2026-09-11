# Lower CI Wall-Clock Time Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded slices through
> `jaunder-dispatch`. This outline exists because CI/Nix source and cache
> boundaries are durable architecture, and measurement and implementation share
> contracts that must remain stable across separate work items.

## Scope

In:

- Durable phase attribution for validation and e2e execution.
- A checked-in baseline from completed GitHub Actions runs.
- Measurement of source invalidation, Cachix eligibility, and warm validation
  candidates.
- The smallest independently beneficial implementation or composition that
  satisfies the approved spec.
- Repeated cold and warm Actions evidence and two-sided regression probes.

Out:

- Weaker or changed-path final verdicts.
- Fewer e2e matrix jobs or altered merge-group semantics.
- Serial cache preparation without a measured wall-clock win.
- CI-provider, runner-hosting, or remote-cache-service replacement.

## Task outline

- [x] Task 1: Expose durable CI phase attribution
  - Contract: extend the existing xtask result/diagnostic model with a shared
    `phases` record: stable `name`, nullable `duration_ms`, `outcome`
    (`success`/`failed`/`unavailable`), and an evidence detail. Required names
    are `nix-evaluation`, `nix-substitution`, `nix-local-build`,
    `vm-startup-readiness`, `gate-execution`, `result-lift`, and
    `post-gate-checks`; unobservable phases are present as `unavailable`, never
    omitted or inferred. Nix classification remains
    `reused`/`substituted`/`built`/`unknown` and cites its observed evidence.
    Preserve `StepResult`, the `xtask-done` sentinel, existing diagnostic paths,
    and failure ordering. Actions summaries render these same records rather
    than reconstructing timings independently.
  - Verification: focused xtask tests prove vocabulary, ordering,
    success/failure recording, and unavailable attribution; an exercised command
    emits both machine-readable diagnostics and the reviewable summary.

- [ ] Task 2: Establish baselines and explore candidates
  - Contract: the checked-in report records run/ref identity, change class,
    affected closure, workflow/toolchain/runner identity, cache classification,
    critical path, phase records, runner-minutes, exclusions, and comparability
    limits. Historical runs remain immutable evidence. Controlled baseline and
    exploratory measurements use one declared source perturbation per candidate;
    treatment pairs are collected only after Task 4 produces the implementation.
  - Verification: the report accounts for the referenced cold PR and warm
    merge-group runs, at least one narrow-change class, every candidate's
    measured critical-path effect, and every excluded/non-comparable sample.
    Each rejected candidate names the measured cost or preserved invariant that
    disqualified it, including rejections made alongside a successful candidate.

- [ ] Task 3: Prove Nix invalidation and Cachix eligibility boundaries
  - Contract: extend the existing source-probe convention rather than creating a
    second probe framework. A cache-boundary check owns named sets of per-ref
    verdict derivations that must remain excluded and non-verdict inputs that
    must be eligible. Derivation classification is structural or exact-name
    based; a broad substring that accidentally captures support inputs is not an
    acceptable contract.
  - Verification: mutation proof demonstrates failures for an admitted final
    verdict, an excluded intended cacheable input, a missing required source
    invalidation, and renewed unrelated invalidation. The unmodified graph
    passes without realizing final test results.

- [ ] Task 4: Implement the measured critical-path change
  - Contract: select only candidates supported by Tasks 2 and 3. Each component
    must independently improve the measured critical path and preserve the
    validation DAG's producer/consumer dependencies. Keep all four e2e jobs,
    coverage/e2e per-ref execution, panic detection, diagnostic lifting, and
    `merge_group` required contexts unchanged.
  - Verification: the candidate-specific smoke scenario exercises the changed
    orchestration or boundary; focused regression checks pass; then
    `devtool run -- cargo xtask check` certifies the complete local feedback
    surface before commit.

- [ ] Task 5: Demonstrate repeated pre-merge CI improvement
  - Contract: collect treatment observations against Task 2's controlled
    baselines and apply the spec's paired medians, adaptive third-pair triggers,
    and cold/warm classification exactly. Warmed PR reruns are the pre-merge
    proxy.
  - Verification: checked-in evidence shows at least two matched pairs per path,
    any required third pairs, independent ≥10% or three-minute results for cold
    and warm paths, runner-minute impact, and unchanged required-check outcomes.

- [ ] Task 6: Corroborate with the merge-group result
  - Contract: record the actual post-change merge-group run on issue #1463 or
    its pull request after enqueue. Compare it with the historical merge-group
    baseline and Task 5's warmed-treatment range under the approved spec.
  - Verification: the linked result independently meets the threshold and falls
    within the proxy range. A contradiction is recorded and leaves the issue
    open for another applicable merge-group observation or corrective change; it
    is never marked complete as a passing delivery.

## Ordering and delegation

- Task 1 precedes new controlled measurements so every observation uses the same
  timing vocabulary.
- Tasks 2 and 3 may run in parallel after Task 1; they share the phase/report
  schema above and do not edit the same probe implementation.
- Task 4 waits for baseline/candidate exploration and boundary proof.
- Task 5 waits for the implementation commit and is the pre-merge delivery
  boundary.
- Task 6 occurs only after explicit merge approval creates a real merge-group
  run; issue closure waits for its result.
- If measurements reject every safe candidate, Task 4 records that result rather
  than adding speculative complexity; the issue remains open because the
  performance acceptance criterion is unmet.

## Risk checks

- Final `jaunder-coverage` and all final `jaunder-e2e` result derivations remain
  Cachix-ineligible on pull-request and merge-group refs.
- Cacheable support inputs are not hidden by verdict-name matching.
- Every relevant source change still changes the derivation that consumes it;
  unrelated source changes retain identity where promised.
- Validation still runs static checks, wasm budget/tests, host tests, coverage,
  doctests, Elisp coverage, server-function coverage, and all failure checks.
- E2e remains SQLite/PostgreSQL × Chromium/Firefox on independent runners, with
  unconditional diagnostics and the zero-panic assertion.
- Timing instrumentation cannot turn a failed command into success, reorder
  gates, or omit evidence on failure.
- Measurement reports unavailable attribution as unavailable; it never infers a
  substitution, local build, VM phase, or cache state without evidence.
- `pull_request`, `push` to `main`, and `merge_group` retain equivalent required
  contexts.
