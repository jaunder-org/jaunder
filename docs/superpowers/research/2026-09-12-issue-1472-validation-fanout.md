# Issue #1472: Validation fan-out topology experiment

## Decision frame

This report evaluates the two-lane experiment defined by the
[approved specification](../../archive/2026-09-12-issue-1472-validation-fanout-topology.md).
The optimization target is elapsed time until the last trustworthy required
check. Concurrent job-time sum is reported separately as a runner-consumption
proxy, not billed minutes and not a success criterion.

The selected treatment runs Rust coverage independently from the remaining
non-e2e validation surface and aggregates both results behind the stable
`Validate (no e2e)` required context. The four-way e2e matrix is unchanged.
There is no serial preparation job or inter-lane artifact transfer.

A topology is retained only if repeated comparable runs demonstrate at least a
10% or three-minute workflow wall-clock reduction without weakening a verdict or
moving another required check onto the critical path.

## Prior evidence

The canonical [#1463 report](2026-09-11-issue-1463-ci-wall-clock.md) established
the earlier critical path and added durable phase recording:

- cold source-changing run
  [34526589815](https://github.com/jaunder-org/jaunder/actions/runs/34526589815):
  40:06 workflow, 40:02 validation;
- warm merge-group run
  [34530528286](https://github.com/jaunder-org/jaunder/actions/runs/34530528286):
  32:04 workflow, 32:00 validation;
- post-#1463 run
  [34641076765](https://github.com/jaunder-org/jaunder/actions/runs/34641076765),
  attempt 2: 28:02 validation, including 495.733 seconds of Rust coverage;
- narrow documentation run
  [34710334717](https://github.com/jaunder-org/jaunder/actions/runs/34710334717):
  23:34 validation, including 428.139 seconds of Rust coverage.

The latter two phase records bound the ideal overlap before duplicated setup,
realization, and contention:

| Observation           | Serialized validation | Coverage lane | Remaining core | Ideal two-lane bound |      Ideal saving |
| --------------------- | --------------------: | ------------: | -------------: | -------------------: | ----------------: |
| 34641076765 attempt 2 |           1,330.366 s |     495.733 s |      834.633 s |            834.633 s | 495.733 s (37.3%) |
| 34710334717           |           1,125.035 s |     428.139 s |      696.896 s |            696.896 s | 428.139 s (38.1%) |

These are physical bounds, not treatment predictions. Separate runners repeat
checkout, Nix installation, Cachix setup, xtask compilation, evaluation, and any
source derivations shared by both lanes. GitHub's job timestamps do not
attribute those repeated costs to substitution versus local build.

## Comparable baseline protocol

PR [#1486](https://github.com/jaunder-org/jaunder/pull/1486) carries one
evolving experiment. The baseline uses the production serial workflow. Its
initial specification/outline head is a narrow documentation change. Two
subsequent heads change only an issue-marked comment in `common/src/lib.rs`,
forcing a source closure without changing behavior. Same-head reruns are warmed
observations; they are not described as cold merely because an earlier attempt
ran first.

All elapsed values are GitHub `started_at` to `completed_at` differences.
Workflow wall-clock is run creation to completion. Runner proxy is the sum of
all concurrent job elapsed times. Setup is the complete shared `setup-ci` action
step. Validation command, coverage probe, and Nix probe are top-level workflow
steps. Available artifacts were retained only for each run's latest attempt;
authenticated artifact download was unavailable during extraction, so baseline
internal Nix phase values are marked unavailable rather than inferred.

## Baseline Actions observations

| Class                            | Run / attempt / head                                                                                 | Result            | Workflow | Validate job | Setup | Validate command | Coverage probe | Nix probe |                 Slowest e2e |    E2E gate | Runner proxy |
| -------------------------------- | ---------------------------------------------------------------------------------------------------- | ----------------- | -------: | -----------: | ----: | ---------------: | -------------: | --------: | --------------------------: | ----------: | -----------: |
| Narrow                           | [34727823744](https://github.com/jaunder-org/jaunder/actions/runs/34727823744), attempt 1, `61f80a7` | success           |  1,440 s |      1,436 s |  86 s |          1,229 s |           21 s |      90 s | PostgreSQL/Firefox, 1,269 s |         4 s |      5,328 s |
| Narrow, same-head rerun          | [34727823744](https://github.com/jaunder-org/jaunder/actions/runs/34727823744), attempt 2, `61f80a7` | success           |  1,518 s |      1,513 s |  85 s |          1,301 s |           25 s |      93 s | PostgreSQL/Firefox, 1,229 s |         4 s |      5,430 s |
| Source marker, changing          | [34730732370](https://github.com/jaunder-org/jaunder/actions/runs/34730732370), attempt 1, `0dcc4ce` | failure; excluded |  2,363 s |      2,358 s |  49 s |          2,154 s |           26 s |     121 s |     SQLite/Firefox, 1,676 s | 5 s, failed |      7,819 s |
| Source-changing, same-head rerun | [34730732370](https://github.com/jaunder-org/jaunder/actions/runs/34730732370), attempt 2, `0dcc4ce` | success           |  1,616 s |      1,612 s |  47 s |          1,406 s |           27 s |     123 s | PostgreSQL/Firefox, 1,204 s |         4 s |      5,199 s |
| Source-changing                  | [34734202102](https://github.com/jaunder-org/jaunder/actions/runs/34734202102), attempt 1, `3e95115` | success           |  2,153 s |      2,149 s |  90 s |          1,930 s |           22 s |      93 s | PostgreSQL/Firefox, 1,763 s |         5 s |      7,970 s |

Attempt 1 of run 34730732370 failed in PostgreSQL/Firefox e2e while validation
completed. The whole observation is diagnostic-only and excluded from threshold
arithmetic, as required by the fixed protocol.

Successful baseline medians:

| Class                              |            Workflow | Validate job | Runner proxy |
| ---------------------------------- | ------------------: | -----------: | -----------: |
| Narrow, two attempts               |     1,479 s (24:39) |    1,474.5 s |      5,379 s |
| Source-changing, mixed cache state | 1,884.5 s (31:24.5) |    1,880.5 s |    6,584.5 s |

The source baseline is noisy: its two successful workflow observations differ by
537 seconds and mix one same-head warmed rerun with one first attempt because
the other first attempt failed e2e and is excluded. It provides repeated
source-changing observations, but not a cache-state-controlled threshold
comparison. The narrow same-head rerun was slower than its first attempt,
demonstrating that "rerun" is a cache-state description, not evidence of
improvement by itself.

## Candidate topology screen

### Two independent lanes

This is the selected live treatment. The historical ideal bound is 7:08–8:16,
while observed setup is 47–90 seconds and aggregation is 4–5 seconds. Setup
occurs concurrently rather than serially, but duplicated Nix evaluation,
realization, source builds, and cache traffic can extend either lane and
increase runner consumption. Only completed treatment runs decide whether the
bound survives those costs.

### Same-runner concurrency

Running independent Nix checks concurrently inside one xtask process avoids a
second checkout and Nix installation, but the full VM supplies only limited CPU,
memory, disk, Nix, Cargo, and PostgreSQL capacity. Coverage already performs
compilation and dual-backend execution. Contention can lengthen both branches,
and no completed Actions evidence quantifies it. It is not live-trialed because
the selected independent-runner topology answers the issue's setup-economics
question directly.

### Wider per-check runner fan-out

WASM tests, doctests, and Emacs Lisp coverage were each tens of seconds in the
#1463 phase records, while shared setup alone was 47–90 seconds in this
baseline. Giving each a runner would duplicate more setup than their serial
stage duration before accounting for Nix realization. The core lane keeps these
smaller checks together.

### Serial cache preparation

Rejected by critical-path arithmetic. The current independent start is
`B + max(V, E)`. Preparation followed by fan-out is
`B + upload + download + setup + max(V, E)`. It can reduce duplicated runner
work but cannot improve elapsed time without a separately measured transfer
effect that exceeds its new serial costs. This experiment introduces no
preparation dependency or artifact transfer.

## Source-changing treatment observations

The first treatment head, `d2c57c4`, introduces the two validation lanes while
retaining the source marker. The second, `14150c9`, rotates only that marker.
Each head ran twice successfully; attempt 2 is a same-head warmed observation.

| Run / attempt / head                                                                                 | Workflow | Core job | Core setup | Core command | Nix probe | Coverage job | Coverage setup | Coverage command | Coverage probe | Aggregate |                 Slowest e2e | E2E gate | Runner proxy |
| ---------------------------------------------------------------------------------------------------- | -------: | -------: | ---------: | -----------: | --------: | -----------: | -------------: | ---------------: | -------------: | --------: | --------------------------: | -------: | -----------: |
| [34737381390](https://github.com/jaunder-org/jaunder/actions/runs/34737381390), attempt 1, `d2c57c4` |  1,245 s |  1,094 s |       60 s |        891 s |     131 s |        674 s |           80 s |            558 s |           28 s |       4 s |     SQLite/Firefox, 1,226 s |      6 s |      5,634 s |
| [34737381390](https://github.com/jaunder-org/jaunder/actions/runs/34737381390), attempt 2, `d2c57c4` |  1,192 s |  1,072 s |       47 s |        884 s |     133 s |        666 s |           47 s |            585 s |           26 s |       9 s | PostgreSQL/Firefox, 1,155 s |      9 s |      5,444 s |
| [34739513225](https://github.com/jaunder-org/jaunder/actions/runs/34739513225), attempt 1, `14150c9` |  1,916 s |  1,851 s |       50 s |      1,664 s |     130 s |        748 s |           56 s |            652 s |           30 s |      16 s |     SQLite/Firefox, 1,645 s |      4 s |      8,117 s |
| [34739513225](https://github.com/jaunder-org/jaunder/actions/runs/34739513225), attempt 2, `14150c9` |  1,188 s |  1,097 s |       51 s |        902 s |     136 s |        655 s |           47 s |            573 s |           26 s |       4 s |     SQLite/Firefox, 1,168 s |      4 s |      5,652 s |

The source-changing treatment median uses attempt 1 from each distinct source
head: **1,580.5 seconds (26:20.5)**. Against the mixed-cache-state
**1,884.5-second (31:24.5)** source baseline median, the observed reduction is
**304 seconds (5:04), or 16.1%**. This is directional evidence, not the
retention comparison, because the successful baseline pair contains one warmed
rerun. The runner proxy rises from 6,584.5 to 6,875.5 seconds: **+291 seconds,
or 4.4%**.

The two warmed treatment attempts have a 1,190-second workflow median. The two
available baseline same-head reruns—one narrow and one source-changing—have a
1,567-second median; the diagnostic reduction is 377 seconds (6:17), or 24.1%.
Those mixed change classes are not used for the retention decision. Their runner
proxies rise from a 5,314.5-second median to 5,548 seconds, or 4.4%.

Critical-path ownership moves as intended but does not disappear. Core owns the
first treatment attempt's validation path; the e2e gate finishes the overall
workflow 2:13 later. On the slower second source head, core again dominates
validation and finishes 1:07 after the slowest e2e job. Both warmed attempts end
on Firefox e2e rather than coverage. Rust coverage is never the treatment
critical path.

The treatment artifacts expose distinct core and coverage diagnostics for each
latest attempt. Authenticated archive download remained unavailable during
extraction, so internal xtask/Nix phase timings are unavailable; no substitution
or local-build attribution is inferred from the top-level commands.

## Narrow treatment observations

The report-only `080bae3` delta is the narrow treatment head. Both attempts
completed successfully.

| Run / attempt / head                                                                                 | Workflow | Core job | Core setup | Core command | Nix probe | Coverage job | Coverage setup | Coverage command | Coverage probe | Aggregate |                 Slowest e2e | E2E gate | Runner proxy |
| ---------------------------------------------------------------------------------------------------- | -------: | -------: | ---------: | -----------: | --------: | -----------: | -------------: | ---------------: | -------------: | --------: | --------------------------: | -------: | -----------: |
| [34742336503](https://github.com/jaunder-org/jaunder/actions/runs/34742336503), attempt 1, `080bae3` |  1,182 s |  1,060 s |       91 s |        849 s |     110 s |        601 s |           57 s |            511 s |           24 s |       3 s | PostgreSQL/Firefox, 1,158 s |      5 s |      5,300 s |
| [34742336503](https://github.com/jaunder-org/jaunder/actions/runs/34742336503), attempt 2, `080bae3` |  1,132 s |  1,101 s |       49 s |        911 s |     133 s |        686 s |           56 s |            590 s |           28 s |       3 s | PostgreSQL/Firefox, 1,111 s |      2 s |      5,392 s |

The narrow treatment median is **1,157 seconds (19:17)**. Against the
**1,479-second (24:39)** baseline median, the reduction is **322 seconds (5:22),
or 21.8%**. Runner proxy falls from 5,379 to 5,346 seconds: **-33 seconds, or
-0.6%**. Firefox e2e owns the critical path in both attempts; the validation
aggregate completed before the e2e gate.

## Decision

**Retain the two-lane topology.** The cache-state-matched narrow comparison has
two successful baseline and two successful treatment observations. Its median
workflow wall-clock falls by 322 seconds (5:22), or 21.8%, independently
clearing both numeric issue thresholds. The mixed-cache-state source comparison
shows a directional 304-second (16.1%) reduction but is not used to qualify the
decision. Its runner proxy rises 4.4%, while the narrow proxy is effectively
flat; runner consumption is reported separately and is not a success criterion.

The retained graph preserves all required surfaces. Core and coverage each
execute the existing producer/consumer implementations; the result-only
`Validate (no e2e)` job fails unless both succeed. The e2e matrix and its gate
are unchanged. Final Rust coverage and e2e verdicts remain per-ref and excluded
from Cachix reuse. No preparation job, transfer edge, or reused final verdict
was introduced.

The critical path now alternates between core validation and Firefox e2e rather
than remaining fixed on serialized validation. Narrow treatment exposes the
unchanged e2e floor, but does not lengthen it: the slowest-e2e median falls from
1,249 seconds at narrow baseline to 1,134.5 seconds at treatment. No added work
moved another required check onto the critical path. Rust coverage did not own
any treatment critical path. Further reduction therefore belongs to the
separately scoped coverage-backend and e2e investigations, not wider validation
fan-out.
