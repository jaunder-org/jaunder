# Issue #828: E2E worker scaling experiment

## Outcome

Measure whether increasing Firefox E2E parallelism reduces the CI matrix long
pole without increasing flakiness or exhausting VM/runner resources. Land either
a data-backed gate configuration or a durable finding that the current 2-worker,
2-vCPU setting is already at the knee.

## Load-bearing decisions

- The gate objective is the slowest Firefox matrix job, not summed suite time or
  Chromium duration.
- The control is the current gate: 2 Playwright workers, 2 VM vCPUs, 3072 MiB,
  and one retry.
- A local go/no-go probe runs on the user-confirmed quiescent host, in fixed
  order: current control, then treatment at 3 workers / 4 VM vCPUs / 3072 MiB.
  Both run `e2e sqlite firefox` through fresh salted derivations.
- A local arm is valid only when its pre-run one-minute load average is at most
  1.5 and process inspection finds no unrelated `cargo`, `nix`, QEMU, browser,
  or Node process using at least 1% CPU in the sample. The same threshold is
  checked after the run; the experiment's own load is recorded but is not
  contamination. An invalid arm is rerun with the same configuration before the
  comparison is calculated.
- Local and CI suite duration is always Playwright report `.stats.duration`;
  build/setup time and GitHub whole-job elapsed time are recorded separately but
  never drive the verdict.
- The local screen asks only whether scaling exists: it advances when
  `.stats.duration` is at least 20% lower with no OOM or infrastructure failure.
  Any local flake is recorded but cannot estimate a flake-rate change from one
  exposure; equal repeated CI exposure owns that veto.
- The completed local treatment was 35.9% faster with no OOM/infrastructure
  failure, so the CI campaign proceeds despite its one fail-then-pass.
- The CI decision uses standard public `ubuntu-24.04` runners, documented as 4
  vCPU and 16 GiB. CI—not the local 16-core host—owns external validity.
- CI measures a control plus the full 2×2 factorial at 3 workers:
  - control: 2 workers / 2 VM vCPUs / 3072 MiB;
  - A: 3 workers / 3 VM vCPUs / 3072 MiB;
  - B: 3 workers / 3 VM vCPUs / 4096 MiB;
  - C: 3 workers / 4 VM vCPUs / 3072 MiB;
  - D: 3 workers / 4 VM vCPUs / 4096 MiB.
- CI runs exactly three valid rounds. Arm order is rotated: control→A→B→C→D,
  A→B→C→D→control, B→C→D→control→A. Each workflow completes before the next
  begins; every arm/round has a distinct `e2eSalt`.
- Every attempted CI arm is recorded. An independently evidenced
  runner/service/network failure before an E2E result exists is replaced
  immediately with the same arm/round; no other result is replaced.
- A treatment OOM or VM resource-exhaustion failure disqualifies that arm and
  cancels its remaining scheduled rounds. A control OOM/resource failure aborts
  the campaign because the baseline environment is no longer valid.
- Every non-disqualified arm has equal exposure: exactly three reports per
  backend. The scaling curve reports `.stats.duration`, GitHub job elapsed time,
  and `flaky + unexpected` for both Firefox backends in every completed round.
- Flakiness is compared per backend at equal exposure: sum `flaky + unexpected`
  over the three valid reports. A treatment is disqualified if either backend's
  sum exceeds the same backend's control sum. Equal nonzero counts do not veto.
- An arm's primary duration is the slower of its SQLite and PostgreSQL median
  `.stats.duration` values. The eligible set contains treatments at least 20%
  below control with no OOM/resource or per-backend flakiness veto.
- From the eligible set, find the lowest (fastest) primary duration. The tie set
  is every eligible arm within 5% of that fastest value. Select the tied arm
  with fewer VM cores, then less memory. If the eligible set is empty, retain
  2/2/3072.
- Backend rows are never collapsed in this experiment, so #817's conditional
  backend-independence premise is not used to decide the winner.
- Measurement mutations (`e2eSalt` and temporary arms) never land. Only the
  selected gate configuration and explanatory comments—or documentation of the
  no-change decision—reach the final branch.

## Result

The quiet-host treatment advanced with a 35.9% reduction. CI then produced three
valid reports per backend for control, B, C, and D. Arm A was disqualified after
its first PostgreSQL VM OOM, and one D attempt was replaced after an
independently evidenced collector-readiness failure before an E2E result
existed.

No treatment was eligible. B and C improved the slower-backend median by only
9.6% and 10.4%. D improved it by 31.0%, but its SQLite flake sum exceeded
control (3 versus 2). A OOMed. The gate therefore remains 2 workers / 2 VM vCPUs
/ 3072 MiB. `docs/observability.md` records every valid report, the replacement,
arm medians, job times, flake comparisons, and the final curve.

## Acceptance

- The quiet-host probe records pre/post load and process inventory, control and
  treatment `.stats.duration`, GitHub-independent setup time,
  `flaky + unexpected`, failures, and the 20% go/no-go calculation.
- The local probe records its 35.9% scaling result, one flaky attempt, and the
  CI go verdict without treating one exposure as a flake-rate estimate.
- CI records the exact three-round, five-arm rotated order, every attempted run
  and any replacement reason, unique salts, runner image/hardware evidence,
  per-backend Firefox `.stats.duration`, whole-job elapsed time, and
  `flaky + unexpected`.
- The final scaling curve gives all completed arms' per-backend values and
  medians, the slower-backend primary duration, percentage improvement versus
  control, equal exposure or explicit OOM disqualification, per-backend
  flakiness comparisons, and the fastest-arm 5% resource tie set.
- A winning configuration updates `e2eGateChecks` and comments that currently
  explain the #61/#155 worker/core/memory choice; all four combinations remain
  in the matrix.
- A no-change result records why 2/2/3072 is the knee so the experiment is not
  reopened without a changed premise.
- Temporary salts/configurations are absent from the final diff and the
  `e2e-scaffold` guard remains green.

## Boundaries

- No product-code, test-content, timeout-budget formula, retry-policy, browser
  matrix, or backend behavior change.
- No performance conclusion from `validate`, concurrent local combinations, a
  non-quiescent host, or cached CI derivations.
- No attempt to optimize Chromium; it is not the gate long pole.
