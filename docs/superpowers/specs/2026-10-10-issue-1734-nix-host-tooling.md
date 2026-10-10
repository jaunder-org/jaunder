# Nix-cached host tooling (#1734)

## Outcome

Routine agent and developer commands use a source-built Nix `xtask` package from
this checkout rather than repeatedly compiling the host orchestrator through
Cargo. Identical declared tooling inputs reuse the same Nix output across
checkouts and unrelated application commits. Qualification uses that same
tooling path with Nix-owned provenance, not a separate Cargo qualification lane.

## Load-bearing decisions

- Nix owns tool identity through derivations and store outputs. Do not add a
  source-fingerprint algorithm, identity ledger, or custom attestation service.
- Git commits identify admitted harness/product source revisions; an executable
  need not carry or equal the source revision's Git commit when its declared Nix
  inputs are identical.
- Building xtask in a standalone derivation is allowed. Executing its host
  orchestrator inside a derivation remains forbidden: no recursive host gate,
  GitHub query, VM lifecycle, or callback from a Nix check into xtask.
- Preserve xtask's separate Cargo workspace, the explicit host xtask/tools unit
  tests, and the existing host versus hermetic validation authorities.
- Preserve the application/check source exclusions and dependency boundaries.
  The packaged xtask must not become an input to application coverage or E2E
  derivations merely because it drives them on the host.
- Declare the complete tooling build closure: local dependencies, manifests and
  lockfiles, Rust/build configuration, embedded catalogs and evidence schemas,
  and required host system-theme/application-CSS resources. Relevant shared
  application inputs may legitimately invalidate the tool.
- Qualification's executable and runtime harness inputs belong to a declared Nix
  graph. Reuse existing Nix packages where they already own browser
  flows/configuration, dependencies, test-support, and VM inputs. Do not create
  a parallel source inventory or independent hash-based compatibility protocol.
- Replace exact-commit executable admission with admission against the Nix
  outputs selected for the admitted source. Unrelated source commits may select
  the same executable output; a genuinely different executable or harness input
  must be rejected before lifecycle mutation or evidence publication.
- Retain clean, immutable source admission and source-drift rejection. Product
  source/target selection, backend parity, qualification fixture semantics,
  leases, lifecycle safety, canary sanitization and evidence validation remain.
- Execute qualification browser flows/configuration from their admitted Nix
  inputs, or an invocation-owned copy of those exact inputs, not mutable ambient
  files that bypass the graph. Writable runtime artifacts remain private run
  state rather than edits to store outputs.
- Resolve the active checkout at runtime. A compiled source directory must not
  identify the checkout or supply its local target paths.
- Default invocations resolve this checkout's current flake, not a remote branch
  or a long-lived devShell executable that can silently lag edited tooling.
- Newly created untracked tooling/build inputs must not be silently omitted by
  the Git-backed flake snapshot. Refuse with actionable admission guidance until
  they are included; do not run an older binary as a successful fallback.
- Cargo remains available for focused host tests and explicit source-development
  diagnostics. It is not a distinct required provenance lane for qualification.

## Acceptance

1. The normal documented command path runs a real Nix-built xtask against the
   current checkout. Its milestone candidate payload matches the existing
   helper, including local resume-state handling, and it retains honest exit
   status and the existing result/sentinel contract.
2. A warm repeat reuses the executable without invoking Rust compilation. Report
   cold versus warm command durations separately; do not confuse GitHub
   gathering cost with launcher/build cost.
3. Matched Nix identity probes demonstrate that unrelated Markdown and server
   implementation changes retain the tool derivation/output, while tooling,
   relevant shared dependency, lockfile, configuration and embedded-data changes
   invalidate it. Declaration failures must not silently drop required inputs.
4. A xtask-only change does not add a dependency or source edge to the expensive
   application coverage/E2E producers. Existing source-closure probes and host
   self-test authorities retain their intended populations.
5. Qualification accepts unchanged Nix tooling across distinct clean source
   commits. It rejects wrong tool outputs, mismatched runtime harness inputs,
   missing Nix provenance and checkout drift before lifecycle mutation. Cover
   ordinary discovery/acceptance and current-format styling qualification.
6. Evidence identifies the admitted source/product revisions and relevant
   standard Nix outputs using existing evidence conventions, with no fabricated
   Git build commit and no custom provenance ledger. Validate any changed
   evidence shape and keep publication/sanitization failure closed.
7. A relocated source-built executable uses the actual checkout and target
   directory for local commands. Focused host proofs cover runtime-root
   discovery and qualification admission; focused browser/VM proof is required
   where the integration changes a real runtime boundary.
8. Preserve Linux and supported macOS package evaluation boundaries and declare
   native/runtime dependencies. Native execution on an unavailable platform is
   reported as unverified, not implied by Linux success.
9. Update the architectural decision/projection, maintained operating guidance,
   agent skill entry points and launcher consumers consistently. No consumer is
   left documenting the replaced exact-commit restriction as current policy.

## Boundaries

No issue-query optimization, product architecture refactor, reduced
backend/browser coverage, narrower required gates, new provenance service, or
hand-maintained identity registry. Commit implementation, regression fixtures
and maintained contracts, not exploratory logs or measurement reports. Retire
this spec after final conformance review; keep enduring rationale in the ADR and
current architecture/process documentation.
