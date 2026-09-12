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

### Excluded Actions observation: #2210

[Run #2210](https://github.com/jaunder-org/jaunder/actions/runs/34624638383)
(`pull_request`, `985f81d`) failed overall because all four e2e jobs hit the
same timing-helper Nix Python type error: an unannotated list inferred without
integer `duration_ms`; the e2e gate consequently failed too. It is therefore
excluded from whole-workflow gate-success evidence, but its independently
successful validation jobs remain diagnostic evidence.

Attempt 1 validation ran 16:54:12–17:38:47 UTC (**44:35**). Setup ended at
16:54:57; the post-setup job tail was 43:50 including probes, while the
validation command itself was **41:32**. Attempt 2 validation succeeded
20:49:59–21:21:12 UTC (**31:13**): setup 0:49, validation command 27:47,
coverage probe 0:24, and Nix probe 2:02.

The correction is `e2e_phases: list[dict[str, object]]`. A local
`cargo xtask e2e sqlite chromium` then passed in **628,158 ms** with the sidecar
path exercised. This confirms the repair locally; #2210 contributes diagnostic
validation timing only.

### Treatment Actions run: #2211

[Run #2211](https://github.com/jaunder-org/jaunder/actions/runs/34632943787) is
green on both attempts, but is not yet a matched-pair threshold result. Attempt
1 was the cold observation: `Validate (no e2e)` was the workflow/job critical
path at **44:59** (18:23:01–19:08:00 UTC), with setup 0:55, validation command
39:15, coverage probe 0:26, and Nix probe 4:11. Its slowest e2e job was
SQLite/Firefox at 25:54. Attempt-1 artifacts were replaced by the rerun, so only
its GitHub step timestamps survive; no phase-sidecar claim is made for that
attempt.

Attempt 2 was the warmed green observation: validation was again the critical
path at **25:20** (19:08:50–19:34:10 UTC), with setup 1:31, validation command
20:34, coverage probe 0:18, and Nix probe 2:46. The slowest e2e job was
PostgreSQL/Firefox at 17:56. Its coverage status reconciled 4,722 expected and
executed tests: census 228,663 ms, instrumented run 139,218 ms, text 9,284 ms,
LCOV 9,179 ms, and all other stages 551 ms combined.

The timing instrumentation initially allowed the two probes to overwrite
`.xtask/last-result.json`. The workflow now preserves
`.xtask/validate-result.json` before probes and uploads it. The supporting
`cargo xtask check` passed in **325,260 ms**. This fixes evidence retention, not
a performance result; matched cold and warmed Actions pairs remain required
before applying the issue threshold.

### Final-head repeat: #2213

[Run #2213](https://github.com/jaunder-org/jaunder/actions/runs/34641076765) was
green on both final-head attempts. Attempt 1 validated in **28:01** and its
durable `validate-result` total was **1,338,856 ms**, including
`nix-static-docs` 14,625 ms, `nix-static-code` 15,110 ms, `wasm-budget` 74,916
ms, `wasm-tests` 15,065 ms, `nix-coverage` 479,028 ms, coverage gate 4,231 ms,
doctests 49,923 ms, doctest gate 4,228 ms, and Elisp producer 9,944 ms. Coverage
reconciled 4,722/4,722 tests; its stages were workspace 23 ms, cleanup 747 ms,
census 278,780 ms, instrumented run 140,433 ms, reconciliation 10 ms, text
11,692 ms, LCOV 11,589 ms, and CRAP 319 ms.

Attempt 2 validated in **28:02** with durable total **1,330,366 ms**:
`nix-static-docs` 4,876 ms, `nix-static-code` 10,052 ms, `wasm-budget` 72,321
ms, `wasm-tests` 20,833 ms, `nix-coverage` 495,733 ms, coverage gate 4,573 ms,
doctests 30,070 ms, doctest gate 3,135 ms, and Elisp producer 10,390 ms.
Coverage reconciled 4,722/4,722 tests; its stages were workspace 24 ms, cleanup
1,327 ms, census 287,046 ms, instrumented run 147,611 ms, reconciliation 13 ms,
text 11,335 ms, LCOV 11,090 ms, and CRAP 319 ms.

The two green final-head attempts are stable repeats. The #2210 attempt-2
diagnostic warm baseline (31:13) versus final-head warmed run (28:02) is **3:11
(191 s, 10.2%)**, numerically clearing the threshold for one warm diagnostic
pair—but #2210's failed e2e matrix and the required pair count prevent a
threshold verdict.

The #2210 attempt-1/#2211 attempt-1 cold comparison is also diagnostic only:
44:35 versus 44:59 regressed 0:24 at workflow/job level, while validation
commands improved 41:32 to 39:15 (**2:17, 5.5%**). It is not a qualifying
threshold observation. Spec-matched cold pairs with prerequisite
realization/substitution evidence, a successful baseline rerun, and the required
warm pairs remain necessary before any verdict.

### Controlled cold-marker diagnostics

Each comparison applies the identical temporary comment marker to
`common/src/lib.rs` and runs the full required graph; the marker was removed
after measurement. They are **controlled diagnostics, not spec-matched pairs**:
the baseline/treatment refs differ by more than the isolated implementation/ref
identity, and the runs lack the per-prerequisite realization/substitution
evidence the approved spec requires. The arithmetic below must not be used for
the threshold calculation.

| Comparison            | Baseline                                                                                      | Treatment                                                                                                       |                Wall-clock diagnostic |                           Runner-time proxy diagnostic |
| --------------------- | --------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | -----------------------------------: | -----------------------------------------------------: |
| A                     | [#1470](https://github.com/jaunder-org/jaunder/actions/runs/34650931625), all green, 44:05    | [#1467](https://github.com/jaunder-org/jaunder/actions/runs/34655060127), all green, 40:44                      |                 +3:21 (201 s), 7.60% |            8,212 s → 7,973 s: **−239 s** (3:59, 2.91%) |
| B — excluded workflow | [baseline](https://github.com/jaunder-org/jaunder/actions/runs/34658567242), all green, 42:55 | [treatment](https://github.com/jaunder-org/jaunder/actions/runs/34662786280), validation 40:41; workflow failed | +2:14 (134 s), 5.20% diagnostic only |                7,856 s → 7,522 s: −334 s (5:34, 4.25%) |
| C                     | [baseline](https://github.com/jaunder-org/jaunder/actions/runs/34665673659), all green, 34:51 | [treatment](https://github.com/jaunder-org/jaunder/actions/runs/34667793477), all green, 35:38                  |                −0:47 (−47 s), −2.25% | 7,238 s → 7,472 s: **+234 s** (3:54, 3.23% regression) |
| D                     | [baseline](https://github.com/jaunder-org/jaunder/actions/runs/34670826980), all green, 43:54 | [treatment](https://github.com/jaunder-org/jaunder/actions/runs/34673226997), all green, 41:47                  |                 +2:07 (127 s), 4.82% |             7,575 s → 7,531 s: **−44 s** (0:44, 0.58%) |

The A/C/D green diagnostics have a median runner-time-proxy saving of **44 s**.
These are summed job elapsed intervals, not billed-minute claims. No valid
matched cold set exists. Warm evidence remains one diagnostic pair plus stable
treatment repeats, short of the original two-pair CI criterion. The 2026-09-12
decision amendment accepts the candidate on its complete local coverage-producer
result without reclassifying these diagnostics.

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

Treatment experiment 3 is the first complete successful local verdict. The
committed run completed in **1,302,640 ms**; `nix-coverage` was **385,744 ms**;
and its gate was **5,800 ms**. Coverage stages were workspace resolution 21 ms,
cleanup 94 ms, census 203,463 ms, instrumented test run 155,359 ms,
reconciliation 7 ms, text report 4,939 ms, LCOV 4,712 ms, and CRAP 222 ms. All
4,692 expected tests executed and reconciled, and the final host coverage gate
passed.

Against the comparable instrumentation baseline, coverage-step time fell
**433,871 ms (7:13.871, 52.9%)**; summed producer stages fell **407,558 ms
(6:47.558, 52.5%)**; and the instrumented run fell **318,530 ms (5:18.530,
67.2%)**. The full local path fell **564,343 ms (9:24.343, 30.2%)**, but that
whole-path comparison is provisional because unrelated static and Elisp
derivation timings varied. Under the 2026-09-12 decision amendment, the complete
local producer result is the acceptance evidence; the Actions comparisons remain
diagnostics with their stated limits.

The measured serialized arithmetic gives an ideal, contention-free overlap
bound: `max(622,186, 257,431 + 19,658) = 622,186 ms`, or **279,511 ms
(4:39.511)** lower than the first run. This is only a physical bound. A
separate-runner fan-out would have only **99,511 ms** of setup, checkout, Nix
evaluation/realization, transfer, aggregation, and queue overhead available
before it misses the three-minute target; same-runner fan-out may instead slow
the coverage producer through CPU, disk, Nix, Cargo, and PostgreSQL contention.

Repository-owner decision (2026-09-12): accept the corrected census-wrapper
treatment for its complete local result. The **52.5% producer-stage reduction**
makes the existing `validate --no-e2e` coverage gate practical to run locally
rather than deferring coverage feedback to CI; that value outweighs the roughly
two-minute diagnostic cold-CI improvement. This does not add coverage to
`prepush`, and the Actions observations remain diagnostics rather than being
reclassified as matched threshold proof.

Internal nextest partitioning with profile merge and one final union report
remains unselected complexity. It would have to preserve the complete coverage
census, combined SQLite/PostgreSQL behavior, and one stateless union verdict.
Splitting coverage into final backend verdicts, reducing e2e, serial cache
preparation, and same-runner broad fan-out remain rejected for the preserved
invariants and unmeasured contention described in the candidate screen.

## Cache and source-boundary evidence

The Cachix-boundary experiment temporarily replaced the broad
`jaunder-coverage|jaunder-e2e` exclusion with an anchored list of seven final
output basenames and added a direct-filter probe for six intended support
outputs. `cargo xtask nix probe-source` passed that experimental direct-name
contract in **207,832 ms** after its first implementation was corrected for the
Nix 2.33 schema.

The experiment did not prove Cachix closure behavior. Cachix documents that
`pushFilter` can still admit an otherwise excluded path through another pushed
path's closure. Because final-verdict ineligibility is load-bearing, direct
basename matching is insufficient evidence. The narrower filter, output catalog,
and direct-filter probe were therefore **reverted**. The accepted branch retains
the existing broad exclusion and changes no Cachix eligibility boundary. The
experiment remains in this report only as rejected-candidate evidence.

## Candidate screen and threshold outcome

| Candidate                                    | Evidence / expected critical-path effect                                                                                                                                                                                                                                                                      | Status and rationale                                                                                                                                                                                                                                                            |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Serial cache-preparation job                 | #2171/#2173 show duplicated cold work but the issue’s critical-path model is `B + max(V,E)` before transfer; preparation adds setup, upload, download, and substitution before fan-out.                                                                                                                       | **Rejected pending measured net win.** It may lower runner-time proxy, but has no demonstrated wall-clock benefit and has no transfer-cost measurement.                                                                                                                         |
| Recombine or reduce the 2×2 e2e matrix       | Matrix e2e already finishes before warm validation; [ADR-0034](../../adr/0034-ci-e2e-matrix-distribution.md) records browser serialization in each VM and distribution as the wall-clock improvement.                                                                                                         | **Rejected.** It violates the preserved matrix-distribution decision and would trade elapsed time for fewer runners.                                                                                                                                                            |
| Validation fan-out / internal partitioning   | The local 901,697-ms run has a 622,186-ms coverage producer and a 277,089-ms non-coverage total. The ideal overlap ceiling is 279,511 ms, but coverage producer→gate→host consumer and the other producer/consumer tails remain ordered; a partitioned coverage implementation must retain one union verdict. | **Highest potential, unproven; not selected.** Same-runner contention and duplicate Nix/setup work may reverse savings. Separate-runner fan-out must keep all added setup/transfer/aggregation under 99,511 ms to retain a three-minute path, then prove matched Actions pairs. |
| Avoid duplicate coverage census binary build | Complete successful experiment 3 reconciled all 4,692 tests and passed the final host coverage gate. Against the instrumentation baseline, coverage-step time fell 433,871 ms (52.9%), producer stages 407,558 ms (52.5%), and the instrumented run 318,530 ms (67.2%).                                       | **Accepted by the 2026-09-12 decision amendment.** The local producer reduction makes the existing `validate --no-e2e` coverage gate practical before CI; diagnostic CI and runner-time comparisons remain reported without being called matched threshold proof.               |
| Narrow Nix source closures                   | #1289 proves docs/static isolation and records supported versus necessary fan-out.                                                                                                                                                                                                                            | **Already beneficial for local realization; no CI wall-clock claim.** Future changes require a source probe; no new closure change is selected by this report.                                                                                                                  |
| Narrow Cachix exclusion to final verdicts    | Anchored output basenames and direct-filter checks passed, but did not prove closure-based final-verdict ineligibility.                                                                                                                                                                                       | **Rejected and reverted.** The accepted branch retains the original broad Cachix exclusion and changes no eligibility boundary.                                                                                                                                                 |

The cold A/B/C/D arithmetic remains diagnostic only; no valid matched cold set
exists, and warm evidence likewise has only one diagnostic pair plus repeats.
The repository owner explicitly accepted the coverage-census reuse on 2026-09-12
because the complete local producer improved **52.5%**, making local coverage
gating materially more practical, while the diagnostic cold-CI median still
improved by about two minutes. The original CI threshold is not claimed. The
narrower Cachix experiment is excluded from the accepted implementation.
Merge-group verification remains pending the explicit merge gate.

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
- [Excluded Actions run #2210](https://github.com/jaunder-org/jaunder/actions/runs/34624638383):
  successful validation timing within an overall failed workflow and the
  e2e-helper failure classification.
- [Treatment Actions run #2211](https://github.com/jaunder-org/jaunder/actions/runs/34632943787):
  cold/warmed successful-attempt timings, artifact-retention limitation, and
  retained warm coverage status.
- [Final-head Actions run #2213](https://github.com/jaunder-org/jaunder/actions/runs/34641076765):
  green diagnostic repeats with durable validation and coverage-stage evidence.
- Controlled cold pairs:
  [A baseline](https://github.com/jaunder-org/jaunder/actions/runs/34650931625),
  [A treatment](https://github.com/jaunder-org/jaunder/actions/runs/34655060127),
  [B baseline](https://github.com/jaunder-org/jaunder/actions/runs/34658567242),
  [B treatment (excluded)](https://github.com/jaunder-org/jaunder/actions/runs/34662786280),
  [C baseline](https://github.com/jaunder-org/jaunder/actions/runs/34665673659),
  [C treatment](https://github.com/jaunder-org/jaunder/actions/runs/34667793477),
  [D baseline](https://github.com/jaunder-org/jaunder/actions/runs/34670826980),
  and
  [D treatment](https://github.com/jaunder-org/jaunder/actions/runs/34673226997):
  full-graph temporary-marker diagnostics and comparison arithmetic.
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
