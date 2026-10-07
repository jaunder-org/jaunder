# Restore performance-producer evaluation (#1701)

## Outcome

Repair the existing performance harness so its storage and browser producers can
evaluate again. The user approved this bounded repair after the profiling
reconnaissance reproduced a missing ordinary-E2E lane argument.

## Load-bearing decisions

- Performance producers share the existing VM/backend lifecycle, not ordinary
  E2E lane execution. Keep lane-only metadata with the ordinary gate script.
- Select exactly one ordinary gate or custom producer script; reject ambiguous
  or absent execution configuration.
- Regression proof evaluates the real producers without executing benchmarks.
  Exercise it through the existing hermetic static-code surface.

## Acceptance

- Storage producers evaluate for SQLite and PostgreSQL.
- Browser producers evaluate for both backends and Chromium/Firefox.
- The regression fails on the original missing-argument defect and passes after
  the repair.
- All eight ordinary E2E gate derivation identities remain unchanged when
  compared under the same source/artifact conditions.

## Boundaries

Do not change product behavior, workloads, dataset shapes, sampling, freshness,
VM resources, timing policy, or baseline import. Do not supply fabricated unused
lane values to measurement producers. Fresh profiling is separate work.
