# Issue #1463: CI wall-clock baseline and candidate screen

## Decision frame and evidence rules

This is the Task 2 baseline required by
[#1463](https://github.com/jaunder-org/jaunder/issues/1463) and its
[approved spec](../specs/2026-09-11-issue-1463-lower-ci-wall-clock.md). The
target is elapsed time until the last trustworthy required check, **not** the
sum of concurrent runner time. The two historical Actions observations below are
immutable baseline evidence, not baseline/treatment pairs: they differ in ref,
event, change, cache population, and cannot isolate an implementation.

GitHub exposes job and top-level-step timestamps for these pre-instrumentation
runs; it does not expose Nix evaluation, substitution, local-build,
VM-start/readiness, or test-execution timing inside the monolithic `xtask`
steps. Those phase fields are therefore **unavailable**, not inferred from job
duration or a cache action. The durable phase recorder added after these runs is
required for future controlled measurements.

The current workflow defines one `ubuntu-24.04` validation job, four independent
`ubuntu-24.04` backend/browser e2e jobs, and an `ubuntu-slim` aggregation job;
all execute on `pull_request` and `merge_group`
([workflow](../../../.github/workflows/ci.yml)). That graph and the
x86_64/full-VM requirement make the following job comparison meaningful as an
observation of the then-current workflow, but not a matched comparison with
historical runs that used a different graph or runner label. Both baseline refs
identify the same setup toolchain:
[`cachix/install-nix-action@v31` and `cachix/cachix-action@v17`](https://github.com/jaunder-org/jaunder/blob/92eb4c55ae01beb5caa4e5070ce3398972e9773c/.github/actions/setup-ci/action.yml),
with `actions/cache@v6` for the host-only xtask cache; the
[merge-group ref carries the same setup action](https://github.com/jaunder-org/jaunder/blob/79698f0b52014d1104af78b3c33ef1894ab4a0b4/.github/actions/setup-ci/action.yml).
Exact Nix input-store attribution remains unavailable from the Actions surface.

## Completed Actions baselines

| Class                                    | Run / ref                                                                                                                                                                                     |                            Workflow wall-clock | Required-check critical path            | Sum of job elapsed time | Cache / realization classification                                                                                                                                            | Comparability limits                                                                                                                                                                                                                               |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------: | --------------------------------------- | ----------------------: | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Cold source-changing PR                  | [#2171](https://github.com/jaunder-org/jaunder/actions/runs/34526589815), `pull_request`, [`92eb4c5`](https://github.com/jaunder-org/jaunder/commit/92eb4c55ae01beb5caa4e5070ce3398972e9773c) |                                          40:06 | `Validate (no e2e)` (40:02)             |                  128:09 | #1463 records roughly 30 locally built derivations versus roughly 5 in the warm observation; per-derivation substitution/build attribution is unavailable in the Actions API. | Source change is `host/src/theme_package/css.rs` only in that commit; it is not a controlled perturbation, and no pre-instrumentation Nix/VM phase split exists.                                                                                   |
| Warm merge-group                         | [#2173](https://github.com/jaunder-org/jaunder/actions/runs/34530528286), `merge_group`, [`79698f0`](https://github.com/jaunder-org/jaunder/commit/79698f0b52014d1104af78b3c33ef1894ab4a0b4)  |                                          32:04 | `Validate (no e2e)` (32:00)             |                   83:27 | The issue records the smaller roughly-five-derivation realization count. This is evidence consistent with a warmed cache, not proof of every path's Cachix status.            | Queue merge commit has two parents and a much larger change set; it is historical context, not the treatment for #2171.                                                                                                                            |
| Narrow/docs-only controlled Nix baseline | [#1289 measurement report](2026-09-04-issue-1289-nix-invalidation-boundaries.md), post-change docs marker                                                                                     | 235.607 s local sequential `validate --no-e2e` | N/A — not an Actions required-check run |                     N/A | `static-docs` realized in 19.994 s; `static-code`, site, wasm tests, coverage, doctests, and Elisp producer reused.                                                           | This is an intentionally isolated local Nix measurement, with an unpurged warm store and no GitHub runner or Actions workflow duration. It is evidence of source closure behavior only, excluded from CI wall-clock and runner-minute comparisons. |

“Sum of job elapsed time” is the sum of GitHub `started_at`→`completed_at`
intervals (including the `ubuntu-slim` gate). It is a runner-time proxy, **not a
claim about billed GitHub minutes**: the six jobs run concurrently and use two
runner classes. In particular, the cold sum exceeds the 40:06 critical path by
88:03; reducing that sum alone is not a wall-clock win.

### Cold PR: #2171

The
[GitHub job record](https://api.github.com/repos/jaunder-org/jaunder/actions/runs/34526589815/jobs?per_page=100)
pins the runner label to `ubuntu-24.04` for validation and every matrix job, and
`ubuntu-slim` for `e2e gate`.

| Job                                                                                                         | GitHub-visible job duration |                                                           Principal visible step duration |
| ----------------------------------------------------------------------------------------------------------- | --------------------------: | ----------------------------------------------------------------------------------------: |
| [Validate (no e2e)](https://github.com/jaunder-org/jaunder/actions/runs/34526589815/job/103036878980)       |                       40:02 | `Validate … via xtask`: 36:38; coverage drift probe: 0:21; Nix source-closure probe: 1:34 |
| [e2e (postgres/firefox)](https://github.com/jaunder-org/jaunder/actions/runs/34526589815/job/103036878867)  |                       25:56 |                                                                  `e2e … via xtask`: 25:04 |
| [e2e (sqlite/firefox)](https://github.com/jaunder-org/jaunder/actions/runs/34526589815/job/103036878962)    |                       25:35 |                                                                  `e2e … via xtask`: 24:43 |
| [e2e (sqlite/chromium)](https://github.com/jaunder-org/jaunder/actions/runs/34526589815/job/103036878976)   |                       19:23 |                                                                  `e2e … via xtask`: 18:32 |
| [e2e (postgres/chromium)](https://github.com/jaunder-org/jaunder/actions/runs/34526589815/job/103036879070) |                       17:10 |                                                                  `e2e … via xtask`: 15:37 |
| [e2e gate](https://github.com/jaunder-org/jaunder/actions/runs/34526589815/job/103045267771)                |                        0:03 |                                                                 require-matrix step: 0:01 |

The gate completed at 20:54:46 UTC; validation completed at 21:08:36 UTC. Thus
validation, rather than the slowest e2e job, was the final required result and
the observed 40:06 wall-clock critical path. Phase attribution for each
monolithic `xtask` step: Nix evaluation **unavailable**; Nix substitution
**unavailable**; Nix local build **unavailable**; VM startup/readiness
**unavailable**; test/gate execution **unavailable**; result lift
**unavailable**; post-gate checks are separately visible only for the two
validation probes.

### Warm merge-group: #2173

The
[GitHub job record](https://api.github.com/repos/jaunder-org/jaunder/actions/runs/34530528286/jobs?per_page=100)
shows the same job names and `ubuntu-24.04` labels, with `ubuntu-slim` for the
aggregate. The queue ref is the merge-group ref required by
[ADR-0077](../../adr/0077-adopt-github-merge-queue.md): it validates combined
state rather than branch-only evidence.

| Job                                                                                                         | GitHub-visible job duration |                                                           Principal visible step duration |
| ----------------------------------------------------------------------------------------------------------- | --------------------------: | ----------------------------------------------------------------------------------------: |
| [Validate (no e2e)](https://github.com/jaunder-org/jaunder/actions/runs/34530528286/job/103049864385)       |                       32:00 | `Validate … via xtask`: 28:36; coverage drift probe: 0:25; Nix source-closure probe: 1:28 |
| [e2e (sqlite/firefox)](https://github.com/jaunder-org/jaunder/actions/runs/34530528286/job/103049864505)    |                       17:06 |                                                                  `e2e … via xtask`: 16:12 |
| [e2e (postgres/firefox)](https://github.com/jaunder-org/jaunder/actions/runs/34530528286/job/103049864459)  |                       14:40 |                                                                  `e2e … via xtask`: 13:17 |
| [e2e (postgres/chromium)](https://github.com/jaunder-org/jaunder/actions/runs/34530528286/job/103049864546) |                       10:32 |                                                                   `e2e … via xtask`: 9:10 |
| [e2e (sqlite/chromium)](https://github.com/jaunder-org/jaunder/actions/runs/34530528286/job/103049864417)   |                        9:02 |                                                                   `e2e … via xtask`: 8:02 |
| [e2e gate](https://github.com/jaunder-org/jaunder/actions/runs/34530528286/job/103055231192)                |                        0:07 |                                                                 require-matrix step: 0:01 |

The e2e gate completed at 21:26:43 UTC, 14:17 before validation completed at
21:41:00 UTC. This supports the current conclusion that warmed validation is the
critical path. The same seven internal phase distinctions remain **unavailable**
for this pre-instrumentation Actions observation.

### Narrow-source baseline from #1289

[#1289](https://github.com/jaunder-org/jaunder/issues/1289) deliberately
measured one source marker at a time. The
[record](2026-09-04-issue-1289-nix-invalidation-boundaries.md) establishes the
relevant narrow closure facts:

- Before ADR-0178, a docs-only marker realized broad `static-checks` for 292.424
  s while all recorded non-static rows reused; the overall local run was 451.615
  s.
- After [ADR-0178](../../adr/0178-split-hermetic-static-check-boundaries.md),
  the docs-only marker realizes only `static-docs` (19.994 s), while
  `static-code` and the unrelated checks reuse; the local run was 235.607 s. The
  clean warm baseline was 160.112 s.
- A web marker still realizes `static-code`, site, coverage, doctests, and the
  Elisp VM producer; a `common` or `macros` marker realizes all source-consuming
  boundaries. This is expected dependency fan-out, not a candidate for
  changed-path skipping.

These measurements make source closure a supported candidate family but do not
demonstrate a CI improvement, a Cachix outcome, or a matched Actions pair.

### Local validation remeasurement

This supplementary local observation was captured from the untracked xtask
sidecar and coverage build log immediately after each run; the measured values
are preserved below. It is not an Actions observation, treatment pair, or
replacement for the historical baselines above.

The first clean `validate --no-e2e` run completed in **901,697 ms**. Its
serialized pre-coverage prefix was **257,431 ms**; `nix-coverage` was **622,186
ms**; `nix-coverage-gate` was **5,458 ms**; and its post-coverage suffix was
**19,658 ms**. The numbers reconcile exactly. The coverage `build.log` records
only `buildPhase completed in 9 minutes 54 seconds` (594,000 ms). It does not
distinguish compilation, SQLite/PostgreSQL execution, LLVM report generation, or
CRAP/report work, so those attributions remain **unavailable**.

An exact same-ref rerun completed in **205,904 ms**, with `nix-coverage` reused
in **772 ms**. This is a reused-output floor, **not** representative per-ref
coverage verdict execution and not a matched Actions pair. It supplies no
post-change improvement claim.

The newer controlled instrumented clean-committed `validate --no-e2e`
measurement completed in **1,866,983 ms**: `nix-static-code` **469,141 ms**,
`nix-coverage` **819,615 ms**, `nix-coverage-gate` **8,265 ms**, and
`nix-elisp-coverage-producer` **313,868 ms**. The coverage status records
workspace resolution 24 ms, profile cleanup 102 ms, test census 292,235 ms,
instrumented test run 473,889 ms, population reconciliation 10 ms, text report
5,008 ms, LCOV 4,858 ms, and CRAP 249 ms.

This controlled local run is not an Actions sample and is not comparable to the
earlier local baseline: the new devtool source invalidated broad derivations. It
identifies a hypothesis, not a treatment result: `cargo nextest list` builds
non-instrumented test binaries before `cargo llvm-cov nextest` builds and runs
the instrumented binaries. A safe experiment must establish whether that
duplicated binary build can be removed while preserving the test census and
union verdict; this report makes no improvement claim.

Treatment experiment 1 was a committed local run of **1,164,766 ms**, but it
**failed**; it supplies no timing or improvement claim. Its census succeeded in
231,941 ms and established the complete expected census of 4,692 tests. The
instrumented run failed in 753 ms because `cargo-llvm-cov` rejects `--no-report`
combined with `--no-clean`; population reconciliation then failed with invalid
JUnit/no tests. This proves only that the instrumented census wrapper built the
complete expected census and that the attempted wrapper combination fails before
execution. The corrected treatment must use documented `show-env` plus a plain
nextest run.

Treatment experiment 2 was a committed local run of **1,265,786 ms** and also
**failed**, so its total is excluded from whole-path timing claims. Its census
took 214,390 ms; the instrumented test run **succeeded** in 140,961 ms; and
population reconciliation **succeeded** for the complete 4,692-test census. That
isolates a **332,928-ms (5:32.928, 70.3%)** reduction in the instrumented
test-run stage relative to the 473,889-ms instrumented baseline. It does not yet
prove a complete verdict: text report generation failed in 116 ms because the
report command lacked the separate-flow `llvm-cov` environment and searched
`/build/source/target/llvm-cov-target` rather than the generated profraw
location. The corrected treatment must apply the same `show-env` wrapper to both
text and LCOV reports.

The measured serialized arithmetic gives an ideal, contention-free overlap
bound: `max(622,186, 257,431 + 19,658) = 622,186 ms`, or **279,511 ms
(4:39.511)** lower than the first run. This is only a physical bound. A
separate-runner fan-out would have only **99,511 ms** of setup, checkout, Nix
evaluation/realization, transfer, aggregation, and queue overhead available
before it misses the three-minute target; same-runner fan-out may instead slow
the coverage producer through CPU, disk, Nix, Cargo, and PostgreSQL contention.

Ranked conclusion: internal nextest partitioning with profile merge and one
final union report is the highest-potential but unproven candidate; it must
preserve the complete coverage census, combined SQLite/PostgreSQL behavior, and
one stateless union verdict. Parallel post-test reports have at most the
unattributed 28,186-ms remainder available, and pre-test preparation has no
measured duration; neither supports a three-minute claim. Splitting coverage
into final backend verdicts, reducing e2e, serial cache preparation, and
same-runner broad fan-out are rejected or unselected for the preserved
invariants and unmeasured contention described in the candidate screen. No
change is selected until a controlled experiment records shard setup,
compile/test/PG spans, profile merge, union verdict equivalence, wall-clock, and
runner-time.

## Cache and source-boundary evidence

The checked-in [setup action](../../../.github/actions/setup-ci/action.yml)
configures Cachix v17 with `pushFilter: "jaunder-coverage|jaunder-e2e"` and
leaves `pathsToPush` empty, so the filter is active. Cachix documents
`pushFilter` as a regular expression excluding derivations from pushing, warns
that it is ignored with `pathsToPush`, and warns that a path can still be pushed
through another path's closure
([Cachix action README](https://github.com/cachix/cachix-action/blob/master/README.md#push-configuration)).
Its daemon hook applies that unanchored expression with `grep -vEe` to each full
`/nix/store/<hash>-<name>` output path
([implementation](https://github.com/cachix/cachix-action/blob/master/src/main.ts#L362-L391)).
Thus it directly filters any matching substring, while still not establishing a
historical run's substitute/build classification or complete closure exclusion.

The intended final per-ref verdict names are `jaunder-coverage`,
`jaunder-coverage-gate`, the four
`jaunder-e2e-{sqlite,postgres}-{chromium,firefox}` results, and
`jaunder-e2e-checks`. They must remain ineligible, preserving the independently
executed coverage and e2e verdicts required by
[ADR-0032](../../adr/0032-e2e-zero-panic-gate.md) and
[ADR-0077](../../adr/0077-adopt-github-merge-queue.md).

Because the pattern is unanchored against full paths, it definitely also
directly filters cacheable support outputs: `jaunder-coverage-source-probe`, the
shared `jaunder-e2e` `buildNpmPackage`, and on-demand
`jaunder-e2e-<backend>-<browser>-single-worker` packages. The source-closure
facts identify the e2e matrix as four independent NixOS derivations over pinned
application/support/end2end inputs with an aggregate `symlinkJoin`; the existing
#1289 probe covers source identities, not this cache boundary. A safe future
cache contract must match the post-hash basename exactly (not merely use
`^jaunder-…$` against a full store path), enumerate the final-verdict set and
disjoint support set, and prove negative final-verdict plus positive support
eligibility. It must also document Cachix's closure caveat.

## Candidate screen — no threshold verdict

| Candidate                                    | Evidence / expected critical-path effect                                                                                                                                                                                                                                                                      | Status and rationale                                                                                                                                                                                                                                                            |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Serial cache-preparation job                 | #2171/#2173 show duplicated cold work but the issue’s critical-path model is `B + max(V,E)` before transfer; preparation adds setup, upload, download, and substitution before fan-out.                                                                                                                       | **Rejected pending measured net win.** It may lower runner-time proxy, but has no demonstrated wall-clock benefit and has no transfer-cost measurement.                                                                                                                         |
| Recombine or reduce the 2×2 e2e matrix       | Matrix e2e already finishes before warm validation; [ADR-0034](../../adr/0034-ci-e2e-matrix-distribution.md) records browser serialization in each VM and distribution as the wall-clock improvement.                                                                                                         | **Rejected.** It violates the preserved matrix-distribution decision and would trade elapsed time for fewer runners.                                                                                                                                                            |
| Validation fan-out / internal partitioning   | The local 901,697-ms run has a 622,186-ms coverage producer and a 277,089-ms non-coverage total. The ideal overlap ceiling is 279,511 ms, but coverage producer→gate→host consumer and the other producer/consumer tails remain ordered; a partitioned coverage implementation must retain one union verdict. | **Highest potential, unproven; not selected.** Same-runner contention and duplicate Nix/setup work may reverse savings. Separate-runner fan-out must keep all added setup/transfer/aggregation under 99,511 ms to retain a three-minute path, then prove matched Actions pairs. |
| Avoid duplicate coverage census binary build | The controlled baseline has a 292,235-ms census before a 473,889-ms instrumented run. Failed experiment 2 reduced the instrumented run to 140,961 ms while reconciling all 4,692 tests: a 332,928-ms (70.3%) stage-only reduction.                                                                            | **Promising but incomplete; not selected.** Its total is failed/non-comparable because text reporting lacked the separate-flow environment. Apply `show-env` to text and LCOV, then prove the complete union verdict before timing a treatment.                                 |
| Narrow Nix source closures                   | #1289 proves docs/static isolation and records supported versus necessary fan-out.                                                                                                                                                                                                                            | **Already beneficial for local realization; no CI wall-clock claim.** Future changes require a source probe; no new closure change is selected by this report.                                                                                                                  |
| Narrow Cachix exclusion to final verdicts    | The expression overmatches cacheable support identities; exact final names and the filter’s closure caveat are known.                                                                                                                                                                                         | **Measurement/probe prerequisite, not an improvement claim.** Must prove final verdict exclusion and support eligibility before any cache-boundary edit.                                                                                                                        |

No candidate has two matched cold and warmed treatment pairs, no adaptive
third-pair condition can be evaluated, and no candidate has a post-change
merge-group observation. Consequently **no 10%/three-minute threshold verdict is
applied**. The report records the baseline and rejections only; it does not
claim a post-change improvement.

## Source index

- [Issue #1463](https://github.com/jaunder-org/jaunder/issues/1463): run
  selection, objective, and historical derivation-count observation.
- [Cold Actions run #2171](https://github.com/jaunder-org/jaunder/actions/runs/34526589815)
  and
  [GitHub jobs API](https://api.github.com/repos/jaunder-org/jaunder/actions/runs/34526589815/jobs?per_page=100):
  run, ref, runner, job, and step timestamps.
- [Warm Actions run #2173](https://github.com/jaunder-org/jaunder/actions/runs/34530528286)
  and
  [GitHub jobs API](https://api.github.com/repos/jaunder-org/jaunder/actions/runs/34530528286/jobs?per_page=100):
  queue ref, runner, job, and step timestamps.
- [#1289 measurement report](2026-09-04-issue-1289-nix-invalidation-boundaries.md),
  [ADR-0178](../../adr/0178-split-hermetic-static-check-boundaries.md), and
  [#1289](https://github.com/jaunder-org/jaunder/issues/1289): controlled
  source-boundary evidence.
- The local xtask sidecar and coverage build log: clean/reused same-ref
  remeasurement, serialized-DAG arithmetic, and the only observed coverage
  sub-boundary; the measured values are preserved above because these runtime
  artifacts are intentionally untracked.
- [Validation dispatch](../../../xtask/src/dispatch.rs),
  [Nix check graph](../../../nix/checks.nix), and
  [coverage producer](../../../tools/devtool/src/coverage/emit.rs): serialized
  dependency and union-coverage constraints for the local candidate screen.
- [CI workflow](../../../.github/workflows/ci.yml),
  [CI setup action](../../../.github/actions/setup-ci/action.yml),
  [Cachix filter documentation](https://github.com/cachix/cachix-action/blob/master/README.md#push-configuration),
  and its
  [full-path filter implementation](https://github.com/cachix/cachix-action/blob/master/src/main.ts#L362-L391):
  workflow graph, runner identity, and filter semantics.
- [ADR-0032](../../adr/0032-e2e-zero-panic-gate.md),
  [ADR-0034](../../adr/0034-ci-e2e-matrix-distribution.md), and
  [ADR-0077](../../adr/0077-adopt-github-merge-queue.md): preserved e2e,
  distribution, and merge-group invariants.
