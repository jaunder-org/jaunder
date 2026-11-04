# Replace standalone CSR bundle build orchestration with Just

## Outcome

Replace the standalone `cargo xtask build-csr [--release]` command and its Rust
orchestration with the sole standalone caller `just build-csr [debug|release]`.
Reuse that one recipe from the existing host-server preparation caller. Delete
the xtask command, dispatch arm, and module; do not retain an alias or duplicate
implementation.

This is the first bounded deletion pilot under #1859, not completion of
replacing xtask. Preserve the user's direction: “To be clear: I want the needs
of fulfilling #1859 (replacing or at least dramatically simplifying
`cargo xtask`) to inform the choices for what things get carved off as
individual pieces in #1864”. Do not change #1864 TS/ERT/static work.

## Approval provenance and status

Human-approved design: Manager request `e0d243bd-007d-40cc-b670-5e6e6fe35820` /
`issue1859-build-csr-spec-v1`; exact answer: “Approve design and Just pin”. This
spec records that approved design. The outline/execution approval is separate:
Manager request `f233afc9-3b25-4b1f-8857-1e373605d628` /
`issue1872-build-csr-outline-v1`; exact answer: “Approve outline and begin”,
qualified to a **CLEAN execution branch**. The Manager has acknowledged both
approvals for this assignment. The newly filed pilot issue is #1872, linked as a
child of #1859; neither approval is merge approval.

## Caller and profile contract

- `just build-csr` selects `debug` by default.
- `just build-csr release` selects `release`; reject other values before
  filesystem mutation.
- Debug runs `cargo build --locked -p csr --target wasm32-unknown-unknown`;
  release adds `--release` (`xtask/src/steps/build_csr.rs:28–42,58–62`).
- The existing internal caller is `HostArtifacts::prepare_interruptible`
  (`xtask/src/steps/host_server.rs:109–156`). Preserve its profile mapping
  (`:41–69`), existing host-artifact lock, and parent `Shell` repository-root
  handoff (`build_csr.rs:43–48`; `host_server.rs:158–166`); do not reacquire its
  lock recursively. Ordinary e2e-local's Nix `csrDebugBundle` path remains
  unchanged (`host_server.rs:41–57,130–155`).

## Root, environment, cleanup, and failure contract

Resolve the same Git top-level as `git::toplevel` (`xtask/src/git.rs:971–973`)
and anchor all paths there. Before root discovery and inherited child execution,
unset `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`,
`GIT_COMMON_DIR`, and `GIT_NAMESPACE`, matching `xtask/src/git.rs:14–31` and
`scripts/cargo-xtask:20–21`. Ambient `GIT_DIR`/`GIT_WORK_TREE` must not redirect
cleanup or Cargo execution.

After the wasm build succeeds, inspect and remove only the fixed
`<root>/target/site` if present, preserving current
`try_exists`/`remove_dir_all` behavior (`build_csr.rs:72–101`): remove a final
`site` symlink whose target exists without deleting its referent; leave a
dangling final symlink untouched because `try_exists` reports absence, then run
the bundler; fail rather than silently deleting a regular file at `site`;
inspection/removal errors fail before bundling. Preserve existing ancestor
behavior: a symlinked `target` is followed. This fixed-path rule is **not** a
physical-containment guarantee. Do not add ancestor-symlink hardening,
caller-selected cleanup paths, or fallback outputs.

Then invoke the checked-out postprocessor, not PATH `devtool`:
`cargo run --locked --manifest-path tools/devtool/Cargo.toml -- csr-bundle --wasm <root>/target/wasm32-unknown-unknown/<profile>/csr.wasm --out <root>/target/site`
(`build_csr.rs:15–26,103–112`). This remains the same shared bundler Nix uses
(`nix/packages.nix:909–923`). A stage failure returns nonzero and prevents every
later stage; the host-server parent still fails and does not start consumers on
recipe failure.

Success retains `target/site/manifest.json` (build-only), `index.html`, and
`pkg/**` (`build_csr.rs:7–9`). Retire xtask's per-phase JSON records; Just
command output and honest exit status are sufficient, with the existing parent
e2e-local result retained.

## Just dependency and exact deletion

Pin Just in the shared dev/CI shell because the one recipe replaces both a
standalone Rust command and the internal host-server call. This is justified by
deleting the full Rust orchestrator, not by adding a wrapper over it. No runtime
package resolution (`npx`) or second build implementation.

Remove `Command::BuildCsr`/help and `command_name()` from
`xtask/src/cli.rs:345–355,777`; its dispatch arm from
`xtask/src/dispatch.rs:413–419`; `pub mod build_csr` from `xtask/src/lib.rs:54`;
and all of `xtask/src/steps/build_csr.rs`. Replace the internal
`host_server.rs:151–156` module call with the same recipe, preserving root
handoff/profile/lock. Update references in `nix/packages.nix:911`,
`nix/dev-shells.nix:82`, `CONTRIBUTING.md:385,656,1586,2128`,
`docs/ARCHITECTURE.md:1938–1940`, and `server/src/site.rs:557`. No
`cargo xtask build-csr` alias remains.

## Acceptance

1. Both profile invocations use exact locked Cargo arguments; invalid profile
   fails before mutation.
2. Nested-directory invocation resolves the intended checkout despite hostile
   repository-redirecting Git variables; children inherit the scrubbed
   environment.
3. Focused filesystem proof confirms: removing a final `site` symlink leaves its
   referent intact; a dangling final symlink remains and is passed to the
   bundler because `try_exists` reports absence; a regular file at `site` fails
   without deletion; symlinked ancestor `target` retains existing followed
   behavior; inspection/removal failure prevents bundler invocation. No
   physical-containment promise or new hardening.
4. Output paths/content match the current bundle and use checked-out devtool
   source. Every stage failure returns nonzero and stops later work.
5. Host-server caller preserves configured debug/release choice, existing lock,
   parent-root handoff, and failure-before-server behavior; ordinary Nix debug
   bundle path remains unchanged.
6. All old CLI/module/dispatch references are removed; docs and shell comments
   point to Just; Just is pinned and available in dev/CI.

## Non-goals

No test-local or e2e-local lifecycle redesign, Nix CSR producer change,
static-family/TS/ERT/Clippy/formatter/coverage migration, hook/CI topology
change, generic observer/helper framework, ADR by default, claim/start/close of
#1859, or merge approval.

## References

Parent/exploration: [#1859](https://github.com/jaunder-org/jaunder/issues/1859);
public exploratory correction:
[issue comment](https://github.com/jaunder-org/jaunder/issues/1859#issuecomment-6558394198);
pilot: [#1872](https://github.com/jaunder-org/jaunder/issues/1872). The review
artifact `.xtask/issue-1859-build-csr-first-migration-proposal.md` is local
ignored evidence, not a public URL.
