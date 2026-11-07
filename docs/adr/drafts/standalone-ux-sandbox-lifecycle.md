# ADR-DRAFT: A focused standalone host UX sandbox owner

- Status: proposed
- Date: 2026-11-07
- Issue: #1893

## Context

`cargo xtask sandbox` currently enters through xtask's CLI, dispatch,
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

## Decision proposed

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
SIGTERM joins the drain without escalation per ADR-0181. Separately propose
automatic escalation after 10 seconds, without requiring a second signal; D1
accepted bounded automatic escalation direction, but the exact 10-second grace
remains unapproved. Preserve initiating status. Do not change
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

This is proposed, not accepted, and grants no implementation authority.
ADR-0028's exception and ADR-0180's owner/signal-clause supersession require
explicit spec/ADR approval. ADR-0046 and ADR-0058 remain unchanged. Ten-second
grace requires approval before implementation.

The new package/ownership boundary and concurrency/storage contracts trigger a
required planning outline after spec approval. No outline is created here;
implementation waits for outline approval and the approved planning contract to
be committed.

Feasibility evidence available before implementation is limited to source/API
inspection and an isolated argv probe. After approvals and implementation, prove
actual artifact output/lock integration, all lifecycle/profile/reset/command
behavior, Jaunder runtime-info readiness, signal distinctions, automatic
escalation and process-group descendant containment/reap on supported platforms.
Apply CONTRIBUTING's cadence: production validation on x86_64 Linux and periodic
development-environment checks on committed changes for Apple Silicon macOS; no
ARM Linux native-delivery gate. Do not require an invented matrix or claim
untested guarantees.
