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

## Normalized comparison

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

The cost of retention is explicit: median validation runner consumption rises
8:14, principally because median setup rises from 2:08 to 6:59. The observed
whole-workflow runner proxy rises 13:42 (13.1%), though e2e variation
contributes to that total. This increase is accepted for the issue branch
because it buys a 23.9% p95 reduction in the targeted validation path, moves one
completed workflow to 20:50, and exposes the Firefox floor that Tasks 3–5
address. The ADR must preserve this trade-off rather than describing fan-out as
free.

## Verdict and failure propagation

**Retain the five-result validation aggregate.** Every retained treatment
attempt passed all five validation jobs, the stable aggregate, all four e2e
jobs, and `e2e gate`. The union and ordering contracts remain covered by the
xtask catalog/workflow tests; no verdict moved to Cachix.

`xtask/src/gate.rs` additionally parses the live workflow aggregate script and
executes it with synthetic result injection. The all-success vector passes; each
of host, hermetic, test-checks, coverage, and source-probe is then changed to
`failure` individually, and every injected case must return non-zero. This
proves failure propagation for every lane without weakening or temporarily
committing a production CI job.

The accepted topology intentionally supersedes ADR-0192's two-lane shape. Task 6
must record that decision and project it into architecture and contributor
documentation before merge.
