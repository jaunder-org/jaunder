# ADR-0244: A focused standalone host UX sandbox owner

- Status: accepted
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
The focused owner reduces orchestration coupling rather than merely relocating
the lifecycle state machine. A standalone bin inside xtask retains coupling;
separate host-runtime plus sandbox crates add an unnecessary shared layer.

## Decision

Use one small root-workspace binary package, `ux-sandbox`, as the focused
application owner, with `just sandbox ...` as its public front door. The helper
depends directly on existing COTS processkit, Tokio, Clap, serde, error, hash
and tempfile crates; it has no xtask, host, or test-support package dependency.
One Tokio runtime owns async processkit child/group lifetimes and signal
handling. Do not add a shared-runtime library or expose xtask private APIs.

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

The single Just recipe retains the pinned per-recipe `[ positional-arguments ]`
attribute and `sandbox *args`, and executes
`nix run --no-update-lock-file --no-write-lock-file .#ux-sandbox -- "$@"` from
the active checkout. Caller argv is forwarded without string reconstruction. The
separate `nix/ux-sandbox.nix` package uses the existing Cargo path-closure,
source-filter, placeholder, vendor and Crane conventions to build only the
helper at its dev profile. Required workspace manifests/configuration, root
lockfile and pinned toolchain remain inputs; excluded workspace targets receive
placeholders, not real application/CSR implementation, public assets or docs.
The helper's own dependency artifact does not inherit product Cargo artifacts or
application environment. The root Cargo path patch requires the retained SQLx
SQLite driver source and parent license notices for metadata resolution, without
adding SQLx to the helper's compiled dependencies. These patch inputs, shared
manifests, lock/toolchain inputs and required helper dependencies can invalidate
this package.

Git-flake capture includes tracked-dirty and intent-to-add helper inputs. New
untracked helper inputs are omitted without automatic refusal by bootstrap;
users must `git add` or `git add -N` them. Do not add staging or a pre-bootstrap
admission check. This limitation is separate from the existing application-input
admission below, which still refuses relevant omitted inputs. The previous Cargo
bootstrap already supported warm reuse, so this change promises neither faster
warm invocation nor a cache hit.

The existing `validate-test-checks` CI job requests an ordinary helper build
after unchanged Cachix setup. The existing helper name is eligible for
publication; build-request wiring and cache eligibility do not establish actual
CI completion, upload or later substitution. Those claims require direct
retained evidence; no cache trust/filter/token machinery or test-verdict caching
is added.

For application preparation, the helper admits relevant untracked checkout
inputs against the selected Nix source and refuses omitted inputs before
selection. It invokes Nix interruptibly to select `.#sandboxArtifacts`, one
immutable linkFarm containing debug-profile `jaunder` and `test-support`
executables, with the selected debug CSR embedded in Jaunder by the existing
`server/build.rs` path and `rust-embed/debug-embed` feature. Each invocation
owns a fresh `.xtask/sandbox-artifacts-*` output link and retains it through
preparation and the full server or command consumer lifetime. Nix/store owns
application artifact build concurrency and reuse. Sandbox no longer publishes
mutable checkout application artifacts or takes `.xtask/host-artifacts.lock` for
preparation. Its distinct workspace, server-lease and application runtime locks
remain unchanged.

`e2e-local` selects the same source-consistent package set through a unique
`.xtask/gcroots/e2e-local-*` output link retained through browser and
server/collector teardown. Ordinary e2e selects `.#sandboxArtifacts` with debug
CSR; `--update-visual-snapshots` selects `.#sandboxArtifactsReleaseCsr`, which
embeds the existing release CSR while keeping server and seed debug-profile.
There is no separate release-CSR flag. E2e also no longer takes the checkout
artifact lock for these immutable outputs. `test-local` retains its separate
`.xtask/host-artifacts.lock` and `.#csrBundle` input. Manual `just build-csr`
and Cargo/Vite `target/site` behavior remain unchanged.

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
subprocess. Its stdout is captured and withheld from operational command output
while stderr and exact child status are preserved. The supervisor prints its URL
after readiness and prints the seeded `user`/`operator` credentials only for a
nonempty profile, including a named resume using its recorded profile. Empty
profiles do not print credential hints. Do not add registry, quarantine, control
socket or arbitrary kernel-safety guarantee.

#### Qualified caller-selected CSR compatibility

During discussion of the `e2e-local` CSR override, the human said exactly:

> It sounds like it's probably a method to atttempt to achieve what using nix
> already gets us---cached builds. By all means keep looking, but my inclination
> is to say don't worry about replicating it.

The Lead applied this direction narrowly to the immutable `e2e-local` path: if
`JAUNDER_CSR_BUNDLE_DIR` is set, e2e refuses before artifact selection, server
startup or seeding, with guidance to unset it. It neither silently ignores the
supplied value nor falls back to Cargo/site output. This is a qualified Lead
application of the human's inclination, not a separate literal “approved”
response. The refusal is e2e-specific; the variable remains a Nix build-time
input and remains available to `test-local` and manual Cargo/Vite workflows.
Release CSR remains selected by the existing `--update-visual-snapshots` mode;
no `--release-csr` option is added.

SIGINT/SIGTERM begin graceful drain. Further SIGINT may force earlier; repeated
SIGTERM joins the drain without escalation per ADR-0181. ProcessKit owns the
automatic group-stop escalation after exactly 10 seconds. The application waits
for the direct child concurrently with group stop and preserves initiating
status (130/143), or the child's exact status when no signal initiated shutdown.
If a successful forced-stop report still lists group members after direct-child
reap, the application makes one bounded follow-up ProcessKit stop/report with a
five-second grace and escalation disabled, consuming further signals without
restarting that operation. Cleanup requires direct-child reap and an explicit
empty-group report. An unconfirmed stop/wait logs diagnostics and retains
group/application guards rather than polling/retrying or releasing storage. This
is the implemented fail-closed ordering, not a universal OS guarantee or a claim
of protection after helper SIGKILL. Do not change `jaunder shut-down`'s separate
30-second wait/no-escalation contract.

Removing sandbox-specific xtask CLI/dispatch/reporting, broad package linkage
and synchronous runtime bridges reduces ownership indirection and
private-package coupling. Replacing mutable application artifact publication
with Nix-built, per-invocation rooted package sets removes the sandbox/e2e
checkout artifact-lock path while retaining `test-local`'s separate lock. The
focused package/manifest, separately filtered Nix helper bootstrap, application
artifact composition, helper adapters, duplicate readiness probe, maintained
tests and required package builds are real costs. Lifecycle and storage
responsibility remains in the helper, and e2e ownership remains in xtask;
neither is counted as eliminated. The tradeoff is structural simplification, not
a performance-improvement claim.

## Maintenance and validation

This numberless draft remains `proposed` until serialized post-merge numbering
and promotion. It records the implemented Just-owned design; ADR-0046 and
ADR-0058 remain unchanged. ADR-0180's dated clarification records only partial
supersession, preserving its historical Decision and remaining contracts.

Maintained public-command and native process tests cover exact argv, immutable
artifact selection/source admission/root lifetime, lifecycle/profile/reset/
command behavior, runtime-info identity/readiness, signal distinctions, the
automatic deadline, and descendant containment/reap before storage cleanup or
guard release. The focused process integration trusts ProcessKit's group-stop
report rather than maintaining the removed membership polling/retry and
six-operation wrapper fault matrix. CONTRIBUTING defines the verification
cadence: production validation on x86_64 Linux and periodic
development-environment checks on committed changes for Apple Silicon macOS,
with no ARM Linux native-delivery gate. Focused tests do not replace repository
gates or establish untested foreign-native guarantees.
