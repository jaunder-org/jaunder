# Issue #1574: Validation critical-path fan-out

## Decision frame

This report records the Task 2 retention evidence for
[#1574](https://github.com/jaunder-org/jaunder/issues/1574). The treatment
replaces the serialized non-e2e core job with independent host, hermetic,
test-check, coverage, and source-probe jobs behind the unchanged
`Validate (no e2e)` context. Local `validate --no-e2e` remains serial.

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

## Cache-state selection

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

## Raw observations

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

### Nix-backed command evidence

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

## Warm normalized comparison

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

## Controlled cold source-invalidating cohort

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

## Provisional verdict and failure propagation

The five-result aggregate now clears the validation latency threshold under both
exact warm-cache and controlled cold-source cohorts. Retention remains **pending
explicit repository-owner review** of the measured runner-cost trade: +11.9%
warm / +19.5% cold whole-workflow p95 and +21.1% warm / +61.1% cold validation
p95. Until that amount is accepted, Task 2 is not complete and the fan-out must
not be described as the final architecture.

Every treatment attempt passed all five validation jobs, the stable aggregate,
all four e2e jobs, and `e2e gate`. The union and ordering contracts remain
covered by the xtask catalog/workflow tests; no verdict moved to Cachix.

`xtask/src/gate.rs` additionally parses the live workflow aggregate script and
executes it with synthetic result injection. The all-success vector passes; each
of host, hermetic, test-checks, coverage, and source-probe is then changed to
`failure` individually, and every injected case must return non-zero. This
proves failure propagation for every lane without weakening or temporarily
committing a production CI job.

If retained, the topology intentionally supersedes ADR-0192's two-lane shape.
Task 6 must record the owner-approved decision and project it into architecture
and contributor documentation before merge.
