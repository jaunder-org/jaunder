# Always-on tracing at project-controlled workflow boundaries

## Outcome

Retain useful ordinary-run timing, execution and provenance evidence at the
local and CI boundaries Jaunder controls, so recurring slowdowns can be
diagnosed after the fact. Capture observed work honestly, not a reconstruction
of Just internals. Use disclosed automatic callsite capture in the trusted
project environment/CI.

**Revised draft; independent review and explicit approval required.** R9 Manager
background `b72fc8dd-1a23-4cb0-a2e6-1a09a1537494` records the human direction:
“No, we're not going to patch just, upstream or otherwise. We just need good
tools for capturing tracing data at the boundaries we control.” Early receipts:
“Allow bounded early receipts”; activation: “Yes, disclosed automatic capture”.
This replaces the full native-root parity objective and ends Just patch/upstream
pursuit. It does not authorize implementation, commit or this document's
approval.

Earlier qualified decisions remain: R6 (`4e908374-0c33-4e11-ba97-9c22106fce2c`),
“Continue; report capture gaps”; R7 (`b9053c81-44e9-41ac-8c13-43292f8f2fc3`),
“Accept native CI expiry”; local-retention amendment
`c7be2062-ef0f-41da-901b-4f91bedffdc7`, “Confirm corrected contract,”
corroborating the direct “Correct”. Prior spec-only approval
`ea7450e0-a2a8-4bc0-af71-906690baedc5` covered hash
`4e31be570e29c02edf89089aa32f2f1a636e7bd886d50f90602b189eace57056`; that version
and pre-revision snapshots remain in ignored planning evidence. #1751 is the
efficiency context; #1753 remains deferred and excluded.

## Approach and tradeoffs

Reuse existing trace writing, focused owner instrumentation, native CI metadata
and analyzer outputs. Add a small passive receipt interface at controlled
callsites, not an executable interceptor, new supervisor or workflow engine.
Owners alone retain ordering, signals, waiting, reconciliation and teardown.
Accept modest recording cost and explicit unknowns rather than patching Just,
recreating dependency state or promising complete process/root attribution.

### Coverage and limits

These are the supported observed scopes, not promises about whole CLI lifetimes:

| Workflow area                      | Project-controlled observation scope                                                                                                            |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Local gates and hooks              | Named `Justfile`/shared-recipe callsites; hook entry where observed; safety owner's classification, host result, reconciliation and docs checks |
| TSC, CSR build, coverage, doctests | Existing setup/tool/Nix producer/verdict calls and available output/reuse evidence                                                              |
| Focused Rust/browser               | Existing focused owners or accepted successors: preparation, CSR/worker/test or server/seed/browser, and owned teardown                         |
| Sandbox                            | `ux-sandbox` helper/application preparation, readiness, bounded commands and shutdown; no application-request recording                         |
| E2E producers/reconciliation       | Existing script's selection, producer attempts/settlement and verifier outcomes; available VM phase artifacts linked separately                 |
| CI validation/E2E/helper build     | Native job/step/run-attempt identity and timing, plus the controlled callsites above and diagnostic artifact publication                        |

A direct Just invocation's full lifetime, parsing/discovery/resolution failures
and work before a controlled callsite are **unobserved/unknown**, not blockers
to this objective. No start/end callback, native dedup event or not-reached
state is invented. Absence of a receipt is not proof of a skip, cache hit or no
work. Record known skips/selections only when an existing owner actually
observes them. A callsite may be entry-only if no safe completion seam exists:
mark terminal status unknown rather than change exec/signal behavior to
manufacture it. Other workflows, raw tool invocations and unrelated environments
are excluded.

Automatic capture is disclosed in maintained developer guidance and CI output;
no per-run tracing opt-in or executable/PATH interception is needed. A bounded
early receipt may be emitted only after entering an existing controlled boundary
in the trusted project environment/CI. Before reliable workload classification,
it contains only an invocation ID, start/elapsed time, fixed stage/outcome and
capture health: no caller text, paths or argument fingerprints. Later resolution
does not authorize detailed capture for excluded operations. If no owned output
location is available, report capture unavailable rather than use a global sink.

### Record, privacy and failure semantics

- Use owner-scoped IDs and explicit parent links, bounded phase/workflow labels,
  UTC lookup time and process-local monotonic duration. Record workload outcome,
  code/signal/timeout only where observed, separately from capture health.
- Record allowlisted commit/dirty/unknown state, workload/lane category,
  available tool/store identity and CI run/job/attempt references. Dirty state
  is not exact source attestation; missing provenance stays unknown.
- Distinguish current acquisition/execution from historical producer evidence in
  reused outputs. Do not infer build versus substitution from elapsed time.
  Preserve overlapping phases and clock frames; do not sum nested durations or
  fabricate cross-host alignment/parentage.
- No blanket argv/env/expanded-command/content/error-text capture, secret data,
  arbitrary paths or argument fingerprints. Reference existing approved
  diagnostic payloads, not copied stdout or indiscriminate uploads. Keep local
  bundles private and CI access within existing repository artifact permissions.
- Publish observed start/completion separately. Preserve actual workload exit,
  required evidence and cleanup when recording fails; visibly report gaps.
  Abrupt death or missing terminal publication is incomplete/unknown, not
  success or confirmed abandonment. Do not rerun work to reconstruct a receipt.

### Retention

Local evidence is private ignored worktree-owned data, separate from
count-pruned runner history: no age expiry, collector or liveness service. It is
removed with separately authorized worktree removal only. Explicitly export and
verify anything that must survive removal; a selection marker is not export
proof or authority to remove a worktree, cache, sandbox or another assignment's
files.

CI ordinary bundles, **including incomplete ones**, use native 14-day artifact
expiry/clock (`expires_at`), not a new completion/last-activity deadline. Honest
lifecycle readback explains status, not expiry eligibility. Owners explicitly
export selected incident/uncertain evidence before expiry. Failed export plus
expiry is visible diagnostic loss; no indefinite conditional hold or service.

Under storage pressure continue the workload, preserve its actual status and
mandatory evidence/cleanup, and report missing new capture. Do not silently
evict local or unexpired evidence early. The owner requests storage/export
intervention; there is no finite-storage guarantee or capacity/offload
framework.

## Acceptance

- Each coverage row automatically records its controlled observations, with
  disclosure. A reader can identify observed work, useful phase timing and
  provenance or explicit unknowns without assuming a native Just root exists.
- Early-receipt fixtures enforce activation and minimal-field limits; excluded
  operations never gain detailed capture. Direct-Just/pre-resolution gaps and
  missing receipts never become invented roots, skips or not-reached claims.
- Success/failure/interruption and unavailable capture preserve workload status
  and mandatory cleanup, including cleanup longer than two seconds. No added
  signal handling, supervision or altered execution/deduplication behavior.
- Cache/provenance/correlation fixtures distinguish current execution from
  reused producer evidence, dirty/unknown inputs, overlapping phases and
  separate attempts. Preserve analyzer per-attempt joins, boot-frame/median and
  malformed-data behavior, documented analyze/run/boot-phases outputs and
  serial/single-worker measurement controls; these are retained diagnostics, not
  new automatic benchmark runs.
- Local fixtures survive age advancement and runner pruning; verified export
  survives test-owned worktree removal. CI fixtures verify native 14-day
  retention for complete/incomplete bundles, honest unknowns and export/expiry
  loss reporting.
- Prove a bounded real controlled path and CI artifact accessibility after
  focused interface proofs, not a benchmark campaign. Commit only maintained
  code/tests, usage/ownership docs and any necessary narrow ADR, not incident
  evidence.

## Boundaries and #1898 coordination

No production tracing, Just patch/upstream work, executable/PATH interception,
new supervisor, dependency engine, scheduler/#1753, uncontrolled cleanup or
weakened coverage/backend/browser/approval guarantees. This is planning only.

#1859 owns passive receipts/privacy/bundles/diagnostic CI; #1898 owns successor
placement/removal; actual owners retain behavior/lifecycle. Serialize
overlapping callsites only, prefer independent foundations then settled
successors, and keep host diagnostic dependencies out of hermetic producer
inputs. Preserve existing analyzers/diagnostics, not old command spelling or
launchers solely for spans. Proposed integrations require owner agreement, not
edits to #1898's approved docs or a blanket tracing-before-retirement
prerequisite.
