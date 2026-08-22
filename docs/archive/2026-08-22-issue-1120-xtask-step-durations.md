# Issue 1120: xtask step durations

## Outcome

Every xtask step result records elapsed wall-clock time, and
`.xtask/last-result.json` exposes that timing so gate-cost analysis can identify
slow steps without scraping raw logs. Human output remains concise while making
slow or failed steps visible enough to diagnose gate friction.

## Load-bearing decisions

- `StepResult` owns the per-step duration field because
  `.xtask/last-result.json` already serializes `CommandResult.steps[]` as the
  machine-readable step list.
- The serialized field is `duration_ms` in milliseconds, matching existing
  command-level `CommandResult.duration_ms` naming and unit conventions.
- All step-producing paths should produce a duration, including command-backed
  steps, pure in-process checks, skipped steps, precheck failures, Nix-backed
  steps, PR report steps, and e2e-local substeps. Cheap in-process checks may
  legitimately report `0` ms.
- `xtask-done:` remains command-level only and keeps its current
  `command=… ok=… exit=… duration_ms=…` shape so existing consumers keep
  working.
- Human output should stay low-noise. It may show duration for failed or slow
  steps, or all steps if the output stays readable; it should not require users
  to open build logs to identify a slow step.
- This issue changes xtask result reporting only. It does not reorder gates,
  change fail-fast behavior, change checked surfaces, or introduce receipt/cache
  policy.

## Acceptance

- Running `cargo xtask check --no-test` writes `.xtask/last-result.json` with
  `duration_ms` on each `steps[]` entry.
- At least one slow-step visibility path is observable from normal terminal
  output without reading raw build logs.
- `xtask-done:` still appears with the existing command-level fields after
  successful and failed xtask command paths.
- Unit tests cover serialization of the step `duration_ms` field and at least
  one helper/path that assigns a nonzero step duration.
- Existing command result serialization tests continue to cover `ok`, `skipped`,
  `detail`, and optional payload behavior.

## Boundaries

- Do not change the meaning of command-level `duration_ms`.
- Do not make duration values part of pass/fail policy.
- Do not add historical trend storage, cache receipts, gate classification, or
  step ordering policy.
