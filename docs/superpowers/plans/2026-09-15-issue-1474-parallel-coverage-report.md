# Issue #1474 parallel coverage evaluation

## Decision

Reject every two-worker treatment. Keep production coverage, the Nix producer
shape, CI topology, and Cachix exclusion filter unchanged.

The best local treatment (`hash` with independent nextest concurrency) improved
median end-to-end elapsed time by 10.375 s (3.56%), below both adoption
thresholds. A separate-runner CI probe reduced its internal warm critical path
by more than three minutes, but the complete warm workflow critical path
improved by only 30.5 s (4.37%) and the portable treatment did not preserve
baseline coverage semantics. Any one of those failures is disqualifying.

Issue #1473 therefore remains open. Backend-derived grouping remains
measurement-only.

## Revisions and environments

### Local

- Source revision: `09afe7ed608a0c8cd6f8605bdea9d54733bbc435`
- Host: Linux 7.2.2, AMD Ryzen 7 6800H, 16 logical CPUs, 32,074,024 KiB physical
  memory
- Condition: user-confirmed quiescent system
- Build state: one warm-up, then two accepted observations per treatment
- Census: 4,941 expected = 4,941 executed + 0 ignored; zero missing and zero
  duplicate identities in every observation
- Checked evidence:
  [`2026-09-15-issue-1474-local-coverage-evidence.json`](2026-09-15-issue-1474-local-coverage-evidence.json)
  records every accepted observation's stage/worker/resource timings and the
  complete slice, hash, and backend worker assignments against their reconciled
  union. Its source is the retained 36,453,742-byte benchmark manifest with
  SHA-256 `ce7df19a2f99820969f51f37722cec05515fe708e31639fa1c02a2a901816357`.

### CI

- Experiment branch revision: `84841873be4e5f9e50936bdd2d4e5064a9a7ce09`
- Synthetic pull-request merge revision:
  `710cb047f2296117c5e4d1436d7b72d0365d1910`
- Run:
  [34942853481](https://github.com/jaunder-org/jaunder/actions/runs/34942853481)
- Runner class: GitHub-hosted `ubuntu-24.04`, Linux X64, image `20260907.300.1`
- Nix: 2.35.2; `flake.lock` SHA-256
  `f121a5eac8b39920fb60b157ded3d42175851491902fdd27be02212299e262c2`
- Treatment: nextest `hash:1/2` + `hash:2/2`, independent per-runner concurrency
- Ordering: cold baseline then cold fan-out; warm fan-out A then warm baseline
  A; warm baseline B then warm fan-out B
- Census: 4,937 expected = 4,937 executed + 0 ignored; zero missing and zero
  duplicate identities in every successful observation
- Cache state: the cold support probe missed Cachix; both warm support probes
  substituted the same support output. Final baseline, worker, aggregate,
  profile, report, and verdict outputs remained forced uncached or
  artifact-only.

## Local results

Elapsed includes per-observation instrumented archive preparation plus
execution, profile processing, text/LCOV/CRAP generation, and verdict. The
checked evidence records preparation (incremental compilation plus archive
creation) resource usage separately for every ordinal, followed by each producer
stage and both worker durations. RSS is the median of each observation's larger
preparation/producer maximum.

| Treatment             | Ordinals | Median elapsed | Delta from baseline | Median max RSS |
| --------------------- | -------: | -------------: | ------------------: | -------------: |
| Baseline              |     1, 8 |      291.135 s |                   — |  4,013,836 KiB |
| Slice / independent   |    2, 14 |      284.695 s |   -6.440 s (-2.21%) |  4,016,328 KiB |
| Slice / fixed         |    3, 13 |      284.785 s |   -6.350 s (-2.18%) |  4,011,494 KiB |
| Hash / independent    |    4, 12 |      280.760 s |  -10.375 s (-3.56%) |  4,013,648 KiB |
| Hash / fixed          |    5, 11 |      289.375 s |   -1.760 s (-0.60%) |  4,010,394 KiB |
| Backend / independent |    6, 10 |      292.495 s |   +1.360 s (+0.47%) |  4,010,934 KiB |
| Backend / fixed       |     7, 9 |      307.120 s |  +15.985 s (+5.49%) |  4,014,896 KiB |

All 14 observations produced the same normalized LCOV line-hit digest
(`0b1527d001ffd5f2164ff76293102e9a3f2b40e00640bd5fb9956e5d2493215b`), normalized
CRAP digest
(`27723816c336319506de40edb624312c7197ae119602c645ab8c96d0fde2aa8d`),
executable-source membership, exclusions, and passing verdict. Raw positive hit
counts varied between rounds; boolean line-hit normalization removed that
harmless repetition-count difference.

The producer's stage contract folds LLVM profile merging into `text-report`; the
separate-runner aggregate likewise measured profile merge and report generation
as one aggregate duration. No accepted observation therefore isolates
profile-merge time. Correcting that instrumentation would require another
controlled benchmark, which is not justified after the treatment already failed
the adoption threshold and exact semantic equivalence. This timing-resolution
gap is an additional production-eligibility failure, not an estimated
measurement.

## Experimental cache boundary

Revision `84841873be4e5f9e50936bdd2d4e5064a9a7ce09` generated the inventory from
Nix output metadata rather than a name search. Its sole upload-eligible output
was `packages.x86_64-linux.coverage-support`. The complete protected final set
was:

- `checks.x86_64-linux.coverage`
- `checks.x86_64-linux.coverage-gate`
- `checks.x86_64-linux.elisp-coverage-producer`
- `checks.x86_64-linux.e2e`
- `checks.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}`
- `packages.x86_64-linux.e2e-checks`
- `packages.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}-single-worker`
- `packages.x86_64-linux.wasm-coverage-{chromium,firefox}`

The automated probe built the eligible support output, examined both its runtime
and derivation closures, required the pinned Rust toolchain, `cargo-llvm-cov`,
and `cargo-nextest` inputs, rejected every final output from those closures,
verified every final derivation disabled substitution and preferred local build,
and perturbed the relevant source/configuration inputs to prove invalidation. CI
run
[34942852876](https://github.com/jaunder-org/jaunder/actions/runs/34942852876)
executed that probe at the experiment revision: the `Validation coverage` job's
`Coverage source-drift probe (#241)` step passed after 480 seconds. The separate
measurement run then observed one cold miss and two substitutions of the same
support output. The final rejected state removes this experimental support
boundary and restores the broad production exclusion filter.

## CI timing results

Internal critical path is baseline producer elapsed, or fan-out support
preparation/compilation + slower worker elapsed + aggregate elapsed.
Orchestration/transfer overhead is the complete job critical path minus that
internal path; it includes checkout, setup, artifact transfer, upload, and
scheduling because Actions does not expose those as one narrower timer. Runner
consumption is the sum of all treatment job intervals.

| Observation     | Cache                       | Internal stages (s)                                        | Internal critical path | Orchestration / transfer | Job critical path | Runner consumption |
| --------------- | --------------------------- | ---------------------------------------------------------- | ---------------------: | -----------------------: | ----------------: | -----------------: |
| Baseline cold A | cold final                  | producer 1,974.328                                         |            1,974.328 s |                119.672 s |           2,094 s |            2,094 s |
| Fan-out cold A  | cold support                | prep 305.267; workers 169.832 / 163.011; aggregate 123.810 |              598.909 s |                441.091 s |           1,040 s |            1,313 s |
| Fan-out warm A  | warm support                | prep 19.855; workers 180.925 / 157.337; aggregate 85.608   |              286.388 s |                346.612 s |             633 s |              903 s |
| Baseline warm A | warm inputs, uncached final | producer 566.527                                           |              566.527 s |                125.473 s |             692 s |              692 s |
| Baseline warm B | warm inputs, uncached final | producer 566.187                                           |              566.187 s |                137.813 s |             704 s |              704 s |
| Fan-out warm B  | warm support                | prep 19.343; workers 273.108 / 150.793; aggregate 89.878   |              382.329 s |                319.671 s |             702 s |              935 s |

Warm medians:

- Internal critical path: 566.357 s baseline versus 334.359 s fan-out, an
  improvement of 231.998 s (40.96%).
- Complete job critical path: 698.0 s baseline versus 667.5 s fan-out, an
  improvement of 30.5 s (4.37%).
- Runner consumption: 698.0 runner-seconds baseline versus 919.0 runner-seconds
  fan-out, a regression of 221.0 runner-seconds (31.66%).

The cold result is reported but is not used to qualify adoption: only one
comparable cold observation per mode was collected.

## Correctness comparison

All three baseline and all three fan-out observations were internally stable and
ended `tests-ok`, with the exact census above. The portable CI treatment
nevertheless changed coverage semantics because archived debug/test binaries
needed a different RustEmbed mode to carry staged site assets across runners.

| Evidence                     | Baseline (all three)                                               | Fan-out (all three)                                                |
| ---------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------ |
| Executable text-report files | 334                                                                | 424                                                                |
| Executable lines             | 70,492                                                             | 79,353                                                             |
| Boolean line-hit digest      | `f192eda975f41e9f75afc768038a1737447bffeadf6697321095eeaac550ccae` | `f33f6517658002a660064a81439235869cbb86a7b426ee37c61f649493304a74` |
| CRAP entries / files         | 3,223 / 373                                                        | 3,223 / 373                                                        |
| Canonical CRAP digest        | `2d70439bc5179b90728ea32b46860812e0481739025c4c49927b056ad4cb8a67` | `486ea23e224f2d61700c9a35b5be394570c6c5d6997596a50eec820b4b6c5c03` |
| Final verdict                | pass                                                               | pass                                                               |

The cross-mode comparison found one baseline-only and 91 fan-out-only
text-report file sections, 32 baseline-only and 8,893 fan-out-only line-hit
records, and 39 changed CRAP entries. Examples include changed coverage for
`macros/src/str_newtype.rs::parse_opts`, `macros/src/lib.rs::server`, and
`server/src/site.rs::embedded_body`. Equal pass/fail verdicts do not make those
inputs equivalent.

## Threshold evaluation

- Local material improvement: **fail** (best 3.56%, 10.375 s).
- CI complete-path material improvement: **fail** (warm median 4.37%, 30.5 s).
- Non-winning surface regression below 10%: **pass** for elapsed time; local
  improved 3.56%.
- Exact merged coverage semantics: **fail**.
- Runner consumption: **regressed 31.66%** on warm medians.
- Required profile-merge timing isolation: **fail** (bundled with report
  generation).
- Selectable strategy: hash is selectable in principle; backend grouping was not
  considered for adoption.

Decision: restore the original production producer/cache/CI topology and retain
the experiment engine, local benchmark harness, approved spec/outline, and this
report as non-production evidence.
