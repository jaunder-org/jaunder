# #1890: Single-command E2E workflows

## Outcome

`just e2e` is the default local full-matrix workflow; CI keeps its standard
distributed matrix and invokes the same minimal parameterized producer and
reconciliation recipe boundaries. This removes redundant Rust-host E2E
scheduling without replacing it with a large shell/Nix or Just orchestration
framework.

## Approach and tradeoffs

Just owns the human-facing workflow and its small set of recipe/dependency
boundaries. Nix continues to define the hermetic VM lane installables and pin
tools/inputs; it is not required to package each operation or own workflow
logic. Use Just's ordinary recipe dependencies and sequencing where they fit. If
its fail-fast dependency behavior cannot provide local continue-all execution
and aggregate failure while preserving diagnostic lifting, use one small,
bounded helper for that specific need—not long embedded shell recipes,
fragmented helper layers, or a general runner. CI's matrix, `needs`,
`if: always()`, artifact actions and aggregate job remain the distributed
choreography. No manual command chain is exposed to users.

The shared source of enabled lane topology remains `end2end/e2e-lanes.json`. CI
derives its matrix JSON from this catalog with pinned `jq` and Actions
`fromJSON`; do not duplicate the lane list in YAML or generate workflow YAML. A
regression check proves emitted producer membership equals the enabled catalog
and each backend reconciliation depends on all three Firefox producers.

`just e2e` attempts every enabled lane despite failures, settles producers,
reconciles both Firefox backends even when artifacts are missing or a producer
failed, and attempts server-function verification only after the authoritative
SQLite/Chromium producer succeeds with complete evidence. It returns nonzero for
any producer, reconciliation or applicable coverage failure. The parameterized
producer recipe runs one catalog lane; reconciliation takes a backend and
artifact root, reports missing/failed producer evidence, and never selects or
builds a VM. Keep Nix outputs/roots and required evidence; lift available
diagnostics before returning producer failure. Cleanup is limited to temporaries
the workflow owns. Provenance remains attribution, not verdict.

Keep existing narrow Rust parsing and verdict functions only for checks ordinary
Playwright/report tooling does not establish: independent census/report
population and ownership, retry-duration pressure, boot-trace integrity, and
static plus trace-derived server-function coverage. Remove their
`CommandResult`/`StepResult` presentation envelopes and automatic build hooks;
preserve parsers/verdicts in place rather than relocating them for naming.
Artifact-input commands receive expected lane/catalog at runtime, without
embedded catalog (`include_str!`) or scheduler `Lane` coupling. Static
inventory/snapshot verification remains independent; regeneration is explicit.

The trace verifier receives explicit
`--capture <path> --producer-output <path> --lane sqlite-chromium-unsplit --catalog <path>`
inputs. The authoritative producer output is the successful immutable output
path selected by that invocation's `nix build --json`; failed kept outputs are
diagnostics only. Reuse the existing output and status checks: `nix/checks.nix`
lifts report, census, lane manifest and capture; `e2eRunAndCapture` quiesces
capture writers, checks archive finalization/member hashes and sizes, and fails
a successful Playwright producer on required retention errors
(`capture_result_file`, `retain_e2e_capture`, `assert_capture_outcome`). Host
population checks in `xtask/src/e2e_ownership.rs` reconcile census, report and
lane ownership. Verifiers must consume these artifacts from the selected
successful output and reject missing, inconsistent or wrong-output (stale)
evidence; no separate receipt, schema or digest inventory is justified absent an
observed cooperative-workflow failure that these checks and a real consumer
cannot handle. These checks establish correspondence to the producer's observed
capture inventory and tested population, not protection against arbitrary hidden
omissions or a partial capture that all existing checks accept. Do not claim
trace parsing or snapshot equality proves more. This uses scheduler-owned output
evidence and does not change browser capture owned by #1884.

This changes implementation ownership only where prior ADRs prescribe Rust
scheduling or verifier presentation. Preserve ADR-0203 topology and aggregate
outcomes, ADR-0165 report population authority, ADR-0081 trace-derived coverage
authority, ADR-0037 diagnostics-before-failure propagation, and ADR-0096's
distinction between capture and attribution. Existing `e2e-local` stays unless
mandatory coupling is demonstrated. No generic framework, arbitrary LOC quota,
or helper-fragmentation rule: judge the complete maintenance cost of production,
configuration, adapters and tests.

## Acceptance

After approval and implementation, verify: all lanes run after early failures;
both Firefox reconciliations run after producer settlement, including
missing/failed artifact cases; coverage is gated on successful authoritative
production and the existing archive-finalization and population checks from its
selected output; aggregate status preserves every failure; and cleanup removes
only owned temporaries while retaining roots and useful evidence. Test missing,
wrong-output/stale, malformed, failed-producer and
archive-incomplete/inconsistent capture evidence against those existing checks.
Do not assert detection of hidden omissions they do not cover. Check lane
partition/population, runtime CI matrix equality and reconciliation
dependencies, census/report union, retry-duration pressure and boot traces.
Confirm static inventory remains independently checkable and the trace verdict
uses only the successful SQLite/Chromium output. These checks establish
contracts, not execution equivalence with the former driver.

Deliverables are the default Just workflow and minimal CI-invocable recipe
boundaries, catalog-derived CI matrix, adapted callers, narrowed verifier
interfaces and focused regression tests. After callers move, delete E2E
scheduling-only paths and redundant build/report wrappers in
`xtask/src/steps/nix.rs`, and remove `xtask/src/e2e_lanes.rs` catalog
embedding/plumbing; preserve non-E2E functions and required narrow verification.
Retain only artifacts with actual CI/verifier consumers. Do not execute these
proofs before approval.

## Boundaries

No browser-flow weakening or #1884 boot/coverage producer changes; preserve
#1885 census behavior; do not take #1891 coverage/doctest work. #1889's gate
calls the full synchronous workflow only after its own landing. This durable
architecture boundary requires human spec approval, then a risk-triggered,
cold-reviewed, human-approved outline before implementation. This spec and ADR
draft are design material, not implementation authorization; no tests have run.
