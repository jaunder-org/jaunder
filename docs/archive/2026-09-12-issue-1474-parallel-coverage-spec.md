# Parallel Rust coverage execution

## Outcome

Jaunder will measure two-worker Rust coverage execution locally and across CI
runners, then ship the smallest fan-out that materially shortens either target
surface without materially regressing the other. If no treatment meets that bar,
production coverage behavior remains unchanged and the checked-in report records
the rejection.

A shipped fan-out preserves `cargo xtask coverage` and the Nix coverage checks
as the authoritative interface and keeps one per-ref merged coverage and CRAP
verdict over the complete root-workspace test population.

## Load-bearing decisions

- The experiment compares the current unpartitioned producer with three
  two-worker treatments: nextest `slice:1/2` + `slice:2/2`, nextest `hash:1/2` +
  `hash:2/2`, and a backend-oriented comparator derived only from current test
  identities.
- Slice and hash treatments are both eligible for production selection.
  Selection follows repeated elapsed-time and census evidence rather than a
  prior preference.
- Backend-oriented selection is measurement-only when it depends on generated
  rstest name patterns. No production classifier may infer backend ownership
  from names such as `case_1_sqlite` or `case_2_postgres`.
- No new test annotations or backend-selection API will be introduced for this
  investigation. A stable explicit classification contract, if later justified,
  requires separate design work.
- One authoritative, unfiltered
  `cargo nextest list --workspace --message-format json` census defines the
  expected population before partitioning.
- The union of worker terminal records must reconcile exactly to that census:
  every expected identity is executed or ignored exactly once, no identity is
  duplicated, and every worker failure remains visible.
- Workers retain `--no-fail-fast`. Orchestration waits for every started worker
  so a failure cannot erase another worker's diagnostics or population evidence.
- Each worker owns distinct JUnit, raw LLVM profile, log, and diagnostic paths.
  Concurrent commands never write a shared report or temporary file.
- Raw profiles are merged before report generation. Text coverage, LCOV, CRAP,
  exclusions, executable-line totals, and the final gate are computed once over
  the merged union.
- The versioned producer status remains one aggregate record with the existing
  required-stage, failure-category, duration, and population invariants.
  Partition details are subordinate diagnostics, not alternate green verdicts.
- Local experiments measure both independent nextest defaults and a fixed
  aggregate concurrency budget divided between workers. A production local
  configuration may use only a treatment whose timing evidence shows no harmful
  oversubscription.
- The CI treatment runs its two workers on separate runners and measures the
  complete preparation, support substitution or transfer, shard, and aggregation
  path. Instrumented compilation is performed once through a dedicated
  support-output boundary rather than duplicated independently by every worker.
- The instrumented support output is eligible for Cachix reuse only if its full
  source, configuration, dependency, compiler, coverage-tool, and test-binary
  inputs participate in its Nix identity.
- Final worker execution outputs, raw profiles, aggregate reports, and final
  coverage verdict outputs remain per-ref results and ineligible for Cachix
  upload or substitution.
- Cache eligibility is proved from derivation/runtime closure membership or an
  equally strong machine-checkable invariant. Derivation names and successful
  upload logs are not safety evidence.
- An automated probe fails if any cache-eligible support output can carry,
  reference, substitute, or otherwise reintroduce a final coverage or e2e
  verdict output.
- The existing broad Cachix exclusion remains unless the support boundary passes
  that proof. A failed proof yields a measured rejection, not a weaker filter.
- Repeated comparable evidence means at least two baseline and two treatment
  observations per claimed target surface on the same source revision,
  equivalent hardware class, and identical declared build/cache state. Run order
  is alternating or otherwise counterbalanced so systematic warming cannot favor
  a treatment. Quiescence-sensitive local observations are valid only during an
  explicitly declared unloaded-system window.
- A production treatment must improve either local or CI coverage elapsed time
  by at least 10% or three minutes. The other measured target surface may vary
  but must not regress by 10% or more.
- Local elapsed time, CI elapsed time, orchestration/transfer time, compilation
  time, profile merge time, and aggregate runner consumption are reported
  separately.
- If the support boundary and fan-out ship, the pull request closes #1473 with
  its closure inventory, safety probe, and cache measurements. #1472 remains
  open because broader validation topology is outside this decision.

## Acceptance

- A checked-in report identifies the exact source revision, hardware/runner
  class, worker strategy, concurrency policy, build/cache state, observation
  order, and stage timings for every accepted observation.
- The report contains at least two comparable baseline and treatment
  observations for each surface used to justify adoption, plus local and CI
  results sufficient to enforce the non-winning surface's regression budget.
- Slice, hash, and backend-oriented treatments report their complete worker
  assignments and union reconciliation against the same authoritative census.
- Evidence proves zero missing and zero duplicate test identities and preserves
  the existing `expected = executed + ignored` contract.
- Controlled failure cases prove that a worker test failure, abnormal worker
  exit, malformed or missing JUnit, missing profile, duplicate identity, and
  incomplete partition all produce a non-green aggregate status. A simultaneous
  two-worker failure with distinct causes preserves both terminal records, both
  diagnostics, and the available population evidence in that single aggregate.
- The merged treatment and baseline produce identical executable-source
  membership, exclusions, line-hit semantics, CRAP policy input, and final
  pass/fail verdict. Any count difference is explained and shown not to weaken
  coverage.
- The derivation/closure inventory names every cache-eligible support output and
  every final coverage/e2e execution or verdict output.
- The automated cache-safety probe enumerates the actual upload-eligible output
  set, proves every final coverage/e2e execution and verdict output is rejected
  for upload and unavailable for cache-only substitution, demonstrates that
  eligible support closures contain no final verdict output, and proves relevant
  source/configuration changes invalidate the support output.
- Completed cold and warm Actions samples quantify support-cache hits, misses,
  transfer or substitution overhead, shard duration, aggregate duration,
  workflow critical path, and runner consumption.
- Final coverage and e2e results execute genuinely for the pull-request head and
  merge-group ref; no previously cached green result can satisfy either required
  verdict.
- `cargo xtask check`, `cargo xtask validate --no-e2e`, coverage status
  validation, coverage/CRAP policy, backend parity, panic detection, and
  merge-group checks retain their existing semantics.
- A treatment is production-eligible only if it satisfies every correctness,
  census, failure-aggregation, coverage-equivalence, cache-isolation,
  concurrency, timing, and regression requirement above. If none does, the
  report states that conclusion and the production producer, CI topology, and
  Cachix filter remain unchanged.

## Boundaries

- Doctest execution and census remain under their separate authority and are not
  folded into LLVM coverage partitions.
- E2E execution topology, browser coverage, Elisp coverage, and general
  validation fan-out are unchanged except for the cache-safety proof's
  obligation not to expose their final verdict outputs.
- This work does not reduce test population, backend parity, coverage
  exclusions, CRAP policy, panic detection, or merge-queue retesting.
- This work does not introduce backend-name parsing as production policy, a
  second coverage command, a compatibility path, or a bypass around the Nix
  coverage authority.
- More than two coverage workers, runtime-history-based scheduling, and
  repository-wide CI topology redesign remain outside scope.
