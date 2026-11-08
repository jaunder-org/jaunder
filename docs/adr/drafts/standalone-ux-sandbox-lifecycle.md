# ADR-DRAFT: A focused standalone host UX sandbox owner

- Status: proposed
- Date: 2026-11-07
- Issue: #1893

## Context

Before #1893, `cargo xtask sandbox` entered through xtask's CLI, dispatch,
result/report protocol and synchronous process adapters. The sandbox lifecycle
is application-specific; xtask is a broad host-tool package and its
processkit/Tokio bridge uses synchronous wrappers/runtime machinery. The
existing xtask `host_server`, `host_signals`, and `process` modules are also
used by `e2e-local` and remain after this change.

ADR-0028 places host orchestration in xtask by default. ADR-0046 keeps
out-of-process real-storage seeding in test-support. ADR-0058 defines host as a
shared host-only floor. ADR-0180 assigns sandbox lifecycle ownership to xtask.
Issue #1893 requires an overall simpler system, not relocation. The
overall-simplification criterion remains binding. A standalone bin inside xtask
retains coupling; separate host-runtime plus sandbox crates add an unnecessary
shared layer.

## Decision

Use one small root-workspace binary package, `ux-sandbox`, as the focused
application owner, invoked only through `just sandbox ...`. The helper depends
directly on existing COTS processkit, Tokio, Clap, serde, error, hash and
tempfile crates; it has no xtask, host, or test-support package dependency. One
Tokio runtime owns async processkit child/group lifetimes and signal handling.
Do not add a shared-runtime library or expose xtask private APIs.

This is a narrow host-placement exception to ADR-0028, not a general move of
host orchestration out of xtask. It removes sandbox-specific xtask
CLI/parser/dispatch/reporting and its synchronous-to-async process bridge. The
lifecycle state machine and safety behavior remain owned and tested in the
helper; they are not counted as eliminated.

The helper locally implements a narrow Jaunder runtime-info identity/address
parser and readiness probe. This duplicates an adapter because xtask's existing
session component is private and coupled to xtask artifact, Shell, report,
process and signal adapters. Keep this duplicate narrow; existing `e2e-local`
adapters remain unchanged and are not credited as savings.

Keep test-support as the existing seed subprocess/real-storage boundary under
ADR-0046; do not add a crate dependency or Media API. Keep ADR-0058's host-floor
invariant unchanged. Supersede ADR-0180 only for the sandbox lifecycle owner and
its literal signal-escalation clause; preserve its remaining behavior contracts
and paths.

## Contract and consequences

The single Just recipe uses the pinned Just per-recipe
`[ positional-arguments ]` attribute and `sandbox *args`; it builds
`cargo build --locked -p ux-sandbox`, then execs
`./target/debug/ux-sandbox "$@"`. Caller argv is forwarded without string
reconstruction. The helper preserves disposable and named workspaces,
empty/standard/demo profiles, init for each newly prepared workspace, resume
without reseeding, staged/reset recovery, allowlisted operational command mode
and exact child status.

The standalone bootstrap publishes the distinct `target/debug/ux-sandbox`
executable, not an e2e-consumed Jaunder/test-support output; no isolated target
cache or artifact lock is needed for that output. The helper then acquires
`.xtask/host-artifacts.lock` around protected mode-specific preparation: server
builds CSR via `just build-csr debug`, Jaunder and test-support; command mode
builds Jaunder only. It validates selected debug artifacts and releases the lock
before long-lived child use. `e2e-local` keeps its existing consumer/teardown
lock lifetime.

Preserve name/path validation, workspace-local profile metadata, stable locks
and ephemeral ready/stopping metadata under `.xtask/sandboxes/.locks/`, server
lease before workspace transition lock, reset two-rename recovery, application
runtime-lock refusal, executable digest/generation fencing, runtime identity
validation, bounded readiness and cleanup barrier. For stopped-workspace command
mode, acquire the server lease first to prove none is held, then
workspace-exclusive ownership for recovery/admission; validate the workspace and
acquire the application runtime lock. While retaining the server lease and
runtime guard, release exclusive workspace ownership, acquire shared workspace
ownership, then release the server lease. Hold runtime and shared workspace
ownership throughout the operational child. For a live managed server, use ready
metadata/digest admission and shared workspace ownership without acquiring the
lease or runtime lock. Seed remains `test-support seed-sandbox-profile` as a
subprocess. Do not add registry, quarantine, control socket or arbitrary
kernel-safety guarantee.

SIGINT/SIGTERM begin graceful drain. Further SIGINT may force earlier; repeated
SIGTERM joins the drain without escalation per ADR-0181. Automatic escalation
occurs after exactly 10 seconds without requiring a second signal, as approved
in the human S1 design decision. Preserve initiating status (130/143), or the
child's exact status when no signal initiated shutdown. Do not change
`jaunder shut-down`'s separate 30-second wait/no-escalation contract. Unexpected
parent exit does not permit cleanup until process-tree containment/reap finishes
on supported platforms; this is a proof obligation, not a universal kernel
guarantee.

The design is simpler only if removing sandbox-specific xtask protocol, broad
package linkage/private adapters and sync/runtime bridges outweighs the new
package/manifest, Just recipe, helper app adapters, duplicate readiness probe,
tests and extra cold command-mode helper build. No raw line-count target
applies; most lifecycle responsibility remains. Reject if actual integration is
relocation or total system is not simpler.

## Approval and proof

Human S1 approved the design, including the exact 10-second grace, ADR-0028
placement exception and partial ADR-0180 owner/signal-clause supersession. Human
O1 approved the required implementation outline and implementation (request
`0a5e9118-864e-47ab-9247-ebaecf0aaa74`, decision `1893-o1`); the planning
contract was committed before implementation. ADR-0046 and ADR-0058 remain
unchanged.

The design is implemented: the public Just helper owns all modes and the old
xtask sandbox route is retired. This draft deliberately remains `proposed` for
serialized post-merge numbering and acceptance; that automation state does not
mean human implementation approval is pending. Historical accepted ADR Decision
text is preserved, with a dated partial-supersession clarification on ADR-0180.

Focused proof covers artifact output/lock integration, lifecycle/profile/reset/
command behavior, actual Jaunder runtime-info readiness, signal distinctions,
the automatic deadline and actual descendant containment/reap on this Linux
host. Earlier real standard/demo Media and legacy-workspace compatibility proofs
remain applicable to the unchanged helper. Independent whole-branch review and
D2 overall-simplification acceptance remain required; focused success does not
substitute for final repository gates or foreign-native proof. Apply
CONTRIBUTING's cadence: production validation on x86_64 Linux and periodic
development-environment checks on committed changes for Apple Silicon macOS; no
ARM Linux native-delivery gate. Do not require an invented matrix or claim
untested guarantees.
