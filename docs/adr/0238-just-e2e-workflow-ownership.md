# ADR-0238: Just owns the E2E workflow entrypoint

- Status: accepted
- Date: 2026-11-05
- Issue: [#1890](https://github.com/jaunder-org/jaunder/issues/1890)

## Context

The migration must remove duplicated Rust-host E2E scheduling without turning it
into a similarly large pile of shell embedded in Nix or Just. The human workflow
should stay a reliable single command, with ordinary tools owning orchestration
wherever they fit.

## Decision

Make `just e2e` the default local full-matrix workflow. Just owns the
user-facing workflow and a minimal set of parameterized producer/reconcile
recipe boundaries that CI invokes directly. Reuse ordinary Just recipe
dependencies and sequencing; if their fail-fast semantics cannot support
continue-all producer attempts and aggregate failure, permit one small bounded
helper for that specific need. Do not put hundreds of lines of shell in recipes,
require a `writeShellApplication` package per operation, fragment the workflow
into helper wrappers, or create a generic runner framework. Nix continues to pin
tools/inputs and define the existing VM lane installables, not own the workflow
logic.

Keep `end2end/e2e-lanes.json` as the one enabled-lane topology source. CI
derives producer matrix JSON at runtime using pinned `jq` and Actions
`fromJSON`, with standard matrix, `needs`, `if: always()`, artifact actions and
aggregate job owning distributed execution. Regression coverage checks exact
equality between enabled catalog lanes and emitted producers, and verifies each
backend reconciliation depends on its three Firefox producers. Local execution
attempts all lanes, settles producers, reconciles both Firefox backends despite
absent/failed artifacts, conditionally verifies server-function coverage only
from a successful complete authoritative producer, and returns an aggregate
failure.

After callers move, delete E2E scheduling-only paths and redundant build/report
wrappers in `xtask/src/steps/nix.rs`, and remove `xtask/src/e2e_lanes.rs`
catalog embedding/plumbing; preserve non-E2E functions and required narrow
verification. Remove report envelopes, but retain existing narrow Rust
parsing/verdict logic in place where standard tools lack exact evidence
semantics: independent census/report population and ownership, retry-duration
pressure, boot-trace integrity, and server-function coverage. Verifier commands
use artifact inputs with runtime lane/catalog expectations, not embedded catalog
or scheduler `Lane` types; remove `CommandResult`/`StepResult` presentation and
automatic build hooks. Static inventory/snapshot checking stays independent and
regeneration explicit.

The trace verifier receives explicit capture, selected producer output, lane and
catalog inputs. The authoritative output is the successful immutable output
selected by that invocation's `nix build --json`; a failed kept output is
diagnostic only. Reuse the existing checks: `nix/checks.nix` lifts report,
census, lane manifest and capture; `e2eRunAndCapture` quiesces writers, verifies
archive finalization/member hashes and sizes, and makes required retention
errors fail a successful producer (`capture_result_file`, `retain_e2e_capture`,
`assert_capture_outcome`). `xtask/src/e2e_ownership.rs` reconciles census,
report and lane ownership. Require verifier inputs to come from that selected
successful output and reject missing, inconsistent or wrong-output (stale)
evidence. Do not add a separate receipt, schema or digest inventory absent an
observed cooperative-workflow failure and real consumer unmet by these checks.
Their authority is limited to the producer-observed capture inventory and tested
population; do not claim detection of arbitrary hidden omissions or partial
evidence that all existing checks accept.

This supersedes implementation-ownership clauses only as necessary. Preserve
ADR-0203's topology and aggregate outcomes, ADR-0165's report population
authority, ADR-0081's trace-derived coverage authority, ADR-0037's
diagnostics-before-failure outcome and ADR-0096's capture/attribution
distinction. ADR-0028's host-only xtask boundary remains unchanged.

## Consequences

The public entrypoint stays one command while Just, GitHub Actions and Nix each
use their existing strengths; bespoke code is limited to necessary evidence
verdicts and any narrowly justified continue-all helper. The design relies on
existing selected-output, capture-finalization, archive-integrity and population
checks and accepts their bounded evidence authority; it does not add attestation
machinery or claim execution equivalence with the old Rust driver. No
implementation is authorized by this draft.
