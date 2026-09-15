# Issue 1473: Prove safe support-output caching

## Outcome

Jaunder either reuses demonstrably safe coverage and end-to-end support outputs
through Cachix for a material CI benefit, or retains the current broad exclusion
with an evidence-backed rejection. Every final coverage and end-to-end verdict
continues to execute for the exact pull-request, push, or merge-group ref being
tested.

## Load-bearing decisions

- A cacheable support output may contain preparation needed by a gate, but it
  must contain no test result, coverage result, trace, status, diagnostic, or
  other evidence capable of standing in for a current-ref verdict.
- Cache eligibility is proved from the actual Nix dependency and runtime closure
  of each admitted output. Output names, upload logs, and naming conventions are
  not safety evidence.
- The proof is fail-closed: an unknown output, incomplete inventory, unavailable
  closure, malformed policy, or unclassified path makes the candidate ineligible
  rather than presumed safe.
- Every final Rust coverage producer and consumer output, every backend/browser
  end-to-end result, and every aggregate or lifted equivalent remains ineligible
  for upload and substitution.
- Reusing preparation must not bypass fresh test execution, zero-panic
  detection, backend/browser coverage, diagnostic lifting, server-function flow
  verification, failure propagation, or merge-group combined-state testing.
- Cache safety and source invalidation are separate obligations. Every admitted
  support output must change identity when any input capable of affecting its
  behavior changes, while unrelated changes should preserve its identity.
- The current broad `jaunder-coverage|jaunder-e2e` exclusion remains in force
  until the stronger safety proof passes. If the available Nix and Cachix
  semantics cannot establish that proof, no cache boundary is narrowed.
- Safety remains structural after the experiment: a checked regression probe
  derives the relevant population and rejects policy drift rather than relying
  on a one-time reviewed list alone.
- The experiment inventories all relevant coverage and end-to-end support and
  final outputs, but performance measurement may prioritize the support outputs
  with the strongest plausible critical-path effect.
- Benefit is measured from comparable completed GitHub Actions runs, including
  cache setup, upload, download, substitution, and any duplicated runner work.
  Matched baseline/treatment observations cover all four combinations of
  source-changing or narrow changes and cold or warmed candidate prerequisites.
  Runs without enough cache-state evidence are reported but excluded; a missing
  combination disqualifies the candidate from narrowing the filter.
- Each of the four combinations produces separate median percentage changes for
  complete required-check wall-clock time and aggregate runner time; the
  combinations are not pooled. A treatment is material when at least one
  combination improves by 10% or more on either axis and no combination
  regresses by 10% or more on either axis. Thus a warmed-path win may qualify,
  but it cannot hide material cold-path or change-class costs.
- Measurement is adaptive and paired within each of the four combinations. Start
  with two matched baseline/treatment pairs and require a third pair when the
  first two improvements have opposite signs, differ by more than two minutes,
  or their two-pair median lies within one minute or three percentage points of
  the 10% threshold. The median of all three pairs is then authoritative.
- If no candidate proves both safety and material benefit, the issue succeeds by
  retaining the broad filter and recording why each candidate was rejected.

## Acceptance

- A checked-in, machine-readable inventory distinguishes every relevant support
  output from every final coverage or end-to-end verdict output and is
  accompanied by a concise human-readable explanation.
- An automated regression check proves that every newly cache-eligible output's
  complete closure excludes all final verdicts and equivalent result-bearing
  artifacts.
- The regression check has positive and negative proof arms: representative
  support outputs are admitted, every final output is rejected, and deliberate
  closure leakage or incomplete classification fails.
- Source-boundary probes demonstrate both required invalidation and preserved
  reuse for every support-output class admitted by the policy.
- Fresh pull-request and merge-group evidence confirms that all required final
  coverage and end-to-end derivations execute for the tested ref even when
  support outputs are substituted.
- A checked-in report links the baseline and treatment runs, identifies all four
  source-changing/narrow and cold/warmed combinations, reports wall-clock and
  runner-time medians separately, accounts for transfer/setup costs, and states
  comparability limitations.
- The report applies the 10% retention rule to each combination separately,
  identifies the qualifying combination and axis, confirms that neither axis
  regresses by 10% or more in any combination, and records either the narrowed
  safe policy or the evidence-backed decision to leave the broad filter
  unchanged.
- Existing CI required contexts and every coverage, backend, browser, panic,
  diagnostic, failure, and merge-group semantic retain their current verdicts.

## Boundaries

- No final or equivalent result-bearing output becomes uploadable or
  substitutable, even as a transitive consequence of admitting support work.
- No changed-path skipping, test suppression, coverage reduction, backend or
  browser reduction, or weakening of fail-closed evidence validation.
- No preparation job, changed CI job graph, or new cache service justified only
  by avoiding duplicated work; it must satisfy the measured retention rule.
- No application-runtime optimization or general Nix source-closure cleanup
  unrelated to proving and measuring the candidate cache boundary.
- No claim of benefit from unmatched runs, name-only evidence, or internal phase
  speedups that do not improve the complete required-check path or runner use.
