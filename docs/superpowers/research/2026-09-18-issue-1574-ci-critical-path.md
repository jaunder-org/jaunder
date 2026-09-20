# Issue #1574: Validation critical-path fan-out

## Decision frame

This report records the Task 2 retention evidence for
[#1574](https://github.com/jaunder-org/jaunder/issues/1574). The retained
treatment replaces the serialized non-e2e core job with independent host,
hermetic, test-check, and coverage jobs behind the unchanged `Validate (no e2e)`
context. The source probe runs after test checks on their Nix-heavy runner while
preserving a separate result. Local `validate --no-e2e` remains serial.

The acceptance target is a 20% reduction in the validation critical path across
at least three successful cache-state-matched observations. Aggregate runner
time is reported separately. Final Rust coverage remains per-ref and is not
accepted from Cachix.

Durations use GitHub's `started_at` and `completed_at` timestamps. Validation
critical path runs from the earliest validation-lane start to the stable
aggregate's completion. Workflow critical path runs from the earliest required
job start to the later of `Validate (no e2e)` and `e2e gate`. Runner-time proxy
is the sum of job elapsed intervals, not a claim about billed minutes. With only
three samples, p95 is linearly interpolated between the second- and
third-slowest values; medians are also reported so the decision is not driven by
one run.

## Five-job candidate

The first candidate used an independent source-probe job. Its cohorts establish
the maximum measured fan-out benefit and expose the cold runner-cost problem
that motivated the retained four-job optimization.

### Cache-state selection

The three controls are consecutive successful `main` runs on immutable heads.
The three treatments are same-head reruns of
`79e13b03a72bc67fe5c3f2079e577e22dd6ba255`. Their retained diagnostics all show
the same warm/substituted classification: no selected Nix build log announced a
local derivation build, while the collected logs contain three fetch plans and
22 `copying path` records per observation.

The full Actions logs also prove that every validation job restored the same
exact host-build cache rather than falling back to a prefix or missing. The
cache contains `~/.cargo/registry`, `~/.cargo/git`, and `xtask/target`; its key
is derived from `xtask/Cargo.lock` and `flake.lock` by `setup-ci`.

| Observation         | Validation jobs | Exact key restored by every job                                                |      Cache size | Misses / prefix restores |
| ------------------- | --------------: | ------------------------------------------------------------------------------ | --------------: | -----------------------: |
| Control 35397128463 |               2 | `xtask-Linux-ea0143eb4115f446814b3805d702290bab87213653d932b3190ad4578a1db6e2` | 3,577,382,950 B |                    0 / 0 |
| Control 35354924272 |               2 | `xtask-Linux-ea0143eb4115f446814b3805d702290bab87213653d932b3190ad4578a1db6e2` | 3,577,382,950 B |                    0 / 0 |
| Control 35353083485 |               2 | `xtask-Linux-ea0143eb4115f446814b3805d702290bab87213653d932b3190ad4578a1db6e2` | 3,577,382,950 B |                    0 / 0 |
| Treatment attempt 2 |               5 | `xtask-Linux-ea0143eb4115f446814b3805d702290bab87213653d932b3190ad4578a1db6e2` | 3,577,382,950 B |                    0 / 0 |
| Treatment attempt 3 |               5 | `xtask-Linux-ea0143eb4115f446814b3805d702290bab87213653d932b3190ad4578a1db6e2` | 3,577,382,950 B |                    0 / 0 |
| Treatment attempt 4 |               5 | `xtask-Linux-ea0143eb4115f446814b3805d702290bab87213653d932b3190ad4578a1db6e2` | 3,577,382,950 B |                    0 / 0 |

Each setup action emitted both `Cache hit for` and `Cache restored from key` for
that full key. Together with the identical Nix-log classification, this closes
the cache-state match across both the host Cargo/xtask inputs and the selected
Nix work.

Two observations are excluded from threshold arithmetic:

- [run 35391070691](https://github.com/jaunder-org/jaunder/actions/runs/35391070691)
  is the 41:30 historical control, but it is cold: two local-build plans, four
  fetch plans, and 484 copied paths. Comparing it with a warm treatment would
  overstate the gain.
- [treatment attempt 1](https://github.com/jaunder-org/jaunder/actions/runs/35398969255/attempts/1)
  is successful but has four fetch plans and 23 copied paths. Attempts 2–4
  provide the required exact warm match instead.

### Raw observations

| Arm       | Run / attempt / immutable head                                                                                                                  | Validation path | Workflow path | Runner proxy | Validation runner proxy | Validation setup sum | Validation command sum | Source probe |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | --------------: | ------------: | -----------: | ----------------------: | -------------------: | ---------------------: | -----------: |
| Control   | [35397128463](https://github.com/jaunder-org/jaunder/actions/runs/35397128463), `157f8457daa6aadff9e713dc80cf35492cafa0a6`                      |           23:44 |         23:44 |       107:32 |                   36:11 |                 2:08 |                  30:40 |         2:27 |
| Control   | [35354924272](https://github.com/jaunder-org/jaunder/actions/runs/35354924272), `4040354094adb15d027e70eac9f50292dcbd5704`                      |           24:22 |         24:23 |       103:21 |                   35:16 |                 2:19 |                  29:20 |         2:40 |
| Control   | [35353083485](https://github.com/jaunder-org/jaunder/actions/runs/35353083485), `45703226a23b66c89683c3ddc88b1cb10a3584fb`                      |           25:01 |         25:01 |       104:30 |                   37:17 |                 2:08 |                  31:30 |         2:39 |
| Treatment | [35398969255 attempt 2](https://github.com/jaunder-org/jaunder/actions/runs/35398969255/attempts/2), `79e13b03a72bc67fe5c3f2079e577e22dd6ba255` |           17:38 |         24:40 |       118:12 |                   44:03 |                 7:02 |                  31:41 |         3:50 |
| Treatment | [35398969255 attempt 3](https://github.com/jaunder-org/jaunder/actions/runs/35398969255/attempts/3), `79e13b03a72bc67fe5c3f2079e577e22dd6ba255` |           17:51 |         23:10 |       120:08 |                   45:04 |                 6:59 |                  32:23 |         4:14 |
| Treatment | [35398969255 attempt 4](https://github.com/jaunder-org/jaunder/actions/runs/35398969255/attempts/4), `79e13b03a72bc67fe5c3f2079e577e22dd6ba255` |           19:07 |         20:50 |       108:56 |                   44:25 |                 6:04 |                  33:01 |         3:56 |

Setup is the sum of the complete shared `setup-ci` action across validation
jobs. Command is the sum of the top-level xtask validation steps; source-probe
is separate. The treatment deliberately duplicates setup across five runners.
The source probe's internal xtask work remained 149–152 seconds, while its
separate top-level job step took 230–254 seconds because it now owns its Nix
entry overhead rather than following the core command in an already-prepared
job.

#### Nix-backed command evidence

The table below sums the independently recorded `static-docs`, `static-code`,
WASM-budget, WASM-test, Rust-coverage, doctest, and Elisp-producer steps. It is
not runner time: those steps overlap in the treatment. The cache classification
comes from the accompanying Nix build logs rather than elapsed-time inference.

| Observation         | Recorded Nix-backed step sum | Local-build plans | Fetch plans | Copied paths |
| ------------------- | ---------------------------: | ----------------: | ----------: | -----------: |
| Control 35397128463 |                      717.9 s |                 0 |           3 |           22 |
| Control 35354924272 |                      628.9 s |                 0 |           3 |           22 |
| Control 35353083485 |                      727.3 s |                 0 |           3 |           22 |
| Treatment attempt 2 |                      644.8 s |                 0 |           3 |           22 |
| Treatment attempt 3 |                      691.5 s |                 0 |           3 |           22 |
| Treatment attempt 4 |                      654.9 s |                 0 |           3 |           22 |

The diagnostics do not timestamp each individual substituted path, so exact
per-path transfer time is unavailable. No local realization is inferred where
Nix did not announce one.

### Warm normalized comparison

| Metric                      | Control median | Treatment median |   Median change | Control p95 | Treatment p95 |         p95 change |
| --------------------------- | -------------: | ---------------: | --------------: | ----------: | ------------: | -----------------: |
| Validation critical path    |          24:22 |            17:51 |  −6:31 (−26.7%) |       24:57 |         18:59 | **−5:58 (−23.9%)** |
| Workflow critical path      |          24:23 |            23:10 |   −1:13 (−5.0%) |       24:57 |         24:31 |      −0:26 (−1.7%) |
| Whole-workflow runner proxy |         104:30 |           118:12 | +13:42 (+13.1%) |      107:14 |        119:56 |    +12:43 (+11.9%) |
| Validation runner proxy     |          36:11 |            44:25 |  +8:14 (+22.8%) |       37:10 |         45:00 |     +7:50 (+21.1%) |

The validation p95 clears the 20% threshold. The overall workflow does not yet
improve by 20% because Firefox becomes the critical path in every retained
sample; that is the next experiment rather than evidence against the validation
split.

The warm cost is explicit: median validation runner consumption rises 8:14,
principally because median setup rises from 2:08 to 6:59. The observed
whole-workflow runner proxy rises 13:42 (13.1%), though e2e variation
contributes to that total.

### Controlled cold source-invalidating cohort

The warm cohort answers steady-state behavior but not the cold source-changing
case. Draft baseline PR
[#1577](https://github.com/jaunder-org/jaunder/pull/1577) preserves the
pre-fan-out topology at `4040354094adb15d027e70eac9f50292dcbd5704`. Treatment PR
[#1583](https://github.com/jaunder-org/jaunder/pull/1583) carries the measured
fan-out at `4e901a9a102ae0cdc98adb2af1efdc053298b6ee`. For each observation, the
arm changes only a unique inert comment in `common/src/lib.rs`; the `common`
source boundary invalidates every broad source consumer. Every validation job
still restored the same exact 3,577,382,950-byte host cache key used by the warm
cohort. These are therefore matched cold Nix observations, not Actions
host-cache misses.

| Arm       | Run / immutable head                                                                                                       | Validation path | Workflow path | Runner proxy | Validation runner proxy | Validation setup sum | Validation command sum | Source probe |
| --------- | -------------------------------------------------------------------------------------------------------------------------- | --------------: | ------------: | -----------: | ----------------------: | -------------------: | ---------------------: | -----------: |
| Baseline  | [35407696576](https://github.com/jaunder-org/jaunder/actions/runs/35407696576), `80dc3d4e1d80be90bda2487aad2c8218acebf052` |           52:10 |         52:10 |       232:58 |                   77:15 |                 2:12 |                  71:36 |         2:38 |
| Baseline  | [35410824248](https://github.com/jaunder-org/jaunder/actions/runs/35410824248), `4b9e8b22625271f6f1b8a71cdd7df572a93d14ca` |           44:03 |         44:03 |       230:01 |                   71:22 |                 2:58 |                  65:11 |         2:01 |
| Baseline  | [35413288969](https://github.com/jaunder-org/jaunder/actions/runs/35413288969), `1c8b55de97e176b0626c5590148eabb5d1ea435f` |           52:38 |         52:38 |       238:15 |                   79:39 |                 2:22 |                  73:38 |         2:40 |
| Treatment | [35416028703](https://github.com/jaunder-org/jaunder/actions/runs/35416028703), `5a5fe5f6f39b0e74e5f9845f45dd54089957377c` |           27:11 |         43:25 |       270:25 |                  115:54 |                 7:33 |                  90:03 |        16:55 |
| Treatment | [35418273318](https://github.com/jaunder-org/jaunder/actions/runs/35418273318), `0b7bfc4ebba805668ee4075744e3897b67ad3c7a` |           31:41 |         45:34 |       285:35 |                  128:36 |                 5:25 |                 104:57 |        16:48 |
| Treatment | [35420489053](https://github.com/jaunder-org/jaunder/actions/runs/35420489053), `64eb8b508537478e72946261f64d6d5a3f11ac48` |           27:55 |         43:14 |       266:24 |                  121:49 |                 6:35 |                  97:19 |        16:36 |

The cold baseline's serialized core locally built the broad source derivations
before its source probe, so the probe then took about two minutes. In the
fan-out, the independent source-probe runner has an empty Nix store and repeats
broad source realization; its top-level step takes about 17 minutes even though
the internal post-realization probe remains about 2.5 minutes. The split also
repeats overlapping cold source realization across the hermetic, test-check,
coverage, and source-probe runners. That duplication is the principal cold
runner-cost penalty.

| Metric                      | Baseline median | Treatment median |   Median change | Baseline p95 | Treatment p95 |          p95 change |
| --------------------------- | --------------: | ---------------: | --------------: | -----------: | ------------: | ------------------: |
| Validation critical path    |           52:10 |            27:55 | −24:15 (−46.5%) |        52:35 |         31:18 | **−21:17 (−40.5%)** |
| Workflow critical path      |           52:10 |            43:25 |  −8:45 (−16.8%) |        52:35 |         45:21 |      −7:14 (−13.8%) |
| Whole-workflow runner proxy |          232:58 |           270:25 | +37:27 (+16.1%) |       237:45 |        284:04 |     +46:19 (+19.5%) |
| Validation runner proxy     |           77:15 |           121:49 | +44:34 (+57.7%) |        79:25 |        127:55 |     +48:30 (+61.1%) |

The cold validation p95 comfortably clears the 20% threshold and moves the
workflow critical path to unchanged Firefox e2e. It does not produce a 20%
overall workflow improvement on its own. The cost is material: p95
whole-workflow runner time rises 19.5%, and p95 validation runner time rises
61.1%, including roughly 14 extra median minutes in the independently cold
source-probe step.

## Source-probe colocation experiment

Because the independent cold source-probe VM accounted for roughly 14 extra
median runner-minutes, disposable PR
[#1589](https://github.com/jaunder-org/jaunder/pull/1589) tested a four-job
variant. It ran the source probe under `always()` after test checks on the same
VM, preserved the two xtask results separately, and made the aggregate depend on
the combined job. Three unique `common/src/lib.rs` markers formed its cold
cohort; attempts 2–4 of the final immutable head formed its warm cohort. Every
job restored the same exact host-cache key used above.

| Cache state | Observation                                                                                                                                     | Validation path | Workflow path | Runner proxy | Validation runner proxy | Validation setup sum | Validation command sum | Source probe |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | --------------: | ------------: | -----------: | ----------------------: | -------------------: | ---------------------: | -----------: |
| Cold        | [35439369264](https://github.com/jaunder-org/jaunder/actions/runs/35439369264), `2b66dbaebcf38754c740032153d3837c27f0ce68`                      |           32:02 |         46:19 |       273:42 |                  113:30 |                 5:01 |                 104:50 |         2:25 |
| Cold        | [35441533690](https://github.com/jaunder-org/jaunder/actions/runs/35441533690), `c992ced3deabc744abc627f683a8274517bdfe3a`                      |           32:32 |         44:25 |       275:36 |                  116:03 |                 5:22 |                 107:06 |         2:12 |
| Cold        | [35443728843 attempt 1](https://github.com/jaunder-org/jaunder/actions/runs/35443728843/attempts/1), `c96adb59923bc6be36ded103d6581a7d26295500` |           32:15 |         45:17 |       274:16 |                  111:40 |                 5:01 |                 103:03 |         2:26 |
| Warm        | [35443728843 attempt 2](https://github.com/jaunder-org/jaunder/actions/runs/35443728843/attempts/2), `c96adb59923bc6be36ded103d6581a7d26295500` |           19:33 |         22:05 |       116:03 |                   45:16 |                 4:59 |                  36:03 |         2:56 |
| Warm        | [35443728843 attempt 3](https://github.com/jaunder-org/jaunder/actions/runs/35443728843/attempts/3), `c96adb59923bc6be36ded103d6581a7d26295500` |           20:45 |         22:29 |       114:14 |                   43:06 |                 5:37 |                  33:53 |         2:27 |
| Warm        | [35443728843 attempt 4](https://github.com/jaunder-org/jaunder/actions/runs/35443728843/attempts/4), `c96adb59923bc6be36ded103d6581a7d26295500` |           16:10 |         22:19 |       107:39 |                   37:40 |                 5:52 |                  29:14 |         1:28 |

The preserved Nix build logs close the retained candidate's cache-state match.
Counts below cover the same eight selected build logs per observation; the full
source-probe step added no local-build plan, fetch plan, or copied-path record.
All six candidate runs also restored the exact host cache key recorded above.

| Cache state    | Observation           | Local-build plans / derivations | Fetch plans | Copied paths |
| -------------- | --------------------- | ------------------------------: | ----------: | -----------: |
| Cold baseline  | 35407696576           |                          3 / 22 |           5 |          470 |
| Cold baseline  | 35410824248           |                          2 / 20 |           5 |          471 |
| Cold baseline  | 35413288969           |                          2 / 20 |           5 |          472 |
| Cold candidate | 35439369264           |                          4 / 26 |           4 |          469 |
| Cold candidate | 35441533690           |                          2 / 21 |           5 |          473 |
| Cold candidate | 35443728843 attempt 1 |                          2 / 21 |           5 |          473 |
| Warm controls  | all three             |                           0 / 0 |      3 each |      22 each |
| Warm candidate | attempts 2–4          |                      0 / 0 each |      3 each |      22 each |

The warm arms are exact realization/substitution matches. Both cold arms perform
broad local source realization with the same order of build plans and copied
paths; the first candidate head additionally realizes the new workflow/xtask
source, so cold matching is categorical rather than a claim of identical
per-derivation work.

The optimization worked mechanically: cold source-probe p95 fell from 16:54 in
the independent treatment to 2:26, cold validation-runner p95 fell by 12:07, and
cold whole-workflow runner p95 fell by 8:36. Against the cold baseline it still
delivered a 38.2% validation-p95 reduction, while limiting the whole-workflow
runner-p95 increase to 15.9% and the validation-runner-p95 increase to 45.8%.

Against the exact warm controls, validation p95 moved from 24:57 to 20:38, a
17.3% reduction; the median moved from 24:22 to 19:33, a 19.8% reduction. Warm
whole-workflow runner p95 rose 8.1%, and warm validation-runner p95 rose 21.2%.
The candidate therefore missed the original 20% p95 gate by 40 seconds, while
the median missed 20% by about three seconds. The warm path was owned by the
unchanged host lane, not by the colocated source probe.

## Retained verdict and failure propagation

**Retain the four-job candidate after explicit repository-owner review.** The
owner revised the close-call retention decision after reviewing the six green
candidate observations, the run mix, and the measured cost. Exact same-head
reruns were only about 11% of the latest non-experiment execution sample, while
21 of the latest 30 merged PRs changed at least one product, web/e2e, host, or
Elisp source boundary. The controlled `common` marker remains a broad worst case
rather than a claim that every source-changing PR invalidates every boundary.

The retained trade is explicit: 17.3% warm and 38.2% broad-cold validation-p95
reductions, for +8.1% warm / +15.9% broad-cold whole-workflow runner p95 and
+21.2% warm / +45.8% broad-cold validation-runner p95. This is preferred to the
five-job variant, which was slightly faster but raised broad-cold validation
runner p95 by 61.1%. Firefox still owns overall workflow latency, so Tasks 3–5
must be evaluated separately.

Every candidate observation passed all four validation jobs, the source-probe
step, the stable aggregate, all four e2e jobs, and `e2e gate`. The union and
ordering contracts remain covered by the xtask catalog/workflow tests; no
verdict moved to Cachix.

`xtask/src/gate.rs` additionally parses the live workflow aggregate script and
executes it with synthetic result injection. The all-success vector passes; each
of host, hermetic, combined test-check/probe, and coverage is changed to
`failure` individually, and every injected case returns non-zero. Workflow
contract coverage also requires the colocated source probe to run under
`always()` without `continue-on-error`, preserving its failure signal and
diagnostics after an earlier test-check failure.

The retained topology intentionally supersedes ADR-0192's two-lane shape. Task 6
must record this owner-approved decision and project it into architecture and
contributor documentation before merge.

## Firefox topology experiment

Task 5 compared the existing workers=2 unsplit Firefox lane, the retained
candidate of two ordinary shards plus one serial-special lane, and an unsplit
workers=4 / 4-vCPU / 6144-MiB control. SQLite and PostgreSQL ran in separate,
fresh NixOS VMs. Production Chromium remained unchanged. Disposable draft PR
[#1596](https://github.com/jaunder-org/jaunder/pull/1596) carried the
experiment; none of its workflow-only commits are retained.

### Cohort and cache authentication

The accepted cohort is attempts 3–5 of
[run 35527421473](https://github.com/jaunder-org/jaunder/actions/runs/35527421473)
at immutable head `d68854b2f9a4c8cc7030df5202a8b6b35c04eb04`. Each attempt
injected only an inert, attempt-qualified `e2eSalt` into the measurement
derivations so Cachix could not substitute a previous VM verdict. Every measured
job:

- restored the exact primary Actions cache key
  `xtask-Linux-ea0143eb4115f446814b3805d702290bab87213653d932b3190ad4578a1db6e2`;
- reported `[nix: realized …drv]` for its salted target VM derivation while
  common dependencies could still substitute from Cachix; and
- produced a fresh report, census, lane manifest, duration manifest, phase
  sidecar, trace capture, journals, and Playwright archive.

All three accepted attempts passed every control, split lane, and reconciliation
job. Attempts 1 and 2 are excluded from successful-cohort arithmetic because the
PostgreSQL workers=4 control failed; those failures are retained below as safety
evidence rather than hidden.

### Job-level observations

Durations are complete GitHub job intervals, including setup and Nix entry. A
split backend's critical path is its slowest of the three concurrently scheduled
lane jobs; its runner proxy is their sum.

| Attempt | Backend    | workers=2 control | Split critical path | Split runner proxy | workers=4 control |
| ------: | ---------- | ----------------: | ------------------: | -----------------: | ----------------: |
|       3 | SQLite     |             22:33 |               11:37 |              28:53 |             16:35 |
|       4 | SQLite     |             18:00 |               12:06 |              28:40 |             20:42 |
|       5 | SQLite     |             21:47 |               10:01 |              26:44 |             21:02 |
|       3 | PostgreSQL |             21:54 |               12:30 |              30:16 |             17:17 |
|       4 | PostgreSQL |             22:12 |               12:37 |              31:04 |             21:06 |
|       5 | PostgreSQL |             22:08 |               11:35 |              30:09 |             18:51 |

| Backend / metric         | workers=2 median | Split median | workers=2 p95 | Split p95 | Split p95 change |
| ------------------------ | ---------------: | -----------: | ------------: | --------: | ---------------: |
| SQLite critical path     |            21:47 |        11:37 |         22:28 |     12:03 |       **−46.4%** |
| PostgreSQL critical path |            22:08 |        12:30 |         22:12 |     12:36 |       **−43.2%** |
| SQLite runner proxy      |            21:47 |        28:40 |         22:28 |     28:52 |           +28.4% |
| PostgreSQL runner proxy  |            22:08 |        30:16 |         22:12 |     30:59 |           +39.6% |

The split clears the required 20% p95 improvement on the slower PostgreSQL
backend by more than twenty percentage points. Aggregate runner time remains
well below the issue's approximate 2× ceiling. Reconciliation itself took
2:28–4:36 per backend in the accepted attempts. Composing each split critical
path with its same-attempt reconciliation duration gives a worst backend path of
17:06, below the retained validation warm p95 of 20:38. This composition is not
presented as a production workflow observation: the disposable workflow's
reconciliation also waited for measurement controls. It establishes that the
retained workflow should return the required-check critical path to validation;
the first production runs must confirm that result.

### Executed evidence and phases

Every accepted reconciliation authenticated the same 318-entry independent
Firefox census on all three lanes, exact lane/shard metadata, and a 316-test
executed union: 132 tests in ordinary shard 1, 129 in ordinary shard 2, and 55
in serial-special. Reports contained zero unexpected tests. The split cohort had
zero retries/flakes, panics, OOMs, or timeouts. Duration-pressure and
boot-decomposition checks passed for every lane.

Attempt 5's sidecars provide a representative phase decomposition:

| Backend / arm        | Playwright or slowest split Playwright | VM readiness | Result lift | Post-gate checks |
| -------------------- | -------------------------------------: | -----------: | ----------: | ---------------: |
| SQLite workers=2     |                                  18:03 |       20.6 s |       2.3 s |            0.4 s |
| SQLite split         |                                   5:36 |  17.5–22.5 s |   1.1–1.6 s |            0.2 s |
| PostgreSQL workers=2 |                                  18:39 |       26.4 s |       2.5 s |            0.4 s |
| PostgreSQL split     |                                   6:46 |  23.1–32.8 s |   1.3–1.6 s |        0.2–0.3 s |
| SQLite workers=4     |                                  17:32 |       24.9 s |       2.8 s |            0.5 s |
| PostgreSQL workers=4 |                                  14:03 |       25.9 s |       2.6 s |            0.5 s |

The independent census deliberately exceeds the report total because it records
all selected source identities before Playwright dependency/shard execution;
reconciliation compares the exact authenticated identities rather than assuming
counts imply equality.

### Resource and artifact verdicts

Workers=4 is rejected as a production alternative. In attempts 1 and 2 its
PostgreSQL arm failed after both retries of `theme-management.spec.ts:122` and
`theme-management.spec.ts:372`; attempt 4 passed only after a retry of test 122.
Thus three successful observations were ultimately collected, but two failures
and one flaky success across five fresh attempts reproduce the state-race
concern behind the existing workers=2 policy. There was no corresponding
split-lane failure or retry.

Attempt 5's compressed lane artifacts totalled 4,127,544 bytes for SQLite and
4,301,434 bytes for PostgreSQL, versus 4,144,706 and 4,225,758 bytes for their
workers=2 controls: approximately 1.00× and 1.02×. The disposable measurement
workflow also uploaded an 8.2-MiB reconciliation bundle that duplicated the
already retained lane evidence. Production does not retain that duplication: its
reconciliation artifact contains only the aggregate result sidecar, while the
original lane-qualified evidence remains available for 14 days.

### Retained Firefox decision

**Retain the three-lane Firefox split for both backends.** It clears the p95
latency threshold, stays below the runner and artifact limits, preserves exact
backend/browser/test authority, and showed no new resource or mutable-state
race. The shared catalog now enables six Firefox lanes and two unsplit Chromium
lanes. Local full `cargo xtask validate` realizes and reconciles that same set;
CI distributes it behind the unchanged required `e2e gate`. Unsplit Firefox
workers=2 and workers=4 remain measurement-only controls, and workers=4 remains
explicitly rejected for production.
