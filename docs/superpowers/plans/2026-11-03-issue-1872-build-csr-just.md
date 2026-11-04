# Build the CSR bundle from one Just recipe — Implementation Outline

> Approved execution outline. Human approval: Manager request
> `f233afc9-3b25-4b1f-8857-1e373605d628` / `issue1872-build-csr-outline-v1`,
> exact answer “Approve outline and begin”, qualified to a **CLEAN execution
> branch**. The assignment establishes that prerequisite on the clean semantic
> branch at accepted `origin/main`. Outline needed because this changes the
> standalone build caller, removes an xtask module/CLI surface, and reuses the
> task from a host-server path with root, environment, cleanup, and
> lock-lifetime contracts.

## Execution summary

Implement the approved #1872 spec as a single `just build-csr [debug|release]`
recipe. First prove root/profile/environment/cleanup behavior in a bounded
temporary-repository harness; then route the existing host-server caller through
the same task while preserving its lock/root handoff, delete the xtask
CLI/module, and update references. The design was approved under Manager request
`e0d243bd-007d-40cc-b670-5e6e6fe35820` / `issue1859-build-csr-spec-v1` (“Approve
design and Just pin”). The outline approval is granted as recorded above;
implementation is authorized within the approved scope, while merge remains a
separate human gate.

**Baseline/preservation:** assigned checkout `issue-1872-build-csr-just` was
clean at `581871f896b7a9294b18c456e75182348b010ec5`, with no unique commits and
none of the three excluded recovery commits as ancestors. It was fast-forwarded
only (`git merge --ff-only origin/main`) to the accepted `origin/main`
`9908de756599c737a809aa72c24f602deaa77cc8`; this establishes the clean-branch
prerequisite. The excluded commits remain excluded.

## Task outline

- [x] Task 1: Provide and prove the standalone recipe contract.
  - Contract: pin Just in the shared dev/CI shell; recipe defaults to debug,
    accepts only debug/release, resolves repository top-level, clears all six
    repository-redirecting `GIT_*` variables for root discovery and inherited
    children, builds the exact locked wasm target, then cleans only fixed
    `target/site`, then invokes checked-out `tools/devtool` via
    `cargo run --locked` with the current `--wasm`/`--out` arguments. No
    user-selected path or fallback.
  - Verification: one focused disposable Git-root/fake-Cargo harness records
    exact debug/release arguments and call order without compiling. Matrix:
    invalid profile and root-discovery failure produce no Cargo calls or
    cleanup; wasm-build failure leaves the prior bundle untouched and never
    invokes the bundler; bundler failure returns nonzero; every child observes
    all six repository-redirecting Git variables absent. Seed filesystem cases
    for final/dangling `site` symlinks, regular file, symlinked `target`
    ancestor, and injected inspection/removal failure; confirm no bundler call
    after failure, no referent deletion, and fake-bundler observation of the
    untouched dangling link. Invoke from nested cwd with hostile Git variables
    and verify selection remains the Justfile checkout. Then run actual pinned
    `just build-csr` once and inspect the expected `target/site` manifest, shell
    and `pkg/**` output.
- [x] Task 2: Transfer the existing caller and remove the old command
      completely.
  - Contract: `HostArtifacts::prepare_interruptible` calls the same recipe with
    its configured profile under its already-held host artifact lock; preserve
    the parent `Shell` root handoff. Do not add a recursive lock. Ordinary
    e2e-local Nix debug-bundle behavior remains untouched. Remove
    `Command::BuildCsr`, Clap help/name mapping, dispatch arm,
    `pub mod build_csr`, and the entire Rust module; update every stale
    docs/comment reference. The final tree has one standalone build-csr
    implementation/caller, not a compatibility alias.
  - Verification: focused host-server caller test/probe verifies profile
    forwarding, existing lock lifetime and parent-root behavior, including
    failure preventing server build/start. Confirm `cargo xtask build-csr` is no
    longer a recognized command; repository search finds no stale
    `BuildCsr`/`build_csr` command/module references or obsolete user-facing
    command text. Run `just build-csr release` with the pinned command and
    verify release wasm path/manifest output.

## Final integrated acceptance (after the two implementation tasks)

- Review the complete intended diff and exact deletions; prove the old Rust
  implementation/CLI is gone, the one Just recipe owns the command sequence, and
  no unrelated pre-existing commits enter the #1872 delivery tree.
- Verify the pinned Just executable is present in **both** default/dev and CI
  shells; verify shell provisioning does not create a second command
  implementation.
- After the focused recipe/caller proof, use the established hook-backed commit
  and push path and the existing pre-PR validation ladder through an authorized
  executor, with current documented selectors/timeouts unchanged. Do not impose
  `cargo xtask validate` (full E2E) as a blanket worker gate. Required focused
  proof, hook checks and pre-PR confidence remain as the project's normal
  lifecycle requires; no gate policy/timeout changes are proposed.

## Risk checks

- Preserve current `try_exists`/`remove_dir_all`: final site symlink removal
  does not delete its referent; a dangling final symlink remains and reaches the
  bundler; regular file at site fails; filesystem errors stop before bundling.
  Symlinked ancestor `target` remains followed; do not claim physical
  containment or add hardening.
- Clear exactly `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`,
  `GIT_OBJECT_DIRECTORY`, `GIT_COMMON_DIR`, and `GIT_NAMESPACE` before root
  selection and child commands.
- Keep `cargo csr-bundle` checked-out source semantics; do not substitute PATH
  devtool or duplicate the bundler.
- Preserve parent Shell root state, host artifact lock ownership, debug/release
  profile mapping, Nix producer, bundle output, and parent failure behavior.
- No static-family, TS/ERT, Clippy/formatter, status-as-data coverage, hooks, CI
  topology, or merge policy changes. No ADR is expected unless implementation
  uncovers a genuinely durable new architecture decision.
