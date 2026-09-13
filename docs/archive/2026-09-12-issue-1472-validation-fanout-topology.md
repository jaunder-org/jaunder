# Issue 1472: Evaluate validation fan-out topology

## Outcome

Determine, with repeated GitHub Actions measurements, whether splitting non-e2e validation into two independently runnable lanes materially shortens the required workflow critical path. Keep the split only if it improves comparable workflow wall-clock by at least 10% or three minutes after setup and aggregation overhead; otherwise land the measured rejection with the production workflow unchanged.

## Current evidence

The post-#1463 workflow runs one `Validate (no e2e)` job beside the existing four-way SQLite/PostgreSQL × Chromium/Firefox e2e matrix. Validation remains the critical path:

- cold source-changing PR run 34526589815: 40:06 workflow, 40:02 validation;
- warm merge-group run 34530528286: 32:04 workflow, 32:00 validation;
- post-#1463 warmed runs are approximately 23–28 minutes, with validation still slower than e2e;
- run 34641076765 recorded roughly 8:16 of Rust coverage inside a 28:02 validation job;
- narrow run 34710334717 recorded roughly 7:08 of Rust coverage inside a 23:34 validation job.

The current validation command serializes host checks, hermetic static checks, WASM budget/tests, host tests, Rust coverage, doctests, and Emacs Lisp coverage. The Nix producers are already separate derivations, but their host orchestration is serial. A serial preparation dependency cannot improve this critical path: it adds setup and transfer before fan-out. The e2e matrix is already distributed and is outside this experiment.

## Selected experiment

Trial two independent full-VM lanes from one evolving pull request:

1. **Core lane** — clean-tree precheck, the ordered host gate, hermetic `static-docs` and `static-code`, WASM budget, host product tests, WASM tests, doctests and their host verdict, and Emacs Lisp coverage and its host verdict.
2. **Coverage lane** — clean-tree precheck, Rust coverage producer, Nix coverage gate, host coverage verdict, and the coverage source-drift probe.
3. **Aggregate lane** — an `ubuntu-slim` result-only job named `Validate (no e2e)` that depends on both lanes and succeeds only when both succeed.

The core and coverage lanes start independently. There is no cache-preparation dependency and no artifact transfer between them. Each lane owns and uploads its own diagnostics. The Nix source-closure probe remains with the core lane.

## Command seam

Add one CI-specific xtask interface analogous to the existing one-combination `cargo xtask e2e <backend> <browser>` interface:

```text
cargo xtask ci-validate core
cargo xtask ci-validate coverage
```

`ci-validate` is host-only and verify-only. It hides lane composition behind a two-value enum rather than exposing individual check toggles. Both lanes use the same private orchestration functions as `cargo xtask validate --no-e2e`; the existing local command remains the complete serial confidence gate and retains its current behavior.

The lane catalogs must be a duplicate-free, exhaustive partition of non-e2e validation. Tests assert that partition and preserve the ordered host-gate invariants. A failure in either lane remains fail-closed and produces the same producer/consumer verdict and diagnostic evidence as the existing full validation path.

## Workflow contract

- Rename the executing validation jobs to non-required internal names such as `Validation core` and `Validation coverage`.
- Reuse `Validate (no e2e)` as the stable aggregate required-check name, so branch protection and `cargo xtask pr watch/land` retain their interface.
- Run both lanes for `pull_request`, `push` to `main`, and `merge_group`.
- Leave the four e2e matrix jobs and `e2e gate` unchanged.
- Keep all build jobs on pinned `ubuntu-24.04` x86_64 full VMs; keep the result-only aggregate on `ubuntu-slim`.
- Preserve per-ref execution and Cachix ineligibility of final coverage and e2e verdict derivations.
- Preserve every existing diagnostic upload and source-drift probe, assigning each to the lane that owns its evidence.

## Measurement protocol

Use temporary committed marker changes on the evolving PR to create comparable source-changing and narrow documentation-only heads. Preserve every completed run URL in the checked-in report, then clean the final commit series before landing.

For baseline and treatment:

- collect at least two successful observations for each compared class;
- use same-head reruns to identify warmed behavior without presenting them as cold runs;
- record workflow wall-clock, required-check critical path, every lane's setup and command time, aggregate time, and the sum of job elapsed time as a runner-consumption proxy;
- record available xtask/Nix phase timings and mark unavailable attribution as unavailable rather than inferring it;
- separate source-changing, narrow-change, and warm observations;
- reject failed, cancelled, or infrastructure-invalid runs from timing comparisons while retaining them as diagnostic evidence;
- compare medians and individual observations, not one favorable run.

The report models but does not live-trial rejected alternatives: same-runner Nix concurrency, wider per-check runner fan-out, and serial cache preparation. It must account for setup, checkout, Nix realization/substitution, cache transfer, aggregation, and contention risks.

## Decision rule

Keep the two-lane topology only if repeated comparable Actions runs demonstrate at least one of:

- a 10% reduction in workflow wall-clock; or
- a three-minute reduction in workflow wall-clock.

The improvement must hold without moving another required check onto the critical path or weakening any verdict. Runner consumption is reported separately and is not itself a success criterion.

If the threshold is not met, revert the experimental workflow and xtask lane interface before landing. Land only the checked-in comparison and measured rejection.

If the threshold is met, retain the implementation and add an ADR that narrowly supersedes ADR-0034's single non-e2e validation-job description while preserving its distributed-check and stable-required-context principles. Project `ARCHITECTURE.md` and operational CI documentation must reflect the accepted topology.

## Acceptance

- A checked-in report identifies the current post-#1463 critical path and cites repeated comparable Actions runs for source-changing, narrow, and warmed observations.
- The report includes stage, setup, aggregation, and runner-consumption measurements with explicit comparability limits.
- Every required validation surface remains present and fail-closed behind one trustworthy `Validate (no e2e)` aggregate result.
- The SQLite/PostgreSQL × Chromium/Firefox e2e matrix and `e2e gate` remain unchanged.
- Final coverage and e2e verdict derivations execute for every tested ref and remain ineligible for Cachix reuse.
- No serial preparation or transfer dependency is introduced.
- The production topology is retained only if it clears the issue threshold; otherwise the workflow remains unchanged and the measured rejection is recorded.

## Boundaries

Do not split Rust coverage by backend; issue #1474 owns that experiment. Do not change Cachix support-output eligibility; issue #1473 owns that proof. Do not parallelize individual host-gate steps, reduce coverage, remove checks, replace hermetic checks with host-only checks, alter e2e distribution, or change merge-queue semantics.
