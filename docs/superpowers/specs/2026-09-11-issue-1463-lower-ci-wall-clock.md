# Issue 1463: Lower CI wall-clock time

## Outcome

The coverage producer becomes fast enough to run in the existing local
`validate --no-e2e` confidence gate instead of deferring coverage feedback to
CI. Pull-request and merge-group CI must remain no slower in a way that negates
that local value, and validation, coverage, and the distributed
SQLite/PostgreSQL × Chromium/Firefox end-to-end gate remain intact. CI reports
enough durable phase timing to attribute future regressions rather than treating
each aggregate Nix invocation as a black box.

## Load-bearing decisions

- The primary optimization target is the local coverage producer used by
  `validate --no-e2e`. CI wall-clock and runner-minute changes are measured and
  reported separately; reducing duplicated runner work alone is not evidence of
  a useful improvement.
- The checked-in baseline uses completed GitHub Actions runs and records run
  identity, ref and change class, job and step durations, critical path, cache
  or realization evidence, and comparability limits.
- The baseline covers three classes:
  - cold source-changing pull requests;
  - warmed merge-group runs;
  - narrow or documentation-only changes that expose unnecessary invalidation.
- Measurement is adaptive and pairwise. Each path starts with two matched
  baseline/treatment pairs. For each pair, absolute improvement is baseline
  workflow duration minus treatment workflow duration; percentage improvement
  divides that difference by the baseline duration. The decision statistic is
  the median paired absolute improvement or the median paired percentage
  improvement.
- A third matched pair is required when the first two improvements have opposite
  signs, differ by more than two minutes, or their two-pair median lies within
  one minute of the three-minute threshold or three percentage points of the 10%
  threshold. The median of all three pairs is then authoritative.
- A matched pair uses the same controlled source-perturbation class and affected
  closure, workflow and required-job graph, runner image, flake lock and
  toolchain, differing only by the candidate implementation and unavoidable ref
  identity. A cold pair starts without the candidate's shared cacheable
  prerequisites valid on the runner and reports their realization or local
  build; a warmed pair reports those prerequisites reused or substituted while
  the per-ref verdict derivations still execute. Runs lacking that evidence are
  reported but excluded from the threshold calculation.
- Before merge, warmed pull-request reruns may supply the post-change warmed
  pairs because they execute the same workflow and required derivations without
  bypassing the explicit merge approval gate. Historical completed merge-group
  runs remain part of the checked-in baseline. The actual post-change
  merge-group run must fall within the observed warmed-treatment range and
  independently meet the threshold against the comparable historical merge-group
  baseline. A contradictory result leaves the issue open for another applicable
  merge-group observation or a corrective change.
- A concise phase-timing summary remains as durable CI diagnostics. It
  distinguishes, where the underlying tools expose the boundary:
  - Nix evaluation and realization, including reuse, substitution, and local
    build attribution;
  - virtual-machine startup and readiness;
  - test or validation execution;
  - result lifting and post-execution gate checks.
- Timing data is both reviewable in the Actions surface and machine-readable in
  the existing xtask result/diagnostic model. Raw verbose tracing and a new
  telemetry system are out of scope.
- Existing completed-run evidence establishes validation as the current critical
  path: the referenced cold run completed in about 40 minutes and the warmed
  merge-group run in about 32 minutes, while all four e2e combinations completed
  earlier.
- The implementation is selected from measured critical-path evidence, not
  architectural plausibility. The Cachix exclusion boundary, unnecessary Nix
  source closure fan-out, and independently runnable warm validation work are
  measured before changing the job graph.
- Final coverage and e2e test-result derivations remain ineligible for Cachix
  reuse. Cacheable build inputs may be excluded only when their output is itself
  a per-ref verdict or contains an equivalent correctness result.
- The existing four independent backend/browser e2e jobs remain distributed
  across runners. Their stable aggregate required check remains dependent on all
  four combinations.
- Pull-request, push-to-main, and `merge_group` triggers retain equivalent
  required validation. Merge-queue combined-state testing is not replaced by
  branch-only evidence.
- Coverage policy, backend parity, browser coverage, wasm and doctest checks,
  Elisp coverage, server-function coverage, e2e panic detection, diagnostic
  lifting, and failure propagation retain their current semantics.
- A serial preparation job is rejected unless measurement includes its setup,
  upload, download, and substitution costs and demonstrates a net critical-path
  win. Avoiding duplicated work alone is insufficient.
- Validation fan-out is rejected when duplicated compilation, Nix evaluation,
  source staging, virtual-machine work, or runner contention erases the
  elapsed-time gain.
- Decision amendment (2026-09-12): the repository owner accepts the selected
  coverage-census reuse when its complete local producer measurement shows a
  material reduction that makes local coverage gating practical, even if cold CI
  improves by only about two minutes. This amendment supersedes the original
  10%/three-minute CI threshold for this measured candidate only. It does not
  add coverage to `prepush`; `validate --no-e2e` remains the existing explicit
  local confidence gate.
- Work still stops with an evidence-backed report rather than adding speculative
  complexity.
- Every changed Nix source or cache boundary gains a fail-closed regression
  probe. A source probe proves both that relevant changes invalidate the
  consumer and that unrelated changes retain identity. A cache probe proves both
  that each named per-ref verdict remains excluded and that each newly admitted
  non-verdict input is eligible for Cachix.
- Existing accepted decisions remain authoritative: ADR-0032 for the zero-panic
  e2e result, ADR-0034 for matrix distribution, ADR-0077 for merge-group testing
  and uncached final results, and ADR-0178 for measurement-led source-boundary
  narrowing.

## Acceptance

- A checked-in report identifies the cold pull-request, warmed merge-group, and
  narrow-change baselines from completed Actions runs, with exact run links,
  critical paths, phase attribution, and stated comparability limits.
- Durable CI output separates the major observable phases instead of reporting
  only one aggregate xtask or Nix duration. Missing distinctions imposed by Nix
  or GitHub are named explicitly rather than inferred.
- The selected coverage-census reuse is accepted by the 2026-09-12 decision
  amendment when:
  - the complete local coverage producer improves materially;
  - the same full test census executes across SQLite and PostgreSQL;
  - the final host coverage gate passes; and
  - CI measurements show no regression large enough to negate the local value.
- Cold and warmed CI observations, exclusions, denominators, and comparisons
  remain in the report as diagnostic evidence. They are not represented as
  matched threshold proof when prerequisite realization evidence is absent.
- The report states runner-minute impact independently from wall-clock impact
  and identifies any saving that affects billing but not the critical path.
- All existing required validation, coverage, backend, browser, wasm, doctest,
  Elisp, panic-detection, diagnostic, failure-propagation, and merge-group
  semantics remain intact.
- The existing broad Cachix exclusion for final coverage and e2e results remains
  unchanged. No cache-eligibility change is part of the accepted implementation.
- Every changed Nix source boundary has a regression probe that fails for both
  missing required invalidation and renewed unrelated invalidation.
- The checked-in report records rejected candidates and the measured cost or
  invariant that disqualified each one.
- The actual post-change merge-group run is linked and classified on issue #1463
  or its pull request before the cycle is closed. It must preserve every
  required-check outcome; CI timing remains diagnostic under the amended
  acceptance decision.

## Boundaries

- No test suppression, coverage reduction, host-only substitution for a hermetic
  gate, or weakening of backend/browser parity.
- No regrouping of the four e2e combinations onto fewer runners.
- No preparation dependency justified only by runner-minute savings.
- No changed-path skipping of final per-ref coverage or e2e execution.
- No broad CI provider migration, self-hosted runner deployment, or new
  remote-cache service.
- No application runtime performance work; this issue concerns gate
  orchestration, derivation boundaries, cache eligibility, and CI observability.
- No promise that GitHub or Nix exposes phase precision it does not provide;
  unavailable attribution is reported as unavailable.
