# ADR-DRAFT: Just Owns Qualification Workflow Entry Points; Nix Owns Provenance

- Status: proposed
- Date: 2026-11-06
- Issue: [#1895](https://github.com/jaunder-org/jaunder/issues/1895)

## Context

Before this migration, the opt-in production-baseline workflows entered through
a host-only xtask command catalog. In discover/accept,
`production_baseline::run` combined revision resolution, origin
validation/fetch, source-specific harness admission, lease acquisition, VM
workflow execution, evidence publication, and `CommandResult` creation. Styling
has a distinct flow in `qualification_run::run`; local smoke intentionally uses
a mutable current-checkout path and publishes no evidence. These are not
interchangeable evidence contracts.

The root Justfile and development shells already provide pinned Just, Nix and
host tooling. Just runs dependencies before dependent recipes and stops on a
failed command; Git resolves expressions and fetches exact commits for ordinary
discover/accept admission; Nix provides immutable host/harness/VM derivation
outputs. These supported tools can own the public interface and ordinary input
setup. They do not individually implement the qualification-specific
source/output admission, lease and VM lifecycle, safe evidence contract, or
domain failure attribution.

ADR-0028 assigns in-derivation artifact production to `devtool` and host Nix
invocation/output analysis to `xtask`; Nix derivations never invoke xtask. It
also describes the generic host `CommandResult` reporting envelope,
`.xtask/last-result.json` sidecar and `xtask-done:` sentinel. ADR-0141 keeps
xtask outside the product workspace and retains explicit host unit tests.
ADR-0225 assigns executable/harness provenance to Nix while preserving the
qualification fixture, backend, lease, lifecycle, sanitization and evidence
guarantees. The needed change is a narrow workflow-ownership carveout, not
reversal of these host/workspace/provenance constraints.

## Decision

Make four root Just recipes the public workflow catalog: discover, accept,
styling qualification and local smoke. Just accepts their normal arguments and
owns the ordinary Git preflight for discover/accept only: require a clean
checkout, record full `HEAD` as expected harness SHA, resolve product inputs
with `git rev-parse --verify`, require the canonical Jaunder `origin`, and fetch
expected harness HEAD plus source and optional target in one
`git fetch --quiet --no-write-fetch-head --refetch origin <commits>` over the
distinct commit set. Ordinary fetch can skip locally complete objects without
asking the server for them; `--refetch` requests the objects afresh. Accept the
larger transfers and normal Git maintenance to prove origin serves each object,
not reachability from a particular named branch. Reject identical resolved
source/target commits. The host leaf receives these full IDs, re-admits its
clean source pin, and requires its pinned SHA to equal the expected harness SHA
before Nix admission or lifecycle mutation. This binds the upstream-verified
harness identity to the actual source without a second origin check/fetch; the
leaf does not repeat product expression resolution or upstream verification.

Styling qualification also captures a clean current `HEAD` and passes that
expected harness SHA to the host, which binds it to its clean `SourcePin` before
Nix admission. Styling retains its existing immutable local
`git+file:…?rev=<commit>` source/harness selection and imposes no
canonical-origin, network, or upstream-availability requirement. Local smoke
retains its mutable path-flake and no-evidence semantics. Just invokes the
pinned host tool via the existing Nix-backed launcher; its dependencies supply
ordered fail-fast composition and honest process status, not a workflow
framework.

Keep source-cleanliness/drift checks and source-specific Nix harness admission
in the focused host executor: that admission selects the fixture graph from the
immutable source, builds and GC-roots the selected output, verifies the
executing host tool and helper against it, and records derivation/output/NAR
identity. Keep the cooperative lease, restricted workspaces/GC roots,
VM/proxy/browser/backend lifecycle, qualification fixture/protocol checks,
canary collection, evidence-specific validation, cleanup/publication order and
product/harness/infrastructure failure attribution there. Styling keeps its
current-source pin and its own harness admission. Local smoke keeps its mutable
path-flake and no-evidence semantics. The host operations accept normalized
inputs and expose focused failure diagnostics plus ordinary process status, not
a generic gate-wide report.

Retire only the workflow-specific public xtask catalog and report path: remove
the `ProductionBaselineCommand`/`Command::ProductionBaseline` public recipe
surface and its old dispatch; route the narrow host execution leaf without a
user-facing catalog. In `xtask/src/main.rs`, Just-owned production-baseline
operations bypass `invocation_trace::execute`'s generic `CommandResult::report`
JSON/human envelope, `.xtask/last-result.json` sidecar and `xtask-done:`
sentinel. Remove those generic result constructions from the migrated workflow
functions. Keep `CommandResult`/`StepResult`, invocation tracing/reporting,
sidecar and sentinel for unrelated xtask commands and keep their existing tests.
The proposed numberless draft is projected in `docs/ARCHITECTURE.md` at
authoring time, with descriptive link text and an explicit proposed/pending
approval-and-implementation qualification. Keep the current workflow description
as current truth until the decision is approved and implemented. At
implementation, update the baseline/styling runbooks and current architecture
projections to match delivered behavior, migrate production/styling user callers
to Just, and retain browser tests as workflow consumers. Do not move xtask into
the product workspace or execute it inside a derivation; ADR-0028/0141's
execution/workspace/test boundaries remain.

Use supported tools for the responsibilities they already cover. The existing
COTS Rust `jsonschema` Draft 2020-12 engine is invoked in-process via
`jsonschema::draft202012::validate` (`xtask/src/production_baseline.rs`); its
crate is in `xtask/Cargo.toml` and `Cargo.lock`. Keep it rather than adding a
schema CLI adapter: another executable/process and serialization edge does not
replace report projection, Markdown parity or publication policy; jq predicates
are not full schema validation. The current Unix lease also owns no-follow
opening, regular-file and owner checks, and private runtime-directory handling
(`xtask/src/production_baseline/platform/unix.rs:9-65`). `flock(1)` supplies
only the advisory descriptor lock; shell adoption still requires the safe-open
owner and adds descriptor lifetime/packaging complexity. Retain the current
lease owner. Assess process supervision separately: Python subprocess timeouts
can kill/reap a child but do not implement the workflow's multiple VM/proxy
process lifecycle, restricted diagnostics and coordinated cleanup. Retain
focused host supervision. Atomic `rename(2)` is only a same-filesystem
primitive, not schema or Markdown checks, canary/prohibited-file scans,
allowlist, collision refusal or publication ordering. No whole-system substitute
is required or proposed; compare the small per-responsibility maintenance costs
rather than assuming absence from a shell proves a capability gap.

Preserve the two existing publication contracts separately. Ordinary
discover/accept retain JSON-schema validation of JSON-authoritative
`summary.json`, mechanically generated Markdown parity, exact two-file
allowlist, canary/forbidden-content and prohibited-file/symlink checks,
collision refusal and atomic publication after successful source recheck.
Styling retains its current styling-specific JSON identity/phase/context
validation, independent canary scans of JSON and separately generated Markdown,
private staging, lifecycle cleanup, source reverification and unique-destination
staging rename; it does not use the ordinary baseline schema/parity validator.
Do not add that validator or change styling evidence meaning. Smoke publishes
neither contract. A failed collection/validation/required cleanup/source
recheck/sanitization cannot produce a durable evidence report; raw state remains
restricted and ignored. Ordinary discover/accept may publish a valid, sanitized
product/protocol-failed outcome report after schema, sanitization, cleanup and
source-recheck requirements pass, while returning nonzero. That is not a
publication-inadmissibility failure. Styling preserves its existing pass-only
publication behavior and does not gain failed-outcome reports.

## Consequences

The public operation names, input normalization and combined harness/source/
target upstream check move to Just/Git for discover/accept; the host binds the
expected harness SHA to its admitted clean source pin before lifecycle mutation.
Styling retains local source/harness admission without a network prerequisite.
Nix continues to identify selected executables/harnesses/VMs. The
workflow-specific host domain work remains where its live checkout, Nix, VM and
evidence controls exist; this is not a wrapper-only migration because Git
preflight, public catalog ownership, process/report boundary and caller paths
move. Existing jsonschema engine is reused; the safe lease and process
supervision remain focused host responsibilities for the specific contracts
standard tools do not cover without adding adapters/runtimes.

This draft narrowly supersedes ADR-0028's assignment of the **generic report
envelope/sidecar/sentinel to these opt-in qualification commands only**; it does
not remove that infrastructure or sentinel for other host commands, and does not
change ADR-0028's host-only xtask/in-derivation devtool litmus. ADR-0141's
workspace ownership and host unit tests remain. ADR-0225's Nix provenance and
retained qualification guarantees remain. Numberless proposed status follows
ADR-0152; this decision is not accepted until its approval/promotion lifecycle.
No implementation is authorized by this draft.
