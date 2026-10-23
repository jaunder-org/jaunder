# Issue #1821: E2E result capture visibility

## Approval provenance

Direct User approval: “1821: approved”, answering
`request1821-capture-visibility-v1` / the supplied approved spec. This file is
the tracked working copy of that approved specification; the approval authorizes
this scope, not PR merge.

## Outcome

Make the existing e2e receipt and copy path report separately what was observed
at the source and what happened at each applicable transfer for `results.json`,
duration report, and census. This distinguishes absent/empty producer output
from later retention loss without claiming that a copied file is complete or
valid.

## Approach and tradeoffs

Extend the existing lane phase receipt and host diagnostics-copy reporting; do
not add a capture framework. Keep two independent facts for each configured
result: source condition (`missing`, `empty`, `nonempty`, or `inspection-error`)
and transfer outcome (`copied`, `copy-error`, or `not-applicable`). Include a
concise error detail for inspection/copy errors. `not-applicable` means no
transfer was attempted (for example, an absent source or an unconfigured
census); it is not success. A nonempty source may contain malformed or partial
JSON; preserve its bytes where a transfer succeeds, but never treat them as a
valid report or verdict.

Report only what the existing boundaries can observe:

1. Observe reporter output or the preserved preflight census at its guest source
   path, before staging.
2. Record the outcome of staging the available source as a lane-qualified guest
   file.
3. Record the guest-to-retained-Nix-output copy outcome in the existing lane
   receipt.
4. Keep host-copy outcomes distinct from guest receipt facts. Surface a failed
   host copy with the affected lane/file/operation through the existing host
   diagnostic channel; the guest receipt cannot prove this final transfer
   succeeded, nor can host diagnostics establish that an absent source was never
   produced.

The census comes from the preserved preflight only on lanes configured to run
that preflight (currently Firefox); its absence on unsplit Chromium is not
reporter failure and must be `not-applicable`. Do not add census production or
alter lane topology. Playwright may produce its JSON only during `onEnd`; no
report is guaranteed if forcibly killed before that hook. Explicit statuses and
concise details cost modest receipt/artifact space but avoid collapsing
production and transport observations.

## Acceptance

- For each configured result, the receipt keeps source condition and transfer
  outcome as separate fields/facts; inspection errors are never classified as
  missing. Error details identify the affected lane/file and operation.
- Source-to-lane-qualified-guest and guest-to-retained-Nix-output outcomes are
  recorded in the lane receipt; retained-output-to-host-diagnostics failures are
  separately surfaced by the existing host diagnostic channel with
  lane/file/operation detail. Neither receipt nor report implies success at a
  later boundary. Census is applicable only where preflight is configured.
- Preserve available bytes, including empty and malformed/partial JSON, whenever
  the corresponding transfer succeeds. Statuses do not assert JSON validity or a
  test verdict.
- After approval, add behavioral regression coverage using the actual
  capture/lift logic with controlled filesystem/transport outcomes—not
  source-string assertions alone. Cover empty source with successful copy; empty
  source with copy error; malformed nonempty byte preservation; missing versus
  inspection error; failed Playwright plus lift error retaining Playwright as
  primary; and Playwright success plus lift error remaining fail-closed.
  Existing source-order checks may supplement this proof.
- Preserve the existing behavior: capture/lift errors do not replace a
  Playwright failure, and cannot make a successful Playwright run pass when
  required capture/lift fails.

## Boundaries

No changes to budgets, timeout, lane topology, worker count, resources, or test
population; no production behavior changes. This does not fix the approximately
25-minute timeout or guarantee `onEnd` after process kill. No browser
instrumentation, telemetry framework, new census production, or broad capture
redesign. Run focused regression coverage only after approval; ordinary hooks
and CI follow the normal workflow.

## Evidence and scope distinctions

PR #1818's corrected chronology identifies attempt-1 job `120624291662` and
attempt-2 job `120648217175`; an earlier lookup had reused the attempt-1 ID for
attempt 2. The two attempts share a head, but no unsupported clock alignment is
inferred between CI wall timestamps, phase durations, and Playwright trace
monotonic timestamps. The retained diagnostics lack the named result files;
current source silently skips some missing files, so the evidence does not
establish whether the reporter failed to produce them or a later transfer lost
them.

ADR-0037 establishes capture-before-failure and retained-Nix-output recovery.
ADR-0203 establishes lane-qualified artifacts and Firefox preflight census
ownership; Chromium remains unsplit and does not run that census preflight.
#1809 concerns OpenTelemetry log loss; #1800 concerns temporary-file lifecycle.
They are distinct and neither is a prerequisite for this focused visibility
repair.
